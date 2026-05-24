//! AAC roundtrip comparison tests against ffmpeg.
//!
//! **Suspended for the clean-room rebuild.** `oxideav-aac` was reset to a
//! register-only scaffold on 2026-05-24 under Hat-3 cold enforcement (its
//! prior encoder-source carried a comment describing matching FFmpeg's
//! AAC encoder behaviour by citing its source file — clean-room violation
//! per docs/IMPLEMENTOR_ROUND.md). The ffmpeg roundtrip harness will be
//! restored once the crate re-grows a codec against the staged ISO/IEC
//! 14496-3 / 13818-7 specifications.

/// Placeholder so the umbrella test crate keeps compiling while
/// `oxideav-aac` is a scaffold. Re-flesh this with the ffmpeg
/// roundtrip harness when the codec lands.
#[test]
fn aac_codec_comparison_suspended_pending_cleanroom_rebuild() {
    eprintln!("skip: oxideav-aac is a clean-room rebuild scaffold (no codec yet)");
}
