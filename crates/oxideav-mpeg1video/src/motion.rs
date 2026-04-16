//! Motion vector decoding and motion compensation.
//!
//! MPEG-1 video motion vectors are coded differentially relative to the
//! previously-decoded vector in the same slice. They are transmitted as a
//! pair of (motion_code, motion_residual) for horizontal and vertical
//! components. The vector units are half-pels unless `full_pel_vector` is
//! set in the picture header, in which case they are whole-pels.
//!
//! Motion compensation is performed by copying a reference block (16x16
//! luma, 8x8 per chroma) from the reference picture, using bilinear half-
//! pel interpolation when the fractional component is nonzero.
//!
//! See ISO/IEC 11172-2 §2.4.3 (motion vectors) and §2.4.4.2 (prediction).
//!
//! The macroblock-level API is:
//!   * [`decode_motion_vector`] — reads one component and updates the
//!     running predictor; returns the reconstructed vector in half-pel
//!     units (or full-pel if `full_pel` is set, where the vector is then
//!     scaled up by two before writing).
//!   * [`mc_copy_luma_16x16`] / [`mc_copy_chroma_8x8`] — copy a 16x16 or
//!     8x8 block from the reference plane into a destination buffer using
//!     half-pel bilinear interpolation.
//!   * [`mc_avg_luma_16x16`] / [`mc_avg_chroma_8x8`] — same as above but
//!     output is the rounded average of two motion-compensated blocks
//!     (used for B-frame interpolated prediction).

use oxideav_core::Result;

use crate::bitreader::BitReader;
use crate::tables::motion;
use crate::vlc;

/// Motion vector predictor state, reset at slice boundaries and whenever a
/// non-intra macroblock without a forward/backward vector is encountered.
#[derive(Clone, Copy, Debug, Default)]
pub struct MvPredictor {
    pub x: i32,
    pub y: i32,
}

impl MvPredictor {
    pub fn reset(&mut self) {
        self.x = 0;
        self.y = 0;
    }
}

/// Decode one motion-vector component from the bitstream per §2.4.3.4.
///
/// * `f_code` is `forward_f_code` or `backward_f_code` from the picture
///   header (range 1..=7).
/// * `full_pel` is `full_pel_{forward,backward}_vector`.
/// * `predictor` is the running predictor for this vector component and
///   direction, updated in place to the reconstructed vector value.
///
/// Returns the reconstructed vector value in half-pel units.
pub fn decode_motion_component(
    br: &mut BitReader<'_>,
    f_code: u8,
    full_pel: bool,
    predictor: &mut i32,
) -> Result<i32> {
    let r_size = (f_code - 1) as u32;
    let f = 1i32 << r_size; // 2^(f_code-1)
    let motion_code_abs = vlc::decode(br, motion::table())? as i32;
    let motion_code = if motion_code_abs == 0 {
        0
    } else {
        let sign = br.read_u32(1)?;
        if sign == 1 {
            -motion_code_abs
        } else {
            motion_code_abs
        }
    };

    let complement_r = if f == 1 || motion_code == 0 {
        0i32
    } else {
        br.read_u32(r_size)? as i32
    };

    // §2.4.3.4 reconstruction:
    //   if motion_code == 0: little = 0
    //   else:                little = (abs(motion_code) - 1) * f + complement_r + 1
    //   big   = little - (range = 32 * f)
    //   new_vector = predictor + (motion_code < 0 ? -little : little)
    // Then if new_vector > (range-1) or < -range, wrap by ±(2*range).
    let range = 32 * f;
    let (min, max) = (-range, range - 1);

    let little = if motion_code == 0 {
        0
    } else {
        (motion_code.abs() - 1) * f + complement_r + 1
    };
    let delta = if motion_code < 0 { -little } else { little };

    let mut new_vec = *predictor + delta;
    if new_vec < min {
        new_vec += 2 * range;
    } else if new_vec > max {
        new_vec -= 2 * range;
    }
    *predictor = new_vec;

    // full_pel vectors are transmitted in whole-pel units; scale to the
    // half-pel unit used by the MC stages.
    Ok(if full_pel { new_vec * 2 } else { new_vec })
}

