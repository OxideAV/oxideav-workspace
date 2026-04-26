//! Main-thread playback engine.
//!
//! Replaces the legacy `Player<D>` + `run_loop` pair: the
//! [`PlayerEngine`] consumes [`EngineMsg`]s produced by a
//! [`crate::job_sink::ChannelSink`] running on the executor's
//! mux thread, drives A/V sync, applies the TUI key bindings
//! (pause / seek / volume / quit), and presents frames to the
//! [`crate::driver::OutputDriver`].
//!
//! Pause is back-pressure: when paused, the engine stops draining
//! `frames_rx`, the channel fills, the executor's mux loop blocks
//! inside its `SyncSender::send`, the decode workers block in their
//! own sends, and the demuxer ultimately stalls. Resume = drain
//! again. Zero executor-side work needed.
//!
//! Seek goes through the executor's [`oxideav_pipeline::ExecutorHandle`]
//! — the engine bumps a local generation counter, calls `seek(...)`,
//! and discards every `Frame` arriving on `frames_rx` until a matching
//! `Barrier(SeekFlush { gen })` lands. The clock origin is then
//! re-anchored from the first post-barrier audio frame's pts.

use std::collections::VecDeque;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use oxideav::pipeline::{BarrierKind, ExecutorHandle};
use oxideav_core::{Error, Frame, MediaType, Result, StreamInfo, VideoFrame};

use crate::driver::{OutputDriver, OverlayState, PlayerEvent, SeekDir};
use crate::media_controls::{MediaCommand, MediaControls, PlaybackState, TrackInfo};
use crate::tui;

/// Soft cap on the number of decoded video frames buffered in the
/// engine's main-thread queue. Lifted verbatim from `player.rs`.
const VIDEO_QUEUE_SOFT_CAP: usize = 60;

/// How stale a decoded video frame can get before we skip it rather
/// than present it. Same constant as the legacy player.
const VIDEO_FRAME_MAX_BEHIND: Duration = Duration::from_millis(100);

/// Cross-thread message produced by [`crate::job_sink::ChannelSink`]
/// on the executor's mux thread, consumed by [`PlayerEngine::run`]
/// on the main thread.
pub enum EngineMsg {
    /// Sink lifecycle: streams are now known. Always the first
    /// message — the main thread uses this to size the audio /
    /// video output drivers before entering the engine loop.
    Started(Vec<StreamInfo>),
    /// One decoded frame ready for presentation. `kind` identifies
    /// the stream (audio / video / extras emitted by multi-port
    /// filters like spectrogram). Synthesised playback jobs use
    /// `MediaType::Unknown` — the engine dispatches on the `Frame`
    /// variant instead, so `kind` is informational only.
    Frame {
        #[allow(dead_code)]
        kind: MediaType,
        frame: Frame,
    },
    /// Flow barrier from the executor (today only `SeekFlush`).
    /// Engine drops in-flight buffers and re-anchors its clock.
    Barrier(BarrierKind),
    /// The executor's `finish()` has run; no more messages will
    /// follow.
    Finished,
}

/// Main-thread playback engine. Constructed after the first
/// [`EngineMsg::Started`] is observed (so the driver's audio +
/// video output sizes are already correct), runs until the user
/// quits or the executor reports finished.
pub struct PlayerEngine {
    driver: Box<dyn OutputDriver>,
    exec_handle: ExecutorHandle,
    frames_rx: Receiver<EngineMsg>,

    // ── streams ─────────────────────────────────────────
    audio_stream: Option<StreamInfo>,
    video_stream: Option<StreamInfo>,
    audio_rate: u32,

