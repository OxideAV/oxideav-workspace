//! AAC windowing — sine and KBD (Kaiser-Bessel-Derived) windows.
//!
//! ISO/IEC 14496-3 §4.6.11 specifies two window shapes:
//! - sine window (W=0): w(n) = sin( pi/N * (n + 0.5) ), 0 ≤ n < N
//! - KBD window (W=1): derived from a Kaiser window of length N/2+1 with
//!   alpha=4 for long (N=1024) and alpha=6 for short (N=128). The window
//!   is constructed so that its squared overlap-add equals 1.
//!
//! Both windows have length N (= 1024 long, 128 short). The first half is
//! used for the prior block's right side, the second half for the new
//! block's left side per §4.6.11 / §4.6.18.

use std::f64::consts::PI;
use std::sync::OnceLock;

/// Long-window length (= IMDCT 2N divided by 2).
pub const LONG_LEN: usize = 1024;
/// Short-window length.
pub const SHORT_LEN: usize = 128;

static SINE_LONG: OnceLock<Vec<f32>> = OnceLock::new();
static SINE_SHORT: OnceLock<Vec<f32>> = OnceLock::new();
static KBD_LONG: OnceLock<Vec<f32>> = OnceLock::new();
static KBD_SHORT: OnceLock<Vec<f32>> = OnceLock::new();

/// Build the AAC/MLT sine window — first half only (length `n`).
///
/// The full 2N-long window is `w[i] = sin((i+0.5)·π/(2N))` for i in [0, 2N).
/// The first half (i in [0, N)) goes from ~0 up to ~sin(π/2 - small) ≈ 1; the
/// second half (i in [N, 2N)) is the time-reversal of the first half (the
/// `synth.rs` doubling-scheme applies `w[N-1-i]` for the falling slope).
/// The squared overlap-add of two consecutive halves gives sin²+cos²=1
/// (partition-of-unity), which is what makes AAC's TDAC OLA reconstruct
/// the input exactly.
fn build_sine(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| ((PI / (2.0 * n as f64)) * (i as f64 + 0.5)).sin() as f32)
        .collect()
}

/// Modified Bessel I0(x) via series expansion. Converges quickly for the
/// alpha values we use (4..=6).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let mut k = 1.0;
    while term > 1e-15 * sum {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
        if k > 200.0 {
            break;
        }
    }
    sum
}

/// Build a KBD window of length `n` with parameter `alpha`.
fn build_kbd(n: usize, alpha: f64) -> Vec<f32> {
    let half = n / 2;
    // Build Kaiser window of length half+1 (per AAC spec).
    let alpha_pi = alpha * PI;
    let denom = bessel_i0(alpha_pi);
    let mut w = vec![0.0f64; half + 1];
    for i in 0..=half {
        let t = (2.0 * i as f64 / half as f64) - 1.0;
        let arg = alpha_pi * (1.0 - t * t).sqrt();
        w[i] = bessel_i0(arg) / denom;
    }
    // Cumulative sum.
    let mut cs = vec![0.0f64; half + 1];
    let mut acc = 0.0;
    for i in 0..=half {
        acc += w[i];
        cs[i] = acc;
    }
    let total = cs[half];
    // KBD[n] = sqrt( cs[n] / total ) for first half, mirror for second.
    let mut out = vec![0.0f32; n];
    for i in 0..half {
        out[i] = (cs[i] / total).sqrt() as f32;
    }
    for i in 0..half {
        // mirror — out[n - 1 - i] = out[i] in standard formulation
        out[n - 1 - i] = out[i];
    }
    out
}

pub fn sine_long() -> &'static [f32] {
    SINE_LONG.get_or_init(|| build_sine(LONG_LEN))
}

pub fn sine_short() -> &'static [f32] {
    SINE_SHORT.get_or_init(|| build_sine(SHORT_LEN))
}

pub fn kbd_long() -> &'static [f32] {
    KBD_LONG.get_or_init(|| build_kbd(LONG_LEN, 4.0))
}

pub fn kbd_short() -> &'static [f32] {
    KBD_SHORT.get_or_init(|| build_kbd(SHORT_LEN, 6.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_at_endpoints() {
        // First-half sine: w[0] = sin(π·0.5/(2N)) ≈ small, w[N-1] ≈ 1.
        let w = sine_long();
        assert!(w[0] > 0.0 && w[0] < 0.01);
        assert!(w[LONG_LEN - 1] > 0.999);
    }

    #[test]
    fn sine_partition_of_unity_full_window() {
        // POU on the doubling-scheme full window:
        //   full_w[i]² + full_w[i+L]² = w[i]² + w[L-1-i]² = sin² + cos² = 1.
        let w = sine_long();
        for i in 0..LONG_LEN {
            let a = w[i];
            let b = w[LONG_LEN - 1 - i];
            let s = a * a + b * b;
            assert!((s - 1.0).abs() < 1e-3, "sine POU off at {i}: {s}");
        }
    }

    #[test]
    fn kbd_partition_of_unity() {
        // KBD POU: w[n]^2 + w[n + N/2]^2 = 1 for n in 0..N/2.
        let w = kbd_long();
        let n = LONG_LEN;
        let half = n / 2;
        for i in 0..half {
            let a = w[i];
            let b = w[i + half];
            let s = a * a + b * b;
            assert!((s - 1.0).abs() < 1e-3, "kbd POU off at {i}: {s}");
        }
    }
}
