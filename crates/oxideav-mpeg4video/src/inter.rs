//! P-VOP macroblock + motion-vector decoding (ISO/IEC 14496-2 §6.3.5, §7.6).
//!
//! Walks one MB at a time:
//! 1. `not_coded` flag (if not the first MB of a slice).
//! 2. MCBPC inter VLC (Table B-13) — yields `(mb_type, cbpc)`.
//! 3. `ac_pred_flag` for intra-in-P MBs only.
//! 4. CBPY VLC (Table B-9).
//! 5. dquant if mb_type is `*Q`.
//! 6. Motion vectors:
//!    * `Inter` / `InterQ` — one MV.
//!    * `Inter4MV` / `Inter4MVQ` — four MVs.
//! 7. Inter texture decode (or intra path for embedded I-MBs).
//! 8. Motion compensation + residual add → reconstructed pels.
//!
//! Skipped MBs (`not_coded == 1`) copy the corresponding 16×16 region from
//! the reference frame verbatim, with MV(0,0).

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::block::{
    apply_ac_prediction, choose_dc_predictor, choose_scan, clip_to_u8, decode_inter_ac,
    decode_intra_ac, decode_intra_dc_diff, reconstruct_inter_block, reconstruct_intra_block,
    record_ac_prediction_cache, BlockNeighbour, PredDir,
};
use crate::headers::vol::VideoObjectLayer;
use crate::headers::vop::VideoObjectPlane;
use crate::iq::{dc_scaler, INTRA_DC_VLC_THR_TABLE};
use crate::mb::{IVopPicture, PredGrid};
use crate::mc::{luma_mv_to_chroma, predict_block};
use crate::tables::{cbpy, mcbpc, mv as mv_tab, vlc};

/// Same `dquant` table as the intra path.
const DQUANT_DELTA: [i32; 4] = [-1, -2, 1, 2];

/// One macroblock's worth of motion vectors (in luma half-pel units).
#[derive(Clone, Copy, Debug, Default)]
pub struct MbMotion {
    /// 1 or 4 vectors. For 1MV mode all four entries are equal.
    pub mv: [(i32, i32); 4],
    /// True when the MB used 4MV mode (one MV per luma block).
    pub four_mv: bool,
}

/// Motion-vector predictor grid: per macroblock, an array of 4 MVs (one per
/// luma block). For 1MV mode all four slots hold the same vector. The grid
/// is what `predict_mv()` consults when computing the median predictor.
#[derive(Clone)]
pub struct MvGrid {
    pub mb_w: usize,
    pub mb_h: usize,
    /// Flat: `[mb_y * mb_w + mb_x]` → 4 MVs.
    pub mvs: Vec<MbMotion>,
}

impl MvGrid {
    pub fn new(mb_w: usize, mb_h: usize) -> Self {
        Self {
            mb_w,
            mb_h,
            mvs: vec![MbMotion::default(); mb_w * mb_h],
        }
    }

    pub fn get(&self, mb_x: usize, mb_y: usize) -> &MbMotion {
        &self.mvs[mb_y * self.mb_w + mb_x]
    }

    pub fn set(&mut self, mb_x: usize, mb_y: usize, m: MbMotion) {
        self.mvs[mb_y * self.mb_w + mb_x] = m;
    }
}

