//! `JobSink` implementation that pushes decoded frames into whichever
//! video + audio engines are compiled in (via `Composite`).
//!
//! Scope (matches the oxideav-job "first cut" plan): fire-and-forget
//! playback — there is no pause / seek / volume-keyboard loop yet.
//! The user quits with Ctrl-C. A follow-up can thread the event loop
//! in parallel with the executor.

use std::time::Duration;

use oxideav::pipeline::JobSink;
use oxideav_core::{Error, Frame, MediaType, Packet, Result, StreamInfo, TimeBase};

use crate::driver::{OutputDriver, PlayerEvent};

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
    /// Set once the user has asked the player to quit (via the window
    /// `q` key, window-close, etc.). Short-circuits subsequent
    /// write_frame calls and disables the finish() audio drain wait.
    quit_requested: bool,
    /// Total audio samples (per channel) we've pushed into the driver
    /// queue since start(). Combined with `driver.audio_queue_len_samples()`
    /// this gives us the currently-playing position:
    ///     played = total_queued - queue_len_samples
    total_audio_samples_queued: u64,
    /// pts of the first audio frame we saw, in that frame's time_base.
    /// Used as the zero-point for A/V-sync arithmetic so a non-zero
    /// start (seek / mid-stream join) doesn't break the compare.
    audio_base_pts: Option<i64>,
    /// time_base of the audio stream, captured from the first frame.
    /// Video frames arrive on the same time_base (per the spectrogram
    /// StreamFilter contract) so the sync compare is a direct pts diff
    /// scaled to samples via this tb's num/den.
    audio_time_base: Option<TimeBase>,
}

impl PlayerSink {
    pub fn new(mute: bool, want_video: bool) -> Self {
        Self {
            driver: None,
            mute,
            want_video,
            audio_rate: 48_000,
            max_buffered: Duration::from_secs(2),
            quit_requested: false,
            total_audio_samples_queued: 0,
            audio_base_pts: None,
            audio_time_base: None,
        }
    }

    fn driver_mut(&mut self) -> Result<&mut Box<dyn OutputDriver>> {
        self.driver
            .as_mut()
            .ok_or_else(|| Error::other("PlayerSink used before start()"))
    }

    fn events_include_quit(events: &[PlayerEvent]) -> bool {
        events.iter().any(|e| matches!(e, PlayerEvent::Quit))
    }

    /// Flip the quit flag and drop the output driver so the window
    /// goes away immediately. The pipeline shutdown that follows our
    /// `Err` return can take a moment (workers may be blocked inside
    /// bounded-channel sends until the abort propagates) — but by the
    /// time that finishes we no longer own a visible window or an
    /// audio stream.
    fn handle_quit(&mut self) {
        self.quit_requested = true;
        // Mute audio output queue then drop the driver: the
        // platform audio thread stops pulling samples, the window
        // closes, and any subsequent write_frame errors out cheaply
        // on `driver_mut()` rather than blocking in present/queue.
        if let Some(mut d) = self.driver.take() {
            d.set_volume(0.0);
            drop(d);
        }
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
        if self.quit_requested {
            // Keep telling the executor we're done. Returning Err each
            // call flips the abort flag; the pipeline tears down in the
            // background and we don't want to block the mux loop here.
            return Err(Error::other("oxideplay: quit requested"));
        }

        // Back-pressure: stall ingestion once the audio queue has more
        // than `max_buffered` of audio waiting.
        let max_samples = (self.audio_rate as u64 * self.max_buffered.as_millis() as u64) / 1000;
        while self.driver_mut()?.audio_queue_len_samples() > max_samples {
            let events = self.driver_mut()?.poll_events();
            if Self::events_include_quit(&events) {
                self.handle_quit();
                return Err(Error::other("oxideplay: quit requested"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        let r = match frame {
            Frame::Audio(a) => {
                // Capture the audio clock from the first frame so video
                // pts can be translated into the same sample-tick frame
                // of reference.
                if self.audio_base_pts.is_none() {
                    self.audio_base_pts = Some(a.pts.unwrap_or(0));
                    self.audio_time_base = Some(a.time_base);
                }
                let r = self.driver_mut()?.queue_audio(a);
                self.total_audio_samples_queued += a.samples as u64;
                r
            }
            Frame::Video(v) => {
                // A/V sync: hold the video frame until the audio clock
                // has reached its pts. Spectrogram emits video on the
                // input audio's time_base, so pts diff maps cleanly to
                // a sample count via `audio_time_base`.
                if let (Some(base), Some(tb), Some(pts)) =
                    (self.audio_base_pts, self.audio_time_base, v.pts)
                {
                    let target_samples = pts_to_samples(pts - base, tb, self.audio_rate);
                    loop {
                        let queued = self.total_audio_samples_queued;
                        let pending = self.driver_mut()?.audio_queue_len_samples();
                        let played = queued.saturating_sub(pending);
                        if played >= target_samples {
                            break;
                        }
                        let events = self.driver_mut()?.poll_events();
                        if Self::events_include_quit(&events) {
                            self.handle_quit();
                            return Err(Error::other("oxideplay: quit requested"));
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                self.driver_mut()?.present_video(v)
            }
            _ => Ok(()),
        };

        // Drain any pending windowing events. winit's event loop on
        // macOS strictly requires main-thread pumping, and the mux/sink
        // loop runs on the caller's thread — calling poll_events here
        // is what keeps the window alive under `--job` / `--inline`.
        let events = self.driver_mut()?.poll_events();
        if Self::events_include_quit(&events) {
            self.handle_quit();
            return Err(Error::other("oxideplay: quit requested"));
        }
        r
    }

    fn finish(&mut self) -> Result<()> {
        if self.quit_requested {
            // On quit we've already dropped the driver to close the
            // window promptly; skip the audio-drain wait.
            return Ok(());
        }
        if let Some(d) = self.driver.as_mut() {
            while d.audio_queue_len_samples() > 0 {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        Ok(())
    }
}

/// Convert a pts delta `p` under `tb` into a sample count at `rate`.
/// Returns 0 for negative inputs (can't target a past sample position).
fn pts_to_samples(p: i64, tb: TimeBase, rate: u32) -> u64 {
    if p <= 0 {
        return 0;
    }
    let num = (tb.0.num as u128).max(1);
    let den = (tb.0.den as u128).max(1);
    let rate = rate as u128;
    // samples = p * num * rate / den
    let s = (p as u128).saturating_mul(num).saturating_mul(rate) / den;
    s.min(u64::MAX as u128) as u64
}
