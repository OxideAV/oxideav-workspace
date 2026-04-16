//! Inverse quantisation for MPEG-4 Part 2 (§7.4.4).
//!
//! Two modes, selected by the VOL's `mpeg_quant` flag:
//! * **H.263 quantisation** (the default XVID/DivX mode) — very simple: each
//!   AC coefficient `l != 0` dequantises to
//!   `(2 * Q * |l| + Q) * sign(l)` if `Q` is odd,
//!   `(2 * Q * |l| + Q - 1) * sign(l)` if `Q` is even.
//! * **MPEG-4 quantisation** — uses an 8x8 quant matrix similar to MPEG-1/2,
//!   with mismatch control.
//!
//! Only the H.263 path is filled in for this session; the MPEG-4 matrix path
//! stubs out clearly.

use oxideav_core::{Error, Result};

use crate::headers::vol::VideoObjectLayer;

/// Luma DC scaler by quantiser, spec Table 7-2.
pub const Y_DC_SCALE_TABLE: [u8; 32] = [
    0, 8, 8, 8, 8, 10, 12, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    34, 36, 38, 40, 42, 44, 46,
];

/// Chroma DC scaler by quantiser, spec Table 7-3.
pub const C_DC_SCALE_TABLE: [u8; 32] = [
    0, 8, 8, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 18, 18,
    19, 20, 21, 22, 23, 24, 25,
];

/// DC-VLC vs plain-13-bit DC threshold by `intra_dc_vlc_thr` (VOP header).
/// Spec Table 6-22 / §7.4.3. `intra_dc_vlc_thr[i]` gives the quant threshold
/// above which plain-13-bit DC coding is used for intra MBs.
///
/// `thr[0] == 99` means "always use VLC"; `thr[7] == 0` means "never use VLC".
pub const INTRA_DC_VLC_THR_TABLE: [u8; 8] = [99, 13, 15, 17, 19, 21, 23, 0];

/// Dequantise one intra block's AC coefficients in-place (index 0 is the DC
/// coefficient and is left untouched — DC is handled separately by the
/// caller, with prediction). `coeffs[i]` is the raw decoded level; on return
/// `coeffs[i]` holds the reconstructed coefficient.
///
/// `quant` is the current `vop_quant` (1..=31 for quant_precision=5).
pub fn dequantise_intra_h263(coeffs: &mut [i32; 64], quant: u32) -> Result<()> {
    if quant == 0 {
        return Err(Error::invalid("mpeg4 iq: quant = 0"));
    }
    let q = quant as i32;
    let q_plus = if q & 1 == 1 { q } else { q - 1 };
    for i in 1..64 {
        let l = coeffs[i];
        if l == 0 {
            continue;
        }
        let abs = l.abs();
        let mut val = 2 * q * abs + q_plus;
        if l < 0 {
            val = -val;
        }
        coeffs[i] = val.clamp(-2048, 2047);
    }
    Ok(())
}

/// MPEG-4 (matrix) intra quantisation — §7.4.4.3.
///
/// For intra blocks with the MPEG-4 quantisation path (VOL `mpeg_quant`),
/// `abs_coef = ((2 * level + k) * wQ * matrix[zz]) / 16`, with `k=0` for
/// intra (§7.4.4.3 (17)). Result saturates to [-2048, 2047]. Index 0 (DC) is
/// untouched; the caller handles DC via the DC scaler.
///
/// `matrix` holds the intra quant matrix in natural (un-zigzagged) order.
pub fn dequantise_intra_mpeg4(
    coeffs: &mut [i32; 64],
    quant: u32,
    vol: &VideoObjectLayer,
) -> Result<()> {
    if quant == 0 {
        return Err(Error::invalid("mpeg4 iq: quant = 0"));
    }
    let matrix = vol
        .intra_quant_matrix
        .unwrap_or(crate::headers::vol::DEFAULT_INTRA_QUANT_MATRIX);
    let wq = quant as i32;
    for i in 1..64 {
        let l = coeffs[i];
        if l == 0 {
            continue;
        }
        // §7.4.4.3 equation (17), intra: |F''| = (2 * |level| * wQ * Q_intra[i]) / 16
        // with sign carried separately. We do not apply mismatch control for
        // intra blocks (§7.4.4.7 is for non-intra only per spec).
        let m = matrix[i] as i32;
        let abs = l.unsigned_abs() as i32;
        let mut val = (2 * abs * wq * m) / 16;
        if l < 0 {
            val = -val;
        }
        coeffs[i] = val.clamp(-2048, 2047);
    }
    Ok(())
}

/// Return the DC scaler for a block, picking the luma or chroma table based on
/// the block index (0..=3 are luma, 4 is Cb, 5 is Cr). Valid for `quant` in
/// 1..=31 for 5-bit quant precision.
pub fn dc_scaler(block_idx: usize, quant: u32) -> u32 {
    let q = (quant as usize).min(31);
    if block_idx < 4 {
        Y_DC_SCALE_TABLE[q] as u32
    } else {
        C_DC_SCALE_TABLE[q] as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h263_intra_even_quant() {
        // Q=4 (even). Level 3 -> 2*4*3 + (4-1) = 24 + 3 = 27.
        let mut c = [0i32; 64];
        c[1] = 3;
        dequantise_intra_h263(&mut c, 4).unwrap();
        assert_eq!(c[1], 27);
    }

    #[test]
    fn h263_intra_odd_quant() {
        // Q=5. Level -2 -> -(2*5*2 + 5) = -25.
        let mut c = [0i32; 64];
        c[2] = -2;
        dequantise_intra_h263(&mut c, 5).unwrap();
        assert_eq!(c[2], -25);
    }

    #[test]
    fn dc_scaler_tables() {
        assert_eq!(dc_scaler(0, 1), 8); // luma
        assert_eq!(dc_scaler(4, 1), 8); // chroma
        assert_eq!(dc_scaler(0, 31), 46);
        assert_eq!(dc_scaler(5, 31), 25);
    }
}
