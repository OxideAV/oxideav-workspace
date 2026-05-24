//! FFV1 roundtrip comparison tests against ffmpeg.
//!
//! **Suspended for the clean-room rebuild.** `oxideav-ffv1` was reset to
//! a decode-frontier scaffold on 2026-05-18 under Hat-3 cold enforcement;
//! its prior `encoder::make_encoder` / `decoder::make_decoder` factories
//! no longer exist (the crate is rebuilding pixel reconstruction +
//! encoder against RFC 9043 from scratch). This ffmpeg roundtrip harness
//! will be restored once those public factories re-land.

/// Placeholder so the umbrella test crate keeps compiling while
/// `oxideav-ffv1` re-grows its decoder/encoder factories. Re-flesh this
/// with the ffmpeg roundtrip harness when they land.
#[test]
fn ffv1_roundtrip_suspended_pending_cleanroom_rebuild() {
    eprintln!("skip: oxideav-ffv1 encoder/decoder factories not yet re-implemented post-orphan");
}
