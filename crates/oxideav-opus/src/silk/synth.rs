//! SILK synthesis filter — RFC 6716 §4.2.7.9.
//!
//! Applies LTP + short-term LPC synthesis to the excitation to
//! reconstruct the internal-rate output. The output is then upsampled
//! to 48 kHz to match Opus's fixed output rate.
//!
//! For an MVP the LTP stage is coalesced with the LPC stage via direct
//! all-pole synthesis — a simplification that still yields audible
//! speech-like output but gives up bit-exactness.

use crate::silk::SilkChannelState;

/// Synthesize internal-rate output from excitation + filter parameters.
///
/// * `excitation` — Q0 excitation samples, length = frame_len.
/// * `lpc` — LPC coefficients (length = `lpc_order`).
/// * `gains_q16` — per sub-frame synthesis gain (Q16).
/// * `pitch_lags` / `ltp_filter` — per sub-frame LTP params; applied
///   for voiced sub-frames.
/// * `ltp_scale_q14` — LTP scaling factor (Q14).
/// * `subframe_len` — sub-frame length at internal rate (40/60/80).
/// * `lpc_order` — 10 for NB/MB, 16 for WB.
/// * `voiced` — apply LTP only when true.
/// * `state` — persistent state (history buffers, prev gain).
pub fn synthesize(
    excitation: &[f32],
    lpc: &[f32],
    gains_q16: &[i32; 4],
    pitch_lags: &[i32; 4],
    ltp_filter: &[[f32; 5]; 4],
    ltp_scale_q14: i32,
    subframe_len: usize,
    lpc_order: usize,
    voiced: bool,
    state: &mut SilkChannelState,
) -> Vec<f32> {
    let frame_len = excitation.len();
    let mut out = vec![0f32; frame_len];

    // Ensure history buffers are large enough.
    if state.lpc_history.len() < lpc_order {
        state.lpc_history.resize(lpc_order, 0.0);
    }
    let ltp_hist_len = 480usize;
    if state.ltp_history.len() < ltp_hist_len {
        state.ltp_history.resize(ltp_hist_len, 0.0);
    }

    let ltp_scale = ltp_scale_q14 as f32 / 16384.0;

    for sf in 0..4 {
        let sf_start = sf * subframe_len;
        let sf_end = sf_start + subframe_len;
        // Overall gain for this sub-frame (Q16 → f32). Scale down to
        // avoid filter blow-up: the excitation here is already in Q0
        // "raw" units from the MVP excitation generator, so we reduce
        // the gain by an additional factor to keep output in [-1, 1].
        let g = (gains_q16[sf].max(1) as f32 / 65536.0) * 1.0e-3;
        let taps = &ltp_filter[sf];
        let lag = pitch_lags[sf];

        for n in sf_start..sf_end {
            // Gained excitation.
            let mut e = excitation[n] * g;

            // LTP contribution (voiced only).
            if voiced && lag > 0 {
                let mut ltp_sum = 0f32;
                for k in 0..5 {
                    let lag_k = lag + (k as i32 - 2);
                    let idx = n as i32 - lag_k;
                    let past = if idx >= 0 {
                        out[idx as usize]
                    } else {
                        let hi = (ltp_hist_len as i32 + idx) as usize;
                        state.ltp_history.get(hi).copied().unwrap_or(0.0)
                    };
                    ltp_sum += taps[k] * past;
                }
                e += ltp_sum * ltp_scale * 0.25;
            }

            // Short-term LPC synthesis: out[n] = e + sum_{k=1..order} lpc[k-1] * out[n-k].
            let mut s = e;
            for k in 1..=lpc_order {
                let idx = n as i32 - k as i32;
                let past = if idx >= 0 {
                    out[idx as usize]
                } else {
                    // Use LPC history (last `lpc_order` samples of the
                    // previous frame).
                    let h_idx = (state.lpc_history.len() as i32 + idx) as usize;
                    state.lpc_history.get(h_idx).copied().unwrap_or(0.0)
                };
                s += lpc[k - 1] * past;
            }
            // Saturate gently to prevent runaway feedback.
            out[n] = s.clamp(-1.0, 1.0);
        }
    }

    // Update state history for next frame.
    let lpc_keep = lpc_order.min(out.len());
    state.lpc_history = out[out.len() - lpc_keep..].to_vec();

    // Shift LTP history.
    let keep = ltp_hist_len.saturating_sub(frame_len);
    let mut new_ltp = Vec::with_capacity(ltp_hist_len);
    new_ltp.extend_from_slice(&state.ltp_history[ltp_hist_len - keep..]);
    new_ltp.extend_from_slice(&out);
    if new_ltp.len() > ltp_hist_len {
        let drop = new_ltp.len() - ltp_hist_len;
        new_ltp.drain(0..drop);
    } else if new_ltp.len() < ltp_hist_len {
        let mut pad = vec![0f32; ltp_hist_len - new_ltp.len()];
        pad.extend(new_ltp);
        new_ltp = pad;
    }
    state.ltp_history = new_ltp;

    out
}

/// Upsample the internal-rate signal to 48 kHz.
///
/// Uses a simple 2× zero-stuff + 2-tap FIR for 8→16, repeated for
/// 16→48 (×3 zero-stuff + FIR). Not a perfect filter but adequate for
/// an audibility test.
pub fn upsample_to_48k(samples: &[f32], src_rate: u32) -> Vec<f32> {
    match src_rate {
        8_000 => upsample(samples, 6),
        12_000 => upsample(samples, 4),
        16_000 => upsample(samples, 3),
        24_000 => upsample(samples, 2),
        48_000 => samples.to_vec(),
        _ => upsample(samples, 48_000 / src_rate as u32),
    }
}

/// Integer-ratio upsample by `factor`, followed by a short low-pass
/// FIR to smear the zero-inserted samples.
fn upsample(samples: &[f32], factor: u32) -> Vec<f32> {
    let f = factor as usize;
    if f <= 1 {
        return samples.to_vec();
    }
    let mut upsampled = vec![0f32; samples.len() * f];
    for (i, &s) in samples.iter().enumerate() {
        upsampled[i * f] = s * (f as f32);
    }
    // Simple symmetric low-pass (hann window, length = 2*f+1).
    let win_len = 2 * f + 1;
    let mut win = vec![0f32; win_len];
    for k in 0..win_len {
        let phase = (k as f32 - f as f32) * core::f32::consts::PI / (f as f32);
        let sinc = if phase.abs() < 1e-6 {
            1.0
        } else {
            phase.sin() / phase
        };
        let hann =
            0.5 - 0.5 * (2.0 * core::f32::consts::PI * k as f32 / (win_len as f32 - 1.0)).cos();
        win[k] = sinc * hann;
    }
    let gain: f32 = win.iter().sum();
    for w in win.iter_mut() {
        *w /= gain;
    }

    let mut out = vec![0f32; upsampled.len()];
    for n in 0..upsampled.len() {
        let mut acc = 0f32;
        for k in 0..win_len {
            let idx = n as i32 + k as i32 - f as i32;
            if idx >= 0 && (idx as usize) < upsampled.len() {
                acc += win[k] * upsampled[idx as usize];
            }
        }
        out[n] = acc;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsample_length() {
        let input = vec![0.0; 160];
        let out = upsample_to_48k(&input, 8_000);
        assert_eq!(out.len(), 960);
    }

    #[test]
    fn upsample_factor_1() {
        let input = vec![1.0, 2.0, 3.0];
        let out = upsample_to_48k(&input, 48_000);
        assert_eq!(out, input);
    }
}
