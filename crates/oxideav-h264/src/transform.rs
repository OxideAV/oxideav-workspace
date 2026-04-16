//! Inverse integer transforms and dequantisation — ITU-T H.264 §8.5.
//!
//! This module covers:
//!
//! * §8.5.6 — quantisation tables (`v[6][3]` / `LevelScale`).
//! * §8.5.10 — inverse Hadamard 4×4 (Intra16×16 DC luma).
//! * §8.5.11 — inverse Hadamard 2×2 (4:2:0 chroma DC).
//! * §8.5.12 — inverse 4×4 integer transform (§8.5.12.2 with the +(1<<5)
//!   rounding offset for AC and the simple `>> 6` for DC-loaded blocks).
//! * §8.5 / §8.5.10 / §7.4.2.1 — dequantisation `c'[i,j] = c[i,j] * scale`.

/// `v[m][n]` — Table 8-13. Indexed by (qp_y % 6, position class). The three
/// position classes in a 4×4 block are:
///   0: positions (0,0) (0,2) (2,0) (2,2)
///   1: positions (1,1) (1,3) (3,1) (3,3)
///   2: all other positions
const V_TABLE: [[i32; 3]; 6] = [
    [10, 16, 13],
    [11, 18, 14],
    [13, 20, 16],
    [14, 23, 18],
    [16, 25, 20],
    [18, 29, 23],
];

/// Position class for each cell of a 4×4 block.
fn pos_class(row: usize, col: usize) -> usize {
    let r_even = row % 2 == 0;
    let c_even = col % 2 == 0;
    if r_even && c_even {
        0
    } else if !r_even && !c_even {
        1
    } else {
        2
    }
}

/// Compute the dequantisation scale `LevelScale4x4(qP%6, i, j)` from §8.5.10.
fn level_scale(qp_mod6: usize, row: usize, col: usize) -> i32 {
    V_TABLE[qp_mod6][pos_class(row, col)]
}

/// Dequantise a 4×4 AC block in raster order. `qp` is the per-block QP value
/// (luma or chroma already adjusted via §8.5.11.1 / §8.5.11.2).
///
/// Per §8.5.10: `c'[i,j] = c[i,j] * LevelScale4x4(qP%6, i, j) << (qP/6)`
/// — applied to all 16 positions for a normal AC 4×4 block. For an Intra16×16
/// AC block, the DC slot (raster 0) is left at 0 and gets filled later from
/// the Hadamard pass.
pub fn dequantize_4x4(coeffs: &mut [i32; 16], qp: i32) {
    let qp = qp.clamp(0, 51);
    let qp6 = (qp / 6) as u32;
    let qmod = (qp % 6) as usize;
    for r in 0..4 {
        for c in 0..4 {
            let i = r * 4 + c;
            coeffs[i] = (coeffs[i] * level_scale(qmod, r, c)) << qp6;
        }
    }
}

/// Inverse 4×4 transform as per §8.5.12.2.
///
/// Input: dequantised coefficient block in raster order.
/// Output: residual sample block in raster order, scaled by `1 << 6`. The
/// caller is responsible for the final `(x + 32) >> 6` rounding when adding
/// to the prediction (matches the spec — the `(1<<5)` is inside the
/// transform but applied per sample).
pub fn idct_4x4(coeffs: &mut [i32; 16]) {
    let mut tmp = [0i32; 16];
    // Horizontal: rows.
    for r in 0..4 {
        let c0 = coeffs[r * 4];
        let c1 = coeffs[r * 4 + 1];
        let c2 = coeffs[r * 4 + 2];
        let c3 = coeffs[r * 4 + 3];
        let e = c0 + c2;
        let f = c0 - c2;
        let g = (c1 >> 1) - c3;
        let h = c1 + (c3 >> 1);
        tmp[r * 4] = e + h;
        tmp[r * 4 + 1] = f + g;
        tmp[r * 4 + 2] = f - g;
        tmp[r * 4 + 3] = e - h;
    }
    // Vertical: cols.
    for c in 0..4 {
        let c0 = tmp[c];
        let c1 = tmp[4 + c];
        let c2 = tmp[8 + c];
        let c3 = tmp[12 + c];
        let e = c0 + c2;
        let f = c0 - c2;
        let g = (c1 >> 1) - c3;
        let h = c1 + (c3 >> 1);
        coeffs[c] = e + h;
        coeffs[4 + c] = f + g;
        coeffs[8 + c] = f - g;
        coeffs[12 + c] = e - h;
    }
    // Spec: residual sample = (transformed + 32) >> 6.
    for v in coeffs.iter_mut() {
        *v = (*v + 32) >> 6;
    }
}