/// Decode one motion-vector component from the bitstream (§7.6.3).
///
/// `f_code` is `vop_fcode_forward` (1..=7). Returns the reconstructed
/// vector component (in luma half-pel units), wrapped into the spec's
/// unrestricted-range domain.
pub fn decode_mv_component(br: &mut BitReader<'_>, f_code: u8, predictor_half: i32) -> Result<i32> {
    let r_size = (f_code - 1) as u32;
    let f = 1i32 << r_size;

    let magnitude = vlc::decode(br, mv_tab::table())? as i32;
    let signed_motion_code = if magnitude == 0 {
        0
    } else {
        let sign = br.read_u1()? as i32;
        if sign == 1 {
            -magnitude
        } else {
            magnitude
        }
    };

    // motion_residual: r_size bits (only when f != 1 and motion_code != 0).
    let residual = if f == 1 || signed_motion_code == 0 {
        0
    } else {
        br.read_u32(r_size)? as i32
    };

    // §7.6.3.1 reconstruction:
    //   if motion_code == 0:  diff = 0
    //   else:                 diff = ((|motion_code| - 1) * f + residual + 1)
    //                              * sign(motion_code)
    let diff_abs = if signed_motion_code == 0 {
        0
    } else {
        (signed_motion_code.abs() - 1) * f + residual + 1
    };
    let diff = if signed_motion_code < 0 {
        -diff_abs
    } else {
        diff_abs
    };

    // The decoded vector is the predictor + diff, then folded into the
    // valid range [-32*f, 32*f - 1] by ±64*f.
    let range = 32 * f;
    let mut mv = predictor_half + diff;
    if mv < -range {
        mv += 2 * range;
    } else if mv >= range {
        mv -= 2 * range;
    }
    Ok(mv)
}

/// Compute the median MV predictor for one luma block (§7.6.2 fig 7-6).
///
/// For 1MV mode the same predictor is used for all four luma blocks AND for
/// chroma. `block_idx` selects which of the 4 luma blocks (0..=3) we're
/// predicting; when `four_mv == false` the caller passes `block_idx = 0`.
///
/// The three reference candidates are `MV1` (left), `MV2` (top) and `MV3`
/// (top-right). For blocks at the picture edge unavailable candidates are
/// substituted per spec: if both top neighbours are missing, MV1 is used as
/// the predictor; if only top-right is missing, MV3 substitutes with the
/// top-left.
///
/// `slice_first_mb_x` and `slice_first_mb_y` identify the first MB in the
/// current video packet (slice). When the current MB is on the first row of
/// the slice, special-cases apply (§7.6.2 / FFmpeg `ff_h263_pred_motion`).
pub fn predict_mv_full(
    grid: &MvGrid,
    mb_x: usize,
    mb_y: usize,
    block_idx: usize,
    four_mv: bool,
    slice_first_mb_x: usize,
    slice_first_mb_y: usize,
) -> (i32, i32) {
    // First-slice-line handling: on the first row of MBs after a resync
    // marker, top/top-right neighbours are unavailable (they're in a
    // previous packet). Special-case logic per `ff_h263_pred_motion`:
    let on_first_slice_line = mb_y == slice_first_mb_y;
    if on_first_slice_line && block_idx < 3 {
        match block_idx {
            0 => {
                if mb_x == slice_first_mb_x {
                    return (0, 0);
                } else if mb_x + 1 == slice_first_mb_x {
                    // The top-right neighbour is in the current packet.
                    // (Rare: only when the slice straddles a row.)
                    let c = grid.get(mb_x + 1, mb_y - 1).mv[2];
                    let left = grid.get(mb_x - 1, mb_y).mv[1];
                    return (median(left.0, 0, c.0), median(left.1, 0, c.1));
                } else {
                    let left = grid.get(mb_x - 1, mb_y).mv[1];
                    return left;
                }
            }
            1 => {
                if mb_x + 1 == slice_first_mb_x {
                    let cur = grid.get(mb_x, mb_y).mv[0];
                    let c = grid.get(mb_x + 1, mb_y - 1).mv[2];
                    return (median(cur.0, 0, c.0), median(cur.1, 0, c.1));
                } else {
                    let cur = grid.get(mb_x, mb_y).mv[0];
                    return cur;
                }
            }
            2 => {
                // Block 2 is below blocks 0/1 of the same MB — those are in
                // the current packet so prediction works normally.
                // Fall through to general path.
            }
            _ => {}
        }
    }

    predict_mv(grid, mb_x, mb_y, block_idx, four_mv)
}

fn median(a: i32, b: i32, c: i32) -> i32 {
    if a > b {
        if b > c {
            b
        } else if a > c {
            c
        } else {
            a
        }
    } else if a > c {
        a
    } else if b > c {
        c
    } else {
        b
    }
}

