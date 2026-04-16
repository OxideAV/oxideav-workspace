//! Speex narrowband sub-mode descriptors (decode-time).
//!
//! Mirrors the eight `nb_submodeN` records in `libspeex/modes.c`. Each
//! sub-mode bundles the choice of LSP unquantizer, LTP (pitch)
//! unquantizer, fixed (innovation) codebook, and a few small flags
//! that drive the decoder loop.
//!
//! The function-pointer layout in the reference uses C function
//! pointers; we translate those into a flat enum so the decoder can
//! switch over them. The numeric values (`shape_bits`, `subvect_size`,
//! `nb_subvect`, `gain_bits`, `comb_gain`, etc.) are taken verbatim
//! from `modes.c`.

use crate::exc_tables::{
    EXC_10_16_TABLE, EXC_10_32_TABLE, EXC_20_32_TABLE, EXC_5_256_TABLE, EXC_5_64_TABLE,
    EXC_8_128_TABLE,
};
use crate::gain_tables::{GAIN_CDBK_LBR, GAIN_CDBK_NB};

/// Which LSP unquantizer to invoke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LspKind {
    /// `lsp_unquant_lbr` — three-stage VQ, 18 bits.
    Lbr,
    /// `lsp_unquant_nb` — five-stage VQ, 30 bits.
    Nb,
}

/// Which LTP (long-term predictor / pitch) unquantizer to invoke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LtpKind {
    /// `forced_pitch_unquant` — single-tap, gain forced from
    /// `ol_pitch_coef` (vocoder-only mode 1).
    Forced,
    /// `pitch_unquant_3tap` — three-tap pitch synth using a gain
    /// codebook.
    ThreeTap,
}

/// LTP configuration block — corresponds to `ltp_params_*` structs.
#[derive(Clone, Copy, Debug)]
pub struct LtpParams {
    pub gain_cdbk: &'static [i8],
    pub gain_bits: u32,
    pub pitch_bits: u32,
}

pub const LTP_PARAMS_NB: LtpParams = LtpParams {
    gain_cdbk: &GAIN_CDBK_NB,
    gain_bits: 7,
    pitch_bits: 7,
};
pub const LTP_PARAMS_VLBR: LtpParams = LtpParams {
    gain_cdbk: &GAIN_CDBK_LBR,
    gain_bits: 5,
    pitch_bits: 0,
};
pub const LTP_PARAMS_LBR: LtpParams = LtpParams {
    gain_cdbk: &GAIN_CDBK_LBR,
    gain_bits: 5,
    pitch_bits: 7,
};
pub const LTP_PARAMS_MED: LtpParams = LtpParams {
    gain_cdbk: &GAIN_CDBK_LBR,
    gain_bits: 5,
    pitch_bits: 7,
};

/// Which innovation (fixed) codebook unquantizer to invoke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InnovKind {
    /// `noise_codebook_unquant` — fills the sub-frame with PRNG noise.
    Noise,
    /// `split_cb_shape_sign_unquant` — split-VQ with per-sub-vector
    /// sign bits; used by all bit-rate sub-modes.
    SplitCb,
}

/// Split-codebook configuration block — `split_cb_params` in C.
#[derive(Clone, Copy, Debug)]
pub struct SplitCbParams {
    pub subvect_size: usize,
    pub nb_subvect: usize,
    pub shape_cb: &'static [i8],
    pub shape_bits: u32,
    /// Have-sign bit. The reference always sets this to 0 for narrowband
    /// (signs are folded into the codebook), but we keep the field for
    /// fidelity with `cb_search.c`.
    pub have_sign: bool,
}

pub const SPLIT_CB_NB_VLBR: SplitCbParams = SplitCbParams {
    subvect_size: 10,
    nb_subvect: 4,
    shape_cb: &EXC_10_16_TABLE,
    shape_bits: 4,
    have_sign: false,
};
pub const SPLIT_CB_NB_ULBR: SplitCbParams = SplitCbParams {
    subvect_size: 20,
    nb_subvect: 2,
    shape_cb: &EXC_20_32_TABLE,
    shape_bits: 5,
    have_sign: false,
};
pub const SPLIT_CB_NB_LBR: SplitCbParams = SplitCbParams {
    subvect_size: 10,
    nb_subvect: 4,
    shape_cb: &EXC_10_32_TABLE,
    shape_bits: 5,
    have_sign: false,
};
pub const SPLIT_CB_NB: SplitCbParams = SplitCbParams {
    subvect_size: 5,
    nb_subvect: 8,
    shape_cb: &EXC_5_64_TABLE,
    shape_bits: 6,
    have_sign: false,
};
pub const SPLIT_CB_NB_MED: SplitCbParams = SplitCbParams {
    subvect_size: 8,
    nb_subvect: 5,
    shape_cb: &EXC_8_128_TABLE,
    shape_bits: 7,
    have_sign: false,
};
/// Reference name `split_cb_sb` — used for NB sub-mode 6 (18.2 kbps).
pub const SPLIT_CB_SB: SplitCbParams = SplitCbParams {
    subvect_size: 5,
    nb_subvect: 8,
    shape_cb: &EXC_5_256_TABLE,
    shape_bits: 8,
    have_sign: false,
};

