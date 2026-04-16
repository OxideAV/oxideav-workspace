//! VP8 inverse Walsh-Hadamard (4×4) and inverse 4×4 DCT — RFC 6386 §14.
//!
//! Both operate on 16-element row-major buffers of i16 / i32. The WHT
//! recovers DC coefficients for the 16 luma blocks of an intra-16×16 MB;
//! the DCT inverts the per-block 4×4 transform.

const COSPI8SQRT2MINUS1: i32 = 20091;
const SINPI8SQRT2: i32 = 35468;

/// Inverse 4×4 DCT (RFC 6386 §14.1 reference). Input/output buffers
/// are 16 entries in row-major order. The `ctx` is destroyed.
pub fn idct4x4(coeffs: &[i16; 16]) -> [i16; 16] {
    let mut work = [0i32; 16];
    // Row pass.
    for i in 0..4 {
        let off = i * 4;
        let a1 = coeffs[off] as i32 + coeffs[off + 2] as i32;
        let b1 = coeffs[off] as i32 - coeffs[off + 2] as i32;
        let temp1 = (coeffs[off + 1] as i32 * SINPI8SQRT2) >> 16;
        let temp2 = coeffs[off + 3] as i32 + ((coeffs[off + 3] as i32 * COSPI8SQRT2MINUS1) >> 16);
        let c1 = temp1 - temp2;
        let temp1 = coeffs[off + 1] as i32 + ((coeffs[off + 1] as i32 * COSPI8SQRT2MINUS1) >> 16);
        let temp2 = (coeffs[off + 3] as i32 * SINPI8SQRT2) >> 16;
        let d1 = temp1 + temp2;
        work[off] = a1 + d1;
        work[off + 3] = a1 - d1;
        work[off + 1] = b1 + c1;
        work[off + 2] = b1 - c1;
    }
    // Column pass.
    let mut out = [0i16; 16];
    for i in 0..4 {
        let a1 = work[i] + work[i + 8];
        let b1 = work[i] - work[i + 8];
        let temp1 = (work[i + 4] * SINPI8SQRT2) >> 16;
        let temp2 = work[i + 12] + ((work[i + 12] * COSPI8SQRT2MINUS1) >> 16);
        let c1 = temp1 - temp2;
        let temp1 = work[i + 4] + ((work[i + 4] * COSPI8SQRT2MINUS1) >> 16);
        let temp2 = (work[i + 12] * SINPI8SQRT2) >> 16;
        let d1 = temp1 + temp2;
        out[i] = clip_short((a1 + d1 + 4) >> 3);
        out[i + 12] = clip_short((a1 - d1 + 4) >> 3);
        out[i + 4] = clip_short((b1 + c1 + 4) >> 3);
        out[i + 8] = clip_short((b1 - c1 + 4) >> 3);
    }
    out
}