/// Inverse 4×4 Hadamard for Intra16×16 DC luma block — §8.5.10.
///
/// Input/output: 16 DC coefficients in 4×4 raster (one per 4×4 sub-block of
/// the macroblock). After the Hadamard the values are dequantised with the
/// `(0,0)` LevelScale entry, and the result populates the DC slot of each
/// 4×4 luma residual block before its own inverse transform.
pub fn inv_hadamard_4x4_dc(dc: &mut [i32; 16], qp: i32) {
    // Hadamard in two passes (rows then columns).
    let mut tmp = [0i32; 16];
    for r in 0..4 {
        let a = dc[r * 4];
        let b = dc[r * 4 + 1];
        let c = dc[r * 4 + 2];
        let d = dc[r * 4 + 3];
        tmp[r * 4] = a + b + c + d;
        tmp[r * 4 + 1] = a + b - c - d;
        tmp[r * 4 + 2] = a - b - c + d;
        tmp[r * 4 + 3] = a - b + c - d;
    }
    for c in 0..4 {
        let a = tmp[c];
        let b = tmp[4 + c];
        let cc = tmp[8 + c];
        let d = tmp[12 + c];
        dc[c] = a + b + cc + d;
        dc[4 + c] = a + b - cc - d;
        dc[8 + c] = a - b - cc + d;
        dc[12 + c] = a - b + cc - d;
    }

    // Dequantise (§8.5.10): scale = LevelScale4x4(qP%6, 0, 0) = V[qP%6][0].
    let qp = qp.clamp(0, 51);
    let qp6 = (qp / 6) as u32;
    let qmod = (qp % 6) as usize;
    let scale = V_TABLE[qmod][0];
    if qp >= 36 {
        let shift = qp6 - 6;
        for v in dc.iter_mut() {
            *v = (*v * scale) << shift;
        }
    } else {
        let shift = 6 - qp6;
        let round = 1i32 << (shift - 1);
        for v in dc.iter_mut() {
            *v = (*v * scale + round) >> shift;
        }
    }
}

/// 2×2 chroma DC inverse Hadamard — §8.5.11.1.
///
/// Input: 4 DC coefficients in raster (00, 01, 10, 11).
pub fn inv_hadamard_2x2_chroma_dc(dc: &mut [i32; 4], qp: i32) {
    let a = dc[0];
    let b = dc[1];
    let c = dc[2];
    let d = dc[3];
    let t0 = a + b;
    let t1 = a - b;
    let t2 = c + d;
    let t3 = c - d;
    dc[0] = t0 + t2;
    dc[1] = t1 + t3;
    dc[2] = t0 - t2;
    dc[3] = t1 - t3;

    let qp = qp.clamp(0, 51);
    let qp6 = (qp / 6) as u32;
    let qmod = (qp % 6) as usize;
    let scale = V_TABLE[qmod][0];
    if qp >= 6 {
        let shift = qp6 - 1;
        for v in dc.iter_mut() {
            *v = (*v * scale) << shift;
        }
    } else {
        // qp < 6 -> qp6 = 0. Apply scale, then >> 1.
        for v in dc.iter_mut() {
            *v = (*v * scale) >> 1;
        }
    }
}

/// QP table for chroma — Table 7-2. Maps `qPI` (luma QP after offset) to
/// chroma QP `QPC`. For QPI < 30, output equals input; for 30..=51, use the
/// table.
pub const QP_CHROMA_TABLE: [i32; 52] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 29, 30, 31, 32, 32, 33, 34, 34, 35, 35, 36, 36, 37, 37, 37, 38, 38, 38, 39, 39,
    39, 39,
];

/// Map a luma-side QP+offset to the chroma QP per §7.4.2.1.
pub fn chroma_qp(qp_y: i32, chroma_qp_index_offset: i32) -> i32 {
    let qpi = (qp_y + chroma_qp_index_offset).clamp(-12, 51);
    if qpi < 0 {
        qpi
    } else {
        QP_CHROMA_TABLE[qpi as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idct_zero_in_zero_out() {
        let mut z = [0i32; 16];
        idct_4x4(&mut z);
        assert!(z.iter().all(|&v| v == 0));
    }

    #[test]
    fn idct_pure_dc_constant() {
        // A pure DC coefficient should produce a constant block.
        // Using the H.264 4×4 transform conventions, a coefficient of 64 at
        // (0,0) (already dequantised) corresponds to a sample value of 1
        // (the >> 6 gives 1 per pixel after the +32 round).
        let mut c = [0i32; 16];
        c[0] = 64;
        idct_4x4(&mut c);
        // All cells should equal the same value.
        let v0 = c[0];
        for &v in c.iter() {
            assert_eq!(v, v0);
        }
    }
}