/// One Speex narrowband sub-mode (`SpeexSubmode` from C).
///
/// `bits_per_frame` is the TOTAL number of bits the sub-mode consumes
/// from the bitstream **including** the sub-mode selector — useful for
/// validating that the encoder advertised the right size and for
/// skipping unsupported sub-modes by bit count.
#[derive(Clone, Copy, Debug)]
pub struct NbSubmode {
    /// `lbr_pitch` in C: `Some(margin)` means low-bit-rate pitch coding
    /// is enabled; the encoder transmits one global open-loop pitch and
    /// each sub-frame's pitch is constrained to ±`margin` of it.
    pub lbr_pitch: Option<i32>,
    /// If true, the sub-mode forces a single global pitch gain (encoded
    /// in 4 bits) instead of unquantizing one per sub-frame.
    pub forced_pitch_gain: bool,
    /// Sub-frame innovation gain bits (0, 1, or 3).
    pub have_subframe_gain: u32,
    /// If true, run innovation unquant twice per sub-frame and sum the
    /// two outputs at decreased weight (sub-mode 7 only).
    pub double_codebook: bool,

    pub lsp: LspKind,
    pub ltp: LtpKind,
    pub ltp_params: LtpParams,
    pub innov: InnovKind,
    pub innov_params: SplitCbParams,
    /// Perceptual postfilter (`comb_gain`) — 0..1.
    pub comb_gain: f32,
    /// Total bits per encoded frame for this sub-mode (incl. selector).
    pub bits_per_frame: u32,
}

const fn empty_split() -> SplitCbParams {
    SplitCbParams {
        subvect_size: 0,
        nb_subvect: 0,
        shape_cb: &[],
        shape_bits: 0,
        have_sign: false,
    }
}

/// 2.15 kbps "vocoder-like" mode used for comfort noise.
pub const NB_SUBMODE_1: NbSubmode = NbSubmode {
    lbr_pitch: Some(0),
    forced_pitch_gain: true,
    have_subframe_gain: 0,
    double_codebook: false,
    lsp: LspKind::Lbr,
    ltp: LtpKind::Forced,
    ltp_params: LTP_PARAMS_VLBR,
    innov: InnovKind::Noise,
    innov_params: empty_split(),
    comb_gain: -1.0, // postfilter disabled
    bits_per_frame: 43,
};

/// 5.95 kbps very-low-bit-rate mode.
pub const NB_SUBMODE_2: NbSubmode = NbSubmode {
    lbr_pitch: Some(0),
    forced_pitch_gain: false,
    have_subframe_gain: 0,
    double_codebook: false,
    lsp: LspKind::Lbr,
    ltp: LtpKind::ThreeTap,
    ltp_params: LTP_PARAMS_VLBR,
    innov: InnovKind::SplitCb,
    innov_params: SPLIT_CB_NB_VLBR,
    comb_gain: 0.6,
    bits_per_frame: 119,
};

/// 8 kbps low-bit-rate mode.
pub const NB_SUBMODE_3: NbSubmode = NbSubmode {
    lbr_pitch: Some(-1),
    forced_pitch_gain: false,
    have_subframe_gain: 1,
    double_codebook: false,
    lsp: LspKind::Lbr,
    ltp: LtpKind::ThreeTap,
    ltp_params: LTP_PARAMS_LBR,
    innov: InnovKind::SplitCb,
    innov_params: SPLIT_CB_NB_LBR,
    comb_gain: 0.55,
    bits_per_frame: 160,
};

