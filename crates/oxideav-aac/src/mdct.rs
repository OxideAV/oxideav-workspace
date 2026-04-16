//! Forward MDCT — the analysis counterpart to [`crate::imdct`].
//!
//! Mirrors the same direct (O(N²)) form. The IMDCT formula (eq. 4.A.43) is
//!   x[n] = 2/N · Σ_{k=0..N/2-1} spec[k] · cos((2π/N)(n+n0)(k+0.5))
//! so the forward transform that round-trips with overlap-add is
//!   spec[k] = Σ_{n=0..N-1} x[n] · cos((2π/N)(n+n0)(k+0.5))
//! with the (2/N) sitting entirely on the inverse side. With a
//! Princen-Bradley sine (or KBD) window applied symmetrically the OLA of
//! consecutive blocks reproduces the input exactly.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::imdct::{LONG_INPUT, SHORT_INPUT};

/// Cosine table cached per `input_n`. `tbl[k * (2*input_n) + n]` —
/// inner loop iterates over `n` for a fixed `k`.
struct CosTable {
    tbl: Vec<f32>,
    n_in: usize,
}

impl CosTable {
    fn new(input_n: usize) -> Self {
        let n_total = 2 * input_n;
        let n0 = (input_n as f64 + 1.0) / 2.0;
        let mut tbl = vec![0.0f32; n_total * input_n];
        for k in 0..input_n {
            for n in 0..n_total {
                let arg = (2.0 * PI / n_total as f64) * (n as f64 + n0) * (k as f64 + 0.5);
                tbl[k * n_total + n] = arg.cos() as f32;
            }
        }
        Self { tbl, n_in: input_n }
    }

    #[inline]
    fn row(&self, k: usize) -> &[f32] {
        let n_total = 2 * self.n_in;
        &self.tbl[k * n_total..(k + 1) * n_total]
    }
}

static LONG_COS: OnceLock<CosTable> = OnceLock::new();
static SHORT_COS: OnceLock<CosTable> = OnceLock::new();

fn long_cos() -> &'static CosTable {
    LONG_COS.get_or_init(|| CosTable::new(LONG_INPUT))
}

fn short_cos() -> &'static CosTable {
    SHORT_COS.get_or_init(|| CosTable::new(SHORT_INPUT))
}

fn mdct_direct(time: &[f32], spec: &mut [f32], cos: &CosTable, input_n: usize) {
    let n_total = 2 * input_n;
    debug_assert_eq!(time.len(), n_total);
    debug_assert!(spec.len() >= input_n);
    // Unscaled forward — combined with the existing 2/N inverse scale,
    // sine windows with the partition-of-unity property give exact TDAC
    // OLA reconstruction. Empirically (see `diagnose_alias_map_short`):
    //   IMDCT(MDCT(δ_n))[m] = δ_n[m]              for non-aliased m
    //                        - δ[L-1-n]            for first half (n<L)
    //                        + δ[3L-1-n]           for second half
    // The minus/plus structure makes the cross-block aliases cancel
    // exactly when the next/previous block contributes the right
    // mirror-windowed terms.
    for k in 0..input_n {
        let row = cos.row(k);
        let mut acc = 0.0f32;
        for n in 0..n_total {
            acc += time[n] * row[n];
        }
        spec[k] = acc;
    }
}

/// Long-block MDCT (2048 in, 1024 out).
pub fn mdct_long(time: &[f32], spec: &mut [f32]) {
    mdct_direct(time, spec, long_cos(), LONG_INPUT);
}

