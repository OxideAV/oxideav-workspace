//! System "Now Playing" integration.
//!
//! Publishes the currently-playing track's title / artist / album /
//! artwork / position / duration to the operating system's media
//! widget (macOS Control Center / Touch Bar / lock-screen / headset
//! play-pause), and surfaces the system's media-key / overlay
//! commands back to the player engine as [`MediaCommand`]s.
//!
//! ## Platform status
//!
//! - **macOS** — implemented in [`macos`]. `MediaPlayer.framework`
//!   is loaded entirely at runtime via `libloading` so it never
//!   ends up in the binary's `LC_LOAD_DYLIB` list.
//! - **Linux (MPRIS over D-Bus)** — TODO (a follow-up round will add
//!   a runtime-loaded `libdbus-1.so.3` backend).
//! - **Windows (SMTC via combase.dll)** — TODO.
//!
//! On any non-macOS platform, or when the `media-controls` cargo
//! feature is off, [`build`] returns a [`NoopMediaControls`] — every
//! method is a no-op. The trait surface stays uniform so the engine
//! never has to `cfg!()` around it.
//!
//! ## Wiring
//!
//! - `oxideplay/main.rs` pre-opens the demuxer once on the main
//!   thread (it has to anyway, for the player engine), pulls
//!   `metadata()` + `attached_pictures()`, builds a [`TrackInfo`],
//!   constructs the [`MediaControls`] via [`build`], hands both to
//!   the engine.
//! - `PlayerEngine::run` calls `set_track` once at startup, then
//!   `set_playback_state` on pause / resume, `set_position` every
//!   tick, and polls `take_command` after the driver's own event
//!   queue.

use std::time::Duration;

#[cfg(all(feature = "media-controls", target_os = "macos"))]
pub mod macos;

/// Track metadata + cover art. Built from a `Demuxer`'s
/// `metadata()` + `attached_pictures()` (or hand-assembled from the
/// input filename when none of those are present, e.g. for a MOD
/// file with only a title).
///
/// All fields are owned (not borrowed) so the struct can cross the
/// `main` → `engine` ownership boundary without lifetime headaches.
/// `#[allow(dead_code)]` because the fields are read only by the
/// macOS impl (gated on `feature = "media-controls"`); without the
/// feature on every other platform the `NoopMediaControls` ignores
/// them.
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct TrackInfo {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Total track duration. Used by the OS widget to draw the seek
    /// bar — `None` produces a widget without one (live streams).
    pub duration: Option<Duration>,
    /// Raw image bytes (typically JPEG or PNG) suitable for handing
    /// to `NSImage initWithData:`. The mime type is informational —
    /// `NSImage` sniffs the format itself.
    pub artwork: Option<Artwork>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Artwork {
    pub mime_type: String,
    pub data: Vec<u8>,
}

/// Three-state player state mirroring `MPNowPlayingPlaybackState`.
/// (`Interrupted` / `Unknown` exist on macOS but neither is useful
/// from the engine, so we don't model them.)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Command coming back from the OS widget. The engine translates
/// these into its own `PlayerEvent`s.
#[derive(Copy, Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub enum MediaCommand {
    Play,
    Pause,
    TogglePlayPause,
    Next,
    Previous,
    /// Absolute position in seconds requested by the seek-bar
    /// scrubber (the OS widget always sends absolute, never
    /// relative).
    Seek(f64),
}

/// Trait the engine talks to. The macOS implementation publishes
/// to `MPNowPlayingInfoCenter`; the noop one drops everything.
///
/// Implementations are constructed once at startup (so the
/// dlopen + selector binding cost is paid once) and kept for the
/// lifetime of the player.
pub trait MediaControls: Send {
    /// Replace the OS widget's track metadata + artwork.
    fn set_track(&mut self, info: &TrackInfo);
    /// Notify the OS widget that the player is playing / paused /
    /// stopped. macOS animates the play-pause button + adjusts the
    /// seek-bar progress accordingly.
    fn set_playback_state(&mut self, state: PlaybackState);
    /// Update the OS widget's seek-bar position. Called once per
    /// engine tick (~60 Hz) — implementations should rate-limit
    /// internally if the underlying API can't take updates that
    /// fast.
    fn set_position(&mut self, elapsed: Duration);
    /// Non-blocking poll for an OS → player command (system
    /// media keys, Touch Bar buttons, lock-screen scrub).
    /// Returns `None` if no command is queued.
    fn take_command(&mut self) -> Option<MediaCommand>;
}

/// No-op implementation: returned by [`build`] on any platform
/// where the OS-side integration isn't compiled in (or where it
/// is, but loading the OS framework failed at startup).
pub struct NoopMediaControls;