fn clip_short(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Inverse 4×4 Walsh-Hadamard (RFC 6386 §14.3) — reconstructs the
/// 16 Y2 DC coefficients.
pub fn iwht4x4(coeffs: &[i16; 16]) -> [i16; 16] {
    let mut work = [0i32; 16];
    for i in 0..4 {
        let a1 = coeffs[i] as i32 + coeffs[i + 12] as i32;
        let b1 = coeffs[i + 4] as i32 + coeffs[i + 8] as i32;
        let c1 = coeffs[i + 4] as i32 - coeffs[i + 8] as i32;
        let d1 = coeffs[i] as i32 - coeffs[i + 12] as i32;
        work[i] = a1 + b1;
        work[i + 4] = c1 + d1;
        work[i + 8] = a1 - b1;
        work[i + 12] = d1 - c1;
    }
    let mut out = [0i16; 16];
    for i in 0..4 {
        let off = i * 4;
        let a2 = work[off] + work[off + 3];
        let b2 = work[off + 1] + work[off + 2];
        let c2 = work[off + 1] - work[off + 2];
        let d2 = work[off] - work[off + 3];
        let a3 = a2 + b2;
        let b3 = c2 + d2;
        let c3 = a2 - b2;
        let d3 = d2 - c2;
        out[off] = ((a3 + 3) >> 3) as i16;
        out[off + 1] = ((b3 + 3) >> 3) as i16;
        out[off + 2] = ((c3 + 3) >> 3) as i16;
        out[off + 3] = ((d3 + 3) >> 3) as i16;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idct_zero_is_zero() {
        let zeros = [0i16; 16];
        let out = idct4x4(&zeros);
        assert!(out.iter().all(|&v| v == 0));
    }

    #[test]
    fn iwht_zero_is_zero() {
        let zeros = [0i16; 16];
        let out = iwht4x4(&zeros);
        assert!(out.iter().all(|&v| v == 0));
    }

    /// Forward 4×4 DCT for verification — derived from the inverse coefficients.
    /// Returns 16 i32 (raw DCT coefficients, not yet quantised).
    #[allow(dead_code)]
    fn fdct4x4(input: &[i32; 16]) -> [i32; 16] {
        // VP8's forward DCT (RFC 6386 reference). Column then row.
        let mut work = [0i32; 16];
        for col in 0..4 {
            let s0 = input[col];
            let s1 = input[col + 4];
            let s2 = input[col + 8];
            let s3 = input[col + 12];
            let a = (s0 + s3) << 3;
            let b = (s1 + s2) << 3;
            let c = (s1 - s2) << 3;
            let d = (s0 - s3) << 3;
            work[col] = a + b;
            work[col + 8] = a - b;
            work[col + 4] = (c * 2217 + d * 5352 + 14500) >> 12;
            work[col + 12] = (d * 2217 - c * 5352 + 7500) >> 12;
        }
        let mut out = [0i32; 16];
        for row in 0..4 {
            let off = row * 4;
            let s0 = work[off];
            let s1 = work[off + 1];
            let s2 = work[off + 2];
            let s3 = work[off + 3];
            let a = s0 + s3;
            let b = s1 + s2;
            let c = s1 - s2;
            let d = s0 - s3;
            out[off] = (a + b + 7) >> 4;
            out[off + 2] = (a - b + 7) >> 4;
            out[off + 1] = ((c * 2217 + d * 5352 + 12000) >> 16) + (if d != 0 { 1 } else { 0 });
            out[off + 3] = (d * 2217 - c * 5352 + 51000) >> 16;
        }
        out
    }

    #[test]
    fn idct_roundtrip_constant_block() {
        // Forward DCT of constant 1 = [16, 0, ..., 0].
        let input = [1i32; 16];
        let coeffs = fdct4x4(&input);
        // Convert coeffs to i16 and run iDCT.
        let mut c16 = [0i16; 16];
        for i in 0..16 {
            c16[i] = coeffs[i] as i16;
        }
        let out = idct4x4(&c16);
        // Should approximately recover constant 1.
        for &v in &out {
            assert!((v as i32 - 1).abs() <= 1, "expected ~1, got {v}");
        }
    }

    #[test]
    fn idct_dc_only() {
        // Forward DCT of constant 1 = [16, 0, 0, ..., 0] (block sum * 4
        // due to scaling). Inverse should approximately recover constant.
        let mut input = [0i16; 16];
        input[0] = 16;
        let out = idct4x4(&input);
        for &v in &out {
            assert_eq!(v, 2, "expected ~2 (=16/8) per cell, got {:?}", out);
        }
    }

    #[test]
    fn iwht_constant_dc() {
        // All-equal input (all 8): each forward WHT row/col would have summed
        // to a single DC coeff; running iWHT on a constant non-DC pattern
        // should still produce a finite, deterministic result.
        let mut input = [0i16; 16];
        input[0] = 64;
        let out = iwht4x4(&input);
        // Output should be all equal to 64 / 8 = 8 since iWHT scales by 1/8.
        // Forward WHT of `[8 8 8 8; ...]` = `[64 0 ...]`. Reverse should
        // recover all 8s.
        for &v in &out {
            assert_eq!(v, 8);
        }
    }
}
