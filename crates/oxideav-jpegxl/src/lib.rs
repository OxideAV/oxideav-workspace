//! JPEG XL (JXL) codec — scaffold.
//!
//! JPEG XL is ISO/IEC 18181 (final specification 2022). It supersedes
//! classic JPEG with a modal design that separates a "VarDCT" mode
//! (spiritual successor to baseline JPEG, shares the DCT-based coding
//! path) from a "Modular" mode (grid-of-pixels prediction + MA-tree
//! entropy coding, friendly to lossless compression and alpha).
//!
//! This crate is currently a **registration scaffold**: it reserves the
//! codec id `jpegxl` in the registry so the aggregator crate reports it
//! in `--list-codecs` output and so a future implementation slots in
//! without restructuring the public surface. Both factories return
//! [`Error::Unsupported`] until the full decoder / encoder land.
//!
//! Follow-up implementation notes (for the eventual landing PR):
//!
//! * Container layer: JPEG XL ships either as a raw codestream (`.jxl`
//!   starting with `FF 0A`) or wrapped in an ISOBMFF-style box container
//!   (`.jxl` starting with `00 00 00 0C 4A 58 4C 20 0D 0A 87 0A`). The
//!   container belongs in an `oxideav-jpegxl`-owned `container` module
//!   mirroring the pattern `oxideav-mjpeg` now uses for `.jpg`.
//! * Bitstream: the VarDCT path reuses a variable-length DCT (8×8 up to
//!   256×256) plus Entropy Subtree Coding; the Modular path uses a
//!   Weighted + Gradient predictor + MA-tree Range coder. Pure Rust
//!   implementations exist (see `jxl-oxide`) and are a reasonable
//!   reference.

use oxideav_codec::{CodecRegistry, Decoder, Encoder};
use oxideav_core::{CodecCapabilities, CodecId, CodecParameters, Error, Result};

/// Public codec id string. Also matches the Cargo feature name `jxl` /
/// `jpegxl` used by the aggregator crate.
pub const CODEC_ID_STR: &str = "jpegxl";

/// Register the JPEG XL decoder + encoder stubs.
pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("jpegxl_stub")
        .with_lossy(true)
        .with_intra_only(true);
    reg.register_decoder_impl(CodecId::new(CODEC_ID_STR), caps.clone(), make_decoder);
    reg.register_encoder_impl(CodecId::new(CODEC_ID_STR), caps, make_encoder);
}

fn make_decoder(_params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    Err(Error::unsupported(
        "jpegxl: decoder not yet implemented; see crate docs for the planned \
         VarDCT + Modular paths",
    ))
}

fn make_encoder(_params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    Err(Error::unsupported(
        "jpegxl: encoder not yet implemented; see crate docs for the planned \
         VarDCT + Modular paths",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_reports_unsupported() {
        let mut reg = CodecRegistry::new();
        register(&mut reg);
        let params = CodecParameters::video(CodecId::new(CODEC_ID_STR));
        match reg.make_decoder(&params) {
            Err(Error::Unsupported(_)) => {}
            Err(other) => panic!("expected Error::Unsupported, got {other:?}"),
            Ok(_) => panic!("expected Error::Unsupported, got a live decoder"),
        }
    }
}