impl MediaControls for NoopMediaControls {
    fn set_track(&mut self, _info: &TrackInfo) {}
    fn set_playback_state(&mut self, _state: PlaybackState) {}
    fn set_position(&mut self, _elapsed: Duration) {}
    fn take_command(&mut self) -> Option<MediaCommand> {
        None
    }
}

/// Construct the platform-appropriate [`MediaControls`]
/// implementation. Falls back to [`NoopMediaControls`] on every
/// platform that isn't covered, OR on the covered platform when
/// `MediaPlayer.framework` (or its dependencies) failed to load —
/// a CI runner or a sandboxed environment can hit that path and
/// shouldn't hard-fail the player.
pub fn build() -> Box<dyn MediaControls> {
    #[cfg(all(feature = "media-controls", target_os = "macos"))]
    {
        match macos::MacosMediaControls::new() {
            Ok(c) => return Box::new(c),
            Err(e) => {
                eprintln!(
                    "oxideplay: media-controls: macOS init failed ({e}); \
                     falling back to no-op (Now Playing won't update)"
                );
            }
        }
    }
    Box::new(NoopMediaControls)
}

/// Build a [`TrackInfo`] from the bits the demuxer surfaces. The
/// `fallback_title` is used when the container carries no title at
/// all (e.g. raw PCM, headerless WAV) — typically the input
/// filename's stem. `total_duration` is the player-engine's
/// already-derived total (from [`crate::derive_duration`]).
///
/// Picks the first attached picture as cover art on the assumption
/// that:
///   1. `oxideav-id3` / `-flac` / `-mp4` already promote the
///      `FrontCover` to index 0 when present;
///   2. when no `FrontCover` exists, *any* attached picture is
///      better than nothing.
pub fn track_info_from_demuxer(
    metadata: &[(String, String)],
    pictures: &[oxideav_core::AttachedPicture],
    fallback_title: Option<String>,
    total_duration: Option<Duration>,
) -> TrackInfo {
    let lookup = |key: &str| -> Option<String> {
        metadata.iter().find_map(|(k, v)| {
            if k.eq_ignore_ascii_case(key) {
                let trimmed = v.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            } else {
                None
            }
        })
    };
    let title = lookup("title").or(fallback_title);
    let artist = lookup("artist").or_else(|| lookup("album_artist"));
    let album = lookup("album");
    let artwork = pictures.first().map(|p| Artwork {
        mime_type: p.mime_type.clone(),
        data: p.data.clone(),
    });
    TrackInfo {
        title,
        artist,
        album,
        duration: total_duration,
        artwork,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{AttachedPicture, PictureType};

    #[test]
    fn noop_does_not_panic() {
        let mut c = NoopMediaControls;
        let info = TrackInfo {
            title: Some("hello".into()),
            ..Default::default()
        };
        c.set_track(&info);
        c.set_playback_state(PlaybackState::Playing);
        c.set_position(Duration::from_secs(1));
        assert!(c.take_command().is_none());
    }

    #[test]
    fn track_info_picks_up_metadata() {
        let meta = vec![
            ("title".to_string(), "Cyber".into()),
            ("artist".into(), "Author".into()),
            ("comment".into(), "ignored".into()),
        ];
        let info = track_info_from_demuxer(
            &meta,
            &[],
            Some("file_stem".into()),
            Some(Duration::from_secs(120)),
        );
        // Container title wins over the fallback.
        assert_eq!(info.title.as_deref(), Some("Cyber"));
        assert_eq!(info.artist.as_deref(), Some("Author"));
        assert_eq!(info.album, None);
        assert_eq!(info.duration, Some(Duration::from_secs(120)));
        assert!(info.artwork.is_none());
    }

    #[test]
    fn track_info_falls_back_to_filename() {
        let info = track_info_from_demuxer(&[], &[], Some("file_stem".into()), None);
        assert_eq!(info.title.as_deref(), Some("file_stem"));
    }

    #[test]
    fn track_info_carries_first_picture_as_artwork() {
        let pic = AttachedPicture {
            mime_type: "image/jpeg".into(),
            picture_type: PictureType::FrontCover,
            description: String::new(),
            data: vec![0xFF, 0xD8, 0xFF],
        };
        let info = track_info_from_demuxer(&[], std::slice::from_ref(&pic), None, None);
        let art = info.artwork.expect("artwork present");
        assert_eq!(art.mime_type, "image/jpeg");
        assert_eq!(art.data, vec![0xFF, 0xD8, 0xFF]);
    }

    #[test]
    fn track_info_skips_blank_metadata_values() {
        // MOD files routinely store " " as the title slot when the
        // tracker padded the field instead of leaving it null. Treat
        // that as "no title" so the fallback (filename) wins.
        let meta = vec![("title".into(), "   ".into())];
        let info = track_info_from_demuxer(&meta, &[], Some("fallback".into()), None);
        assert_eq!(info.title.as_deref(), Some("fallback"));
    }
}
