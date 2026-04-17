//! G.728 codec registration.
//!
//! Registers the `g728` codec id for both decode and encode. The encoder
//! and decoder share the backward-adaptive LPC + log-gain predictor
//! machinery in [`crate::predictor`] and the shape / gain codebooks in
//! [`crate::tables`], so a bitstream produced by this encoder round-trips
//! cleanly through the in-tree decoder (subject to the placeholder
//! codebook caveat documented on both sides).

use oxideav_codec::{CodecRegistry, Decoder, Encoder};
use oxideav_core::{CodecCapabilities, CodecId, CodecParameters, Result};

pub fn register(reg: &mut CodecRegistry) {
    let cid = CodecId::new(super::CODEC_ID_STR);
    let caps = CodecCapabilities::audio("g728_sw")
        .with_lossy(true)
        .with_intra_only(false)
        .with_max_channels(1)
        .with_max_sample_rate(super::SAMPLE_RATE);
    reg.register_both(cid, caps, make_decoder, make_encoder);
}

fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    super::decoder::make_decoder(params)
}

fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    super::encoder::make_encoder(params)
}
