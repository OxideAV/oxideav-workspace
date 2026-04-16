//! MPEG-1 Layer III polyphase synthesis filter bank.
//!
//! Same 32-band → 32-output-samples-per-step filter as Layers I/II. The
//! per-channel state carries a 1024-sample FIFO `v`. Each step:
//!   1. Shift v[64..1024] = v[0..960] (or reverse).
//!   2. Matrix v[0..64] = M · subbands[0..32] with
//!      M[i][k] = cos(((2k+1)(16+i)) * pi / 64).
//!   3. Build 512 u[] entries by interleaving v[].
//!   4. u *= D (synthesis window).
//!   5. Output 32 samples = sum of windowed taps.

use crate::window::synthesis_window;

/// Per-channel synthesis state.
pub struct SynthesisState {
    v: Box<[f32; 1024]>,
}

impl Default for SynthesisState {
    fn default() -> Self {
        Self {
            v: Box::new([0.0; 1024]),
        }
    }
}

impl SynthesisState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run one step: 32 subband inputs → 32 PCM outputs.
    pub fn synthesize(&mut self, subbands: &[f32; 32], out: &mut [f32; 32]) {
        // 1. Shift v.
        for i in (64..1024).rev() {
            self.v[i] = self.v[i - 64];
        }

        // 2. Matrix.
        let m = matrix();
        for i in 0..64 {
            let mut acc = 0.0f32;
            for k in 0..32 {
                acc += m[i][k] * subbands[k];
            }
            self.v[i] = acc;
        }

        // 3. Build u[].
        let mut u = [0.0f32; 512];
        for i in 0..8 {
            for j in 0..32 {
                u[64 * i + j] = self.v[128 * i + j];
                u[64 * i + 32 + j] = self.v[128 * i + 96 + j];
            }
        }

        // 4. Window.
        let d = synthesis_window();
        for i in 0..512 {
            u[i] *= d[i];
        }

        // 5. Sum 16 taps per output sample.
        for j in 0..32 {
            let mut acc = 0.0f32;
            for i in 0..16 {
                acc += u[32 * i + j];
            }
            out[j] = acc;
        }
    }
}

/// Run 18 synthesis steps on a subband-major 32×18 buffer to produce 576
/// PCM samples for one granule, one channel. Output is time-interleaved
/// in `pcm_out[0..576]`.
pub fn synthesize_granule(
    state: &mut SynthesisState,
    subband_samples: &[[f32; 18]; 32],
    pcm_out: &mut [f32; 576],
) {
    for step in 0..18 {
        let mut sub = [0.0f32; 32];
        for sb in 0..32 {
            sub[sb] = subband_samples[sb][step];
        }
        let mut out = [0.0f32; 32];
        state.synthesize(&sub, &mut out);
        for j in 0..32 {
            pcm_out[step * 32 + j] = out[j];
        }
    }
}

// Build the 64×32 matrix lazily.
use std::sync::OnceLock;
static MATRIX_STORAGE: OnceLock<[[f32; 32]; 64]> = OnceLock::new();

fn matrix() -> &'static [[f32; 32]; 64] {
    MATRIX_STORAGE.get_or_init(|| {
        let mut m = [[0.0f32; 32]; 64];
        let pi = std::f64::consts::PI;
        for i in 0..64 {
            for k in 0..32 {
                let angle = ((2 * k + 1) as f64) * ((16 + i) as f64) * pi / 64.0;
                m[i][k] = angle.cos() as f32;
            }
        }
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesis_zero_input_converges_to_silence() {
        let mut state = SynthesisState::new();
        let sub = [0.0f32; 32];
        let mut out = [0.0f32; 32];
        for _ in 0..16 {
            state.synthesize(&sub, &mut out);
        }
        for s in out.iter() {
            assert!(s.abs() < 1e-5);
        }
    }

    #[test]
    fn synthesis_impulse_is_finite() {
        let mut state = SynthesisState::new();
        let mut sub = [0.0f32; 32];
        sub[0] = 1.0;
        let mut out = [0.0f32; 32];
        state.synthesize(&sub, &mut out);
        for s in out.iter() {
            assert!(s.is_finite());
        }
    }

    #[test]
    fn synthesize_granule_outputs_576_samples() {
        let mut state = SynthesisState::new();
        let sub = [[0.0f32; 18]; 32];
        let mut pcm = [0.0f32; 576];
        synthesize_granule(&mut state, &sub, &mut pcm);
        // All silence -> output all zeros.
        assert!(pcm.iter().all(|&v| v.abs() < 1e-5));
    }
}
