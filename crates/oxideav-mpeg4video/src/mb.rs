//! Macroblock-level decoding for MPEG-4 Part 2 I-VOPs (spec §6.3.7).
//!
//! Per-MB decode sequence (intra / I-VOP path):
//! 1. MCBPC VLC (Table B-10) → `(mb_type, cbpc)` — cbpc gives the 2 chroma
//!    block-coded flags (§7.4.1.2).
//! 2. `ac_pred_flag` — 1 bit.
//! 3. CBPY VLC (Table B-9) → 4 luma block-coded flags.
//!    For intra MBs the decoded value is bit-inverted (§7.4.1.2) — note that
//!    our table already maps the raw VLC to its face value, so the caller
//!    XORs with 0xF to obtain the actual CBPY for intra.
//! 4. If `mb_type == INTRA+Q`: `dquant` — 2-bit signed quant adjustment.
//! 5. Per block (Y0..Y3, Cb, Cr):
//!    * intra-DC VLC + signed residual bits (or plain 13-bit DC for
//!      high-quant short-header mode — not yet handled),
//!    * if the block is `coded`, AC coefficients via tcoef VLC walk,
//!    * DC + AC prediction (§7.4.3),
//!    * dequantisation and IDCT,
//!    * write into the picture buffer.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::block::{
    apply_ac_prediction, choose_dc_predictor, choose_scan, clip_to_u8, decode_intra_ac,
    decode_intra_dc_diff, reconstruct_intra_block, record_ac_prediction_cache, BlockNeighbour,
    PredDir,
};
use crate::headers::vol::VideoObjectLayer;
use crate::headers::vop::VideoObjectPlane;
use crate::iq::{dc_scaler, INTRA_DC_VLC_THR_TABLE};
use crate::tables::{cbpy, mcbpc, vlc};

/// Signed `dquant` adjustment — Table 6-20 / 7-3. 2-bit field indexing
/// `[-1, -2, 1, 2]` for the four codes `0, 1, 2, 3`.
const DQUANT_DELTA: [i32; 4] = [-1, -2, 1, 2];

/// Reconstructed I-VOP: three pel planes (Y, Cb, Cr) sized to the MB-aligned
/// image. Planes are stride-packed (stride == width for each plane).
pub struct IVopPicture {
    pub width: usize,
    pub height: usize,
    pub mb_width: usize,
    pub mb_height: usize,
    pub y: Vec<u8>,
    pub cb: Vec<u8>,
    pub cr: Vec<u8>,
    pub y_stride: usize,
    pub c_stride: usize,
}

impl IVopPicture {
    pub fn new(width: usize, height: usize) -> Self {
        let mb_w = width.div_ceil(16);
        let mb_h = height.div_ceil(16);
        let y_stride = mb_w * 16;
        let c_stride = mb_w * 8;
        let y_h = mb_h * 16;
        let c_h = mb_h * 8;
        Self {
            width,
            height,
            mb_width: mb_w,
            mb_height: mb_h,
            y_stride,
            c_stride,
            y: vec![0u8; y_stride * y_h],
            cb: vec![0u8; c_stride * c_h],
            cr: vec![0u8; c_stride * c_h],
        }
    }
}

/// Neighbour cache layout for AC/DC prediction. One grid per plane:
/// * Y: `2 * mb_width` columns × `2 * mb_height` rows (2×2 blocks per MB).
/// * Cb, Cr: `mb_width` × `mb_height` (one block per MB).
///
/// All slots are initialised with `BlockNeighbour::default()` (dc=1024,
/// is_intra=false, quant=1), so out-of-bounds predictions yield the spec's
/// default "half-range" DC.
pub struct PredGrid {
    pub y: Vec<BlockNeighbour>,  // size (2*mbw) * (2*mbh)
    pub cb: Vec<BlockNeighbour>, // size mbw * mbh
    pub cr: Vec<BlockNeighbour>, // size mbw * mbh
    pub y_stride: usize,
    pub c_stride: usize,
}

