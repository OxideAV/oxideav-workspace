//! Apple ProRes codec — scaffold.
//!
//! Apple ProRes is a family of intermediate-codec variants designed for
//! broadcast post-production: high-bit-depth (10-bit / 12-bit) YUV 4:2:2
//! (and 4:4:4 for the RGBA-capable 4444 / 4444 XQ profiles), intra-only,
//! per-slice DCT coding, with a tightly specified rate-distortion
//! target per profile so editors can predict bitrate from duration.
//!
//! **Six profiles**: Proxy, LT, 422 (Standard), HQ, 4444, 4444 XQ. The
//! first four share the same core YUV 4:2:2 10-bit bitstream with
//! per-profile quantisation matrices and target bitrates; 4444 adds a
//! fourth alpha plane and YUV 4:4:4 support; 4444 XQ bumps the bitrate
//! target for mastering workflows.
//!
//! This crate is currently a **registration scaffold**: it reserves the
//! codec id `prores` in the registry so the aggregator reports it in
//! `oxideav list` output and so a future implementation slots in
//! without restructuring the public surface. Both factories return
//! [`Error::Unsupported`] until the decoder / encoder land.
//!
//! Follow-up implementation notes (for the eventual landing PR):
//!
//! * Bitstream entry point: frame is preceded by a 8-byte frame header
//!   with the FourCC `icpf`; the header declares profile, chroma
//!   format, interlace mode, and picture dimensions. Slices are
//!   independently decodable units carrying entropy-coded AC/DC
//!   coefficients for each 8×8 block.
//! * Entropy coding: custom VLC tables per plane + coefficient run mode,
//!   not compatible with standard JPEG or H.264 VLCs. Tables are fixed
//!   (spec-defined), not transmitted.
//! * DCT: classic integer-approx 8×8 forward / inverse; quantisation
//!   matrices differ per profile (and between luma/chroma for 4444).
//! * Container: ProRes typically ships inside MOV / MP4 with sample
//!   entry FourCC `apch` / `apcn` / `apcs` / `apco` / `ap4h` / `ap4x`
//!   depending on profile. Wiring those FourCCs to our codec id
//!   belongs in `oxideav-mp4`'s sample-entry mapping once pixels
//!   actually decode.
//! * Reference implementations: FFmpeg's `libavcodec/proresdec2.c` +
//!   `proresenc_kostya.c` / `proresenc_anatoliy.c` are the de-facto
//!   reference; the SMPTE RDD 36 publication is the formal spec.

use oxideav_codec::{CodecRegistry, Decoder, Encoder};
use oxideav_core::{CodecCapabilities, CodecId, CodecParameters, Error, Result};

/// Public codec id. Matches the Cargo feature name `prores` used by the
/// aggregator crate.
pub const CODEC_ID_STR: &str = "prores";

/// Register the ProRes decoder + encoder stubs.
pub fn register(reg: &mut CodecRegistry) {
    // ProRes is technically lossy but is often used as a mastering
    // intermediate where the quantisation is aggressive enough to be
    // considered "visually lossless" at HQ / 4444 XQ. Flag as
    // intra-only either way.
    let caps = CodecCapabilities::video("prores_stub")
        .with_lossy(true)
        .with_intra_only(true);
    reg.register_decoder_impl(CodecId::new(CODEC_ID_STR), caps.clone(), make_decoder);
    reg.register_encoder_impl(CodecId::new(CODEC_ID_STR), caps, make_encoder);
}

fn make_decoder(_params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    Err(Error::unsupported(
        "prores: decoder not yet implemented; see crate docs for the planned \
         VLC + DCT path (SMPTE RDD 36)",
    ))
}

fn make_encoder(_params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    Err(Error::unsupported(
        "prores: encoder not yet implemented; see crate docs for the planned \
         VLC + DCT path (SMPTE RDD 36)",
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

    #[test]
    fn encoder_reports_unsupported() {
        let mut reg = CodecRegistry::new();
        register(&mut reg);
        let params = CodecParameters::video(CodecId::new(CODEC_ID_STR));
        match reg.make_encoder(&params) {
            Err(Error::Unsupported(_)) => {}
            Err(other) => panic!("expected Error::Unsupported, got {other:?}"),
            Ok(_) => panic!("expected Error::Unsupported, got a live encoder"),
        }
    }
}