pub fn predict_mv(
    grid: &MvGrid,
    mb_x: usize,
    mb_y: usize,
    block_idx: usize,
    four_mv: bool,
) -> (i32, i32) {
    // For the 1-MV case, the block_idx is always 0.
    let block_idx = if four_mv { block_idx } else { 0 };

    // Spec §7.6.2 figure 7-6: For luma block `block_idx ∈ {0,1,2,3}`:
    //   MV1 = left neighbour
    //   MV2 = top neighbour
    //   MV3 = top-right neighbour
    //
    // In the 1MV case the neighbour MBs each carry one vector replicated 4x.
    // In the 4MV case we sample specific sub-blocks per the spec's diagram.
    //
    // Our sub-block layout (luma blocks 0..=3 within an MB):
    //   0 1
    //   2 3
    //
    // For `block_idx == 0`: MV1 is block 1 of left MB, MV2 is block 2 of top
    // MB, MV3 is block 2 of top-right MB.
    // For `block_idx == 1`: MV1 is block 0 of THIS MB, MV2 is block 3 of top
    // MB, MV3 is block 2 of top-right MB.
    // For `block_idx == 2`: MV1 is block 3 of left MB, MV2 is block 0 of THIS
    // MB, MV3 is block 1 of THIS MB.
    // For `block_idx == 3`: MV1 is block 2 of THIS MB, MV2 is block 1 of THIS
    // MB, MV3 is block 0 of THIS MB.
    //
    // The "this MB" predictors are only valid if those sub-blocks have been
    // decoded already. Since we decode in raster order across luma blocks
    // (0,1,2,3), this works.

    let cur = grid.get(mb_x, mb_y);
    let left = if mb_x > 0 {
        Some(grid.get(mb_x - 1, mb_y))
    } else {
        None
    };
    let top = if mb_y > 0 {
        Some(grid.get(mb_x, mb_y - 1))
    } else {
        None
    };
    let top_right = if mb_y > 0 && mb_x + 1 < grid.mb_w {
        Some(grid.get(mb_x + 1, mb_y - 1))
    } else {
        None
    };

    let pick =
        |opt: Option<&MbMotion>, idx: usize| -> Option<(i32, i32)> { opt.map(|m| m.mv[idx]) };

    type OptMv = Option<(i32, i32)>;
    let (mv1, mv2, mv3): (OptMv, OptMv, OptMv) = match block_idx {
        0 => (pick(left, 1), pick(top, 2), pick(top_right, 2)),
        1 => (Some(cur.mv[0]), pick(top, 3), pick(top_right, 2)),
        2 => (pick(left, 3), Some(cur.mv[0]), Some(cur.mv[1])),
        3 => (Some(cur.mv[2]), Some(cur.mv[1]), Some(cur.mv[0])),
        _ => unreachable!(),
    };

    // Substitute defaults per §7.6.2:
    //   If MV1 alone is unavailable: MV1 = MV2 = MV3 = 0.
    //   Else if MV2 unavailable but MV1 available: MV2 = MV3 = MV1.
    //   Else if MV3 unavailable: MV3 = (0,0).
    let (mv1, mv2, mv3) = match (mv1, mv2, mv3) {
        (None, _, _) => ((0, 0), (0, 0), (0, 0)),
        (Some(a), None, _) => (a, a, a),
        (Some(a), Some(b), None) => (a, b, (0, 0)),
        (Some(a), Some(b), Some(c)) => (a, b, c),
    };

    // Component-wise median.
    let med = |a: i32, b: i32, c: i32| -> i32 {
        // median of three.
        if a > b {
            if b > c {
                b
            } else if a > c {
                c
            } else {
                a
            }
        } else if a > c {
            a
        } else if b > c {
            c
        } else {
            b
        }
    };

    (med(mv1.0, mv2.0, mv3.0), med(mv1.1, mv2.1, mv3.1))
}

