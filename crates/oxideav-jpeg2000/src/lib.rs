//! JPEG 2000 (ISO/IEC 15444) codec — scaffold.
//!
//! JPEG 2000 replaces baseline JPEG's 8×8 DCT with a multi-resolution
//! discrete wavelet transform (DWT — reversible 5/3 for lossless,
//! irreversible 9/7 for lossy), coded block-by-block through Tier-1
//! bit-plane arithmetic coding (MQ coder) and reorganised by
//! Tier-2 packet headers into quality/resolution layers.
//!
//! This crate is currently a **registration scaffold**: it reserves the
//! codec id `jpeg2000` so the aggregator surfaces the format in its
//! listing and so the full decoder / encoder can slot in later without
//! reshuffling the public surface. Both factories return
//! [`Error::Unsupported`].
//!
//! Follow-up implementation notes (for the landing PR):
//!
//! * Container: two wrappers coexist in the wild. `.j2k` is the raw
//!   codestream starting with SOC (`FF 4F`). `.jp2` is an ISOBMFF-style
//!   box wrapper (`00 00 00 0C 6A 50 20 20 0D 0A 87 0A`) that carries
//!   the codestream plus JP2 Colour Specification / Metadata boxes. A
//!   future `container` module should mirror the pattern used by
//!   `oxideav-mjpeg` for `.jpg`.
//! * Bitstream: DWT + MQ coder are both pure-math passes; a reasonable
//!   reference implementation is OpenJPEG (BSD-2) — port / re-derive.

use oxideav_codec::{CodecRegistry, Decoder, Encoder};
use oxideav_core::{CodecCapabilities, CodecId, CodecParameters, Error, Result};

/// Public codec id string. Matches the Cargo features `jpeg2000` / `jp2`
/// in the aggregator crate.
pub const CODEC_ID_STR: &str = "jpeg2000";

/// Register the JPEG 2000 decoder + encoder stubs.
pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("jpeg2000_stub")
        .with_lossy(true)
        .with_intra_only(true);
    reg.register_decoder_impl(CodecId::new(CODEC_ID_STR), caps.clone(), make_decoder);
    reg.register_encoder_impl(CodecId::new(CODEC_ID_STR), caps, make_encoder);
}

fn make_decoder(_params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    Err(Error::unsupported(
        "jpeg2000: decoder not yet implemented; see crate docs for the planned \
         DWT + MQ coder path",
    ))
}

fn make_encoder(_params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    Err(Error::unsupported(
        "jpeg2000: encoder not yet implemented; see crate docs for the planned \
         DWT + MQ coder path",
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
