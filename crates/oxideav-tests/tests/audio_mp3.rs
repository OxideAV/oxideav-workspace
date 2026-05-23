//! MP3 (MPEG-1/2 Audio Layer III) decode/encode comparison tests against ffmpeg.
//!
//! **Suspended for the clean-room rebuild.** `oxideav-mp3` was reset to a
//! register-only scaffold on 2026-05-24 under Hat-3 cold enforcement (its
//! prior decode tables and decode-loop structures were documented as
//! consulted from external reference implementations, which the clean-room
//! policy forbids regardless of those references' licensing). The
//! decode/encode-vs-ffmpeg harness will be restored once the crate
//! re-grows a codec against the staged ISO/IEC 11172-3 / 13818-3 Layer III
//! specification.

/// Placeholder so the umbrella test crate keeps compiling while
/// `oxideav-mp3` is a scaffold. Re-flesh this with the ffmpeg
/// comparison harness when the codec lands.
#[test]
fn mp3_codec_comparison_suspended_pending_cleanroom_rebuild() {
    eprintln!("skip: oxideav-mp3 is a clean-room rebuild scaffold (no codec yet)");
}
