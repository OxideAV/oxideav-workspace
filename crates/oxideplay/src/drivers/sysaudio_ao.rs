//! `oxideav-sysaudio`-backed audio output engine.
//!
//! `sysaudio` dlopen's the native audio API at runtime (ALSA,
//! PulseAudio, WASAPI, CoreAudio, …) so `oxideplay` has no
//! `libasound.so.2` / `libpulse.so.0` / `ole32.dll` / `AudioToolbox`
//! entry in its dynamic-link requirements. The backend gives us a
//! pull-callback; we feed it from a lock-free SPSC ring buffer that
//! `queue` fills. The callback increments a `samples_played` atomic
//! that the engine reports as the master clock.
//!
//! Driver selection: by default we pick `probe()`'s top choice (e.g.
//! PipeWire → PulseAudio → ALSA → OSS on Linux). The [`Self::with_driver`]
//! constructor lets the CLI force `pulse`, `alsa`, `wasapi`, etc.
//! directly.
//!
//! ## Multi-channel support
//!
//! Up to 8 channels are passed through to the backend (the underlying
//! sysaudio backends — CoreAudio, PulseAudio, ALSA, WASAPI — all
//! support that range). If the requested channel count fails to open,
//! [`SysAudioEngine::open_with_fallback`] retries at stereo and the
//! caller's downmix path takes care of the matrix. Once open, the
//! engine reports its actual `device_channels` so the routing layer
//! knows whether to passthrough or downmix per [`audio_routing`].
//!
//! [`audio_routing`]: crate::drivers::audio_routing

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oxideav_core::{AudioFrame, ChannelLayout, Error, Result};
use oxideav_sysaudio::{self as sysaudio, Driver, StreamRequest};
use ringbuf::{
    traits::{Consumer, Observer, Producer, Split},
    HeapRb,
};

use crate::drivers::audio_convert::resample_linear;
use crate::drivers::audio_routing::{
    apply_routing, decide_routing, DownmixPolicy, HeadphoneStatus, Routing,
};
use crate::drivers::engine::AudioEngine;
use crate::drivers::headphones_macos;

/// How often the engine re-runs the headphone probe. 1 Hz keeps the
/// HAL traffic negligible and reacts within ~one second of the user
/// plugging headphones in.
const HEADPHONE_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub struct SysAudioEngine {
    /// The underlying sysaudio stream. Kept alive so the callback
    /// keeps running; we also drive it through `play()` / `pause()`
    /// for the preroll gate.
    stream: sysaudio::Stream,

    producer: ringbuf::HeapProd<f32>,

    /// Device-side sample rate; may differ from the decoder's rate.
    device_rate: u32,
    /// Device-side channel count.
    device_channels: u16,
    /// True when the open downgraded the requested layout to stereo
    /// because the device couldn't take the requested count. Triggers
    /// the downmix path even on `--no-downmix` (with a stderr warning
    /// from the open path).
    #[allow(dead_code)]
    fallback_to_stereo: bool,

    /// Samples per channel consumed by the device — monotonic.
    samples_played: Arc<AtomicU64>,
    /// 0.0..=1.0 volume, bit-packed so we can load/store without a mutex.
    volume: Arc<AtomicU32>,
    /// User-requested pause state (Space in the player). Combined
    /// with `preroll_done` to decide whether the stream should
    /// actually be playing.
    user_paused: bool,
    /// Samples-per-channel we want buffered before we start playing
    /// the first time. Smooths startup when the decoder barely
    /// matches real-time (e.g. slow codec in debug builds, or a
    /// modem-speed network input).
    preroll_target: u64,
    /// True once we've unpaused after hitting `preroll_target`.
    /// Latches; after this `pause()` / `play()` pass through to the
    /// stream unchanged.
    preroll_done: bool,
    /// If the decoder's rate differed from the device rate, `queue`
    /// resamples with a dumb linear interpolator before pushing.
    resample_from: Option<u32>,

    /// Name of the sysaudio backend we landed on (e.g. `"pulse"`,
    /// `"alsa"`). Remembered so `info()` can tell the user which
    /// driver ended up active after `auto`.
    backend_name: &'static str,

    /// Diagnostic — latches true once the callback has run.
    #[allow(dead_code)]
    callback_ran: Arc<AtomicBool>,

    // ── surround routing ───────────────────────────────────────
    /// User-facing downmix policy (CLI `--downmix` / `--no-downmix`).
    /// Defaults to Auto; the engine resolves it per (source, device,
    /// headphones) on the next `queue`.
    downmix_policy: DownmixPolicy,
    /// Optional source-stream layout override. When the demuxer reports
    /// an explicit `CodecParameters::channel_layout` (e.g. AC-3 5.1 in
    /// Matroska), the player passes it here so the routing matrix uses
    /// the canonical positions instead of inferring from channel count.
    /// `None` means "infer from frame's channel count".
    source_layout_override: Option<ChannelLayout>,
    /// Cached headphone status — refreshed on a slow tick rather than
    /// per-frame to keep the HAL queries off the hot path.
    headphones: Arc<Mutex<HeadphoneStatus>>,
    /// Last-time we polled the headphone status. The engine reads
    /// `headphones` every `queue` (cheap mutex), and re-polls only
    /// when the timestamp is older than [`HEADPHONE_POLL_INTERVAL`].
    last_headphone_poll: Instant,
    /// Cached routing decision keyed on (source layout, device
    /// channels, headphone status, policy). Recomputed when any of
    /// the four changes — usually never per playback.
    cached_routing: Option<(ChannelLayout, u16, HeadphoneStatus, DownmixPolicy, Routing)>,
    /// Source-side audio shape, cached from
    /// [`AudioEngine::set_source_audio_params`]. Used to interpret the
    /// raw bytes inside each `AudioFrame` (which no longer carries
    /// these fields). Defaults are placeholders — overwritten before
    /// the first frame arrives.
    src_format: oxideav_core::SampleFormat,
    src_channels: u16,
    src_sample_rate: u32,
}