    // ── A/V sync ────────────────────────────────────────
    /// Decoded video frames pending presentation. Popped in
    /// pts-order as wallclock catches up.
    video_queue: VecDeque<VideoFrame>,
    /// Audio-pts duration of the *last* audio frame seen. Used as
    /// the master clock anchor.
    clock_origin: Duration,
    /// Driver master-clock samples at the moment of the last seek.
    clock_baseline_samples: u64,
    /// Cumulative wall-clock duration of audio queued to the
    /// driver. Adds up samples_played / sample_rate.
    last_audio_end: Duration,
    /// pts of the most recent video frame pushed into `video_queue`.
    last_video_pts: Option<i64>,
    /// pts of the most recent video frame actually presented.
    last_video_presented_pts: Option<i64>,

    // ── state ───────────────────────────────────────────
    paused: bool,
    volume: f32,
    /// Independent of `volume` — when `muted=true` the driver gets
    /// `set_volume(0.0)` but `volume` keeps the user's choice so
    /// unmute restores it. The egui overlay's mute toggle reads + writes
    /// this through `PlayerEvent::ToggleMute`.
    muted: bool,
    /// `Some(gen)` while a seek is mid-flight; cleared when the
    /// matching `Barrier(SeekFlush { gen })` lands.
    seek_pending_gen: Option<u32>,
    /// Local copy of the last gen we sent. Bumped in `apply_seek`.
    seek_gen_counter: u32,
    /// Sticky flag — set when the demuxer reports `Unsupported` on
    /// its first seek so subsequent UI seeks no-op silently.
    seek_supported: bool,
    /// True once the executor has reported `Finished`.
    executor_done: bool,

    // ── UI ──────────────────────────────────────────────
    tui_guard: Option<tui::TuiGuard>,
    last_status: Instant,
    duration: Option<Duration>,

    // ── system Now Playing widget ───────────────────────
    /// macOS Control-Center / lock-screen / Touch-Bar
    /// integration (no-op on every other platform, and on macOS
    /// when the `media-controls` cargo feature is off OR
    /// MediaPlayer.framework failed to load). The engine pushes
    /// state changes here and polls `take_command` after the
    /// driver's own event queue.
    media_controls: Box<dyn MediaControls>,
    /// One-shot: the engine's first iteration calls
    /// `media_controls.set_track(track)` once the driver is up.
    /// Setting it before then would race the driver init on
    /// macOS.
    track_info: TrackInfo,
    /// Last `PlaybackState` we pushed to the OS widget — used to
    /// suppress redundant `set_playback_state` calls on every
    /// tick (the engine's `paused` flag toggles at user-input
    /// rate, not per tick, but we want to be defensive).
    last_pushed_state: Option<PlaybackState>,
    /// True until the first `set_track` push has gone out.
    media_controls_pending_track: bool,
}

