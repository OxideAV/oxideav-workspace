//! Pure-Rust IFFT scaffold and IMDCT placeholder (RFC 6716 §4.3.7).
//!
//! Lands a verified iterative radix-2 inverse complex FFT (`ifft_radix2`)
//! that the next agent can wrap into a full IMDCT-via-FFT (CELT uses an
//! IMDCT of length 2N where N = 120 << LM; the standard "DCT-IV via N/2
//! complex FFT" trick is the right pattern).
//!
//! What's NOT yet landed:
//!
//! * Pre-twiddle / post-twiddle factors — exposed as a TODO in the
//!   placeholder `imdct` function below.
//! * Window application + overlap-add against the previous frame's tail
//!   (RFC §4.3.7 final paragraph + libopus `clt_mdct_backward`).
//! * Short-block (transient) splitting into 2/4/8 sub-MDCTs.

use core::f32::consts::PI;

/// Iterative bit-reversal permutation (in place).
fn bit_reverse(a: &mut [(f32, f32)]) {
    let n = a.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            a.swap(i, j);
        }
    }
}

/// In-place radix-2 complex IFFT. `a.len()` must be a power of two.
///
/// Convention: e^{+2πi/N} (positive sign exponent) and 1/N normalisation,
/// matching `numpy.fft.ifft`.
pub fn ifft_radix2(a: &mut [(f32, f32)]) {
    let n = a.len();
    debug_assert!(n.is_power_of_two());
    bit_reverse(a);
    let mut size = 2usize;
    while size <= n {
        let half = size / 2;
        let theta = 2.0 * PI / size as f32;
        let (wr_step, wi_step) = (theta.cos(), theta.sin());
        let mut i = 0;
        while i < n {
            let (mut wr, mut wi) = (1.0f32, 0.0f32);
            for k in 0..half {
                let (xr, xi) = a[i + k];
                let (yr, yi) = a[i + k + half];
                let tr = yr * wr - yi * wi;
                let ti = yr * wi + yi * wr;
                a[i + k] = (xr + tr, xi + ti);
                a[i + k + half] = (xr - tr, xi - ti);
                let (new_wr, new_wi) = (wr * wr_step - wi * wi_step, wr * wi_step + wi * wr_step);
                wr = new_wr;
                wi = new_wi;
            }
            i += size;
        }
        size <<= 1;
    }
    let inv_n = 1.0 / n as f32;
    for s in a.iter_mut() {
        s.0 *= inv_n;
        s.1 *= inv_n;
    }
}

/// Inverse MDCT placeholder.
///
/// Currently UNIMPLEMENTED — the full CELT IMDCT (RFC 6716 §4.3.7) needs:
///   1. pre-twiddle of `coeff` into a length-N/2 complex sequence,
///   2. one IFFT (use `ifft_radix2`),
///   3. post-twiddle into 2N real samples,
///   4. window + overlap-add with previous frame's tail.
///
/// For now this just zeros `out`, so callers can wire the rest of the
/// pipeline up without panicking.
pub fn imdct(coeff: &[f32], out: &mut [f32]) {
    let _ = coeff;
    for v in out.iter_mut() {
        *v = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ifft_round_trips_dc() {
        let mut a = vec![(1.0f32, 0.0f32); 8];
        ifft_radix2(&mut a);
        // After IFFT of constant, sample [0] is the average → 1.0; rest near 0.
        assert!((a[0].0 - 1.0).abs() < 1e-5);
        for s in &a[1..] {
            assert!(s.0.abs() < 1e-5 && s.1.abs() < 1e-5);
        }
    }

    /// IFFT of an impulse is a flat array (all-1 / N after normalisation).
    #[test]
    fn ifft_of_impulse_is_flat() {
        let mut a = vec![(0.0f32, 0.0f32); 16];
        a[0] = (16.0, 0.0);
        ifft_radix2(&mut a);
        for s in &a {
            assert!((s.0 - 1.0).abs() < 1e-5, "expected 1.0, got {:?}", s);
        }
    }

    #[test]
    fn imdct_placeholder_is_silent() {
        let coeff = vec![1.0f32; 16];
        let mut out = vec![0.5f32; 32];
        imdct(&coeff, &mut out);
        assert!(out.iter().all(|v| *v == 0.0));
    }
}
