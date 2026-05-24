//! Vorbis roundtrip comparison tests against ffmpeg.
//!
//! **Suspended for the clean-room rebuild.** `oxideav-vorbis` was reset
//! to a scaffold under Hat-3 cold enforcement; its prior
//! `encoder::make_encoder` factory no longer exists. This ffmpeg
//! roundtrip harness will be restored once the encoder re-lands against
//! the staged Vorbis I specification.

/// Placeholder so the umbrella test crate keeps compiling while
/// `oxideav-vorbis` is a scaffold. Re-flesh this with the ffmpeg
/// roundtrip harness when the encoder lands.
#[test]
fn vorbis_roundtrip_suspended_pending_cleanroom_rebuild() {
    eprintln!("skip: oxideav-vorbis encoder factory not yet re-implemented post-orphan");
}
