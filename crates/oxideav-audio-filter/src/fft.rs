//! Tiny radix-2 Cooley-Tukey FFT.
//!
//! Pure Rust, no allocations in the hot path. Sized only for spectrogram-style
//! analysis (`n` up to a few thousand). For realistic sizes the simple
//! iterative implementation here is plenty fast and avoids a heavyweight
//! dependency.
//!
//! Conventions:
//! - In-place complex transform via [`fft_inplace`].
//! - Real-input convenience [`real_fft`] returns the first `n/2 + 1` complex
//!   bins (Hermitian half).
//! - Forward direction only (no inverse needed by the spectrogram).

/// Compact complex number used by the FFT routines.
#[derive(Clone, Copy, Debug, Default)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    pub fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    pub fn magnitude(&self) -> f32 {
        (self.re * self.re + self.im * self.im).sqrt()
    }
}

/// In-place radix-2 forward FFT. `data.len()` must be a power of two.
pub fn fft_inplace(data: &mut [Complex]) {
    let n = data.len();
    assert!(n.is_power_of_two(), "FFT size must be a power of two");
    if n <= 1 {
        return;
    }

    // Bit-reversal permutation
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            data.swap(i, j);
        }
    }

    // Cooley-Tukey butterflies
    let mut len = 2usize;
    while len <= n {
        let half = len / 2;
        let theta = -2.0 * std::f32::consts::PI / len as f32;
        let (wstep_s, wstep_c) = theta.sin_cos();
        let mut i = 0;
        while i < n {
            let mut w_re = 1.0f32;
            let mut w_im = 0.0f32;
            for k in 0..half {
                let a = data[i + k];
                let b = data[i + k + half];
                let t_re = w_re * b.re - w_im * b.im;
                let t_im = w_re * b.im + w_im * b.re;
                data[i + k] = Complex::new(a.re + t_re, a.im + t_im);
                data[i + k + half] = Complex::new(a.re - t_re, a.im - t_im);
                let new_re = w_re * wstep_c - w_im * wstep_s;
                let new_im = w_re * wstep_s + w_im * wstep_c;
                w_re = new_re;
                w_im = new_im;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Compute the forward FFT of a real input. Returns `n/2 + 1` complex bins
/// (the non-redundant half of the Hermitian-symmetric output).
pub fn real_fft(input: &[f32]) -> Vec<Complex> {
    let n = input.len();
    assert!(n.is_power_of_two(), "real_fft size must be a power of two");
    let mut buf: Vec<Complex> = input.iter().map(|&x| Complex::new(x, 0.0)).collect();
    fft_inplace(&mut buf);
    buf.truncate(n / 2 + 1);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_input_yields_dc_bin() {
        let n = 64;
        let input = vec![1.0f32; n];
        let bins = real_fft(&input);
        // Bin 0 magnitude == sum of inputs
        assert!((bins[0].re - n as f32).abs() < 1.0e-3);
        // Other bins ~ 0
        for b in &bins[1..] {
            assert!(b.magnitude() < 1.0e-3);
        }
    }

    #[test]
    fn impulse_yields_flat_spectrum() {
        let n = 32;
        let mut input = vec![0.0f32; n];
        input[0] = 1.0;
        let bins = real_fft(&input);
        for b in &bins {
            assert!((b.magnitude() - 1.0).abs() < 1.0e-4);
        }
    }

    #[test]
    fn cosine_peaks_at_expected_bin() {
        let n = 128;
        let bin = 8;
        let input: Vec<f32> = (0..n)
            .map(|k| (2.0 * std::f32::consts::PI * bin as f32 * k as f32 / n as f32).cos())
            .collect();
        let bins = real_fft(&input);
        let mut max_idx = 0;
        let mut max_mag = 0.0f32;
        for (i, b) in bins.iter().enumerate() {
            let m = b.magnitude();
            if m > max_mag {
                max_mag = m;
                max_idx = i;
            }
        }
        assert_eq!(max_idx, bin);
    }
}
