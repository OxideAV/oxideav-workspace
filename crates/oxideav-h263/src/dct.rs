//! 8×8 forward DCT for the H.263 encoder.
//!
//! Uses the same textbook f32 cosine table as `oxideav-mpeg4video::block::idct8x8`
//! so `idct(fdct(x)) ≈ x` to within rounding.

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

/// Forward DCT of an 8×8 block in natural order, in-place.
///
/// Caller is responsible for any level shift. H.263 INTRADC encodes the DC
/// coefficient using `pel_dc = round(F[0,0] / 8)` after the transform — i.e.
/// no `-128` level shift is applied to the input pels. With our normalisation
/// (`fdct` of constant `v` yields `8 v` at DC), `pel_dc` matches the decoder's
/// reconstruction `dc * 8`.
pub fn fdct8x8(block: &mut [f32; 64]) {
    let t = cos_table();
    let mut tmp = [0.0f32; 64];

    for y in 0..8 {
        for k in 0..8 {
            let mut s = 0.0f32;
            for n in 0..8 {
                s += t[k][n] * block[y * 8 + n];
            }
            tmp[y * 8 + k] = s;
        }
    }
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
    use oxideav_mpeg4video::block::idct8x8;

    #[test]
    fn round_trip() {
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
    fn dc_of_constant_block() {
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
