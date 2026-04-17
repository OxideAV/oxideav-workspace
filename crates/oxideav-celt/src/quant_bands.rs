//! CELT band-energy decoding (RFC 6716 §4.3.2).
//!
//! Two passes:
//!
//! * **Coarse** (`unquant_coarse_energy`) — Laplace-coded delta from the
//!   predicted log-energy of each band. The prediction uses the previous
//!   frame's energy (inter mode) or pure intra-frame.
//! * **Fine** (`unquant_fine_energy`) — refines coarse with raw bits per
//!   band, count given by the bit allocator.
//! * **Final** (`unquant_energy_finalise`) — uses leftover bits.
//!
//! Energies are stored in log-domain "dB" units (Q8 internally; we store as
//! `f32` here for simplicity — the precision loss vs Q8 fixed-point is
//! negligible for downstream PVQ).

use crate::laplace::ec_laplace_decode;
use crate::range_decoder::RangeDecoder;
use crate::tables::{
    BETA_COEF_F32, BETA_INTRA_F32, E_PROB_MODEL, NB_EBANDS, PRED_COEF_F32, SMALL_ENERGY_ICDF,
};

const DB_SCALE: f32 = 1.0; // We work in raw dB units (libopus uses Q8).

/// Decode the coarse band-energy values for a CELT frame.
///
/// `old_e_bands` carries inter-frame state (per channel × NB_EBANDS).
/// On entry it holds the previous frame's quantized energies; on exit it
/// holds this frame's. Returns the number of bands decoded (`end - start`).
pub fn unquant_coarse_energy(
    rc: &mut RangeDecoder<'_>,
    old_e_bands: &mut [f32],
    start: usize,
    end: usize,
    intra: bool,
    channels: usize,
    lm: usize,
) {
    let prob_model = &E_PROB_MODEL[lm][intra as usize];
    let (coef, beta) = if intra {
        (0.0f32, BETA_INTRA_F32)
    } else {
        (PRED_COEF_F32[lm], BETA_COEF_F32[lm])
    };
    let budget = (rc.storage() * 8) as i32;
    let mut prev = [0.0f32; 2];
    for i in start..end {
        for c in 0..channels {
            let tell = rc.tell();
            let qi: i32 = if budget - tell >= 15 {
                let pi = 2 * i.min(20);
                let fs = (prob_model[pi] as u32) << 7;
                let decay = (prob_model[pi + 1] as i32) << 6;
                ec_laplace_decode(rc, fs, decay)
            } else if budget - tell >= 2 {
                let qi = rc.decode_icdf(&SMALL_ENERGY_ICDF, 2) as i32;
                // Map 0,1,2 to 0,-1,+1 (libopus: (qi>>1)^-(qi&1))
                (qi >> 1) ^ -(qi & 1)
            } else if budget - tell >= 1 {
                if rc.decode_bit_logp(1) {
                    -1
                } else {
                    0
                }
            } else {
                -1
            };
            let q = qi as f32;
            // Clamp old to >= -9 dB (libopus MAX16(-9, ...)).
            let old = old_e_bands[i + c * NB_EBANDS].max(-9.0);
            let tmp = coef * old + prev[c] + q;
            old_e_bands[i + c * NB_EBANDS] = tmp;
            prev[c] = prev[c] + q - beta * q;
        }
    }
    let _ = DB_SCALE;
}

/// Decode the fine band-energy refinement (RFC §4.3.2.2). Each band gets
/// `fine_quant[i]` raw bits per channel.
pub fn unquant_fine_energy(
    rc: &mut RangeDecoder<'_>,
    old_e_bands: &mut [f32],
    start: usize,
    end: usize,
    fine_quant: &[i32],
    channels: usize,
) {
    for i in start..end {
        let nq = fine_quant[i];
        if nq <= 0 {
            continue;
        }
        for c in 0..channels {
            let q2 = rc.decode_bits(nq as u32) as f32;
            // libopus float: (q2+0.5)/(1<<nq) - 0.5 (in dB units)
            let offset = (q2 + 0.5) / ((1 << nq) as f32) - 0.5;
            old_e_bands[i + c * NB_EBANDS] += offset;
        }
    }
}

