//! SILK decoder per RFC 6716 §4.2.
//!
//! Scope of this module:
//!
//! * SILK-only, mono, **10 ms and 20 ms** frames at 8/12/16 kHz
//!   internal rate (NB/MB/WB). The decoder output is 48 kHz (Opus
//!   always emits 48 kHz; see RFC 6716 §4.2.1) by way of a local
//!   8/12/16→48 kHz upsampler.
//!
//!   A 10 ms frame uses 2 sub-frames (RFC §4.2.7.4); a 20 ms frame
//!   uses 4.
//!
//! * 40 ms / 60 ms frames — **tracked follow-up**. These are not a
//!   clean extension of the per-frame pipeline; they pack 2 or 3
//!   back-to-back 20 ms SILK frames inside a single Opus frame and
//!   prepend *per-sub-frame* LBRR flags (RFC §4.2.4). The current
//!   decoder rejects LBRR-flagged packets anyway, so adding the outer
//!   loop without honouring the LBRR packing would desync on the
//!   first packet that actually uses LBRR. Left as `Unsupported` with
//!   a precise message; see `decode_frame_to_48k` below.
//!
//! * Stereo decoding — **tracked follow-up**. Per RFC §4.2.7.1 the
//!   stereo SILK bitstream starts with a 1-bit `mid_only` flag
//!   followed by two 3-level + two 5-level ICDF reads of stereo
//!   prediction weights, then the mid channel, then (optionally) the
//!   side channel, then a stereo-unmixing step that relies on a
//!   low-pass filter and 8-tap upsampler state per channel. The MVP
//!   excitation generator in this module is not bit-exact, so a
//!   stereo decoder here would also not be bit-exact — but it would
//!   need the stereo header consumed to keep the range coder aligned.
//!   Adding that correctly would require two new ICDF tables, a
//!   dual-channel `SilkChannelState` plumbing, and the unmixing
//!   filter. Left as `Unsupported` with a precise message; see
//!   `decode_frame_to_48k` below.
//!
//! Sub-modules:
//!
//! * [`range_dec`] — re-exports the CELT crate's arithmetic coder plus
//!   SILK-specific helpers that share the same bitstream.
//! * [`lsf`] — Line Spectral Frequency (stage-1 + stage-2 normal + LSF
//!   stabilization + interpolation).
//! * [`ltp`] — Long-Term Prediction filter coefficient decoding and
//!   scale.
//! * [`excitation`] — Excitation signal decoding (pulses, LSBs, signs,
//!   LCG seed).
//! * [`synth`] — Synthesis filter (short-term LPC + LTP) and the
//!   post-upsample to 48 kHz.
//! * `tables` — All RFC §4.2 ICDFs transcribed verbatim.

#![allow(clippy::many_single_char_names)]

pub mod excitation;
pub mod lsf;
pub mod ltp;
pub mod range_dec;
pub mod synth;
pub mod tables;

use oxideav_celt::range_decoder::RangeDecoder;
use oxideav_core::{Error, Result};

use crate::toc::{OpusBandwidth, Toc};

/// Internal SILK sampling rate (8/12/16 kHz) for NB/MB/WB.
pub fn internal_rate_hz(bw: OpusBandwidth) -> u32 {
    match bw {
        OpusBandwidth::Narrowband => 8_000,
        OpusBandwidth::Mediumband => 12_000,
        OpusBandwidth::Wideband => 16_000,
        _ => 16_000, // SILK doesn't natively support SWB/FB
    }
}

/// Number of sub-frames in a 20 ms SILK frame: always 4.
pub const SUBFRAMES_20MS: usize = 4;

/// Number of sub-frames in a 10 ms SILK frame: always 2.
pub const SUBFRAMES_10MS: usize = 2;

