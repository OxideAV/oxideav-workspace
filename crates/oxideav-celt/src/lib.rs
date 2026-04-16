//! CELT — the MDCT path of Opus (RFC 6716 §4.3) — partial.
//!
//! What's landed:
//!
//! * Bit-exact RFC §4.1 range decoder (port of libopus `entdec.c`).
//! * §4.3 / Table 56 frame header symbol decoding (silence, post-filter,
//!   transient, intra).
//! * §4.3.2.1 coarse band-energy decode (`unquant_coarse_energy`) —
//!   Laplace decoder + `e_prob_model` / `pred_coef` / `beta_coef` tables.
//! * §4.3.2.2 fine band-energy decode (`unquant_fine_energy`) and
//!   §4.3.2.3 finalise pass (`unquant_energy_finalise`) — both ready,
//!   pending the bit allocator to compute their inputs.
//! * Static tables: `EBAND_5MS`, `E_PROB_MODEL`, `BAND_ALLOCATION`,
//!   `LOG2_FRAC_TABLE`, prediction coefficients.
//! * Pure-Rust radix-2 IFFT scaffold (`mdct::ifft_radix2`) for the
//!   eventual IMDCT.
//!
//! What's still pending (returns `Unsupported` from the opus crate):
//!
//! * §4.3.3 bit allocation (`clt_compute_allocation` + skip/intensity/dual).
//! * §4.3.4 PVQ shape decoding (split-band recursion + spreading).
//! * §4.3.5 anti-collapse processing.
//! * §4.3.6 final denormalisation (band energy × shape).
//! * §4.3.7 IMDCT pre/post-twiddle + window + overlap-add.
//! * §4.3.8 pitch post-filter convolution.

#![allow(
    dead_code,
    clippy::needless_range_loop,
    clippy::unnecessary_cast,
    clippy::double_parens,
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items
)]

pub mod header;
pub mod laplace;
pub mod mdct;
pub mod quant_bands;
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