/// Decode a single P-VOP macroblock at `(mb_x, mb_y)`. Reads from `br`,
/// writes to `pic`, updates `pred_grid` (for intra-in-P AC/DC prediction)
/// and `mv_grid` (for motion-vector prediction). Returns the new quant.
///
/// `slice_first_mb` is the (mb_x, mb_y) of the first MB in the current
/// video packet. Used for first-slice-line MV prediction (§7.6.2).
#[allow(clippy::too_many_arguments)]
pub fn decode_p_mb(
    br: &mut BitReader<'_>,
    mb_x: usize,
    mb_y: usize,
    quant_in: u32,
    vol: &VideoObjectLayer,
    vop: &VideoObjectPlane,
    pic: &mut IVopPicture,
    pred_grid: &mut PredGrid,
    mv_grid: &mut MvGrid,
    reference: &IVopPicture,
    slice_first_mb: (usize, usize),
) -> Result<u32> {
    // 1. not_coded — 1-bit flag (§6.3.5).
    let not_coded = br.read_u1()? == 1;
    if not_coded {
        // Skipped MB: copy the 16×16 luma + 8×8 chroma blocks from the
        // reference at the same position with MV(0,0).
        copy_skipped_mb(pic, reference, mb_x, mb_y);
        // Reset MV predictor grid — skipped MBs contribute (0,0) to future
        // MVs (§7.6.7).
        mv_grid.set(mb_x, mb_y, MbMotion::default());
        // Reset prediction grid (no intra info).
        reset_pred_grid_mb(pred_grid, mb_x, mb_y);
        return Ok(quant_in);
    }

    // 2. MCBPC inter VLC (Table B-13). Loop on stuffing.
    let mcbpc_v = loop {
        let v = vlc::decode(br, mcbpc::p_table())?;
        if v != mcbpc::INTER_STUFFING {
            break v;
        }
    };
    let (mb_type, cbpc) = mcbpc::decompose_inter(mcbpc_v);

    // 3. ac_pred_flag — only for Intra/IntraQ MBs in P-VOP.
    let ac_pred =
        matches!(mb_type, mcbpc::PMbType::Intra | mcbpc::PMbType::IntraQ) && br.read_u1()? == 1;

    // 4. CBPY. For inter MBs the value is bit-inverted (§7.4.1.2).
    let cbpy_raw = vlc::decode(br, cbpy::table())?;
    let cbpy = match mb_type {
        mcbpc::PMbType::Intra | mcbpc::PMbType::IntraQ => cbpy_raw,
        _ => cbpy_raw ^ 0xF,
    };

    // 5. dquant if needed.
    let mut quant = quant_in;
    if matches!(
        mb_type,
        mcbpc::PMbType::InterQ | mcbpc::PMbType::IntraQ | mcbpc::PMbType::Inter4MVQ
    ) {
        let d = br.read_u32(2)? as usize;
        let new_q = (quant as i32) + DQUANT_DELTA[d];
        quant = new_q.clamp(1, 31) as u32;
    }

    // 6. Motion vectors (skip for intra MBs).
    let mut motion = MbMotion::default();
    let four_mv = matches!(
        mb_type,
        mcbpc::PMbType::Inter4MV | mcbpc::PMbType::Inter4MVQ
    );
    let is_intra = matches!(mb_type, mcbpc::PMbType::Intra | mcbpc::PMbType::IntraQ);

    if !is_intra {
        let f_code = vop.vop_fcode_forward.max(1);
        if four_mv {
            for blk in 0..4 {
                let (px, py) = predict_mv_full(
                    mv_grid,
                    mb_x,
                    mb_y,
                    blk,
                    true,
                    slice_first_mb.0,
                    slice_first_mb.1,
                );
                let mvx = decode_mv_component(br, f_code, px)?;
                let mvy = decode_mv_component(br, f_code, py)?;
                motion.mv[blk] = (mvx, mvy);
            }
            motion.four_mv = true;
            mv_grid.set(mb_x, mb_y, motion);
        } else {
            let (px, py) = predict_mv_full(
                mv_grid,
                mb_x,
                mb_y,
                0,
                false,
                slice_first_mb.0,
                slice_first_mb.1,
            );
            let mvx = decode_mv_component(br, f_code, px)?;
            let mvy = decode_mv_component(br, f_code, py)?;
            motion.mv = [(mvx, mvy); 4];
            motion.four_mv = false;
            mv_grid.set(mb_x, mb_y, motion);
        }
    } else {
        // Intra MB carries (0,0) as future neighbour predictor.
        mv_grid.set(mb_x, mb_y, MbMotion::default());
    }

    // 7. Texture: per-block luma + chroma decode.
    if is_intra {
        // Embedded intra MB inside a P-VOP — uses the same intra path as
        // I-VOP. We reuse `decode_one_intra_block`-style logic via the
        // intra MB helpers, but inlined here to share the per-MB context
        // with the P path.
        decode_intra_blocks_in_p(
            br, mb_x, mb_y, cbpy, cbpc, ac_pred, quant, vol, vop, pic, pred_grid,
        )?;
        return Ok(quant);
    }

    // Inter MB: 6 blocks, each with cbp bit. Decode residual + add predictor.
    let luma_coded = [
        (cbpy >> 3) & 1 != 0,
        (cbpy >> 2) & 1 != 0,
        (cbpy >> 1) & 1 != 0,
        cbpy & 1 != 0,
    ];
    let chroma_coded = [(cbpc >> 1) & 1 != 0, cbpc & 1 != 0];

    // For each luma block, apply MC and add residual.
    for blk in 0..4 {
        let (mvx, mvy) = motion.mv[blk];
        let (sub_x, sub_y) = match blk {
            0 => (0, 0),
            1 => (8, 0),
            2 => (0, 8),
            3 => (8, 8),
            _ => unreachable!(),
        };
        let blk_px = (mb_x * 16 + sub_x) as i32;
        let blk_py = (mb_y * 16 + sub_y) as i32;

        // Build prediction block.
        let mut pred_buf = [0u8; 64];
        predict_block(
            &reference.y,
            reference.y_stride,
            reference.y_stride as i32,
            (reference.y.len() / reference.y_stride) as i32,
            blk_px,
            blk_py,
            mvx,
            mvy,
            8,
            vop.rounding_type,
            &mut pred_buf,
            8,
        );

        // Decode residual if coded; otherwise zero.
        let mut residual = [0i32; 64];
        if luma_coded[blk] {
            decode_inter_ac(br, &mut residual, &crate::headers::vol::ZIGZAG)?;
            let mut out = [0i32; 64];
            reconstruct_inter_block(&mut residual, vol, quant, &mut out)?;
            // Add predictor + residual, clip.
            for j in 0..8 {
                for i in 0..8 {
                    let v = pred_buf[j * 8 + i] as i32 + out[j * 8 + i];
                    pic.y[(blk_py as usize + j) * pic.y_stride + (blk_px as usize + i)] =
                        clip_to_u8(v);
                }
            }
        } else {
            // No residual: just the predictor.
            for j in 0..8 {
                for i in 0..8 {
                    pic.y[(blk_py as usize + j) * pic.y_stride + (blk_px as usize + i)] =
                        pred_buf[j * 8 + i];
                }
            }
        }
    }

    // Chroma. For 1MV mode use the single MV scaled to chroma; for 4MV mode
    // use the average of the 4 luma MVs scaled.
    let (cmx, cmy) = if four_mv {
        let sx: i32 = motion.mv.iter().map(|(x, _)| *x).sum();
        let sy: i32 = motion.mv.iter().map(|(_, y)| *y).sum();
        (luma_mv_to_chroma(sx / 4), luma_mv_to_chroma(sy / 4))
    } else {
        (
            luma_mv_to_chroma(motion.mv[0].0),
            luma_mv_to_chroma(motion.mv[0].1),
        )
    };
    for plane_idx in 0..2 {
        let (ref_plane, ref_stride) = if plane_idx == 0 {
            (&reference.cb, reference.c_stride)
        } else {
            (&reference.cr, reference.c_stride)
        };
        let blk_px = (mb_x * 8) as i32;
        let blk_py = (mb_y * 8) as i32;
        let mut pred_buf = [0u8; 64];
        predict_block(
            ref_plane,
            ref_stride,
            ref_stride as i32,
            (ref_plane.len() / ref_stride) as i32,
            blk_px,
            blk_py,
            cmx,
            cmy,
            8,
            vop.rounding_type,
            &mut pred_buf,
            8,
        );
        let coded = chroma_coded[plane_idx];
        let mut residual = [0i32; 64];
        if coded {
            decode_inter_ac(br, &mut residual, &crate::headers::vol::ZIGZAG)?;
            let mut out = [0i32; 64];
            reconstruct_inter_block(&mut residual, vol, quant, &mut out)?;
            let dst_plane = if plane_idx == 0 {
                &mut pic.cb
            } else {
                &mut pic.cr
            };
            for j in 0..8 {
                for i in 0..8 {
                    let v = pred_buf[j * 8 + i] as i32 + out[j * 8 + i];
                    dst_plane[(blk_py as usize + j) * pic.c_stride + (blk_px as usize + i)] =
                        clip_to_u8(v);
                }
            }
        } else {
            let dst_plane = if plane_idx == 0 {
                &mut pic.cb
            } else {
                &mut pic.cr
            };
            for j in 0..8 {
                for i in 0..8 {
                    dst_plane[(blk_py as usize + j) * pic.c_stride + (blk_px as usize + i)] =
                        pred_buf[j * 8 + i];
                }
            }
        }
    }

    // Inter MBs reset the AC/DC prediction grid (no intra info).
    reset_pred_grid_mb(pred_grid, mb_x, mb_y);

    Ok(quant)
}

