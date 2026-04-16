//! MPEG-1 Audio Layer II (MP2 / MUSICAM) decoder.
//!
//! Implements the full Layer II decode pipeline per ISO/IEC 11172-3:
//! frame header + CRC skip → bit-allocation decode (tables B.2a–d) →
//! SCFSI + scalefactor decode → 3-/5-/9-level grouped-sample ungrouping
//! and per-sample requantisation → 32-band polyphase subband synthesis.
//!
//! Supports all MPEG-1 sample rates (32 / 44.1 / 48 kHz), every stereo
//! mode (mono / stereo / joint-stereo / dual-channel), and all valid
//! bitrate/mode combinations.
//!
//! MPEG-2 LSF (ISO 13818-3) and MPEG-2.5 are rejected with `Unsupported`.

#![allow(
    clippy::needless_range_loop,
    clippy::unnecessary_cast,
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items,
    clippy::excessive_precision
)]

pub mod bitalloc;
pub mod bitreader;
pub mod decoder;
pub mod header;
pub mod requant;
pub mod synth;
pub mod tables;

use oxideav_codec::{CodecRegistry, Decoder};
use oxideav_core::{CodecCapabilities, CodecId, CodecParameters, Result};

pub const CODEC_ID_STR: &str = "mp2";

pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::audio("mp2_sw")
        .with_lossy(true)
        .with_intra_only(true)
        .with_max_channels(2)
        .with_max_sample_rate(48_000);
    reg.register_decoder_impl(CodecId::new(CODEC_ID_STR), caps, make_decoder);
}

fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    decoder::make_decoder(params)
}
