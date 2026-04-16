//! CELT — the MDCT path of Opus (RFC 6716 §4.3) — partial.
//!
//! What's landed:
//!
//! * Full RFC 6716 §4.1 range decoder (`ec_dec_bits`, `ec_dec_icdf`,
//!   `ec_dec_uint`, `ec_dec_bit_logp`).
//! * RFC 6716 §4.3 / Table 56 frame header symbol decoding (silence,
//!   post-filter octave/period/gain/tapset, transient flag, intra flag).
//! * Static band-edge table (`tables::EBAND_5MS`) and per-bandwidth
//!   end-band lookup used by all downstream stages.
//!
//! What's still pending (returns `Unsupported` from the opus crate):
//!
//! * §4.3.2 coarse + fine band energy decoding (Laplace decoder).
//! * §4.3.3 bit allocation (band boost, trim, skip, intensity, dual stereo).
//! * §4.3.4 PVQ shape decoding (split-band recursion + spreading).
//! * §4.3.5 anti-collapse processing.
//! * §4.3.7 inverse MDCT (CELT's 4-fold radix-N/4 kernel).
//! * §4.3.8 pitch post-filter convolution.
//!
//! The decoder is registered so the framework can detect CELT-carrying
//! streams today; `make_decoder` currently returns `Unsupported`.

#![allow(
    dead_code,
    clippy::needless_range_loop,
    clippy::unnecessary_cast,
    clippy::double_parens,
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items
)]

pub mod header;
pub mod range_decoder;
pub mod tables;

use oxideav_codec::{CodecRegistry, Decoder};
use oxideav_core::{CodecCapabilities, CodecId, CodecParameters, Error, Result};

pub const CODEC_ID_STR: &str = "celt";

pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::audio("celt_sw")
        .with_lossy(true)
        .with_intra_only(false)
        .with_max_channels(2)
        .with_max_sample_rate(48_000);
    reg.register_decoder_impl(CodecId::new(CODEC_ID_STR), caps, make_decoder);
}

fn make_decoder(_params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    Err(Error::unsupported(
        "CELT decoder is a scaffold — range decoder done; band decode + MDCT pending",
    ))
}
