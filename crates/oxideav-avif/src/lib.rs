//! AVIF (AV1 Image File Format) codec — scaffold.
//!
//! AVIF wraps one or more AV1 keyframes in an ISOBMFF "heif" container
//! (MIAF profile). The visual bitstream is AV1 proper, so a full
//! implementation ultimately delegates bitstream decoding to
//! `oxideav-av1` and only adds the container-side HEIF / `meta` / `ipco`
//! / `ipma` handling plus the colour / alpha auxiliary-image plumbing.
//!
//! This crate is currently a **registration scaffold**: it reserves the
//! codec id `avif` so the aggregator surfaces the format in its listing,
//! and lets a future implementation slot in without reshuffling the
//! public surface. Both factories return [`Error::Unsupported`].
//!
//! Follow-up implementation notes (for the landing PR):
//!
//! * Container: a new sibling crate or a `container` submodule should
//!   parse the HEIF meta-box layout (`ftyp` with `avif`/`avis` brands,
//!   `meta` → `iloc`/`iinf`/`iref`/`iprp`/`pitm`). Emit one packet per
//!   primary image + one per alpha auxl if present.
//! * Bitstream: reuse `oxideav-av1`'s keyframe path. The only AVIF-
//!   specific bits are ICC profile handling and colour-info boxes
//!   (`colr`) feeding `CodecParameters::pixel_format` / primaries.

use oxideav_codec::{CodecRegistry, Decoder, Encoder};
use oxideav_core::{CodecCapabilities, CodecId, CodecParameters, Error, Result};

/// Public codec id string. Matches the aggregator-crate Cargo feature `avif`.
pub const CODEC_ID_STR: &str = "avif";

/// Register the AVIF decoder + encoder stubs.
pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("avif_stub")
        .with_lossy(true)
        .with_intra_only(true);
    reg.register_decoder_impl(CodecId::new(CODEC_ID_STR), caps.clone(), make_decoder);
    reg.register_encoder_impl(CodecId::new(CODEC_ID_STR), caps, make_encoder);
}

fn make_decoder(_params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    Err(Error::unsupported(
        "avif: decoder not yet implemented; see crate docs for the planned \
         HEIF container + AV1 keyframe path",
    ))
}

fn make_encoder(_params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    Err(Error::unsupported(
        "avif: encoder not yet implemented; see crate docs for the planned \
         HEIF container + AV1 keyframe path",
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
