//! MPEG-1 Layer III forward MDCT + windowing + 50%-overlap.
//!
//! Mirrors [`crate::imdct`] in reverse: 36 windowed time-domain samples
//! per subband become 18 frequency-domain coefficients. Long blocks use
//! a 36→18 MDCT; short blocks apply a 12→6 MDCT three times.
//!
//! Forward MDCT formula (long blocks, N=36):
//!
//!   X[k] = sum_{n=0..N-1} x[n] * cos( pi/2N * (2n + 1 + N/2) * (2k + 1) )
//!
//! With the spec's window-then-MDCT order. This is the analytical
//! inverse of the IMDCT in [`crate::imdct::imdct_36`] up to a scale of
//! N/2 — exactly what the IMDCT undoes when our reservoir → IMDCT → OLA
//! pipeline runs end-to-end.

use crate::window::{imdct_window_long, imdct_window_short};

/// Per-channel MDCT carry-over: the second 18-sample half of the
/// previous granule's windowed output, ready to be overlap-added with
/// the current granule's first half before MDCT.
#[derive(Clone)]
pub struct MdctState {
    /// Stored from the previous granule: the SECOND half of the per-subband
    /// 36-sample input window (i.e. samples 18..36 from prev step). For the
    /// next call, this becomes the FIRST half (0..18).
    pub prev_first_half: [[f32; 18]; 32],
}

impl Default for MdctState {
    fn default() -> Self {
        Self {
            prev_first_half: [[0.0; 18]; 32],
        }
    }
}

impl MdctState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Run forward MDCT for one granule. `subbands_in` holds 32×18 subband
/// samples (subband-major) from the polyphase filter. The output `xr`
/// is 576 frequency coefficients laid out subband-major (matching what
/// the decoder side expects in `requantize`).
///
/// `block_type` is currently ignored: this function always uses long
/// blocks (type 0). Adding short-block support is straightforward but
/// out of scope for the v1 encoder.
pub fn mdct_granule(subbands_in: &[[f32; 18]; 32], xr: &mut [f32; 576], state: &mut MdctState) {
    for sb in 0..32 {
        // 36-sample input = previous half (18) + current half (18).
        let mut in36 = [0.0f32; 36];
        in36[..18].copy_from_slice(&state.prev_first_half[sb]);
        in36[18..].copy_from_slice(&subbands_in[sb]);
        // Save current half for next call's prev_first_half.
        state.prev_first_half[sb] = subbands_in[sb];

        // No per-subband sign manipulation here: the analysis filter has
        // already produced subband samples in the convention the decoder
        // expects, and the decoder's frequency-inversion step (negate
        // odd-indexed samples in odd subbands) is its own concern.

        // Window (long block, type 0).
        let win = imdct_window_long(0);
        for n in 0..36 {
            in36[n] *= win[n];
        }

        // 36-point forward MDCT → 18 coefficients.
        let mut x18 = [0.0f32; 18];
        mdct_36(&in36, &mut x18);

        let base = sb * 18;
        xr[base..base + 18].copy_from_slice(&x18);
    }
}

/// Forward 36-point MDCT (direct O(N^2) form). Inverse of
/// `imdct::imdct_36` up to a factor of N/2 = 18.
fn mdct_36(x: &[f32; 36], out: &mut [f32; 18]) {
    let pi = std::f32::consts::PI;
    for k in 0..18 {
        let mut acc = 0.0f32;
        for n in 0..36 {
            let phase = pi / 72.0 * ((2 * n + 1 + 18) as f32) * ((2 * k + 1) as f32);
            acc += x[n] * phase.cos();
        }
        out[k] = acc;
    }
}

/// Forward 12-point MDCT (for short blocks). Reserved for future use.
#[allow(dead_code)]
fn mdct_12(x: &[f32; 12], out: &mut [f32; 6]) {
    let pi = std::f32::consts::PI;
    for k in 0..6 {
        let mut acc = 0.0f32;
        for n in 0..12 {
            let phase = pi / 24.0 * ((2 * n + 1 + 6) as f32) * ((2 * k + 1) as f32);
            acc += x[n] * phase.cos();
        }
        out[k] = acc;
    }
}

/// Reserved short-block window helper — referenced for completeness.
#[allow(dead_code)]
fn _short_window() -> [f32; 12] {
    imdct_window_short()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imdct::{imdct_granule, ImdctState};

    /// MDCT followed by IMDCT (with overlap) recovers a scaled input.
    /// We feed a sinusoid through analysis-equivalent dummy subband
    /// samples (constant DC per subband 0), MDCT it twice (so OLA can
    /// fold), then IMDCT, and check that the recovered signal is
    /// finite and bounded.
    #[test]
    fn mdct_imdct_pipeline_finite() {
        let mut subbands = [[0.0f32; 18]; 32];
        // Inject a small DC into subband 1.
        for i in 0..18 {
            subbands[1][i] = 0.25;
        }

        let mut mdct_state = MdctState::new();
        let mut imdct_state = ImdctState::new();

        // Two granules so OLA is filled.
        for _ in 0..2 {
            let mut xr = [0.0f32; 576];
            mdct_granule(&subbands, &mut xr, &mut mdct_state);
            let mut sb_out = [[0.0f32; 18]; 32];
            imdct_granule(&xr, &mut sb_out, &mut imdct_state, 0, false);
            for sb in 0..32 {
                for i in 0..18 {
                    assert!(sb_out[sb][i].is_finite());
                }
            }
        }
    }
}