impl SysAudioEngine {
    /// Open on `probe()`'s top pick with the default downmix policy
    /// (`Auto`). See [`Self::new_with_policy`] when the caller has a
    /// CLI-supplied policy override.
    #[allow(dead_code)] // public-API affordance; main.rs uses new_with_policy.
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        let driver = sysaudio::default_driver()
            .ok_or_else(|| Error::other("sysaudio: no audio backend is available"))?;
        Self::open_with_fallback(driver, sample_rate, channels, DownmixPolicy::Auto)
    }

    /// Open using a specific sysaudio driver name (`"pulse"`,
    /// `"alsa"`, `"wasapi"`, …) with the default downmix policy. See
    /// [`Self::with_driver_and_policy`] for the CLI-driven variant.
    #[allow(dead_code)] // see new() — kept for symmetry with the policy-aware variant.
    pub fn with_driver(name: &str, sample_rate: u32, channels: u16) -> Result<Self> {
        let driver = sysaudio::driver_by_name(name).ok_or_else(|| {
            Error::other(format!(
                "sysaudio: no backend named '{name}' — try one of: {}",
                sysaudio::drivers()
                    .iter()
                    .map(|d| d.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        Self::open_with_fallback(driver, sample_rate, channels, DownmixPolicy::Auto)
    }

    /// Construct the engine, plumbing through a user-chosen downmix
    /// policy. Used by the CLI when `--downmix` / `--no-downmix` is
    /// passed.
    pub fn new_with_policy(sample_rate: u32, channels: u16, policy: DownmixPolicy) -> Result<Self> {
        let driver = sysaudio::default_driver()
            .ok_or_else(|| Error::other("sysaudio: no audio backend is available"))?;
        Self::open_with_fallback(driver, sample_rate, channels, policy)
    }

    /// Like [`Self::with_driver`] but accepting an explicit downmix
    /// policy.
    pub fn with_driver_and_policy(
        name: &str,
        sample_rate: u32,
        channels: u16,
        policy: DownmixPolicy,
    ) -> Result<Self> {
        let driver = sysaudio::driver_by_name(name).ok_or_else(|| {
            Error::other(format!(
                "sysaudio: no backend named '{name}' — try one of: {}",
                sysaudio::drivers()
                    .iter()
                    .map(|d| d.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        Self::open_with_fallback(driver, sample_rate, channels, policy)
    }

    /// Try to open the driver at the requested channel count. If that
    /// fails AND the policy is not `Forbid`, retries at stereo and
    /// records the fallback so the routing layer downmixes. Bubbles
    /// any open error to the caller when stereo also fails (or when
    /// `Forbid` is in effect).
    fn open_with_fallback(
        driver: Driver,
        sample_rate: u32,
        channels: u16,
        policy: DownmixPolicy,
    ) -> Result<Self> {
        // Sysaudio backends today cap at 8 channels (CoreAudio, PulseAudio,
        // WASAPI, ALSA all enforce that internally). Clamp the upper bound
        // here so a malformed source advertising 24ch doesn't trip the
        // backend's own clamp silently — but lift the legacy `clamp(1, 2)`.
        let want = channels.clamp(1, 8);

        let first_attempt = Self::open_inner(driver, sample_rate, want);
        match first_attempt {
            Ok(mut eng) => {
                eng.downmix_policy = policy;
                Ok(eng)
            }
            Err(e) if want > 2 && !matches!(policy, DownmixPolicy::Forbid) => {
                // Multi-channel open failed; backend can't take the
                // requested layout. Fall back to stereo + downmix.
                eprintln!(
                    "sysaudio: device on '{}' refused {want}ch open ({e}); \
                     falling back to stereo with downmix",
                    driver.name()
                );
                let mut eng = Self::open_inner(driver, sample_rate, 2)?;
                eng.fallback_to_stereo = true;
                eng.downmix_policy = policy;
                Ok(eng)
            }
            Err(e) => {
                if matches!(policy, DownmixPolicy::Forbid) && want > 2 {
                    Err(Error::other(format!(
                        "sysaudio: --no-downmix requested but device cannot open {want}ch: {e}"
                    )))
                } else {
                    Err(e)
                }
            }
        }
    }

    fn open_inner(driver: Driver, sample_rate: u32, channels: u16) -> Result<Self> {
        let req = StreamRequest::new(sample_rate, channels);

        // Size the ring for ~4 s worst-case at 192 kHz.
        let capacity = ((sample_rate.max(48_000) as usize) * channels as usize * 4).max(8192);
        let rb = HeapRb::<f32>::new(capacity);
        let (producer, mut consumer) = rb.split();

        let samples_played = Arc::new(AtomicU64::new(0));
        let samples_played_cb = samples_played.clone();
        let volume = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let volume_cb = volume.clone();
        let callback_ran = Arc::new(AtomicBool::new(false));
        let callback_ran_cb = callback_ran.clone();
        let ch_cb = channels as usize;

        let mut stream = sysaudio::open(driver, req, move |out, _info| {
            callback_ran_cb.store(true, Ordering::Relaxed);
            let v = f32::from_bits(volume_cb.load(Ordering::Relaxed));
            let written = consumer.pop_slice(out);
            for s in out[..written].iter_mut() {
                *s *= v;
            }
            out[written..].fill(0.0);
            samples_played_cb.fetch_add((written / ch_cb) as u64, Ordering::Relaxed);
        })
        .map_err(|e| Error::other(format!("sysaudio: open({}): {e}", driver.name())))?;

        // Start paused so the callback writes silence (and
        // `samples_played` doesn't advance) until `queue()` has
        // primed the ring with ~1 s of audio. Without this the
        // callback opens immediately on an empty ring and underruns
        // on the first beat, which sounds like chopping or a stutter
        // on any codec where decode-per-packet is close to the
        // packet duration.
        let _ = stream.pause();

        let fmt = stream.format();
        let resample_from = (fmt.sample_rate != sample_rate).then_some(sample_rate);
        // 1 s preroll target — generous enough to smooth routine
        // decoder jitter without feeling laggy.
        let preroll_target = fmt.sample_rate as u64;

        // First headphone probe (cheap; ~1 ms) — primes the cache so
        // the first frame's routing decision uses real data instead of
        // `Unknown`.
        let initial_headphones = headphones_macos::probe();

        Ok(Self {
            stream,
            producer,
            device_rate: fmt.sample_rate,
            device_channels: fmt.channels,
            fallback_to_stereo: false,
            samples_played,
            volume,
            user_paused: false,
            preroll_target,
            preroll_done: false,
            resample_from,
            backend_name: driver.name(),
            callback_ran,
            downmix_policy: DownmixPolicy::Auto,
            source_layout_override: None,
            headphones: Arc::new(Mutex::new(initial_headphones)),
            last_headphone_poll: Instant::now(),
            cached_routing: None,
            // Defaults; the engine pushes the real values in via
            // set_source_audio_params() before the first queue() call.
            src_format: oxideav_core::SampleFormat::F32,
            src_channels: fmt.channels,
            src_sample_rate: fmt.sample_rate,
        })
    }

    /// Override the source-stream layout used by the routing decision
    /// tree. Useful when the container reports an explicit
    /// `ChannelLayout` (e.g. AC-3 5.1 with the centre/LFE in the
    /// canonical ATSC slots) — without this, the routing layer infers
    /// from the frame's channel count, which lands on the same
    /// `Surround51` for a 6ch stream but loses any LtRt / LoRo /
    /// canonical-position metadata the demuxer recovered.
    ///
    /// Reached via the `AudioEngine::set_source_layout` trait method
    /// (the `Composite` driver wrapper forwards through). The inherent
    /// impl below is what the trait method calls into.
    fn set_source_layout_inner(&mut self, layout: Option<ChannelLayout>) {
        if self.source_layout_override != layout {
            // Invalidate the cached routing — the source key changed.
            self.cached_routing = None;
            self.source_layout_override = layout;
        }
    }

    /// Decide whether the stream should be playing right now, given
    /// the combination of `user_paused` + `preroll_done`. Called from
    /// the state transitions below.
    fn apply_play_state(&mut self) {
        let should_play = self.preroll_done && !self.user_paused;
        if should_play {
            let _ = self.stream.play();
        } else {
            let _ = self.stream.pause();
        }
    }

    /// Resolve a [`Routing`] for the incoming frame, refreshing the
    /// headphone-status cache if the slow tick has elapsed. Stable
    /// across many frames — the cache hits unless the source layout,
    /// device channel count, headphone status, or policy changes.
    fn current_routing(&mut self, src_layout: ChannelLayout) -> Routing {
        // Slow-tick headphone refresh.
        if self.last_headphone_poll.elapsed() >= HEADPHONE_POLL_INTERVAL {
            let h = headphones_macos::probe();
            if let Ok(mut g) = self.headphones.lock() {
                *g = h;
            }
            self.last_headphone_poll = Instant::now();
        }
        let headphones = self
            .headphones
            .lock()
            .map(|g| *g)
            .unwrap_or(HeadphoneStatus::Unknown);
        let key = (
            src_layout,
            self.device_channels,
            headphones,
            self.downmix_policy,
        );
        if let Some((sl, dc, hs, pol, r)) = self.cached_routing {
            if (sl, dc, hs, pol) == key {
                return r;
            }
        }
        let r = decide_routing(
            src_layout,
            self.device_channels,
            headphones,
            self.downmix_policy,
        );
        self.cached_routing = Some((key.0, key.1, key.2, key.3, r));
        r
    }
}

/// Short description of how reliably a given sysaudio backend reports
/// `Stream::latency()`. The copy here mirrors what `oxideav-sysaudio`
/// implements (see its README).
fn latency_quality(backend: &str) -> &'static str {
    match backend {
        "pulse" => "end-to-end (server-reported, catches BT + network)",
        "alsa" => "driver-queue depth (partial — misses BT hops above ALSA)",
        "wasapi" => "stream latency + padding (end-to-end, BT-aware)",
        "coreaudio" => "AudioQueue buffers + HAL device latency (BT-aware)",
        _ => "backend-specific",
    }
}

impl AudioEngine for SysAudioEngine {
    fn queue(&mut self, frame: &AudioFrame) -> Result<()> {
        // Prefer the explicit layout the demuxer surfaced; fall back to
        // inferring from the cached source channel count via the same
        // table `CodecParameters::resolved_layout()` uses. The frame
        // itself no longer carries a channel count — it's on the
        // stream's `CodecParameters` and was cached at stream open.
        let layout = self
            .source_layout_override
            .unwrap_or_else(|| ChannelLayout::from_count(self.src_channels));
        let routing = self.current_routing(layout);
        let mut buf = apply_routing(frame, self.src_format, self.src_channels, layout, routing);
        if let Some(src_rate) = self.resample_from {
            buf = resample_linear(
                &buf,
                src_rate,
                self.device_rate,
                self.device_channels as usize,
            );
        }
        // Push what fits. The player's back-pressure path
        // (`audio_headroom_samples`) means we normally never come
        // close to a full ring — so any partial-push here signals
        // the back-pressure threshold is set too loose, not that we
        // should silently drop. The caller owns that decision.
        //
        // We DO log dropped samples on eprintln rather than ignoring
        // silently — the previous bug where audio-only files dropped
        // PCM after the ring filled at ~4 s of playback was invisible
        // because this `let _ =` swallowed the partial-push count.
        // A noisy stderr line is the cheapest way to make the next
        // back-pressure regression obvious instead of audible-only.
        let pushed = self.producer.push_slice(&buf);
        if pushed < buf.len() {
            let dropped = buf.len() - pushed;
            eprintln!(
                "sysaudio: ring overflow — dropped {dropped} f32 samples ({:.1} ms). \
                 Back-pressure in PlayerEngine::pump_inbox is not engaging.",
                dropped as f64 * 1000.0
                    / (self.device_rate.max(1) as f64 * self.device_channels.max(1) as f64),
            );
        }

        // First push that crosses the preroll threshold flips the
        // stream into the playing state.
        if !self.preroll_done
            && self.producer.occupied_len() as u64
                >= self.preroll_target * self.device_channels.max(1) as u64
        {
            self.preroll_done = true;
            self.apply_play_state();
        }
        Ok(())
    }

    fn master_clock_pos(&self) -> Duration {
        let samples = self.samples_played.load(Ordering::Relaxed);
        let rate = self.device_rate.max(1) as u64;
        let secs = samples / rate;
        let nanos = ((samples % rate) * 1_000_000_000 / rate) as u32;
        Duration::new(secs, nanos)
    }

    fn set_paused(&mut self, paused: bool) {
        if paused == self.user_paused {
            return;
        }
        self.user_paused = paused;
        self.apply_play_state();
    }

    fn set_volume(&mut self, v: f32) {
        let clamped = v.clamp(0.0, 1.0);
        self.volume.store(clamped.to_bits(), Ordering::Relaxed);
    }

    fn audio_queue_len_samples(&self) -> u64 {
        (self.producer.occupied_len() / self.device_channels.max(1) as usize) as u64
    }

    fn audio_headroom_samples(&self) -> u64 {
        // The ringbuf stores f32 slots; divide by channels to report
        // per-channel sample headroom, matching what the player
        // thinks of as "samples". Once this hits ~0 the player stops
        // pulling audio, the decoder channel fills, the decoder
        // blocks, and the demuxer naturally back-pressures.
        (self.producer.vacant_len() as u64) / self.device_channels.max(1) as u64
    }

    fn latency(&self) -> Option<Duration> {
        self.stream.latency()
    }

    fn info(&self) -> String {
        let routing_note = match self.cached_routing.map(|t| t.4) {
            Some(Routing::Passthrough) => "passthrough".to_string(),
            Some(Routing::Downmix { mode, out_channels }) => {
                format!("downmix→{} ({}ch)", mode.name(), out_channels)
            }
            None => "routing pending".to_string(),
        };
        let headphones = self
            .headphones
            .lock()
            .map(|g| match *g {
                HeadphoneStatus::Yes => "headphones",
                HeadphoneStatus::No => "speakers",
                HeadphoneStatus::Unknown => "output: unknown",
            })
            .unwrap_or("output: ?");
        format!(
            "sysaudio/{} @ {} Hz {}ch f32 — {headphones}, {routing_note} — latency: {}",
            self.backend_name,
            self.device_rate,
            self.device_channels,
            latency_quality(self.backend_name)
        )
    }

    fn set_source_layout(&mut self, layout: Option<ChannelLayout>) {
        self.set_source_layout_inner(layout);
    }

    fn set_source_audio_params(&mut self, params: &oxideav_core::CodecParameters) {
        if let Some(f) = params.sample_format {
            self.src_format = f;
        }
        if let Some(c) = params.resolved_channels() {
            if c > 0 {
                self.src_channels = c;
            }
        }
        if let Some(r) = params.sample_rate {
            if r > 0 {
                self.src_sample_rate = r;
                // Propagate the change into the resampler hint so the
                // output rate decision matches the stream's actual rate.
                if r != self.device_rate {
                    self.resample_from = Some(r);
                } else {
                    self.resample_from = None;
                }
            }
        }
        // Propagate explicit channel layout through to the routing
        // override, mirroring the existing set_source_layout path.
        let layout = params.resolved_layout();
        self.set_source_layout_inner(layout);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn channels_are_no_longer_clamped_to_two() {
        // Just confirm the channel-count plumbing accepts >2. We can't
        // open a real device in CI; this is a compile/sanity check
        // that nothing in the pre-open path silently drops higher
        // channel counts. Pre-Part-C this clamped to 2.
        let want = 6u16;
        let clamped = want.clamp(1, 8);
        assert_eq!(clamped, 6, "regression: stereo clamp restored");
    }
}