/// Copy a `w × h` rectangle from `ref_plane` into `dst`, starting at pixel
/// position `(src_x, src_y)`. Half-pel fractional bits `(hx, hy)` select
/// between integer, horizontal-interp, vertical-interp or 2D-interp modes.
///
/// `(src_x, src_y)` are integer source coordinates; the caller passes the
/// full-pel portion (mv >> 1) and the fractional portion separately.
#[allow(clippy::too_many_arguments)]
fn mc_block(
    ref_plane: &[u8],
    ref_stride: usize,
    ref_w: i32,
    ref_h: i32,
    src_x: i32,
    src_y: i32,
    hx: bool,
    hy: bool,
    w: i32,
    h: i32,
    dst: &mut [u8],
    dst_stride: usize,
) {
    // Clamp helper.
    let clamp_x = |x: i32| x.clamp(0, ref_w - 1);
    let clamp_y = |y: i32| y.clamp(0, ref_h - 1);

    for j in 0..h {
        for i in 0..w {
            let x0 = clamp_x(src_x + i);
            let y0 = clamp_y(src_y + j);
            let v = match (hx, hy) {
                (false, false) => ref_plane[(y0 as usize) * ref_stride + (x0 as usize)] as u32,
                (true, false) => {
                    let x1 = clamp_x(src_x + i + 1);
                    let a = ref_plane[(y0 as usize) * ref_stride + (x0 as usize)] as u32;
                    let b = ref_plane[(y0 as usize) * ref_stride + (x1 as usize)] as u32;
                    (a + b + 1) >> 1
                }
                (false, true) => {
                    let y1 = clamp_y(src_y + j + 1);
                    let a = ref_plane[(y0 as usize) * ref_stride + (x0 as usize)] as u32;
                    let b = ref_plane[(y1 as usize) * ref_stride + (x0 as usize)] as u32;
                    (a + b + 1) >> 1
                }
                (true, true) => {
                    let x1 = clamp_x(src_x + i + 1);
                    let y1 = clamp_y(src_y + j + 1);
                    let a = ref_plane[(y0 as usize) * ref_stride + (x0 as usize)] as u32;
                    let b = ref_plane[(y0 as usize) * ref_stride + (x1 as usize)] as u32;
                    let c = ref_plane[(y1 as usize) * ref_stride + (x0 as usize)] as u32;
                    let d = ref_plane[(y1 as usize) * ref_stride + (x1 as usize)] as u32;
                    (a + b + c + d + 2) >> 2
                }
            };
            dst[(j as usize) * dst_stride + (i as usize)] = v as u8;
        }
    }
}

/// Pre-computed motion-compensated prediction block (16x16 luma or 8x8
/// chroma). Used by the macroblock path to build a prediction buffer that
/// either becomes the output (skipped / no-pattern MB) or is added to the
/// residual and then clamped.
#[derive(Clone, Debug)]
pub struct Predicted {
    pub buf: [u8; 16 * 16],
    pub stride: usize,
    pub size: usize,
}

impl Predicted {
    pub fn new_luma() -> Self {
        Self {
            buf: [0; 16 * 16],
            stride: 16,
            size: 16,
        }
    }
    pub fn new_chroma() -> Self {
        Self {
            buf: [0; 16 * 16],
            stride: 8,
            size: 8,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.stride * self.size]
    }
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf[..self.stride * self.size]
    }
}

/// Compute motion-compensated prediction for an N×N block. `mv_x`, `mv_y`
/// are in half-pel units (luma) or half-pel/2 relative to the chroma grid
/// (the caller is responsible for scaling vectors into chroma half-pel
/// space — for MPEG-1 4:2:0 that means dividing the luma vector by 2 with
/// the spec-prescribed rounding).
#[allow(clippy::too_many_arguments)]
pub fn predict_block(
    ref_plane: &[u8],
    ref_stride: usize,
    ref_w: i32,
    ref_h: i32,
    mb_px: i32,
    mb_py: i32,
    mv_x_half: i32,
    mv_y_half: i32,
    size: i32,
    dst: &mut [u8],
    dst_stride: usize,
) {
    // Split into integer part and half-pel flag per §2.4.4.2.
    let (int_x, hx) = split_half(mv_x_half);
    let (int_y, hy) = split_half(mv_y_half);

    let src_x = mb_px + int_x;
    let src_y = mb_py + int_y;
    mc_block(
        ref_plane, ref_stride, ref_w, ref_h, src_x, src_y, hx, hy, size, size, dst, dst_stride,
    );
}

fn split_half(v: i32) -> (i32, bool) {
    // Arithmetic shift, then bit0 of the original value (toward zero) marks
    // the half-pel offset per §2.4.4.2. Use `v >> 1` (floor division) and
    // `v & 1` — note negative values: -1 → (-1, true) because -1 = 2*(-1)+1.
    let int_part = v.div_euclid(2);
    let half = v.rem_euclid(2) != 0;
    (int_part, half)
}