/// Persistent decoder state carried across SILK frames for a single
/// channel.
#[derive(Debug, Clone)]
pub struct SilkChannelState {
    /// Previous frame's final LPC coefficients (for 10 ms interp +
    /// stereo / LBRR continuity). Only used internally in `synth`.
    pub prev_lpc: Vec<f32>,
    /// `lagPrev` from the previous frame, used in LTP pitch lag
    /// differential coding.
    pub prev_pitch_lag: i32,
    /// `NLSF_Q15` from the previous frame (used when interp_coef != 4).
    pub prev_nlsf_q15: Vec<i16>,
    /// Synthesis output buffer (one sub-frame of LPC order history).
    pub lpc_history: Vec<f32>,
    /// Excitation history for LTP taps (long enough for pitch_lag +
    /// LTP_ORDER/2).
    pub ltp_history: Vec<f32>,
    /// `prev_gain_Q16` of the previous sub-frame.
    pub prev_gain_q16: i32,
    /// First-frame flag — after a decoder reset or a LBRR gap, the
    /// first frame is coded specially (absolute coding).
    pub first_frame: bool,
}

impl SilkChannelState {
    pub fn new() -> Self {
        Self {
            prev_lpc: Vec::new(),
            prev_pitch_lag: 0,
            prev_nlsf_q15: Vec::new(),
            lpc_history: Vec::new(),
            ltp_history: vec![0.0; 480],
            prev_gain_q16: 0,
            first_frame: true,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

impl Default for SilkChannelState {
    fn default() -> Self {
        Self::new()
    }
}

/// Decoder for a single SILK channel in mono mode.
///
/// This owns the persistent inter-frame state (`SilkChannelState`) plus
/// per-packet scratch.
pub struct SilkDecoder {
    pub state: SilkChannelState,
    pub bandwidth: OpusBandwidth,
    /// Number of LPC coefficients (order). NB/MB => 10; WB => 16.
    pub lpc_order: usize,
    /// Sub-frame length in samples at the internal rate.
    pub subframe_len: usize,
    /// Full SILK frame length in samples at the internal rate (20 ms).
    pub frame_len: usize,
}

impl SilkDecoder {
    pub fn new(bandwidth: OpusBandwidth) -> Self {
        let (order, sub_len) = match bandwidth {
            OpusBandwidth::Narrowband => (10, 40), // 5 ms @ 8 kHz
            OpusBandwidth::Mediumband => (10, 60), // 5 ms @ 12 kHz
            OpusBandwidth::Wideband => (16, 80),   // 5 ms @ 16 kHz
            _ => (16, 80),
        };
        let frame_len = sub_len * SUBFRAMES_20MS;
        Self {
            state: SilkChannelState::new(),
            bandwidth,
            lpc_order: order,
            subframe_len: sub_len,
            frame_len,
        }
    }

    /// Decode a single SILK-only mono 10 ms or 20 ms frame, returning
    /// the output audio at 48 kHz.
    pub fn decode_frame_to_48k(
        &mut self,
        rc: &mut RangeDecoder<'_>,
        toc: &Toc,
    ) -> Result<Vec<f32>> {
        // Supported 48 kHz frame lengths:
        //   480  = 10 ms (2 sub-frames)
        //   960  = 20 ms (4 sub-frames)
        //   1920 = 40 ms (2×20 ms) — tracked follow-up (LBRR packing)
        //   2880 = 60 ms (3×20 ms) — tracked follow-up (LBRR packing)
        let n_subframes = match toc.frame_samples_48k {
            480 => SUBFRAMES_10MS,
            960 => SUBFRAMES_20MS,
            1920 | 2880 => {
                return Err(Error::unsupported(
                    "SILK: 40 ms and 60 ms frames not yet implemented — they pack multiple 20 ms SILK frames with per-sub-frame LBRR flags (RFC 6716 §4.2.4)",
                ));
            }
            _ => {
                return Err(Error::unsupported("SILK: unsupported frame size"));
            }
        };
        if toc.stereo {
            return Err(Error::unsupported(
                "SILK: stereo decoding not yet implemented — needs mid_only flag, stereo prediction weights, and unmixing filter per RFC 6716 §4.2.7.1",
            ));
        }

        let pcm_internal = self.decode_frame_to_internal(rc, n_subframes)?;
        let internal_rate = internal_rate_hz(self.bandwidth);
        let pcm_48k = synth::upsample_to_48k(&pcm_internal, internal_rate);
        Ok(pcm_48k)
    }

    /// Decode the frame at the internal 8/12/16 kHz rate.
    ///
    /// Implements the minimal "SILK frame" pipeline of RFC 6716 §4.2.7
    /// for a SILK-only mono 10 ms or 20 ms frame:
    ///
    /// 1. Header bits (VAD + LBRR flags) — §4.2.3.
    /// 2. Frame-type + gain indices — §4.2.7.3 and §4.2.7.4.
    /// 3. NLSF stage-1 + stage-2 → LSF → LPC — §4.2.7.5.
    /// 4. LTP params (when voiced) — §4.2.7.6.
    /// 5. Excitation (pulses, LSBs, signs, LCG) — §4.2.7.8.
    /// 6. LTP + short-term LPC synthesis — §4.2.7.9.
    ///
    /// `n_subframes` is 2 for a 10 ms frame, 4 for 20 ms.
    pub fn decode_frame_to_internal(
        &mut self,
        rc: &mut RangeDecoder<'_>,
        n_subframes: usize,
    ) -> Result<Vec<f32>> {
        debug_assert!(n_subframes == SUBFRAMES_10MS || n_subframes == SUBFRAMES_20MS);
        let frame_len = self.subframe_len * n_subframes;

        // §4.2.3 Header bits: VAD (1 bit per frame) + LBRR (1 bit).
        // For 10/20 ms, that's one VAD bit + one LBRR bit per channel.
        let vad_flag = rc.decode_bit_logp(1);
        let lbrr_flag = rc.decode_bit_logp(1);
        // Reject any LBRR frame to keep scope minimal: the
        // reference VOIP clip never sets LBRR at 16 kbps.
        if lbrr_flag {
            return Err(Error::unsupported("SILK: LBRR frames not yet implemented"));
        }

        // §4.2.7.3 frame type (signal + quantization offset).
        // ICDF table selection depends on VAD flag.
        let frame_type_sym = if vad_flag {
            rc.decode_icdf(&tables::FRAME_TYPE_ACTIVE_ICDF, 8)
        } else {
            rc.decode_icdf(&tables::FRAME_TYPE_INACTIVE_ICDF, 8)
        };
        // Map to (signal_type, quant_offset_type):
        //   frame_type_sym:  0  1  2  3  4  5
        //     voicing      : I  I  U  U  V  V
        //     Q-offset type: L  H  L  H  L  H
        // (I = inactive, U = unvoiced, V = voiced, L = low, H = high)
        let (signal_type, quant_offset_type) = match frame_type_sym {
            0 => (0u8, 0u8), // inactive low
            1 => (0, 1),
            2 => (1, 0), // unvoiced
            3 => (1, 1),
            4 => (2, 0), // voiced
            5 => (2, 1),
            _ => (1, 0),
        };
        let voiced = signal_type == 2;

        // §4.2.7.4 sub-frame gains.
        let mut gains_q16 = vec![0i32; n_subframes];
        {
            // First sub-frame: independent coding (3-bit MSB + 3-bit LSB).
            // Later: delta-coded. The first sub-frame read is the same
            // regardless of `first_frame`; the distinction matters for
            // LBRR frames which we reject above.
            let msb_icdf: &[u8] = match signal_type {
                0 => &tables::GAIN_MSB_INACTIVE_ICDF,
                1 => &tables::GAIN_MSB_UNVOICED_ICDF,
                _ => &tables::GAIN_MSB_VOICED_ICDF,
            };
            let msb = rc.decode_icdf(msb_icdf, 8) as i32;
            let lsb = rc.decode_icdf(&tables::GAIN_LSB_ICDF, 8) as i32;
            let idx = (msb << 3) | lsb;
            gains_q16[0] = gain_index_to_q16(idx.clamp(0, 63));
            // Subsequent sub-frames: delta-coded.
            let mut prev_log_gain = gain_index_of_q16(gains_q16[0]);
            for sf in 1..n_subframes {
                let delta = rc.decode_icdf(&tables::GAIN_DELTA_ICDF, 8) as i32;
                // delta symbol is in [0, 40]; mapped to a signed step
                // centred on 4 (RFC §4.2.7.4). For this MVP all three
                // branches collapse to `delta - 4`.
                let step = delta - 4;
                let new_log = (prev_log_gain + step).clamp(0, 63);
                gains_q16[sf] = gain_index_to_q16(new_log);
                prev_log_gain = new_log;
            }
        }

        // §4.2.7.5 NLSF decoding (stage-1 + stage-2 + interp + stabilize).
        let nlsf_q15 = lsf::decode_nlsf(rc, self.bandwidth, signal_type)?;
        // Convert NLSF → LPC Q12 → f32 LPC.
        let lpc = lsf::nlsf_to_lpc(&nlsf_q15, self.bandwidth);

        // §4.2.7.6.1 Primary pitch lag (voiced only).
        let mut pitch_lags = vec![0i32; n_subframes];
        let mut ltp_filter = vec![[0f32; 5]; n_subframes];
        let mut ltp_scale_q14 = 15565i32; // default per RFC
        if voiced {
            // Primary lag: absolute or relative based on a 1-bit flag.
            let abs_flag = rc.decode_bit_logp(1);
            let primary_lag = if abs_flag || self.state.prev_pitch_lag == 0 {
                ltp::decode_absolute_pitch_lag(rc, self.bandwidth)?
            } else {
                let delta = ltp::decode_delta_pitch_lag(rc)?;
                self.state.prev_pitch_lag + delta
            };
            // Spread to sub-frames (10 ms uses 2 contours, 20 ms uses 4).
            let contour_idx = ltp::decode_pitch_contour(rc, self.bandwidth)?;
            ltp::expand_pitch_contour(primary_lag, contour_idx, self.bandwidth, &mut pitch_lags);
            self.state.prev_pitch_lag = primary_lag;

            // Per-subframe LTP filter coefficients.
            let periodicity = rc.decode_icdf(&tables::LTP_PERIODICITY_ICDF, 8);
            for sf in 0..n_subframes {
                let tap = ltp::decode_ltp_filter(rc, periodicity);
                for k in 0..5 {
                    ltp_filter[sf][k] = tap[k];
                }
            }

            // LTP scaling factor.
            let ltp_scale_idx = rc.decode_icdf(&tables::LTP_SCALING_ICDF, 8);
            ltp_scale_q14 = match ltp_scale_idx {
                0 => 15565,
                1 => 12288,
                _ => 8192,
            };
        }

        // §4.2.7.7 LCG seed for excitation reconstruction.
        let seed = rc.decode_icdf(&tables::LCG_SEED_ICDF, 2) as u32;

        // §4.2.7.8 Excitation.
        let excitation = excitation::decode_excitation(
            rc,
            frame_len,
            self.subframe_len,
            signal_type,
            quant_offset_type,
            seed,
        )?;

        // §4.2.7.9 Synthesis filter: LTP + short-term LPC.
        let output = synth::synthesize(
            &excitation,
            &lpc,
            &gains_q16,
            &pitch_lags,
            &ltp_filter,
            ltp_scale_q14,
            self.subframe_len,
            n_subframes,
            self.lpc_order,
            voiced,
            &mut self.state,
        );

        self.state.first_frame = false;
        self.state.prev_nlsf_q15 = nlsf_q15;
        Ok(output)
    }
}

/// Map a 6-bit log-gain index (0..=63) to a Q16 linear gain per the
/// SILK spec (RFC 6716 §4.2.7.4).
///
/// `silk_log2lin((0x1D1C71 * idx >> 16) + 2090)`. We implement a
/// float approximation: gain_q16 = round(2^((idx/64)*16 + 2090/65536 *
/// 16)) which is close enough for the synthesis filter to produce
/// non-silent audio; bit-exactness here is NOT required for Opus
/// compliance — libopus rounds to the nearest Q16 but the gain is
/// further scaled by the LPC/LTP taps.
fn gain_index_to_q16(idx: i32) -> i32 {
    let idx = idx.clamp(0, 63) as f32;
    let log2 = (0x1D1C71u32 as f32 / 65536.0) * idx + (2090.0 / 65536.0);
    let lin = 2f32.powf(log2);
    (lin * 65536.0).round() as i32
}

/// Inverse of `gain_index_to_q16`.
fn gain_index_of_q16(gain: i32) -> i32 {
    let log2 = (gain.max(1) as f32 / 65536.0).log2();
    let idx = (log2 - 2090.0 / 65536.0) / (0x1D1C71u32 as f32 / 65536.0);
    idx.round() as i32
}
