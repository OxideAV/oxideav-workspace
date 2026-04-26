//! Output-driver abstraction.
//!
//! An `OutputDriver` is the thing that actually puts decoded audio + video
//! on the user's speakers / screen. The player core pushes frames to it,
//! polls events from it, and asks it what the current master-clock position
//! is (which is usually driven by the audio output rate).

use oxideav_core::{AudioFrame, ChannelLayout, Result, VideoFrame};
use std::time::Duration;

/// Snapshot of player state passed to the on-screen overlay each
/// frame. Plain data — no references back to the engine — so the
/// overlay can be swapped between drivers without rewiring lifetimes.
///
/// Wired up by the winit driver (egui paints on top of the wgpu
/// surface). The SDL2 driver ignores it; its default `set_overlay_state`
/// is a no-op. Without the `egui` feature compiled in nothing reads
/// these fields — silenced rather than gated to keep the trait surface
/// uniform across feature combinations.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct OverlayState {
    pub playing: bool,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub volume: f32,
    pub muted: bool,
    pub video_size: Option<(u32, u32)>,
    pub codec_name: Option<String>,
    /// True when the source has reported it can't seek (sticky after
    /// the first failure). The overlay greys out the seek bar.
    pub seekable: bool,
}

/// A user action emitted by the TUI, the window, or any other input surface.
///
/// The player merges these into a single event queue. `f32` payloads
/// (volume) deliberately drop the `Eq` derive that the original enum
/// carried — `Eq` on floats is a footgun and no consumer needs it.
///
/// `SeekAbsolute` / `SetVolume` / `ToggleMute` are only emitted by the
/// egui overlay (winit driver, `egui` feature). They're declared
/// unconditionally so `apply_event` stays exhaustive across feature
/// combos — `#[allow(dead_code)]` keeps -D warnings clean when those
/// variants are unused.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)]
pub enum PlayerEvent {
    /// Quit the player.
    Quit,
    /// Toggle paused / playing.
    TogglePause,
    /// Seek forward or backward by a duration.
    SeekRelative(Duration, SeekDir),
    /// Nudge the volume up or down (percentage points, -100..=100).
    VolumeDelta(i32),
    /// Seek to an absolute timestamp from the start of the file. Used
    /// by the egui overlay's seek bar — the existing `SeekRelative`
    /// can't express "go to 47% of the timeline" cleanly when the
    /// user drags the thumb across multiple frames.
    SeekAbsolute(Duration),
    /// Set the output volume to an absolute value in `[0.0, 1.0]`.
    /// Emitted by the overlay's volume slider.
    SetVolume(f32),
    /// Toggle muted state (overlay's speaker icon).
    ToggleMute,
}

/// Direction for relative seeks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeekDir {
    Forward,
    Back,
}

/// Abstraction over "the thing that draws frames + plays samples".
///
/// For the SDL2 implementation, both audio and video live in the same
/// driver. A future ALSA-only or wgpu driver could implement this trait
/// too without the player caring.
pub trait OutputDriver {
    /// Present one decoded video frame. A headless/audio-only driver
    /// should simply discard the frame.
    fn present_video(&mut self, frame: &VideoFrame) -> Result<()>;

    /// Queue a decoded audio frame for playback. The driver is expected
    /// to own its audio callback and consume this buffer as samples are
    /// needed by the output device.
    fn queue_audio(&mut self, frame: &AudioFrame) -> Result<()>;

    /// Poll any user-input events (window keys, gamepad buttons, etc.).
    fn poll_events(&mut self) -> Vec<PlayerEvent>;

    /// Current position of the master clock. Implementations should use the
    /// audio clock when available; otherwise a wall-clock-based monotonic
    /// counter is acceptable.
    fn master_clock_pos(&self) -> Duration;

    /// Pause / resume both audio and video output.
    fn set_paused(&mut self, paused: bool);

    /// Set the output volume (0.0 = mute, 1.0 = unity gain).
    fn set_volume(&mut self, vol: f32);

    /// Approximate number of samples currently buffered in the audio queue.
    /// The player uses this to throttle decoding so we don't run ahead of
    /// the output and starve memory.
    fn audio_queue_len_samples(&self) -> u64 {
        0
    }

    /// Remaining samples the audio backend can accept before it starts
    /// dropping. See `AudioEngine::audio_headroom_samples`.
    fn audio_headroom_samples(&self) -> u64 {
        u64::MAX
    }

    /// Per-engine one-liner descriptions for the startup banner —
    /// `(video, audio)`. `None` on either side means that engine is
    /// disabled (e.g. `--vo null`). Default returns `(None, None)`;
    /// `Composite` overrides to surface the real engines' `info()`.
    fn engine_info(&self) -> (Option<String>, Option<String>) {
        (None, None)
    }

    /// Push the latest player state to the driver so its overlay UI
    /// (egui inside the winit driver) can re-render on the next
    /// present. Called every engine tick. Drivers without an overlay
    /// just no-op.
    fn set_overlay_state(&mut self, _state: OverlayState) {}

    /// Tell the driver the source's authoritative speaker layout (from
    /// `CodecParameters::resolved_layout`). Audio backends that do
    /// surround-aware downmix consult this once at stream open;
    /// drivers without an audio path no-op. Default impl is a no-op
    /// so existing OutputDriver impls compile unchanged.
    fn set_source_layout(&mut self, _layout: Option<ChannelLayout>) {}
}

/// Blanket impl so `Box<dyn OutputDriver>` can stand in for a concrete
/// `D: OutputDriver` everywhere the player is generic. This lets `main`
/// pick between SDL2 and winit at runtime by building different
/// boxed drivers and passing them into the same `Player<Box<dyn _>>`.
impl<D: OutputDriver + ?Sized> OutputDriver for Box<D> {
    fn present_video(&mut self, frame: &VideoFrame) -> Result<()> {
        (**self).present_video(frame)
    }
    fn queue_audio(&mut self, frame: &AudioFrame) -> Result<()> {
        (**self).queue_audio(frame)
    }
    fn poll_events(&mut self) -> Vec<PlayerEvent> {
        (**self).poll_events()
    }
    fn master_clock_pos(&self) -> Duration {
        (**self).master_clock_pos()
    }
    fn set_paused(&mut self, paused: bool) {
        (**self).set_paused(paused)
    }
    fn set_volume(&mut self, vol: f32) {
        (**self).set_volume(vol)
    }
    fn audio_queue_len_samples(&self) -> u64 {
        (**self).audio_queue_len_samples()
    }
    fn audio_headroom_samples(&self) -> u64 {
        (**self).audio_headroom_samples()
    }
    fn engine_info(&self) -> (Option<String>, Option<String>) {
        (**self).engine_info()
    }
    fn set_overlay_state(&mut self, state: OverlayState) {
        (**self).set_overlay_state(state)
    }
    fn set_source_layout(&mut self, layout: Option<ChannelLayout>) {
        (**self).set_source_layout(layout)
    }
}
