//! ITU-T G.729 (CS-ACELP, 8 kbit/s) decoder — first real implementation.
//!
//! Pipeline (all pure-Rust):
//! - `bitreader`: 80-bit frame → 15 bit fields (L0..L3, P1/P0, C1/S1,
//!   GA1/GB1, P2, C2/S2, GA2/GB2), per §3.6 / Table 8.
//! - `lpc`: LSP decode from index quadruple via MA-4 predictor + safety
//!   monotonicity (§3.2.4), LSP ↔ LPC (§3.2.6), LSP interpolation
//!   between the two subframes (§3.2.5).
//! - `lsp_tables`: static codebook tables. Rows verbatim from the spec
//!   are retained; the rest are procedurally synthesised so every
//!   index produces a valid monotone LSF codeword. See the
//!   module-level doc for the follow-up path.
//! - `synthesis`: adaptive codebook (fractional-pitch, 1/3 resolution),
//!   algebraic fixed codebook (4-track × 4-pulse), two-stage gain VQ,
//!   10th-order all-pole synthesis filter, short-term + long-term
//!   postfilter with tilt compensation and AGC.
//! - `decoder`: orchestrates the pipeline per packet and exposes it via
//!   the `oxideav_codec::Decoder` trait.
//!
//! Reference: ITU-T Recommendation G.729 (January 2007 edition) +
//! Annex A (`G.729a` simplified-complexity variant).

#![allow(
    clippy::needless_range_loop,
    clippy::unnecessary_cast,
    clippy::excessive_precision,
    clippy::approx_constant,
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items
)]

pub mod bitreader;
pub mod codec;
pub mod decoder;
pub mod lpc;
pub mod lsp_tables;
pub mod synthesis;

use oxideav_codec::CodecRegistry;

pub const CODEC_ID_STR: &str = "g729";

/// Number of samples per G.729 frame (10 ms @ 8 kHz).
pub const FRAME_SAMPLES: usize = 80;

/// Number of samples per 5-ms subframe.
pub const SUBFRAME_SAMPLES: usize = 40;

/// Number of subframes per frame.
pub const SUBFRAMES_PER_FRAME: usize = 2;

/// LPC order (10th-order short-term predictor).
pub const LPC_ORDER: usize = 10;

/// Encoded frame size in bytes (80 bits = 10 bytes).
pub const FRAME_BYTES: usize = 10;

/// Sample rate (Hz).
pub const SAMPLE_RATE: u32 = 8_000;

/// Register G.729 with the codec registry.
pub fn register(reg: &mut CodecRegistry) {
    codec::register(reg);
}
