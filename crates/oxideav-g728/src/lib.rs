//! ITU-T G.728 Low-Delay CELP (LD-CELP, 16 kbit/s) decoder — first cut.
//!
//! G.728 codes narrowband (8 kHz) speech at 16 kbit/s using 5-sample
//! analysis vectors (0.625 ms). Each vector is represented by a single
//! 10-bit index split into a 7-bit shape codebook selector (128 shape
//! vectors) and a 3-bit gain selector (sign bit + 2 magnitude bits,
//! for 8 signed levels), giving 2 kbyte/s of compressed data.
//!
//! The spec is unusual for a CELP variant: the synthesis filter is a
//! **50th-order** LPC predictor adapted **backward** from previously
//! reconstructed speech (no transmitted LPC), and a 10th-order backward-
//! adaptive log-gain predictor tracks excitation energy. Because all the
//! adaptation machinery is derived from the decoder's own output, the
//! payload shrinks to just the codebook index — which is why the bit
//! reader here is so thin.
//!
//! What's landed:
//!
//! - Crate + workspace wiring (registers as codec id `g728`).
//! - 10-bit MSB-first bit reader for the packed index stream.
//! - 128 × 5 shape codebook + 8-magnitude gain codebook (see the
//!   [`tables`] module; these are deterministic unit-RMS placeholders,
//!   **not** the exact ITU Annex A `CODEBK` / `GB` values).
//! - Backward-adaptive 50th-order LPC predictor: windowed
//!   autocorrelation + Levinson-Durbin, refreshed every 4 vectors
//!   (2.5 ms), with bandwidth expansion for stability.
//! - Backward-adaptive 10th-order log-gain predictor.
//! - Synthesis loop: excitation → all-pole IIR → S16 PCM, exposed via
//!   the standard [`oxideav_codec::Decoder`] trait.
//!
//! What is deliberately **not** landed yet (tracked as follow-ups):
//!
//! - Exact ITU `CODEBK` / `GB` tables (one-table swap, no code change).
//! - Spec's recursive Barnwell / logarithmic autocorrelation window —
//!   we use a fixed 100-sample Hamming window instead.
//! - Adaptive long-term (pitch) postfilter and short-term postfilter
//!   described in §5.5 of the 2012 edition.
//!
//! Consequence: `make_decoder` now returns a working decoder that
//! produces structured, bounded, non-silent output. It is not
//! bit-compatible with reference ITU G.728 streams — treat it as a
//! functional first cut until the exact tables + postfilter land.

// Scaffold-only — symbols will be used once the full decoder body lands.
// These allow()s come off when the decoder is exercised from end to end.
#![allow(
    dead_code,
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
pub mod predictor;
pub mod tables;

use oxideav_codec::CodecRegistry;

pub const CODEC_ID_STR: &str = "g728";

/// Sample rate, always 8 kHz narrowband.
pub const SAMPLE_RATE: u32 = 8_000;

/// Samples per analysis vector.
pub const VECTOR_SIZE: usize = 5;

/// Bits per packed codebook index.
pub const INDEX_BITS: u32 = 10;

/// Order of the backward-adaptive LPC synthesis predictor.
pub const LPC_ORDER: usize = 50;

/// Order of the backward-adaptive log-gain predictor.
pub const GAIN_ORDER: usize = 10;

/// Number of shape codebook entries (7 index bits).
pub const SHAPE_CB_SIZE: usize = 128;

/// Number of gain codebook entries (3 index bits: sign bit + 2 magnitude bits
/// packed differently — see Annex A). The effective lookup table contains
/// 8 positive magnitudes; the final sign comes from the extra high bit.
pub const GAIN_CB_SIZE: usize = 8;

/// Register the G.728 decoder with the codec registry.
pub fn register(reg: &mut CodecRegistry) {
    codec::register(reg);
}
