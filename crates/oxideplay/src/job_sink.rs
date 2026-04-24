//! `JobSink` implementation that pushes decoded frames into whichever
//! video + audio engines are compiled in (via `Composite`).
//!
//! Scope (matches the oxideav-job "first cut" plan): fire-and-forget
//! playback — there is no pause / seek / volume-keyboard loop yet.
//! The user quits with Ctrl-C. A follow-up can thread the event loop
//! in parallel with the executor.

use std::time::Duration;

use oxideav::pipeline::JobSink;
use oxideav_core::{Error, Frame, MediaType, Packet, Result, StreamInfo};

use crate::driver::OutputDriver;

pub struct PlayerSink {
    driver: Option<Box<dyn OutputDriver>>,
    mute: bool,
    want_video: bool,
    /// Sample rate of the first audio stream (for back-pressure).
    audio_rate: u32,
    /// Cap how far the audio queue may run ahead of the speakers. If
    /// the driver buffer goes beyond this we sleep briefly in
    /// write_frame to throttle decoding.
    max_buffered: Duration,
}

impl PlayerSink {
    pub fn new(mute: bool, want_video: bool) -> Self {
        Self {
            driver: None,
            mute,
            want_video,
            audio_rate: 48_000,
            max_buffered: Duration::from_secs(2),
        }
    }

    fn driver_mut(&mut self) -> Result<&mut Box<dyn OutputDriver>> {
        self.driver
            .as_mut()
            .ok_or_else(|| Error::other("PlayerSink used before start()"))
    }
}

impl JobSink for PlayerSink {
    fn start(&mut self, streams: &[StreamInfo]) -> Result<()> {
        let mut sr = 48_000u32;
        let mut ch = 2u16;
        let mut video_dims: Option<(u32, u32)> = None;
        for s in streams {
            match s.params.media_type {
                MediaType::Audio => {
                    sr = s.params.sample_rate.unwrap_or(48_000);
                    ch = s.params.channels.unwrap_or(2);
                }
                MediaType::Video => {
                    if let (Some(w), Some(h)) = (s.params.width, s.params.height) {
                        video_dims = Some((w, h));
                    }
                }
                _ => {}
            }
        }
        if !self.want_video {
            video_dims = None;
        }
        self.audio_rate = sr.max(1);

        // Mirror the status block that a plain `oxideplay <file>`
        // prints — list the streams the sink will receive, then the
        // engines that will render them.
        eprintln!(
            "oxideplay: job sink @display started with {} stream(s)",
            streams.len()
        );
        for s in streams {
            match s.params.media_type {
                MediaType::Audio => eprintln!(
                    "  audio: {} {}ch @ {} Hz",
                    s.params.codec_id,
                    s.params.channels.unwrap_or(0),
                    s.params.sample_rate.unwrap_or(0)
                ),
                MediaType::Video => eprintln!(
                    "  video: {} {}x{}",
                    s.params.codec_id,
                    s.params.width.unwrap_or(0),
                    s.params.height.unwrap_or(0)
                ),
                _ => {}
            }
        }

        // `--job` has no --vo / --ao of its own yet; default to auto
        // selection, matching what a plain `oxideplay <file>` invocation
        // does.
        let mut d = crate::build_driver("auto", "auto", sr, ch, video_dims)?;
        if self.mute {
            d.set_volume(0.0);
        }
        let (vo_info, ao_info) = d.engine_info();
        match vo_info {
            Some(s) => eprintln!("  vo: {s}"),
            None => eprintln!("  vo: null (video disabled)"),
        }
        match ao_info {
            Some(s) => eprintln!("  ao: {s}"),
            None => eprintln!("  ao: null (audio disabled)"),
        }
        self.driver = Some(d);
        Ok(())
    }

    fn write_packet(&mut self, _kind: MediaType, _pkt: &Packet) -> Result<()> {
        Err(Error::unsupported(
            "oxideplay: @display sink needs decoded frames; remove `codec` or set it to the source codec with a decoder",
        ))
    }

    fn write_frame(&mut self, _kind: MediaType, frame: &Frame) -> Result<()> {
        let max_samples = (self.audio_rate as u64 * self.max_buffered.as_millis() as u64) / 1000;
        while self.driver_mut()?.audio_queue_len_samples() > max_samples {
            // Pump the windowing event loop while sleeping so the
            // video window stays responsive during audio back-pressure.
            let _ = self.driver_mut()?.poll_events();
            std::thread::sleep(Duration::from_millis(5));
        }
        let d = self.driver_mut()?;
        let r = match frame {
            Frame::Audio(a) => d.queue_audio(a),
            Frame::Video(v) => d.present_video(v),
            _ => Ok(()),
        };
        // Drain any pending windowing events. winit's event loop on
        // macOS strictly requires main-thread pumping, and the mux/sink
        // loop runs on the caller's thread — calling poll_events here
        // is what keeps the window alive under `--job` / `--inline`.
        let _ = self.driver_mut()?.poll_events();
        r
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(d) = self.driver.as_mut() {
            while d.audio_queue_len_samples() > 0 {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        Ok(())
    }
}