impl PlayerEngine {
    /// Construct the engine. The driver should already match the audio
    /// + video shape declared in `streams` (caller built it after the
    ///   first `EngineMsg::Started` arrived).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mut driver: Box<dyn OutputDriver>,
        exec_handle: ExecutorHandle,
        frames_rx: Receiver<EngineMsg>,
        streams: &[StreamInfo],
        duration: Option<Duration>,
        tui_guard: Option<tui::TuiGuard>,
        media_controls: Box<dyn MediaControls>,
        track_info: TrackInfo,
    ) -> Self {
        let audio_stream = streams
            .iter()
            .find(|s| s.params.media_type == MediaType::Audio)
            .cloned();
        let video_stream = streams
            .iter()
            .find(|s| s.params.media_type == MediaType::Video)
            .cloned();
        let audio_rate = audio_stream
            .as_ref()
            .and_then(|s| s.params.sample_rate)
            .unwrap_or(48_000);

        // Push the source's resolved channel layout into the driver so
        // surround-aware backends pick the right downmix matrix
        // (passthrough / LoRo / Binaural / etc.) before the first
        // frame arrives. `None` triggers the channel-count fallback in
        // the driver itself.
        let src_layout = audio_stream
            .as_ref()
            .and_then(|s| s.params.resolved_layout());
        driver.set_source_layout(src_layout);

        // Push the full per-stream `CodecParameters` so the driver can
        // cache sample format / channel count / pixel format / width /
        // height — those used to live on every Audio/VideoFrame, but
        // the slim moved them onto the stream's `CodecParameters`. The
        // driver consults its cache on each frame instead of reading
        // the (now slim) frame.
        if let Some(s) = audio_stream.as_ref() {
            driver.set_source_audio_params(&s.params);
        }
        if let Some(s) = video_stream.as_ref() {
            driver.set_source_video_params(&s.params);
        }

        Self {
            driver,
            exec_handle,
            frames_rx,
            audio_stream,
            video_stream,
            audio_rate,
            video_queue: VecDeque::new(),
            clock_origin: Duration::ZERO,
            clock_baseline_samples: 0,
            last_audio_end: Duration::ZERO,
            last_video_pts: None,
            last_video_presented_pts: None,
            paused: false,
            volume: 1.0,
            muted: false,
            seek_pending_gen: None,
            seek_gen_counter: 0,
            seek_supported: true,
            executor_done: false,
            tui_guard,
            last_status: Instant::now(),
            duration,
            media_controls,
            track_info,
            last_pushed_state: None,
            media_controls_pending_track: true,
        }
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if muted {
            self.driver.set_volume(0.0);
        } else {
            self.driver.set_volume(self.volume);
        }
    }

    /// Run the playback loop on the calling thread until the user
    /// quits or the executor reports finished. Owns the main thread's
    /// life — winit's macOS event loop must be pumped here.
    pub fn run(mut self) -> Result<()> {
        let tick_interval = Duration::from_millis(16);
        let status_interval = Duration::from_secs(1);

        loop {
            // 0a. First-time push of the cached track metadata to
            //     the OS Now Playing widget. Done lazily on the
            //     first iteration (rather than in `new`) so that
            //     the driver is fully initialised by then — on
            //     macOS the MediaPlayer.framework's
            //     defaultCenter expects a NSRunLoop, which winit
            //     spins up during its own setup.
            if self.media_controls_pending_track {
                self.media_controls.set_track(&self.track_info);
                let st = if self.paused {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Playing
                };
                self.media_controls.set_playback_state(st);
                self.last_pushed_state = Some(st);
                self.media_controls_pending_track = false;
            }

            // 0b. Publish the latest player state to the driver so
            //    the on-screen overlay (winit/egui) can render it
            //    BEFORE we poll its events — egui needs to know the
            //    current play / pause / seek state to draw the right
            //    icons before it processes the user's click on them.
            let state = self.overlay_state();
            self.driver.set_overlay_state(state);

            // 0c. Push the current position to the OS widget. The
            //     impl rate-limits internally so calling every
            //     tick is fine.
            self.media_controls.set_position(self.position());

            // 0d. Sync paused/playing state to the OS widget.
            //     `set_playback_state` is cheap when nothing
            //     changed (the impl compares against its cached
            //     state), but we still avoid the call when we
            //     haven't observed a transition.
            let want_state = if self.paused {
                PlaybackState::Paused
            } else {
                PlaybackState::Playing
            };
            if self.last_pushed_state != Some(want_state) {
                self.media_controls.set_playback_state(want_state);
                self.last_pushed_state = Some(want_state);
            }

            // 1. Gather events (driver + tui + OS Now Playing).
            let mut events = self.driver.poll_events();
            if self.tui_guard.is_some() {
                events.extend(tui::poll_events(Duration::ZERO));
            }
            // Pull every queued OS-side command (system media
            // keys, lock-screen scrub, Touch Bar) and translate
            // into PlayerEvents so the existing dispatch handles
            // them uniformly.
            while let Some(cmd) = self.media_controls.take_command() {
                match cmd {
                    MediaCommand::Play => {
                        if self.paused {
                            events.push(PlayerEvent::TogglePause);
                        }
                    }
                    MediaCommand::Pause => {
                        if !self.paused {
                            events.push(PlayerEvent::TogglePause);
                        }
                    }
                    MediaCommand::TogglePlayPause => {
                        events.push(PlayerEvent::TogglePause);
                    }
                    MediaCommand::Seek(secs) => {
                        let secs = secs.max(0.0);
                        events.push(PlayerEvent::SeekAbsolute(Duration::from_secs_f64(secs)));
                    }
                    // Next / Previous have no engine equivalent
                    // today (no playlist) — drop them. Reserved
                    // for a follow-up that wires playlists.
                    MediaCommand::Next | MediaCommand::Previous => {}
                }
            }
            let mut keep_going = true;
            for ev in events {
                if !self.apply_event(ev) {
                    keep_going = false;
                    break;
                }
            }
            if !keep_going {
                break;
            }

            // 2. Drain a bounded chunk of frames into our queues. If
            //    we're paused, skip — back-pressure naturally pins the
            //    executor.
            if !self.paused {
                self.pump_inbox()?;
            }

            // 3. Trim + present at most one video frame.
            if !self.paused {
                self.trim_video_queue();
                self.present_one_video_frame()?;
            }

            // 4. Status output.
            self.draw_status(status_interval);

            // 5. Exit conditions: executor done + audio drained.
            if self.executor_done && self.audio_drained() && !self.paused {
                break;
            }

            std::thread::sleep(tick_interval);
        }
        // Best-effort cleanup. The executor handle's Drop sets the
        // abort flag and the worker tears down in the background.
        self.exec_handle.request_abort();
        Ok(())
    }

    // ───────────────────── internals ─────────────────────

    /// Decide whether `pump_inbox` should still be draining frames.
    ///
    /// Back-pressure rule: stop draining when EITHER the audio ring or
    /// the video queue is at its soft cap. Stopping the drain causes
    /// the bounded `frames_rx` to fill, which blocks the executor's
    /// mux-thread sender, which back-pressures the decoders.
    ///
    /// The pre-engine `player.rs` path used
    /// `try_recv_subset(want_audio, want_video)` to pick a single stream's
    /// next frame and could throttle each independently. The new
    /// `ChannelSink` multiplexes both streams onto a single bounded
    /// channel so we can't pick selectively — stopping the whole drain
    /// when EITHER side is full is functionally equivalent: the channel
    /// fills, the sender blocks, the decoder pipeline blocks. Audio is
    /// the master clock and keeps draining via the device callback, so
    /// `audio_low` always recovers within ~250 ms once the device
    /// consumes a chunk.
    ///
    /// The pre-fix `audio_low && video_full` (AND) silently dropped PCM
    /// samples once the audio ring hit its 4 s capacity on audio-only
    /// files: with no video stream `video_full` is always false, so the
    /// AND was always false, so the pump never throttled audio. Symptom:
    /// scrambled audio that appeared ~4 s into playback on any file
    /// where the decoder outpaced realtime — exactly the rhmst.mod /
    /// halluc.mod breakage the user reported. See
    /// `tests::audio_only_back_pressure_engages_on_audio_low` below.
    pub(crate) fn should_throttle_drain(
        audio_headroom_samples: u64,
        audio_headroom_floor: u64,
        video_queue_len: usize,
        video_queue_cap: usize,
    ) -> bool {
        let audio_low = audio_headroom_samples < audio_headroom_floor;
        let video_full = video_queue_len >= video_queue_cap;
        audio_low || video_full
    }

    fn pump_inbox(&mut self) -> Result<()> {
        // Bound the per-tick inbox drain so we don't starve the
        // event loop when the executor is producing fast.
        const PER_TICK_BUDGET: usize = 32;
        for _ in 0..PER_TICK_BUDGET {
            let audio_headroom_floor = self.audio_rate as u64 / 4;
            if Self::should_throttle_drain(
                self.driver.audio_headroom_samples(),
                audio_headroom_floor,
                self.video_queue.len(),
                VIDEO_QUEUE_SOFT_CAP,
            ) {
                return Ok(());
            }

            let msg = match self.frames_rx.try_recv() {
                Ok(m) => m,
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(()),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.executor_done = true;
                    return Ok(());
                }
            };
            match msg {
                EngineMsg::Started(_) => {
                    // Already consumed by the entry point; ignore
                    // any duplicate Started messages defensively.
                }
                EngineMsg::Frame { kind: _, frame } => {
                    if self.seek_pending_gen.is_some() {
                        // Pre-barrier payload — drop it.
                        continue;
                    }
                    // Dispatch on the Frame variant, not the MuxTrack's
                    // declared kind: synthesised plain-playback uses
                    // `@display: {all: [...]}` which resolves to
                    // `MediaType::Unknown` while still emitting typed
                    // audio / video frames.
                    match frame {
                        Frame::Audio(af) => {
                            if af.sample_rate > 0 {
                                self.last_audio_end += Duration::from_secs_f64(
                                    af.samples as f64 / af.sample_rate as f64,
                                );
                            }
                            self.driver.queue_audio(&af)?;
                        }
                        Frame::Video(vf) => {
                            if let Some(p) = vf.pts {
                                self.last_video_pts = Some(p);
                            }
                            self.video_queue.push_back(vf);
                        }
                        _ => {}
                    }
                }
                EngineMsg::Barrier(BarrierKind::SeekFlush { generation }) => {
                    // Match against the most recent seek we issued.
                    // If they line up, clear pending + re-anchor; if
                    // not (stale barrier), forward through.
                    if Some(generation) == self.seek_pending_gen {
                        self.on_seek_landed();
                    }
                }
                EngineMsg::Finished => {
                    self.executor_done = true;
                }
            }
        }
        Ok(())
    }

    fn present_one_video_frame(&mut self) -> Result<()> {
        let now = self.position();
        let video_tb = self.video_stream.as_ref().map(|s| s.time_base);
        let epsilon = Duration::from_millis(50);
        if let Some(vf) = self.video_queue.front() {
            let pts_secs = match (vf.pts, video_tb) {
                (Some(p), Some(tb)) => tb.seconds_of(p),
                _ => 0.0,
            };
            let target = if pts_secs.is_finite() && pts_secs > 0.0 {
                Duration::from_secs_f64(pts_secs)
            } else {
                Duration::ZERO
            };
            if target <= now + epsilon {
                let vf = self.video_queue.pop_front().unwrap();
                self.last_video_presented_pts = vf.pts;
                self.driver.present_video(&vf)?;
            }
        }
        Ok(())
    }

    fn trim_video_queue(&mut self) {
        let Some(tb) = self.video_stream.as_ref().map(|s| s.time_base) else {
            return;
        };
        let now = self.position();
        while let Some(front) = self.video_queue.front() {
            let pts_secs = front.pts.map(|p| tb.seconds_of(p)).unwrap_or(0.0);
            let target = Duration::from_secs_f64(pts_secs.max(0.0));
            if target + VIDEO_FRAME_MAX_BEHIND < now {
                self.video_queue.pop_front();
            } else {
                break;
            }
        }
    }

    fn apply_event(&mut self, ev: PlayerEvent) -> bool {
        match ev {
            PlayerEvent::Quit => return false,
            PlayerEvent::TogglePause => {
                self.paused = !self.paused;
                self.driver.set_paused(self.paused);
            }
            PlayerEvent::SeekRelative(d, dir) => {
                if !self.seek_supported {
                    return true;
                }
                let cur = self.position();
                let target = match dir {
                    SeekDir::Forward => cur + d,
                    SeekDir::Back => cur.saturating_sub(d),
                };
                if let Err(e) = self.apply_seek(target) {
                    if let Error::Unsupported(_) = e {
                        self.seek_supported = false;
                    } else {
                        eprintln!("oxideplay: seek failed: {e}");
                    }
                }
            }
            PlayerEvent::SeekAbsolute(target) => {
                if !self.seek_supported {
                    return true;
                }
                if let Err(e) = self.apply_seek(target) {
                    if let Error::Unsupported(_) = e {
                        self.seek_supported = false;
                    } else {
                        eprintln!("oxideplay: seek failed: {e}");
                    }
                }
            }
            PlayerEvent::VolumeDelta(d) => {
                self.volume = (self.volume + (d as f32) / 100.0).clamp(0.0, 1.0);
                if !self.muted {
                    self.driver.set_volume(self.volume);
                }
            }
            PlayerEvent::SetVolume(v) => {
                self.volume = v.clamp(0.0, 1.0);
                if !self.muted {
                    self.driver.set_volume(self.volume);
                }
            }
            PlayerEvent::ToggleMute => {
                self.muted = !self.muted;
                if self.muted {
                    self.driver.set_volume(0.0);
                } else {
                    self.driver.set_volume(self.volume);
                }
            }
        }
        true
    }

    /// Build a fresh `OverlayState` snapshot from current engine state.
    /// Called every tick before pushing it to the driver.
    fn overlay_state(&self) -> OverlayState {
        let video_size =
            self.video_stream
                .as_ref()
                .and_then(|s| match (s.params.width, s.params.height) {
                    (Some(w), Some(h)) => Some((w, h)),
                    _ => None,
                });
        let codec_name = self
            .video_stream
            .as_ref()
            .map(|s| s.params.codec_id.to_string())
            .or_else(|| {
                self.audio_stream
                    .as_ref()
                    .map(|s| s.params.codec_id.to_string())
            });
        OverlayState {
            playing: !self.paused,
            position: self.position(),
            duration: self.duration,
            volume: self.volume,
            muted: self.muted,
            video_size,
            codec_name,
            seekable: self.seek_supported,
        }
    }

    fn apply_seek(&mut self, target: Duration) -> Result<()> {
        let (stream_idx, tb) = if let Some(v) = &self.video_stream {
            (v.index, v.time_base)
        } else if let Some(a) = &self.audio_stream {
            (a.index, a.time_base)
        } else {
            return Err(Error::unsupported("nothing to seek"));
        };
        let pts = (target.as_secs_f64() / tb.as_rational().as_f64()).round() as i64;
        // Drop pre-seek video buffers immediately; audio already
        // queued to the device will play out, and the seek-pending
        // discard takes care of fresh frames in flight.
        self.video_queue.clear();
        self.seek_gen_counter = self.seek_gen_counter.wrapping_add(1);
        self.seek_pending_gen = Some(self.seek_gen_counter);
        self.exec_handle.seek(stream_idx, pts, tb)
    }

    fn on_seek_landed(&mut self) {
        // Re-anchor the clock origin at the seek target. We don't
        // know the demuxer's exact landing pts here (the executor
        // doesn't surface it via the barrier), so the cleanest
        // re-anchor is to capture the next audio frame's pts —
        // which `last_audio_end` will start tracking afresh from
        // here. Until that lands, position() reads stale; it
        // self-corrects within ~one packet duration.
        self.clock_baseline_samples = self
            .driver
            .master_clock_pos()
            .as_secs_f64()
            .max(0.0)
            .mul_add(self.audio_rate as f64, 0.0) as u64;
        // For now, the clock origin stays at last_audio_end (which
        // accumulates from this point).
        self.clock_origin = self.last_audio_end;
        self.seek_pending_gen = None;
    }

    fn position(&self) -> Duration {
        let raw = self.driver.master_clock_pos();
        let base = Duration::from_secs_f64(
            self.clock_baseline_samples as f64 / self.audio_rate.max(1) as f64,
        );
        self.clock_origin + raw.saturating_sub(base)
    }

    fn audio_drained(&self) -> bool {
        if self.audio_stream.is_none() {
            return true;
        }
        self.last_audio_end > Duration::ZERO && self.driver.audio_queue_len_samples() == 0
    }

    fn draw_status(&mut self, status_interval: Duration) {
        let now = Instant::now();
        let snap = self.timings();
        let drift_str = tui::format_drift(self.position(), &snap);
        if now.duration_since(self.last_status) >= status_interval {
            if self.tui_guard.is_some() {
                let _ = tui::draw_status(
                    self.position(),
                    self.duration,
                    self.paused,
                    self.volume,
                    self.seek_supported,
                    Some(&drift_str),
                );
            } else {
                let dur = self
                    .duration
                    .map(tui::format_duration)
                    .unwrap_or_else(|| "?".into());
                eprintln!(
                    "oxideplay: {} / {}  vol {:>3}%  {}{}",
                    tui::format_duration(self.position()),
                    dur,
                    (self.volume * 100.0).round() as i32,
                    drift_str,
                    if self.paused { "  [paused]" } else { "" },
                );
            }
            self.last_status = now;
        } else if self.tui_guard.is_some() {
            let _ = tui::draw_status(
                self.position(),
                self.duration,
                self.paused,
                self.volume,
                self.seek_supported,
                Some(&drift_str),
            );
        }
    }

    fn timings(&self) -> tui::PlayerTimings {
        fn to_dur(pts: Option<i64>, s: Option<&StreamInfo>) -> Option<Duration> {
            let (p, s) = (pts?, s?);
            let secs = s.time_base.seconds_of(p);
            if secs.is_finite() && secs >= 0.0 {
                Some(Duration::from_secs_f64(secs))
            } else {
                None
            }
        }
        tui::PlayerTimings {
            audio: if self.last_audio_end > Duration::ZERO {
                Some(self.last_audio_end)
            } else {
                None
            },
            video_decoded: to_dur(self.last_video_pts, self.video_stream.as_ref()),
            video_presented: to_dur(self.last_video_presented_pts, self.video_stream.as_ref()),
            video_queue_len: self.video_queue.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin the back-pressure gate. Pre-fix this was `audio_low &&
    /// video_full`, so an audio-only file (`video_queue_len = 0`,
    /// `video_queue_cap = 60`) would NEVER throttle the drain even
    /// when the audio ring was completely full. Symptom: PCM samples
    /// silently dropped at the `producer.push_slice` site in
    /// `sysaudio_ao::queue`, which the user heard as scrambled audio
    /// starting ~4 s into playback (the moment the 4 s audio ring
    /// filled). The OR fix throttles whenever EITHER side is at its
    /// soft cap, restoring per-stream back-pressure semantics.
    #[test]
    fn audio_only_back_pressure_engages_on_audio_low() {
        // No video queue (audio-only file) + audio ring nearly empty
        // (1 sample of headroom, floor at 11025) → drain MUST throttle.
        assert!(
            PlayerEngine::should_throttle_drain(1, 11025, 0, 60),
            "audio-only path failed to throttle on audio_low — \
             back-pressure regression (the rhmst.mod / halluc.mod scramble bug)"
        );
        // Healthy headroom + empty video queue → drain freely.
        assert!(
            !PlayerEngine::should_throttle_drain(176_400, 11025, 0, 60),
            "engine throttled with plenty of headroom — would stall playback"
        );
    }

    #[test]
    fn video_only_back_pressure_engages_on_video_full() {
        // Video queue full + plenty of audio headroom (or no audio at
        // all — `audio_headroom_samples` returns u64::MAX in that case)
        // → throttle.
        assert!(PlayerEngine::should_throttle_drain(u64::MAX, 11025, 60, 60));
        assert!(!PlayerEngine::should_throttle_drain(
            u64::MAX,
            11025,
            10,
            60
        ));
    }

    #[test]
    fn mixed_audio_video_back_pressure_engages_on_either() {
        // Both full → throttle.
        assert!(PlayerEngine::should_throttle_drain(1, 11025, 60, 60));
        // Audio low, video healthy → throttle (audio side).
        assert!(PlayerEngine::should_throttle_drain(1, 11025, 10, 60));
        // Video full, audio healthy → throttle (video side).
        assert!(PlayerEngine::should_throttle_drain(176_400, 11025, 60, 60));
        // Both healthy → drain.
        assert!(!PlayerEngine::should_throttle_drain(176_400, 11025, 10, 60));
    }
}