/// 11 kbps medium-bit-rate mode.
pub const NB_SUBMODE_4: NbSubmode = NbSubmode {
    lbr_pitch: Some(-1),
    forced_pitch_gain: false,
    have_subframe_gain: 1,
    double_codebook: false,
    lsp: LspKind::Lbr,
    ltp: LtpKind::ThreeTap,
    ltp_params: LTP_PARAMS_MED,
    innov: InnovKind::SplitCb,
    innov_params: SPLIT_CB_NB_MED,
    comb_gain: 0.45,
    bits_per_frame: 220,
};

/// 15 kbps high-bit-rate mode.
pub const NB_SUBMODE_5: NbSubmode = NbSubmode {
    lbr_pitch: Some(-1),
    forced_pitch_gain: false,
    have_subframe_gain: 3,
    double_codebook: false,
    lsp: LspKind::Nb,
    ltp: LtpKind::ThreeTap,
    ltp_params: LTP_PARAMS_NB,
    innov: InnovKind::SplitCb,
    innov_params: SPLIT_CB_NB,
    comb_gain: 0.25,
    bits_per_frame: 300,
};

/// 18.2 kbps high-bit-rate mode.
pub const NB_SUBMODE_6: NbSubmode = NbSubmode {
    lbr_pitch: Some(-1),
    forced_pitch_gain: false,
    have_subframe_gain: 3,
    double_codebook: false,
    lsp: LspKind::Nb,
    ltp: LtpKind::ThreeTap,
    ltp_params: LTP_PARAMS_NB,
    innov: InnovKind::SplitCb,
    innov_params: SPLIT_CB_SB,
    comb_gain: 0.15,
    bits_per_frame: 364,
};

/// 24.6 kbps high-bit-rate mode.
pub const NB_SUBMODE_7: NbSubmode = NbSubmode {
    lbr_pitch: Some(-1),
    forced_pitch_gain: false,
    have_subframe_gain: 3,
    double_codebook: true,
    lsp: LspKind::Nb,
    ltp: LtpKind::ThreeTap,
    ltp_params: LTP_PARAMS_NB,
    innov: InnovKind::SplitCb,
    innov_params: SPLIT_CB_NB,
    comb_gain: 0.05,
    bits_per_frame: 492,
};

/// 3.95 kbps very-low-bit-rate mode.
pub const NB_SUBMODE_8: NbSubmode = NbSubmode {
    lbr_pitch: Some(0),
    forced_pitch_gain: true,
    have_subframe_gain: 0,
    double_codebook: false,
    lsp: LspKind::Lbr,
    ltp: LtpKind::Forced,
    ltp_params: LTP_PARAMS_VLBR,
    innov: InnovKind::SplitCb,
    innov_params: SPLIT_CB_NB_ULBR,
    comb_gain: 0.5,
    bits_per_frame: 79,
};

/// Look up an NB sub-mode by its 4-bit selector. `0` and any
/// out-of-range index map to `None` (silence / not transmitted).
pub fn nb_submode(id: u32) -> Option<&'static NbSubmode> {
    match id {
        1 => Some(&NB_SUBMODE_1),
        2 => Some(&NB_SUBMODE_2),
        3 => Some(&NB_SUBMODE_3),
        4 => Some(&NB_SUBMODE_4),
        5 => Some(&NB_SUBMODE_5),
        6 => Some(&NB_SUBMODE_6),
        7 => Some(&NB_SUBMODE_7),
        8 => Some(&NB_SUBMODE_8),
        _ => None,
    }
}

/// Wideband-layer skip table from `nb_celp.c` — used to advance past a
/// wideband layer when the decoder only consumes the embedded NB part.
pub const WB_SKIP_TABLE: [i32; 8] = [0, 36, 112, 192, 352, 0, 0, 0];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submodes_match_reference_bit_counts() {
        // Bit counts taken from `libspeex/modes.c`.
        assert_eq!(NB_SUBMODE_1.bits_per_frame, 43);
        assert_eq!(NB_SUBMODE_2.bits_per_frame, 119);
        assert_eq!(NB_SUBMODE_3.bits_per_frame, 160);
        assert_eq!(NB_SUBMODE_4.bits_per_frame, 220);
        assert_eq!(NB_SUBMODE_5.bits_per_frame, 300);
        assert_eq!(NB_SUBMODE_6.bits_per_frame, 364);
        assert_eq!(NB_SUBMODE_7.bits_per_frame, 492);
        assert_eq!(NB_SUBMODE_8.bits_per_frame, 79);
    }

    #[test]
    fn invalid_submode_returns_none() {
        assert!(nb_submode(0).is_none());
        assert!(nb_submode(9).is_none());
    }
}
