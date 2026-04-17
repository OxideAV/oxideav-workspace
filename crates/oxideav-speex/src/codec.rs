//! Speex codec registration.
//!
//! The decoder handles narrowband + wideband modes; the encoder is
//! narrowband mode-5 only. Both sides share the `"speex"` codec id and
//! register together via `register_both` so the registry knows to offer
//! the same implementation block for decode and encode.

use oxideav_codec::{CodecRegistry, Decoder, Encoder};
use oxideav_core::{CodecCapabilities, CodecId, CodecParameters, Result};

pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::audio("speex_sw")
        .with_lossy(true)
        .with_intra_only(false)
        .with_max_channels(2)
        .with_max_sample_rate(32_000);
    reg.register_both(
        CodecId::new(super::CODEC_ID_STR),
        caps,
        make_decoder,
        make_encoder,
    );
}

fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    super::decoder::make_decoder(params)
}

fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    super::encoder::make_encoder(params)
}