/// Copy a skipped MB's 16×16 luma + 2×8×8 chroma from the reference frame.
fn copy_skipped_mb(pic: &mut IVopPicture, reference: &IVopPicture, mb_x: usize, mb_y: usize) {
    let px = mb_x * 16;
    let py = mb_y * 16;
    for j in 0..16 {
        for i in 0..16 {
            pic.y[(py + j) * pic.y_stride + (px + i)] =
                reference.y[(py + j) * reference.y_stride + (px + i)];
        }
    }
    let cx = mb_x * 8;
    let cy = mb_y * 8;
    for j in 0..8 {
        for i in 0..8 {
            pic.cb[(cy + j) * pic.c_stride + (cx + i)] =
                reference.cb[(cy + j) * reference.c_stride + (cx + i)];
            pic.cr[(cy + j) * pic.c_stride + (cx + i)] =
                reference.cr[(cy + j) * reference.c_stride + (cx + i)];
        }
    }
}

/// Reset the AC/DC prediction slots for one MB (used when an inter or
/// skipped MB clears intra prediction state — non-intra blocks predict
/// `dc=1024, ac=0` for downstream consumers).
fn reset_pred_grid_mb(grid: &mut PredGrid, mb_x: usize, mb_y: usize) {
    let positions: [(usize, usize); 4] = [
        (mb_x * 2, mb_y * 2),
        (mb_x * 2 + 1, mb_y * 2),
        (mb_x * 2, mb_y * 2 + 1),
        (mb_x * 2 + 1, mb_y * 2 + 1),
    ];
    for (bx, by) in positions {
        let idx = by * grid.y_stride + bx;
        grid.y[idx] = BlockNeighbour::default();
    }
    let cidx = mb_y * grid.c_stride + mb_x;
    grid.cb[cidx] = BlockNeighbour::default();
    grid.cr[cidx] = BlockNeighbour::default();
}

