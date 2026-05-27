//! GSM Full Rate (06.10) roundtrip comparison tests against ffmpeg.
//!
//! **Suspended for the clean-room rebuild.** `oxideav-gsm` was reset to a
//! register-only scaffold on 2026-05-25 under Hat-3 cold enforcement (its
//! prior tables had external-library provenance that could not be defended
//! as clean-room). The encode/decode-vs-ffmpeg harness will be restored
//! once the crate re-grows an encoder + decoder against the staged ETSI GSM
//! 06.10 specification. Tracked on the consolidated restoration task
//! (workspace task #1029) per the "neutralize-don't-abandon" rule.

/// Placeholder so the umbrella test crate keeps compiling while
/// `oxideav-gsm` is a scaffold. Re-flesh this with the ffmpeg
/// encode-decode-comparison harness when the encoder + decoder land.
#[test]
fn gsm_roundtrip_comparison_suspended_pending_cleanroom_rebuild() {
    eprintln!("skip: oxideav-gsm is a clean-room rebuild scaffold (no encoder/decoder yet)");
}