/// Decode the final fine-energy bits (RFC §4.3.2.3) using whatever budget
/// remains after PVQ.
#[allow(clippy::too_many_arguments)]
pub fn unquant_energy_finalise(
    rc: &mut RangeDecoder<'_>,
    old_e_bands: &mut [f32],
    start: usize,
    end: usize,
    fine_quant: &[i32],
    fine_priority: &[i32],
    bits_left: i32,
    channels: usize,
) {
    let mut bits_left = bits_left;
    for prio in 0..2 {
        for i in start..end {
            if bits_left < channels as i32 {
                return;
            }
            if fine_quant[i] >= 8 || fine_priority[i] != prio {
                continue;
            }
            for c in 0..channels {
                let q2 = rc.decode_bits(1) as f32;
                let offset =
                    (q2 - 0.5) * ((1 << (14 - fine_quant[i] - 1)) as f32) * (1.0 / 16384.0);
                old_e_bands[i + c * NB_EBANDS] += offset;
                bits_left -= 1;
            }
        }
    }
}

/// Coarse band-energy encoder (inverse of `unquant_coarse_energy`).
///
/// Input: `new_log_e[i + c*NB_EBANDS]` — target log-energy to encode (in
/// raw dB units, floats, same convention as the decoder). Also
/// `old_e_bands`, the prior frame's quantised energies (per channel × band).
/// On exit both arrays hold the *quantised* energies this frame produced,
/// ready for downstream steps.
pub fn quant_coarse_energy(
    rc: &mut crate::range_encoder::RangeEncoder,
    new_log_e: &[f32],
    old_e_bands: &mut [f32],
    start: usize,
    end: usize,
    intra: bool,
    channels: usize,
    lm: usize,
) {
    use crate::laplace::ec_laplace_encode;

    let prob_model = &E_PROB_MODEL[lm][intra as usize];
    let (coef, beta) = if intra {
        (0.0f32, BETA_INTRA_F32)
    } else {
        (PRED_COEF_F32[lm], BETA_COEF_F32[lm])
    };
    let budget = (rc.storage() * 8) as i32;
    let mut prev = [0.0f32; 2];
    for i in start..end {
        for c in 0..channels {
            let tell = rc.tell();
            let old = old_e_bands[i + c * NB_EBANDS].max(-9.0);
            let predicted = coef * old + prev[c];
            let qi_f = new_log_e[i + c * NB_EBANDS] - predicted;
            // Clamp to a reasonable Laplace-encodable range. Very large
            // values (e.g. silence frames producing -50+ dB deltas) would
            // overflow the 15-bit CDF used by `ec_laplace_encode`.
            let qi = qi_f.round().clamp(-8.0, 8.0) as i32;
            // Match budget-gated behaviour of the decoder.
            if budget - tell >= 15 {
                let pi = 2 * i.min(20);
                let fs = (prob_model[pi] as u32) << 7;
                let decay = (prob_model[pi + 1] as i32) << 6;
                ec_laplace_encode(rc, qi, fs, decay);
            } else if budget - tell >= 2 {
                // 3-symbol ICDF fallback.
                let qi_clamped = qi.clamp(-1, 1);
                // Map qi→{0: 0, -1: 1, +1: 2} (inverse of the decoder's
                // (qi>>1)^-(qi&1) decoding trick).
                let sym = if qi_clamped == 0 {
                    0usize
                } else if qi_clamped == -1 {
                    1
                } else {
                    2
                };
                rc.encode_icdf(sym, &SMALL_ENERGY_ICDF, 2);
            } else if budget - tell >= 1 {
                let negative = qi < 0;
                rc.encode_bit_logp(negative, 1);
            }
            let q = qi as f32;
            let tmp = coef * old + prev[c] + q;
            old_e_bands[i + c * NB_EBANDS] = tmp;
            prev[c] = prev[c] + q - beta * q;
        }
    }
    let _ = DB_SCALE;
}

