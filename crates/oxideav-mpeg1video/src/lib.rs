//! Pure-Rust MPEG-1 video (ISO/IEC 11172-2) decoder + I-frame encoder.
//!
//! Current status:
//! * Milestone 1 — sequence / GOP / picture header parsing: done.
//! * Milestone 2 — I-frame decode with intra DCT blocks and YUV 4:2:0 output.
//! * Milestone 3 — P-frames: forward motion compensation (half-pel bilinear),
//!   non-intra block decode, coded_block_pattern handling, skipped-MB forward
//!   propagation.
//! * Milestone 4 — B-frames: forward + backward motion vector decode,
//!   interpolated prediction (averaged), skipped-MB MV inheritance, and
//!   display-order PTS reconstruction from temporal_reference + GOP anchors.
//! * Milestone 5 — I-frame encoder: sequence / GOP / picture headers,
//!   forward DCT, intra quantisation, DC differential + AC run/level VLC,
//!   one-slice-per-row output. P/B encode is explicitly out of scope.
//!
//! This crate intentionally has no runtime dependencies beyond `oxideav-core`
//! and `oxideav-codec`.

#![allow(clippy::needless_range_loop)]

pub mod bitreader;
pub mod bitwriter;
pub mod block;
pub mod dct;
pub mod decoder;
pub mod encoder;
pub mod headers;
pub mod mb;
pub mod motion;
pub mod picture;
pub mod start_codes;
pub mod tables;
pub mod vlc;

use oxideav_codec::CodecRegistry;
use oxideav_core::{CodecCapabilities, CodecId};

pub const CODEC_ID_STR: &str = "mpeg1video";

pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("mpeg1video_sw")
        .with_lossy(true)
        .with_intra_only(false)
        .with_max_size(4096, 4096);
    let id = CodecId::new(CODEC_ID_STR);
    reg.register_decoder_impl(id.clone(), caps.clone(), decoder::make_decoder);
    // The encoder is intra-only (only I-pictures are produced).
    let enc_caps = caps.with_intra_only(true);
    reg.register_encoder_impl(id, enc_caps, encoder::make_encoder);
}
