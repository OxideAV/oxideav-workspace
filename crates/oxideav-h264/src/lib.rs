//! Pure-Rust H.264 / AVC (ITU-T H.264 | ISO/IEC 14496-10) decoder.
//!
//! Scope of v1:
//!
//! * NAL unit framing in **both** Annex B (`00 00 [00] 01` start codes) and
//!   AVCC length-prefixed form (used inside MP4 `mdat`).
//! * Emulation-prevention byte stripping (`§7.4.1.1`).
//! * Sequence Parameter Set parsing (`§7.3.2.1.1`) — profile, level, chroma
//!   format, bit depth, frame size, picture order count type, frame
//!   cropping, VUI presence flag.
//! * Picture Parameter Set parsing (`§7.3.2.2`) — entropy coding mode,
//!   slice group map, default reference index counts, weighted prediction,
//!   deblocking control, transform-8×8 flag.
//! * Slice header parsing (`§7.3.3`) — slice type, frame number, POC,
//!   reference list overrides, prediction weight table skip, deblocking
//!   override.
//! * AVCDecoderConfigurationRecord parsing for MP4 `avcC` boxes
//!   (`ISO/IEC 14496-15 §5.2.4.1`).
//!
//! Pixel reconstruction for I-slices is implemented for both CAVLC and
//! CABAC entropy modes (intra prediction + residual decode + inverse
//! transforms + optional deblocking). The CABAC path currently targets the
//! common I_16×16 / I_NxN subset; I_PCM inside CABAC is not yet wired.
//!
//! Out of scope (returns `Error::Unsupported`):
//! * **P / B slices** (`§8.4` motion-compensated prediction).
//! * **Interlaced** coding / MBAFF (`§7.4.2.1.1` `frame_mbs_only_flag = 0`).
//! * **8×8 transform** (`§8.5.13`), 4:2:2 / 4:4:4 chroma formats, bit depths
//!   above 8.
//! * Encoder.
//!
//! This crate has no runtime dependencies beyond `oxideav-core` and
//! `oxideav-codec`.

#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::manual_memcpy)]
#![allow(clippy::derivable_impls)]

pub mod bitreader;
pub mod cabac;
pub mod cavlc;
pub mod deblock;
pub mod decoder;
pub mod intra_pred;
pub mod mb;
pub mod mb_type;
pub mod nal;
pub mod picture;
pub mod pps;
pub mod slice;
pub mod sps;
pub mod tables;
pub mod transform;

use oxideav_codec::CodecRegistry;
use oxideav_core::{CodecCapabilities, CodecId};

/// The canonical oxideav codec id for H.264 / AVC video.
pub const CODEC_ID_STR: &str = "h264";

/// Register this decoder with a codec registry.
pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("h264_sw")
        .with_lossy(true)
        .with_intra_only(false)
        .with_max_size(8192, 8192);
    reg.register_decoder_impl(CodecId::new(CODEC_ID_STR), caps, decoder::make_decoder);
}
