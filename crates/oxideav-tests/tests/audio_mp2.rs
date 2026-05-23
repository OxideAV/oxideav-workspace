//! MP2 (MPEG-1 Audio Layer II) decode comparison tests against ffmpeg.
//!
//! **Suspended for the clean-room rebuild.** `oxideav-mp2` was reset to a
//! register-only scaffold on 2026-05-24 under Hat-3 cold enforcement (its
//! prior bit-allocation / synthesis tables had external-library provenance
//! that could not be defended as clean-room). The decode-vs-ffmpeg harness
//! will be restored once the crate re-grows a decoder against the staged
//! ISO/IEC 11172-3 / 13818-3 specification.

/// Placeholder so the umbrella test crate keeps compiling while
/// `oxideav-mp2` is a scaffold. Re-flesh this with the ffmpeg
/// decode-comparison harness when the decoder lands.
#[test]
fn mp2_decode_comparison_suspended_pending_cleanroom_rebuild() {
    eprintln!("skip: oxideav-mp2 is a clean-room rebuild scaffold (no decoder yet)");
}
