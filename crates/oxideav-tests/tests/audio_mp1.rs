//! MP1 (MPEG-1 Audio Layer I) decode comparison tests against ffmpeg.
//!
//! **Suspended for the clean-room rebuild.** `oxideav-mp1` was reset to a
//! register-only scaffold on 2026-05-24 under Hat-3 cold enforcement (its
//! prior 512-tap synthesis-window table had external-library provenance
//! that could not be defended as clean-room). The decode-vs-ffmpeg harness
//! will be restored once the crate re-grows a decoder against the staged
//! ISO/IEC 11172-3 Layer I specification.

/// Placeholder so the umbrella test crate keeps compiling while
/// `oxideav-mp1` is a scaffold. Re-flesh this with the ffmpeg
/// decode-comparison harness when the decoder lands.
#[test]
fn mp1_decode_comparison_suspended_pending_cleanroom_rebuild() {
    eprintln!("skip: oxideav-mp1 is a clean-room rebuild scaffold (no decoder yet)");
}
