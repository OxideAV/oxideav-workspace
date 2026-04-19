//! cpal-backed audio output for the winit driver.
//!
//! cpal opens a pulled-callback output stream; we feed it from a
//! lock-free SPSC ring buffer that `queue_audio` fills. The callback
//! increments a `samples_played` atomic counter which the driver
//! reports as the audio master clock.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use oxideav_core::{AudioFrame, Error, Result};
use ringbuf::{
    traits::{Consumer, Observer, Producer, Split},
    HeapRb,
};

use crate::drivers::audio_convert::{resample_linear, to_f32_interleaved};

pub struct AudioOut {
    // Kept alive so the cpal callback keeps running. No direct access
    // to the inner data.
    _stream: cpal::Stream,

    producer: ringbuf::HeapProd<f32>,

    /// Device-side sample rate; may differ from the decoder's rate.
    /// When it does, `queue_audio` resamples before pushing.
    device_rate: u32,
    /// Device-side channel count (clamped to 1 or 2).
    device_channels: u16,

    /// Samples per channel consumed by the device. Grows monotonically
    /// while the stream is playing.
    samples_played: Arc<AtomicU64>,
    /// 0.0..=1.0 volume, bit-packed so we can load/store without a mutex.
    volume: Arc<AtomicU32>,
    /// Mirror of the paused state so we don't call `pause/play`
    /// repeatedly.
    paused: bool,
    /// If the decoder's rate differed from the device rate, we
    /// resample with a dumb linear interpolator.
    resample_from: Option<u32>,

    /// Latches `true` if the callback is ever invoked. Used nowhere
    /// critical; primarily a diagnostic hook.
    #[allow(dead_code)]
    callback_ran: Arc<AtomicBool>,
}

impl AudioOut {
    /// Open cpal's default output device. Prefers exact-match on
    /// (sample_rate, channels, F32); otherwise falls back to the
    /// device's default config and enables on-the-fly resampling.
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        let channels = channels.clamp(1, 2);
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| Error::other("cpal: no default output device"))?;

        let (config, device_rate, device_channels) = pick_config(&device, sample_rate, channels)?;
        let resample_from = (device_rate != sample_rate).then_some(sample_rate);

        // Ring-buffer sized to ~4 s of audio — the main-thread decode
        // pump tops it up every tick.
        let capacity = (device_rate as usize * device_channels as usize * 4).max(8192);
        let rb = HeapRb::<f32>::new(capacity);
        let (producer, mut consumer) = rb.split();

        let samples_played = Arc::new(AtomicU64::new(0));
        let samples_played_cb = samples_played.clone();
        let volume = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let volume_cb = volume.clone();
        let callback_ran = Arc::new(AtomicBool::new(false));
        let callback_ran_cb = callback_ran.clone();

        let err_fn = |e| eprintln!("oxideplay: cpal stream error: {e}");
        let ch = device_channels as usize;
        let data_cb = move |out: &mut [f32], _info: &cpal::OutputCallbackInfo| {
            callback_ran_cb.store(true, Ordering::Relaxed);
            let v = f32::from_bits(volume_cb.load(Ordering::Relaxed));
            let written = consumer.pop_slice(out);
            for s in out[..written].iter_mut() {
                *s *= v;
            }
            // Underrun = silence; don't leak stale stack memory.
            out[written..].fill(0.0);
            samples_played_cb.fetch_add((written / ch) as u64, Ordering::Relaxed);
        };

        let stream = device
            .build_output_stream(&config, data_cb, err_fn, None)
            .map_err(|e| Error::other(format!("cpal: build_output_stream: {e}")))?;
        stream
            .play()
            .map_err(|e| Error::other(format!("cpal: play: {e}")))?;

        Ok(Self {
            _stream: stream,
            producer,
            device_rate,
            device_channels,
            samples_played,
            volume,
            paused: false,
            resample_from,
            callback_ran,
        })
    }

    pub fn queue_audio(&mut self, frame: &AudioFrame) -> Result<()> {
        // 1. Normalise to f32 interleaved at the device channel count.
        let mut buf = to_f32_interleaved(frame, self.device_channels);
        // 2. Resample if rates disagree.
        if let Some(src_rate) = self.resample_from {
            buf = resample_linear(
                &buf,
                src_rate,
                self.device_rate,
                self.device_channels as usize,
            );
        }
        // 3. Push into the ring. If the ring is full we drop — SDL
        // behaves similarly (SDL_QueueAudio would succeed but the
        // output device would just keep playing what's already queued).
        // In practice the producer beats the consumer and there's
        // always room.
        let _ = self.producer.push_slice(&buf);
        Ok(())
    }

    pub fn master_clock_pos(&self) -> Duration {
        let samples = self.samples_played.load(Ordering::Relaxed);
        let rate = self.device_rate.max(1) as u64;
        let secs = samples / rate;
        let nanos = ((samples % rate) * 1_000_000_000 / rate) as u32;
        Duration::new(secs, nanos)
    }

    pub fn set_paused(&mut self, paused: bool) {
        if paused == self.paused {
            return;
        }
        if paused {
            let _ = self._stream.pause();
        } else {
            let _ = self._stream.play();
        }
        self.paused = paused;
    }

    pub fn set_volume(&mut self, v: f32) {
        let clamped = v.clamp(0.0, 1.0);
        self.volume.store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn audio_queue_len_samples(&self) -> u64 {
        // `occupied_len` counts f32 slots; divide by channel count to
        // match the "samples" unit the player expects.
        (self.producer.occupied_len() / self.device_channels.max(1) as usize) as u64
    }
}

/// Pick a `StreamConfig` closest to the caller's request. Returns
/// (config, actual_rate, actual_channels).
fn pick_config(
    device: &cpal::Device,
    want_rate: u32,
    want_channels: u16,
) -> Result<(StreamConfig, u32, u16)> {
    let supported = device
        .supported_output_configs()
        .map_err(|e| Error::other(format!("cpal: supported_output_configs: {e}")))?
        .collect::<Vec<_>>();

    // Prefer an exact (rate, channels, F32) match.
    for cfg in &supported {
        if cfg.sample_format() == SampleFormat::F32
            && cfg.channels() == want_channels
            && cfg.min_sample_rate().0 <= want_rate
            && cfg.max_sample_rate().0 >= want_rate
        {
            let cfg = (*cfg).with_sample_rate(cpal::SampleRate(want_rate));
            let (ch, rate) = (cfg.channels(), cfg.sample_rate().0);
            return Ok((cfg.config(), rate, ch));
        }
    }

    // Fallback: device default.
    let default = device
        .default_output_config()
        .map_err(|e| Error::other(format!("cpal: default_output_config: {e}")))?;
    if default.sample_format() != SampleFormat::F32 {
        return Err(Error::other(
            "cpal: default output is not f32 — not supported yet",
        ));
    }
    let ch = default.channels().clamp(1, 2);
    let rate = default.sample_rate().0;
    let mut cfg: StreamConfig = default.into();
    cfg.channels = ch;
    Ok((cfg, rate, ch))
}
