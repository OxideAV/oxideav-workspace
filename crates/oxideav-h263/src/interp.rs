//! Half-pel bilinear interpolation for H.263 motion compensation (§6.1.2).
//!
//! Baseline H.263 uses the simple bilinear half-pel filter with rounding
//! offset `+1` / `+2` (spec equation (5) / (6)). There is no round-type flag
//! for baseline — that lives in Annex D (unrestricted MV + full-pel UMV) and
//! Annex U, which are out of scope.
//!
//! The source coordinates are `(blk_px + mv_x_half/2, blk_py + mv_y_half/2)`
//! where `mv_*_half` is in half-pel units. Samples outside the reference
//! plane are replicated from the nearest edge (boundary handling equivalent
//! to the one MPEG-4 UMV uses — H.263 baseline mathematically forbids
//! out-of-picture MVs, but in practice every production decoder clamps at
//! the edge because encoders do emit edge-hugging vectors at picture
//! boundaries).

/// Predict an `n x n` block from `ref_plane` starting at natural block
/// position `(blk_px, blk_py)` with motion vector `(mv_x_half, mv_y_half)` in
/// luma half-pel units. Writes the predicted samples into `dst` with stride
/// `dst_stride`.
///
/// H.263 baseline rounds half-pel positions to nearest (bilinear `(a+b+1)>>1`
/// for one half-pel axis, `(a+b+c+d+2)>>2` for the corner).
#[allow(clippy::too_many_arguments)]
pub fn predict_block(
    ref_plane: &[u8],
    ref_stride: usize,
    ref_w: i32,
    ref_h: i32,
    blk_px: i32,
    blk_py: i32,
    mv_x_half: i32,
    mv_y_half: i32,
    n: i32,
    dst: &mut [u8],
    dst_stride: usize,
) {
    let int_x = mv_x_half >> 1;
    let int_y = mv_y_half >> 1;
    let hx = (mv_x_half & 1) != 0;
    let hy = (mv_y_half & 1) != 0;

    let src_x = blk_px + int_x;
    let src_y = blk_py + int_y;

    let sample = |x: i32, y: i32| -> u32 {
        let xc = x.clamp(0, ref_w - 1) as usize;
        let yc = y.clamp(0, ref_h - 1) as usize;
        ref_plane[yc * ref_stride + xc] as u32
    };

    for j in 0..n {
        for i in 0..n {
            let v = match (hx, hy) {
                (false, false) => sample(src_x + i, src_y + j),
                (true, false) => {
                    let a = sample(src_x + i, src_y + j);
                    let b = sample(src_x + i + 1, src_y + j);
                    (a + b + 1) >> 1
                }
                (false, true) => {
                    let a = sample(src_x + i, src_y + j);
                    let b = sample(src_x + i, src_y + j + 1);
                    (a + b + 1) >> 1
                }
                (true, true) => {
                    let a = sample(src_x + i, src_y + j);
                    let b = sample(src_x + i + 1, src_y + j);
                    let c = sample(src_x + i, src_y + j + 1);
                    let d = sample(src_x + i + 1, src_y + j + 1);
                    (a + b + c + d + 2) >> 2
                }
            };
            dst[(j as usize) * dst_stride + (i as usize)] = v as u8;
        }
    }
}

/// Sum of absolute differences between an `n x n` source block and a
/// candidate reference block selected by half-pel MV `(mv_x_half, mv_y_half)`.
/// Used by the encoder's motion estimator.
#[allow(clippy::too_many_arguments)]
pub fn sad_block(
    src_plane: &[u8],
    src_stride: usize,
    src_x: i32,
    src_y: i32,
    ref_plane: &[u8],
    ref_stride: usize,
    ref_w: i32,
    ref_h: i32,
    blk_px: i32,
    blk_py: i32,
    mv_x_half: i32,
    mv_y_half: i32,
    n: i32,
) -> u32 {
    // Reuse predict_block by materialising the reference patch — cheap for
    // 16x16 and keeps the half-pel logic in one place.
    let n_sz = n as usize;
    let mut pred = vec![0u8; n_sz * n_sz];
    predict_block(
        ref_plane, ref_stride, ref_w, ref_h, blk_px, blk_py, mv_x_half, mv_y_half, n, &mut pred,
        n_sz,
    );
    let mut sad = 0u32;
    for j in 0..n_sz {
        for i in 0..n_sz {
            let s = src_plane[((src_y as usize) + j) * src_stride + (src_x as usize) + i] as i32;
            let p = pred[j * n_sz + i] as i32;
            sad += (s - p).unsigned_abs();
        }
    }
    sad
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_copy() {
        let refp: [u8; 16] = [0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33];
        let mut dst = [0u8; 4];
        predict_block(&refp, 4, 4, 4, 0, 0, 0, 0, 2, &mut dst, 2);
        assert_eq!(dst, [0, 1, 10, 11]);
        // MV (2,0) half = +1 integer pel.
        predict_block(&refp, 4, 4, 4, 0, 0, 2, 0, 2, &mut dst, 2);
        assert_eq!(dst, [1, 2, 11, 12]);
    }

    #[test]
    fn half_pel_horizontal() {
        let refp: [u8; 16] = [0, 10, 20, 30, 0, 10, 20, 30, 0, 10, 20, 30, 0, 10, 20, 30];
        let mut dst = [0u8; 4];
        predict_block(&refp, 4, 4, 4, 0, 0, 1, 0, 2, &mut dst, 2);
        // (0+10+1)/2=5, (10+20+1)/2=15, ...
        assert_eq!(dst, [5, 15, 5, 15]);
    }
}
