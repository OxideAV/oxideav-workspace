//! MPEG-1 Layer III IMDCT + windowing + overlap-add.
//!
//! The hybrid filterbank transforms 18 frequency-domain coefficients per
//! subband into 18 time-domain samples (long blocks) or 3 sets of 6
//! samples (short blocks). The 36-point IMDCT is preceded by Shlien's
//! DCT-IV trick → 18-point DCT, but for bring-up a direct IMDCT works
//! fine.
//!
//! Formula (long blocks, 36 samples):
//!   x[i] = sum_{k=0..17} X[k] * cos( pi/72 * (2i + 1 + 18) * (2k + 1) )
//!
//! Short blocks use a 12-point IMDCT repeated 3 times with overlapping
//! windows.
//!
//! After IMDCT the 36-sample block is windowed and overlap-added with the
//! previous granule's "overlap" half.

use crate::window::{imdct_window_long, imdct_window_short};

/// Per-channel overlap state for the IMDCT. 32 subbands × 18 samples of
/// carry-over per channel.
#[derive(Clone)]
pub struct ImdctState {
    pub overlap: [[f32; 18]; 32],
}

impl Default for ImdctState {
    fn default() -> Self {
        Self {
            overlap: [[0.0; 18]; 32],
        }
    }
}

impl ImdctState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Run IMDCT + window + overlap-add for one granule's 576 frequency
/// coefficients. Outputs a 32×18 time-domain array (subband-major order).
pub fn imdct_granule(
    xr: &[f32; 576],
    out: &mut [[f32; 18]; 32],
    state: &mut ImdctState,
    block_type: u8,
    mixed_block_flag: bool,
) {
    // MDCT freq-domain anti-alias convention: the xr array is laid out
    // subband-major already (first 18 samples = subband 0, next 18 =
    // subband 1, etc).
    for sb in 0..32 {
        let base = sb * 18;
        let sub_bt = if mixed_block_flag && sb < 2 {
            0
        } else {
            block_type
        };
        let mut raw = [0.0f32; 36];

        if sub_bt == 2 {
            // Short blocks: 3x 12-point IMDCT + short window.
            let win = imdct_window_short();
            let mut tmp = [0.0f32; 36];
            for w in 0..3 {
                let coeffs: [f32; 6] = [
                    xr[base + w],
                    xr[base + w + 3],
                    xr[base + w + 6],
                    xr[base + w + 9],
                    xr[base + w + 12],
                    xr[base + w + 15],
                ];
                let mut out12 = [0.0f32; 12];
                imdct_12(&coeffs, &mut out12);
                // Window and place in tmp at offset 6*w + 6.
                for i in 0..12 {
                    tmp[6 * w + 6 + i] += out12[i] * win[i];
                }
            }
            raw = tmp;
        } else {
            // Long / start / stop.
            let mut x = [0.0f32; 18];
            x.copy_from_slice(&xr[base..base + 18]);
            imdct_36(&x, &mut raw);
            let win = imdct_window_long(sub_bt);
            for i in 0..36 {
                raw[i] *= win[i];
            }
        }

        // Every odd subband gets alternate-sample sign flip (freq→time
        // mirroring per spec §2.4.3.4). Done only when block_type != 2.
        if sb & 1 == 1 && sub_bt != 2 {
            for i in 0..18 {
                raw[18 + i] = -raw[18 + i];
                let _ = i;
            }
        }

        // Overlap-add: first 18 samples + previous overlap -> out[sb].
        for i in 0..18 {
            out[sb][i] = raw[i] + state.overlap[sb][i];
            state.overlap[sb][i] = raw[18 + i];
        }
    }
}

/// 36-point IMDCT. Straightforward O(N^2) form — swap for a fast DCT when
/// performance matters.
fn imdct_36(x: &[f32; 18], out: &mut [f32; 36]) {
    let pi = std::f32::consts::PI;
    for n in 0..36 {
        let mut acc = 0.0f32;
        for k in 0..18 {
            let phase = pi / 72.0 * ((2 * n + 1 + 18) as f32) * ((2 * k + 1) as f32);
            acc += x[k] * phase.cos();
        }
        out[n] = acc;
    }
}

fn imdct_12(x: &[f32; 6], out: &mut [f32; 12]) {
    let pi = std::f32::consts::PI;
    for n in 0..12 {
        let mut acc = 0.0f32;
        for k in 0..6 {
            let phase = pi / 24.0 * ((2 * n + 1 + 6) as f32) * ((2 * k + 1) as f32);
            acc += x[k] * phase.cos();
        }
        out[n] = acc;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imdct_zero_input_is_zero() {
        let xr = [0.0f32; 576];
        let mut out = [[0.0f32; 18]; 32];
        let mut state = ImdctState::new();
        imdct_granule(&xr, &mut out, &mut state, 0, false);
        for sb in 0..32 {
            for i in 0..18 {
                assert!(out[sb][i].abs() < 1e-6);
            }
        }
    }

    #[test]
    fn imdct_produces_finite_output() {
        let mut xr = [0.0f32; 576];
        xr[0] = 1.0;
        xr[18] = 1.0;
        let mut out = [[0.0f32; 18]; 32];
        let mut state = ImdctState::new();
        imdct_granule(&xr, &mut out, &mut state, 0, false);
        for sb in 0..32 {
            for i in 0..18 {
                assert!(out[sb][i].is_finite());
            }
        }
    }
}