impl PredGrid {
    pub fn new(mb_width: usize, mb_height: usize) -> Self {
        let y_stride = mb_width * 2;
        let y_len = y_stride * (mb_height * 2);
        let c_stride = mb_width;
        let c_len = c_stride * mb_height;
        Self {
            y: vec![BlockNeighbour::default(); y_len],
            cb: vec![BlockNeighbour::default(); c_len],
            cr: vec![BlockNeighbour::default(); c_len],
            y_stride,
            c_stride,
        }
    }
}

/// Decode one intra MB body, starting with the MCBPC VLC and ending having
/// written 16×16 luma + 8×8 Cb + 8×8 Cr samples into `pic`. Advances the
/// AC/DC predictor cache in `grid`. Returns the new quantiser (possibly
/// adjusted by `dquant`).
pub fn decode_intra_mb(
    br: &mut BitReader<'_>,
    mb_x: usize,
    mb_y: usize,
    quant_in: u32,
    vol: &VideoObjectLayer,
    vop: &VideoObjectPlane,
    pic: &mut IVopPicture,
    grid: &mut PredGrid,
) -> Result<u32> {
    // 1. MCBPC (loop over stuffing codewords).
    let mcbpc_v = loop {
        let v = vlc::decode(br, mcbpc::i_table())?;
        if v != mcbpc::STUFFING {
            break v;
        }
    };
    // Table B-10 decode: values 0..=3 are "intra" (cbpc = value), values 4..=7
    // are "intra+Q" (cbpc = value - 4).
    let (is_intra_q, cbpc) = if mcbpc_v < 4 {
        (false, mcbpc_v)
    } else if mcbpc_v < 8 {
        (true, mcbpc_v - 4)
    } else {
        return Err(Error::invalid("mpeg4 MB: invalid mcbpc value"));
    };

    // 2. ac_pred_flag (§6.3.7).
    let ac_pred = br.read_u1()? == 1;

    // 3. CBPY. For MPEG-4 Part 2 I-VOPs the VLC result is used directly —
    // ffmpeg only inverts in the inter path. (H.263 Annex I AIC also does not
    // invert.) Each bit of the 4-bit value flags a Y block as coded.
    let cbpy = vlc::decode(br, cbpy::table())?;

    // 4. Optional dquant.
    let mut quant = quant_in;
    if is_intra_q {
        let d = br.read_u32(2)? as usize;
        let new_q = (quant as i32) + DQUANT_DELTA[d];
        quant = new_q.clamp(1, 31) as u32;
    }

    // 5. Per-block decode (order: Y0, Y1, Y2, Y3, Cb, Cr).
    // Each block's `coded` flag comes from CBPY (top 4 bits, order block 0-3
    // = bits 3,2,1,0 per spec) / CBPC (bits 1,0 for Cb, Cr).
    let luma_coded = [
        (cbpy >> 3) & 1 != 0,
        (cbpy >> 2) & 1 != 0,
        (cbpy >> 1) & 1 != 0,
        cbpy & 1 != 0,
    ];
    let chroma_coded = [(cbpc >> 1) & 1 != 0, cbpc & 1 != 0];
    let _ = chroma_coded;
    // We use explicit coded-block flags per §6.3.8 Table 7-7 layout.

    // intra_dc_vlc_thr governs whether to use plain-13-bit DC instead of the
    // DC-size VLC. `thr[intra_dc_vlc_thr]` is the QP threshold; when the
    // current quant is >= threshold the plain path is used. For typical
    // ffmpeg streams `intra_dc_vlc_thr == 0` so the VLC path is always used;
    // we still respect the threshold for correctness.
    let thr = INTRA_DC_VLC_THR_TABLE[vop.intra_dc_vlc_thr as usize] as u32;
    let use_intra_dc_vlc = quant < thr.max(1);

    // Decode each of the 6 blocks.
    for block_idx in 0..6 {
        let coded = if block_idx < 4 {
            match block_idx {
                0 => luma_coded[0],
                1 => luma_coded[1],
                2 => luma_coded[2],
                3 => luma_coded[3],
                _ => unreachable!(),
            }
        } else if block_idx == 4 {
            (cbpc >> 1) & 1 != 0
        } else {
            cbpc & 1 != 0
        };

        decode_one_intra_block(
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

    Ok(quant)
}

#[allow(clippy::too_many_arguments)]
fn decode_one_intra_block(
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
    // --- Decode DC ---
    let (dc_diff, dc_pred_dir, predicted_dc_pel) = if use_intra_dc_vlc {
        let diff = decode_intra_dc_diff(br, block_idx)?;
        let (pred_pel, dir) = predict_dc(block_idx, mb_x, mb_y, grid);
        (diff, dir, pred_pel)
    } else {
        // Plain 8-bit DC (short-header high-QP mode).
        let raw = br.read_u32(8)? as i32;
        let (_pred_pel, dir) = predict_dc(block_idx, mb_x, mb_y, grid);
        let scale = dc_scaler(block_idx, quant) as i32;
        return plain_dc_block(
            br, block_idx, coded, ac_pred, raw, scale, dir, mb_x, mb_y, quant, vol, pic, grid,
        );
    };

    // --- Decode AC coefficients (if coded) ---
    let mut coeffs = [0i32; 64];
    let scan = choose_scan(ac_pred, dc_pred_dir);

    if coded {
        decode_intra_ac(br, &mut coeffs, scan)?;
    }

    // --- AC prediction (§7.4.3.2): add predicted ACs in level domain. ---
    if ac_pred {
        apply_ac_from_neighbour(block_idx, dc_pred_dir, mb_x, mb_y, quant, grid, &mut coeffs);
    }

    // --- Record this block's post-prediction, pre-dequant ACs for future
    // neighbours BEFORE dequantisation (level space per spec §7.4.3.2). ---
    let nbr_idx = block_neighbour_index(block_idx, mb_x, mb_y, grid);
    // DC reconstruction per §7.4.3.1 and FFmpeg's mpeg4_get_level_dc:
    //   pred_dc_in_units = (pred_pel + scale/2) / scale
    //   recon_units = pred_dc_in_units + dc_diff
    //   recon_pel = recon_units * scale
    let scale_pel = dc_scaler(block_idx, quant) as i32;
    let pred_units = (predicted_dc_pel + scale_pel / 2) / scale_pel;
    let recon_units = pred_units + dc_diff;
    let recon_dc = (recon_units * scale_pel).clamp(0, 2047);
    {
        let nbr = block_neighbour_mut(grid, block_idx, nbr_idx);
        nbr.dc = recon_dc;
        nbr.quant = quant as u8;
        nbr.is_intra = true;
        record_ac_prediction_cache(&coeffs, nbr);
    }

    // --- Reconstruct DC (pel domain) into coeffs[0] for IDCT. ---
    coeffs[0] = recon_dc;

    // --- Dequant + IDCT. ---
    let mut out = [0i32; 64];
    reconstruct_intra_block(&mut coeffs, vol, quant, &mut out)?;

    write_block_to_picture(pic, block_idx, mb_x, mb_y, &out);
    Ok(())
}

/// Plain-DC path (used when `intra_dc_vlc_thr` and the current quant say so).
/// Reads the 8-bit DC directly and skips AC prediction.
#[allow(clippy::too_many_arguments)]
fn plain_dc_block(
    br: &mut BitReader<'_>,
    block_idx: usize,
    coded: bool,
    ac_pred: bool,
    raw_dc: i32,
    dc_scale: i32,
    dir: PredDir,
    mb_x: usize,
    mb_y: usize,
    quant: u32,
    vol: &VideoObjectLayer,
    pic: &mut IVopPicture,
    grid: &mut PredGrid,
) -> Result<()> {
    // AC still uses the tcoef VLC walk if the block is coded.
    let mut coeffs = [0i32; 64];
    let scan = choose_scan(ac_pred, dir);
    if coded {
        decode_intra_ac(br, &mut coeffs, scan)?;
    }
    // AC prediction for plain DC path is skipped — the DC is absolute so
    // neighbour ACs mean nothing here.
    let _ = ac_pred;

    coeffs[0] = raw_dc * dc_scale;

    let mut out = [0i32; 64];
    reconstruct_intra_block(&mut coeffs, vol, quant, &mut out)?;

    let nbr_idx = block_neighbour_index(block_idx, mb_x, mb_y, grid);
    let nbr = block_neighbour_mut(grid, block_idx, nbr_idx);
    nbr.dc = coeffs[0];
    nbr.quant = quant as u8;
    nbr.is_intra = true;
    // Cache ACs (from `coeffs` — the dequantised ACs — as a fallback).
    record_ac_prediction_cache(&coeffs, nbr);
    write_block_to_picture(pic, block_idx, mb_x, mb_y, &out);
    Ok(())
}

/// For block `block_idx` in macroblock `(mb_x, mb_y)`, look up the three
/// reference neighbour DCs and return the predicted DC + chosen direction.
fn predict_dc(block_idx: usize, mb_x: usize, mb_y: usize, grid: &PredGrid) -> (i32, PredDir) {
    let (left, top_left, top) = lookup_neighbour_dcs(block_idx, mb_x, mb_y, grid);
    choose_dc_predictor(left, top_left, top)
}

/// Fetch the DCs of the three reference neighbours for this block (§7.4.3.1).
/// Returns `(left, top_left, top)` in pel domain. Missing / non-intra
/// neighbours contribute `1024`.
fn lookup_neighbour_dcs(
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
    grid: &PredGrid,
) -> (i32, i32, i32) {
    // Figure out the block's (bx, by) in its plane grid.
    let (plane, bx, by, stride) = block_grid_position(block_idx, mb_x, mb_y, grid);
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

/// Like `lookup_neighbour_dcs` but returns a mutable reference to the full
/// neighbour slot at this block's position (so we can update it after decode).
fn block_neighbour_index(
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
    grid: &PredGrid,
) -> (usize, usize) {
    let (bx, by, stride) = match block_idx {
        0 => (mb_x * 2, mb_y * 2, grid.y_stride),
        1 => (mb_x * 2 + 1, mb_y * 2, grid.y_stride),
        2 => (mb_x * 2, mb_y * 2 + 1, grid.y_stride),
        3 => (mb_x * 2 + 1, mb_y * 2 + 1, grid.y_stride),
        4 => (mb_x, mb_y, grid.c_stride),
        5 => (mb_x, mb_y, grid.c_stride),
        _ => unreachable!(),
    };
    (by * stride + bx, block_idx)
}

fn block_neighbour_mut(
    grid: &mut PredGrid,
    block_idx: usize,
    idx: (usize, usize),
) -> &mut BlockNeighbour {
    let (flat, _bi) = idx;
    match block_idx {
        0..=3 => &mut grid.y[flat],
        4 => &mut grid.cb[flat],
        5 => &mut grid.cr[flat],
        _ => unreachable!(),
    }
}

/// Resolve a block's position in its plane's grid. Returns `(plane_slice,
/// bx, by, stride)` — `plane_slice` is an immutable borrow used for reads.
fn block_grid_position(
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

/// Apply AC prediction from a chosen neighbour onto `block` (level space).
/// `dir` picks left vs top neighbour.
fn apply_ac_from_neighbour(
    block_idx: usize,
    dir: PredDir,
    mb_x: usize,
    mb_y: usize,
    quant: u32,
    grid: &PredGrid,
    coeffs: &mut [i32; 64],
) {
    let (plane, bx, by, stride) = block_grid_position(block_idx, mb_x, mb_y, grid);
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

/// Write the 8×8 reconstructed block samples into the picture. `out[i]` is a
/// pel value in [-256, 255]; this function clips to [0, 255] and stores as
/// `u8` into the Y / Cb / Cr buffer at the block's pel position.
fn write_block_to_picture(
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
            let s = out[dy * 8 + dx];
            plane[(py + dy) * stride + (px + dx)] = clip_to_u8(s);
        }
    }
}
