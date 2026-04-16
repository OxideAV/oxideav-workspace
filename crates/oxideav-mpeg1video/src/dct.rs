//! 8×8 forward & inverse DCT wrappers. Uses the same textbook f32 transforms
//! as oxideav-mjpeg's JPEG decoder (duplicated rather than re-exported to
//! avoid a cross-crate dependency — the implementation is short and perfectly
//! self-contained).

use std::f32::consts::PI;
use std::sync::OnceLock;

fn cos_table() -> &'static [[f32; 8]; 8] {
    static T: OnceLock<[[f32; 8]; 8]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [[0.0f32; 8]; 8];
        for k in 0..8 {
            let c_k = if k == 0 {
                (1.0_f32 / 2.0_f32).sqrt()
            } else {
                1.0
            };
            for n in 0..8 {
                t[k][n] = 0.5 * c_k * ((2 * n + 1) as f32 * k as f32 * PI / 16.0).cos();
            }
        }
        t
    })
}

/// Inverse DCT of an 8×8 block. Input is natural order (dequantised). In-place.
pub fn idct8x8(block: &mut [f32; 64]) {
    let t = cos_table();
    let mut tmp = [0.0f32; 64];

    for y in 0..8 {
        for n in 0..8 {
            let mut s = 0.0f32;
            for k in 0..8 {
                s += t[k][n] * block[y * 8 + k];
            }
            tmp[y * 8 + n] = s;
        }
    }
    for x in 0..8 {
        for m in 0..8 {
            let mut s = 0.0f32;
            for k in 0..8 {
                s += t[k][m] * tmp[k * 8 + x];
            }
            block[m * 8 + x] = s;
        }
    }
}

/// Forward DCT of an 8×8 block in natural order, in-place. The MPEG-1 spec
/// defines FDCT and IDCT to be exactly inverse pairs (within rounding); here
/// we use the same f32 cosine table as the IDCT, so `idct(fdct(x)) ≈ x`.
///
/// Caller is responsible for the level shift if desired (MPEG-1 intra DC is
/// usually written without a level shift — DC is stored in 0..=2040 directly,
/// derived from sample sum/8 — so this routine does NOT subtract 128).
pub fn fdct8x8(block: &mut [f32; 64]) {
    let t = cos_table();
    let mut tmp = [0.0f32; 64];

    // Row-wise forward: tmp[y][k] = Σn t[k][n] * block[y][n]
    for y in 0..8 {
        for k in 0..8 {
            let mut s = 0.0f32;
            for n in 0..8 {
                s += t[k][n] * block[y * 8 + n];
            }
            tmp[y * 8 + k] = s;
        }
    }
    // Column-wise.
    for x in 0..8 {
        for k in 0..8 {
            let mut s = 0.0f32;
            for n in 0..8 {
                s += t[k][n] * tmp[n * 8 + x];
            }
            block[k * 8 + x] = s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idct_of_fdct_round_trip() {
        let mut block = [0.0f32; 64];
        for i in 0..64 {
            block[i] = ((i * 7) % 255) as f32;
        }
        let original = block;
        fdct8x8(&mut block);
        idct8x8(&mut block);
        for i in 0..64 {
            assert!(
                (block[i] - original[i]).abs() < 1e-2,
                "round-trip mismatch at {i}: got {} want {}",
                block[i],
                original[i]
            );
        }
    }

    #[test]
    fn fdct_dc_of_constant_block() {
        // A constant block of value v has DC coefficient = 8 * v with our
        // normalisation.
        let mut block = [100.0f32; 64];
        fdct8x8(&mut block);
        assert!((block[0] - 800.0).abs() < 1e-2, "DC = {}", block[0]);
        for i in 1..64 {
            assert!(block[i].abs() < 1e-2, "AC[{i}] = {}", block[i]);
        }
    }
}