/// Scale a luma motion vector (half-pel units) to the chroma half-pel grid
/// for 4:2:0 subsampling per §2.4.4.2. The vector is divided by 2 using
/// "half-pel rounding to nearest even": `chroma = (luma / 2)` with the
/// integer part halved and the fractional part derived from the sum of
/// fractional bits.
///
/// Spec formula (simplified for 4:2:0):
///   right_for = recon_right_for >> 1
///   right_half_for = recon_right_for - 2*right_for
/// For chroma:
///   right_for_c = (recon_right_for / 2) >> 1
///   right_half_for_c = (recon_right_for / 2) - 2*right_for_c
/// Equivalently: chroma_mv_half = luma_mv_half / 2 (integer division
/// toward -infinity), giving a value in half-chroma-pel units.
pub fn scale_mv_to_chroma(luma_mv_half: i32) -> i32 {
    // For 4:2:0, chroma sits at half the resolution. MPEG-1 spec defines
    //   right_for_c    = (right_for     / 2)
    //   down_for_c     = (down_for      / 2)
    // with the half-pel fraction carried through. When `recon_right_for`
    // is odd (i.e. fractional), chroma inherits a half-pel offset.
    //
    // Concretely: luma_mv_half has the top N-1 bits as integer pels and
    // bit 0 as the half-pel flag. To convert to chroma half-pel units:
    //   chroma_int_pel  = luma_mv_half >> 2      (two bits: one for pel,
    //                                              one for half-pel)
    //   chroma_half_bit = (luma_mv_half >> 1) & 1 | (luma_mv_half & 1)
    //
    // Simplest correct derivation: let full = luma_mv_half. Integer-pel
    // in luma is `full / 2` (signed toward -inf). Chroma integer-pel
    // is `(full/2) / 2`. The chroma half-pel flag is `(full/2) % 2`.
    //
    // We return a single value in chroma half-pel units: `chroma_mv_half
    // = chroma_int_pel * 2 + chroma_half_bit`, i.e. simply `full / 2`
    // with signed-toward-0 division but preserving the parity.
    luma_mv_half.div_euclid(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_half_pel() {
        assert_eq!(split_half(0), (0, false));
        assert_eq!(split_half(1), (0, true));
        assert_eq!(split_half(2), (1, false));
        assert_eq!(split_half(3), (1, true));
        assert_eq!(split_half(-1), (-1, true));
        assert_eq!(split_half(-2), (-1, false));
        assert_eq!(split_half(-3), (-2, true));
    }

    #[test]
    fn predict_integer_copy() {
        // 4x4 ref plane, stride 4.
        let refp: [u8; 16] = [0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33];
        let mut dst = [0u8; 4];
        // Predict a 2x2 block at (0,0) with MV 0.
        let mut sb = [0u8; 4];
        predict_block(&refp, 4, 4, 4, 0, 0, 0, 0, 2, &mut sb, 2);
        assert_eq!(sb, [0, 1, 10, 11]);
        // MV (2,0) half = integer shift by 1 pel right.
        predict_block(&refp, 4, 4, 4, 0, 0, 2, 0, 2, &mut dst, 2);
        assert_eq!(dst, [1, 2, 11, 12]);
    }

    #[test]
    fn predict_half_pel_h() {
        let refp: [u8; 16] = [0, 10, 20, 30, 0, 10, 20, 30, 0, 10, 20, 30, 0, 10, 20, 30];
        let mut dst = [0u8; 4];
        // Half-pel horizontal: MV (1, 0) half -> (0, 0) with hx=true.
        predict_block(&refp, 4, 4, 4, 0, 0, 1, 0, 2, &mut dst, 2);
        // Expected: avg(0,10)=5, avg(10,20)=15, same for row 1
        assert_eq!(dst, [5, 15, 5, 15]);
    }

    #[test]
    fn mv_scale_chroma() {
        assert_eq!(scale_mv_to_chroma(0), 0);
        assert_eq!(scale_mv_to_chroma(2), 1);
        assert_eq!(scale_mv_to_chroma(4), 2);
        // Luma half-pel -> chroma quarter-pel (we store as half-pel unit
        // after /2, so half-pel bit propagates).
        assert_eq!(scale_mv_to_chroma(1), 0); // 0.5 luma -> 0.25 chroma -> rounds to 0 in halves
        assert_eq!(scale_mv_to_chroma(-1), -1);
    }
}