/// Intra MB inside a P-VOP — same flow as the I-VOP intra MB but inlined to
/// receive the already-decoded `cbpc/cbpy/quant/ac_pred` from the P MCBPC
/// path. Updates `pic` and `pred_grid`.
#[allow(clippy::too_many_arguments)]
fn decode_intra_blocks_in_p(
    br: &mut BitReader<'_>,
    mb_x: usize,
    mb_y: usize,
    cbpy: u8,
    cbpc: u8,
    ac_pred: bool,
    quant: u32,
    vol: &VideoObjectLayer,
    vop: &VideoObjectPlane,
    pic: &mut IVopPicture,
    grid: &mut PredGrid,
) -> Result<()> {
    let luma_coded = [
        (cbpy >> 3) & 1 != 0,
        (cbpy >> 2) & 1 != 0,
        (cbpy >> 1) & 1 != 0,
        cbpy & 1 != 0,
    ];
    let thr = INTRA_DC_VLC_THR_TABLE[vop.intra_dc_vlc_thr as usize] as u32;
    let use_intra_dc_vlc = quant < thr.max(1);

    for block_idx in 0..6 {
        let coded = if block_idx < 4 {
            luma_coded[block_idx]
        } else if block_idx == 4 {
            (cbpc >> 1) & 1 != 0
        } else {
            cbpc & 1 != 0
        };
        decode_one_intra_block_p(
            br,
            block_idx,
            coded,
            ac_pred,
            use_intra_dc_vlc,
            mb_x,
            mb_y,
            quant,
            vol,
            pic,
            grid,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_one_intra_block_p(
    br: &mut BitReader<'_>,
    block_idx: usize,
    coded: bool,
    ac_pred: bool,
    use_intra_dc_vlc: bool,
    mb_x: usize,
    mb_y: usize,
    quant: u32,
    vol: &VideoObjectLayer,
    pic: &mut IVopPicture,
    grid: &mut PredGrid,
) -> Result<()> {
    if !use_intra_dc_vlc {
        return Err(Error::unsupported(
            "mpeg4 P-VOP intra MB: plain-13-bit DC path not yet implemented",
        ));
    }
    let dc_diff = decode_intra_dc_diff(br, block_idx)?;
    let (predicted_dc_pel, dc_pred_dir) = predict_dc_p(block_idx, mb_x, mb_y, grid);

    let mut coeffs = [0i32; 64];
    let scan = choose_scan(ac_pred, dc_pred_dir);
    if coded {
        decode_intra_ac(br, &mut coeffs, scan)?;
    }
    if ac_pred {
        apply_ac_from_neighbour_p(block_idx, dc_pred_dir, mb_x, mb_y, quant, grid, &mut coeffs);
    }

    let scale_pel = dc_scaler(block_idx, quant) as i32;
    let pred_units = (predicted_dc_pel + scale_pel / 2) / scale_pel;
    let recon_units = pred_units + dc_diff;
    let recon_dc = (recon_units * scale_pel).clamp(0, 2047);

    {
        let nbr = block_neighbour_mut_p(grid, block_idx, mb_x, mb_y);
        nbr.dc = recon_dc;
        nbr.quant = quant as u8;
        nbr.is_intra = true;
        record_ac_prediction_cache(&coeffs, nbr);
    }
    coeffs[0] = recon_dc;

    let mut out = [0i32; 64];
    reconstruct_intra_block(&mut coeffs, vol, quant, &mut out)?;
    write_intra_block(pic, block_idx, mb_x, mb_y, &out);
    Ok(())
}

fn predict_dc_p(block_idx: usize, mb_x: usize, mb_y: usize, grid: &PredGrid) -> (i32, PredDir) {
    let (left, top_left, top) = lookup_neighbour_dcs_p(block_idx, mb_x, mb_y, grid);
    choose_dc_predictor(left, top_left, top)
}

fn lookup_neighbour_dcs_p(
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
    grid: &PredGrid,
) -> (i32, i32, i32) {
    let (plane, bx, by, stride) = block_grid_position_p(block_idx, mb_x, mb_y, grid);
    let read = |px: isize, py: isize| -> i32 {
        if px < 0 || py < 0 {
            return 1024;
        }
        let rows = plane.len() / stride;
        if (px as usize) >= stride || (py as usize) >= rows {
            return 1024;
        }
        let idx = (py as usize) * stride + (px as usize);
        if plane[idx].is_intra {
            plane[idx].dc
        } else {
            1024
        }
    };
    let left = read(bx as isize - 1, by as isize);
    let top = read(bx as isize, by as isize - 1);
    let top_left = read(bx as isize - 1, by as isize - 1);
    (left, top_left, top)
}

fn block_grid_position_p(
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
    grid: &PredGrid,
) -> (&[BlockNeighbour], usize, usize, usize) {
    match block_idx {
        0 => (&grid.y, mb_x * 2, mb_y * 2, grid.y_stride),
        1 => (&grid.y, mb_x * 2 + 1, mb_y * 2, grid.y_stride),
        2 => (&grid.y, mb_x * 2, mb_y * 2 + 1, grid.y_stride),
        3 => (&grid.y, mb_x * 2 + 1, mb_y * 2 + 1, grid.y_stride),
        4 => (&grid.cb, mb_x, mb_y, grid.c_stride),
        5 => (&grid.cr, mb_x, mb_y, grid.c_stride),
        _ => unreachable!(),
    }
}

fn block_neighbour_mut_p(
    grid: &mut PredGrid,
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
) -> &mut BlockNeighbour {
    let (bx, by, stride) = match block_idx {
        0 => (mb_x * 2, mb_y * 2, grid.y_stride),
        1 => (mb_x * 2 + 1, mb_y * 2, grid.y_stride),
        2 => (mb_x * 2, mb_y * 2 + 1, grid.y_stride),
        3 => (mb_x * 2 + 1, mb_y * 2 + 1, grid.y_stride),
        4 => (mb_x, mb_y, grid.c_stride),
        5 => (mb_x, mb_y, grid.c_stride),
        _ => unreachable!(),
    };
    let flat = by * stride + bx;
    match block_idx {
        0..=3 => &mut grid.y[flat],
        4 => &mut grid.cb[flat],
        5 => &mut grid.cr[flat],
        _ => unreachable!(),
    }
}

fn apply_ac_from_neighbour_p(
    block_idx: usize,
    dir: PredDir,
    mb_x: usize,
    mb_y: usize,
    quant: u32,
    grid: &PredGrid,
    coeffs: &mut [i32; 64],
) {
    let (plane, bx, by, stride) = block_grid_position_p(block_idx, mb_x, mb_y, grid);
    let (nx, ny) = match dir {
        PredDir::Left => (bx as isize - 1, by as isize),
        PredDir::Top => (bx as isize, by as isize - 1),
    };
    if nx < 0 || ny < 0 {
        return;
    }
    let rows = plane.len() / stride;
    if (nx as usize) >= stride || (ny as usize) >= rows {
        return;
    }
    let nbr = &plane[(ny as usize) * stride + (nx as usize)];
    if !nbr.is_intra {
        return;
    }
    apply_ac_prediction(coeffs, dir, nbr, quant as u8);
}

fn write_intra_block(
    pic: &mut IVopPicture,
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
    out: &[i32; 64],
) {
    let (plane, stride, px, py) = match block_idx {
        0 => (pic.y.as_mut_slice(), pic.y_stride, mb_x * 16, mb_y * 16),
        1 => (pic.y.as_mut_slice(), pic.y_stride, mb_x * 16 + 8, mb_y * 16),
        2 => (pic.y.as_mut_slice(), pic.y_stride, mb_x * 16, mb_y * 16 + 8),
        3 => (
            pic.y.as_mut_slice(),
            pic.y_stride,
            mb_x * 16 + 8,
            mb_y * 16 + 8,
        ),
        4 => (pic.cb.as_mut_slice(), pic.c_stride, mb_x * 8, mb_y * 8),
        5 => (pic.cr.as_mut_slice(), pic.c_stride, mb_x * 8, mb_y * 8),
        _ => unreachable!(),
    };
    for dy in 0..8 {
        for dx in 0..8 {
            plane[(py + dy) * stride + (px + dx)] = clip_to_u8(out[dy * 8 + dx]);
        }
    }
}