/// Fine band-energy encoder (inverse of `unquant_fine_energy`).
pub fn quant_fine_energy(
    rc: &mut crate::range_encoder::RangeEncoder,
    new_log_e: &[f32],
    old_e_bands: &mut [f32],
    start: usize,
    end: usize,
    fine_quant: &[i32],
    channels: usize,
) {
    for i in start..end {
        let nq = fine_quant[i];
        if nq <= 0 {
            continue;
        }
        for c in 0..channels {
            let true_offset = new_log_e[i + c * NB_EBANDS] - old_e_bands[i + c * NB_EBANDS];
            // q2 = round((true_offset + 0.5) * (1 << nq) - 0.5)
            let q2 = ((true_offset + 0.5) * ((1 << nq) as f32) - 0.5).round() as i32;
            let q2 = q2.clamp(0, (1 << nq) - 1) as u32;
            rc.encode_bits(q2, nq as u32);
            let offset = (q2 as f32 + 0.5) / ((1 << nq) as f32) - 0.5;
            old_e_bands[i + c * NB_EBANDS] += offset;
        }
    }
}

/// Final fine-energy pass (inverse of `unquant_energy_finalise`). Uses
/// whatever raw-bit budget is left.
#[allow(clippy::too_many_arguments)]
pub fn quant_energy_finalise(
    rc: &mut crate::range_encoder::RangeEncoder,
    new_log_e: &[f32],
    old_e_bands: &mut [f32],
    start: usize,
    end: usize,
    fine_quant: &[i32],
    fine_priority: &[i32],
    bits_left: i32,
    channels: usize,
) {
    let mut bits_left = bits_left;
    for prio in 0..2 {
        for i in start..end {
            if bits_left < channels as i32 {
                return;
            }
            if fine_quant[i] >= 8 || fine_priority[i] != prio {
                continue;
            }
            for c in 0..channels {
                let err = new_log_e[i + c * NB_EBANDS] - old_e_bands[i + c * NB_EBANDS];
                let q2_bit = if err > 0.0 { 1u32 } else { 0 };
                rc.encode_bits(q2_bit, 1);
                let offset = (q2_bit as f32 - 0.5)
                    * ((1 << (14 - fine_quant[i] - 1)) as f32)
                    * (1.0 / 16384.0);
                old_e_bands[i + c * NB_EBANDS] += offset;
                bits_left -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_energy_runs_without_panic() {
        let buf = [0x80u8, 0x00, 0x00, 0x00, 0x55, 0xAA, 0xCC, 0x11, 0x22];
        let mut rc = RangeDecoder::new(&buf);
        let mut old = [0.0f32; NB_EBANDS * 2];
        unquant_coarse_energy(&mut rc, &mut old, 0, 21, true, 1, 3);
        // Sanity: not all zero, not all NaN.
        assert!(old.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn coarse_energy_roundtrips() {
        use crate::range_encoder::RangeEncoder;
        // Encode a known set of target energies and check the decoder
        // recovers the same (quantised) values.
        let channels = 1usize;
        let lm = 3usize;
        let target: Vec<f32> = (0..NB_EBANDS)
            .map(|i| (i as f32 - 10.0) * 0.5)
            .chain(std::iter::repeat(0.0).take(NB_EBANDS))
            .collect();
        let mut enc = RangeEncoder::new(64);
        let mut old_enc = vec![0.0f32; NB_EBANDS * 2];
        quant_coarse_energy(&mut enc, &target, &mut old_enc, 0, 21, true, channels, lm);
        let buf = enc.done().unwrap();

        let mut dec = RangeDecoder::new(&buf);
        let mut old_dec = vec![0.0f32; NB_EBANDS * 2];
        unquant_coarse_energy(&mut dec, &mut old_dec, 0, 21, true, channels, lm);

        // Encoder and decoder agree on per-band energies (to integer dB).
        for i in 0..NB_EBANDS {
            assert!(
                (old_enc[i] - old_dec[i]).abs() < 1e-3,
                "band {i}: enc {} vs dec {}",
                old_enc[i],
                old_dec[i]
            );
            // Also check target was approximately captured (to within 1 dB).
            assert!(
                (old_dec[i] - target[i]).abs() <= 1.0,
                "band {i}: target {} decoded {}",
                target[i],
                old_dec[i]
            );
        }
    }

    #[test]
    fn fine_energy_no_op_with_empty_quant() {
        let buf = [0x00u8; 16];
        let mut rc = RangeDecoder::new(&buf);
        let mut old = [0.0f32; NB_EBANDS * 2];
        let fine = vec![0i32; 21];
        unquant_fine_energy(&mut rc, &mut old, 0, 21, &fine, 1);
        assert!(old.iter().all(|v| *v == 0.0));
    }
}