/// Short-block MDCT (256 in, 128 out).
pub fn mdct_short(time: &[f32], spec: &mut [f32]) {
    mdct_direct(time, spec, short_cos(), SHORT_INPUT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imdct::{imdct_long, imdct_short};
    use crate::window::{sine_long, sine_short, LONG_LEN, SHORT_LEN};

    /// MDCT(IMDCT(δ_k)) = 2·δ_k under the (forward 1.0 · inverse 2/N) pair.
    /// The factor-of-2 reflects the basis inner product Σ A[n,k]² = N
    /// over n in [0, 2L). It self-cancels under proper TDAC OLA — see
    /// `long_round_trip_with_sine_ola`.
    #[test]
    fn round_trip_unit_basis_long_gives_2x() {
        let input_n = LONG_INPUT;
        let mut spec = vec![0.0f32; input_n];
        spec[5] = 1.0;
        let mut time = vec![0.0f32; 2 * input_n];
        imdct_long(&spec, &mut time);
        let mut spec2 = vec![0.0f32; input_n];
        mdct_long(&time, &mut spec2);
        let on = spec2[5];
        let off: f32 = (0..input_n)
            .filter(|&k| k != 5)
            .map(|k| spec2[k].abs())
            .sum::<f32>()
            / (input_n as f32 - 1.0);
        assert!((on - 2.0).abs() < 0.05, "on bin = {on}, want 2");
        assert!(off < 0.05, "off energy {off}");
    }

    /// Verify that windowed MDCT followed by windowed IMDCT recovers the
    /// input under sine/sine OLA across two consecutive blocks.
    #[test]
    fn long_round_trip_with_sine_ola() {
        let n = LONG_LEN;
        let n2 = 2 * n;
        // Construct a known sequence: x[i] = sin(2*pi*5*i / N).
        let total = 3 * n;
        let mut x = vec![0.0f32; total];
        for i in 0..total {
            x[i] = (2.0 * PI * 5.0 * i as f64 / n as f64).sin() as f32;
        }
        let win = sine_long();
        // Block 0: time samples 0..2N, windowed with sine.
        let mut t0 = vec![0.0f32; n2];
        // Block 1: time samples N..3N (advances N; overlaps half).
        let mut t1 = vec![0.0f32; n2];
        for i in 0..n {
            t0[i] = x[i] * win[i];
            t0[n + i] = x[n + i] * win[n - 1 - i];
            t1[i] = x[n + i] * win[i];
            t1[n + i] = x[2 * n + i] * win[n - 1 - i];
        }
        let mut s0 = vec![0.0f32; n];
        let mut s1 = vec![0.0f32; n];
        mdct_long(&t0, &mut s0);
        mdct_long(&t1, &mut s1);

        let mut o0 = vec![0.0f32; n2];
        let mut o1 = vec![0.0f32; n2];
        imdct_long(&s0, &mut o0);
        imdct_long(&s1, &mut o1);
        // Re-window.
        for i in 0..n {
            o0[i] *= win[i];
            o0[n + i] *= win[n - 1 - i];
            o1[i] *= win[i];
            o1[n + i] *= win[n - 1 - i];
        }
        // Reconstruct samples N..2N as o0[N..2N] + o1[0..N].
        let mut max_err = 0.0f32;
        for i in 0..n {
            let recon = o0[n + i] + o1[i];
            let want = x[n + i];
            let err = (recon - want).abs();
            if err > max_err {
                max_err = err;
            }
        }
        assert!(max_err < 1e-3, "round-trip max err {max_err}");
    }

    #[test]
    fn short_round_trip_dc() {
        // Constant DC signal through MDCT/IMDCT/OLA should be near-constant
        // in the overlap region.
        let n = SHORT_LEN;
        let n2 = 2 * n;
        let win = sine_short();
        let mut t0 = vec![0.0f32; n2];
        let mut t1 = vec![0.0f32; n2];
        for i in 0..n {
            t0[i] = win[i]; // x[i] = 1.0; block 0 covers x[0..2N]
            t0[n + i] = win[n - 1 - i];
            t1[i] = win[i]; // block 1 covers x[N..3N]
            t1[n + i] = win[n - 1 - i];
        }
        let mut s0 = vec![0.0f32; n];
        let mut s1 = vec![0.0f32; n];
        mdct_short(&t0, &mut s0);
        mdct_short(&t1, &mut s1);
        let mut o0 = vec![0.0f32; n2];
        let mut o1 = vec![0.0f32; n2];
        imdct_short(&s0, &mut o0);
        imdct_short(&s1, &mut o1);
        for i in 0..n {
            o0[i] *= win[i];
            o0[n + i] *= win[n - 1 - i];
            o1[i] *= win[i];
            o1[n + i] *= win[n - 1 - i];
        }
        // x[N+n] = 1.0 ; reconstruction comes from the right half of o0
        // plus the left half of o1.
        for i in 0..n {
            let recon = o0[n + i] + o1[i];
            assert!((recon - 1.0).abs() < 1e-3, "short OLA at {i}: {recon}");
        }
    }
}
