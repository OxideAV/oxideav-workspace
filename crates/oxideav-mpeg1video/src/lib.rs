//! Pure-Rust MPEG-1 video (ISO/IEC 11172-2) decoder.
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
//!
//! This crate intentionally has no runtime dependencies beyond `oxideav-core`
//! and `oxideav-codec`.

#![allow(clippy::needless_range_loop)]

pub mod bitreader;
pub mod block;
pub mod dct;
pub mod decoder;
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
    reg.register_decoder_impl(CodecId::new(CODEC_ID_STR), caps, decoder::make_decoder);
}
