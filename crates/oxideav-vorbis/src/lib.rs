// Parallel-array index loops are idiomatic in audio codec code; let clippy
// nag elsewhere.
#![allow(clippy::needless_range_loop)]

//! Vorbis audio codec.
//!
//! Decoder is feature-complete for the common q3-q10 file shapes: matches
//! libvorbis / lewton output within float rounding on the test fixtures.
//! Encoder is in early development — the three Vorbis headers are emitted
//! today, audio packet encoding (MDCT, floor quantisation, residue VQ
//! search) is a follow-up.

pub mod audio_packet;
pub mod bitreader;
pub mod bitwriter;
pub mod codebook;
pub mod dbtable;
pub mod decoder;
pub mod encoder;
pub mod floor;
pub mod identification;
pub mod imdct;
pub mod libvorbis_setup;
pub mod residue;
pub mod setup;
pub mod setup_writer;

use oxideav_codec::{CodecRegistry, Decoder, Encoder};
use oxideav_core::{CodecCapabilities, CodecId, CodecParameters, Result};

pub const CODEC_ID_STR: &str = "vorbis";

pub fn register(reg: &mut CodecRegistry) {
    let cid = CodecId::new(CODEC_ID_STR);
    let caps = CodecCapabilities::audio("vorbis_sw")
        .with_lossy(true)
        .with_max_channels(255);
    reg.register_decoder_impl(cid.clone(), caps.clone(), make_decoder);
    reg.register_encoder_impl(cid, caps, make_encoder);
}

fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    decoder::make_decoder(params)
}

fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    encoder::make_encoder(params)
}

pub use identification::{parse_identification_header, Identification};
