//! I-slice macroblock decode — ITU-T H.264 §7.3.5 / §8.3 / §8.5.
//!
//! Wires CAVLC residual decode, dequantisation, inverse transforms and intra
//! prediction together for every macroblock in an I-slice and writes
//! reconstructed samples into a [`Picture`].
//!
//! Macroblock layer syntax order (`§7.3.5.1`):
//!
//! 1. `mb_type`
//! 2. If `mb_type == I_NxN`: 16 × `prev_intra4x4_pred_mode_flag` (+ `rem`)
//! 3. If `chroma_format_idc != 0`: `intra_chroma_pred_mode`
//! 4. If `mb_type == I_NxN`: `coded_block_pattern` (`me(v)`)
//! 5. If `cbp != 0` or `mb_type starts I_16x16`: `mb_qp_delta`
//! 6. Residual: luma DC (Intra16x16 only) + 16 luma 4×4 + 2 chroma DC + 8 chroma AC

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::cavlc::{decode_residual_block, BlockKind};
use crate::intra_pred::{
    predict_intra_16x16, predict_intra_4x4, predict_intra_chroma, Intra16x16Mode,
    Intra16x16Neighbours, Intra4x4Mode, Intra4x4Neighbours, IntraChromaMode, IntraChromaNeighbours,
};
use crate::mb_type::{decode_i_slice_mb_type, IMbType};
use crate::picture::{MbInfo, Picture, INTRA_DC_FAKE};
use crate::pps::Pps;
use crate::slice::SliceHeader;
use crate::sps::Sps;
use crate::tables::decode_cbp_intra;
use crate::transform::{
    chroma_qp, dequantize_4x4, idct_4x4, inv_hadamard_2x2_chroma_dc, inv_hadamard_4x4_dc,
};

/// Per-block (4×4) raster ordering of the residual blocks within a
/// macroblock — §8.5.1, Figure 6-12. Block N covers `(LUMA_BLOCK_RASTER[N])`.
pub const LUMA_BLOCK_RASTER: [(usize, usize); 16] = [
    (0, 0),
    (0, 1),
    (1, 0),
    (1, 1),
    (0, 2),
    (0, 3),
    (1, 2),
    (1, 3),
    (2, 0),
    (2, 1),
    (3, 0),
    (3, 1),
    (2, 2),
    (2, 3),
    (3, 2),
    (3, 3),
];

/// Top-left slice decode entry — drives the macroblock loop.
pub fn decode_i_slice_data(
    br: &mut BitReader<'_>,
    sh: &SliceHeader,
    sps: &Sps,
    pps: &Pps,
    pic: &mut Picture,
) -> Result<()> {
    let mb_w = sps.pic_width_in_mbs();
    let mb_h = sps.pic_height_in_map_units();
    let total_mbs = mb_w * mb_h;
    let mut mb_addr = sh.first_mb_in_slice;
    if mb_addr >= total_mbs {
        return Err(Error::invalid("h264 slice: first_mb_in_slice out of range"));
    }
    let mut prev_qp = (pps.pic_init_qp_minus26 + 26 + sh.slice_qp_delta).clamp(0, 51);

    while mb_addr < total_mbs {
        let mb_x = mb_addr % mb_w;
        let mb_y = mb_addr / mb_w;
        decode_one_mb(br, sps, pps, sh, mb_x, mb_y, pic, &mut prev_qp)?;
        mb_addr += 1;
    }
    Ok(())
}

fn decode_one_mb(
    br: &mut BitReader<'_>,
    sps: &Sps,
    pps: &Pps,
    sh: &SliceHeader,
    mb_x: u32,
    mb_y: u32,
    pic: &mut Picture,
    prev_qp: &mut i32,
) -> Result<()> {
    let mb_type = br.read_ue()?;
    let imb = decode_i_slice_mb_type(mb_type)
        .ok_or_else(|| Error::invalid(format!("h264 slice: bad I mb_type {mb_type}")))?;

    if matches!(imb, IMbType::IPcm) {
        return decode_pcm_mb(br, mb_x, mb_y, pic, *prev_qp);
    }

    // Read intra4x4 modes if I_NxN, else use a sentinel.
    let mut intra4x4_modes = [INTRA_DC_FAKE; 16];
    if matches!(imb, IMbType::INxN) {
        // Iterate in raster of 4×4 sub-blocks per spec coding order
        // (LUMA_BLOCK_RASTER walks the zig-zag scan of 4×4 blocks).
        for blk in 0..16usize {
            let (br_row, br_col) = LUMA_BLOCK_RASTER[blk];
            let prev_flag = br.read_flag()?;
            let predicted =
                predict_intra4x4_mode_with(pic, mb_x, mb_y, br_row, br_col, &intra4x4_modes);
            let mode = if prev_flag {
                predicted
            } else {
                let rem = br.read_u32(3)? as u8;
                if rem < predicted {
                    rem
                } else {
                    rem + 1
                }
            };
            intra4x4_modes[br_row * 4 + br_col] = mode;
        }
    }

    // Chroma pred mode (always present when chroma_format_idc != 0).
    let chroma_pred_mode = if sps.chroma_format_idc != 0 {
        let v = br.read_ue()?;
        if v > 3 {
            return Err(Error::invalid(format!(
                "h264 mb: intra_chroma_pred_mode {v} > 3"
            )));
        }
        IntraChromaMode::from_u8(v as u8).unwrap()
    } else {
        IntraChromaMode::Dc
    };

    // CBP: only for I_NxN. For I_16x16 the cbp comes baked into mb_type.
    let (cbp_luma, cbp_chroma) = match imb {
        IMbType::INxN => {
            let cbp_raw = br.read_ue()?;
            decode_cbp_intra(cbp_raw)
                .ok_or_else(|| Error::invalid(format!("h264 mb: bad CBP {cbp_raw}")))?
        }
        IMbType::I16x16 {
            cbp_luma,
            cbp_chroma,
            ..
        } => (cbp_luma, cbp_chroma),
        IMbType::IPcm => unreachable!(),
    };

    let needs_qp_delta =
        matches!(imb, IMbType::I16x16 { .. }) || (cbp_luma != 0 || cbp_chroma != 0);
    if needs_qp_delta {
        let dqp = br.read_se()?;
        // QP_Y wraps mod 52 with a small offset; spec equation §7.4.5.
        *prev_qp = ((*prev_qp + dqp + 52) % 52).clamp(0, 51);
    }
    let qp_y = *prev_qp;

    // Initialise MB info.
    {
        let info = pic.mb_info_mut(mb_x, mb_y);
        *info = MbInfo {
            qp_y,
            coded: true,
            intra: true,
            intra4x4_pred_mode: intra4x4_modes,
            ..Default::default()
        };
    }

    // Predict + residual + reconstruct luma.
    match imb {
        IMbType::I16x16 {
            intra16x16_pred_mode,
            ..
        } => decode_luma_intra_16x16(
            br,
            sps,
            pps,
            sh,
            mb_x,
            mb_y,
            pic,
            intra16x16_pred_mode,
            cbp_luma,
            qp_y,
        )?,
        IMbType::INxN => decode_luma_intra_nxn(
            br,
            sps,
            pps,
            sh,
            mb_x,
            mb_y,
            pic,
            &intra4x4_modes,
            cbp_luma,
            qp_y,
        )?,
        IMbType::IPcm => unreachable!(),
    }

    // Chroma reconstruction.
    decode_chroma(
        br,
        sps,
        pps,
        sh,
        mb_x,
        mb_y,
        pic,
        chroma_pred_mode,
        cbp_chroma,
        qp_y,
    )?;

    Ok(())
}

// -----------------------------------------------------------------------------
// I_PCM
// -----------------------------------------------------------------------------

fn decode_pcm_mb(
    br: &mut BitReader<'_>,
    mb_x: u32,
    mb_y: u32,
    pic: &mut Picture,
    prev_qp: i32,
) -> Result<()> {
    while !br.is_byte_aligned() {
        let _ = br.read_u1()?;
    }
    let lstride = pic.luma_stride();
    let lo = pic.luma_off(mb_x, mb_y);
    for r in 0..16 {
        for c in 0..16 {
            pic.y[lo + r * lstride + c] = br.read_u32(8)? as u8;
        }
    }
    let cstride = pic.chroma_stride();
    let co = pic.chroma_off(mb_x, mb_y);
    for r in 0..8 {
        for c in 0..8 {
            pic.cb[co + r * cstride + c] = br.read_u32(8)? as u8;
        }
    }
    for r in 0..8 {
        for c in 0..8 {
            pic.cr[co + r * cstride + c] = br.read_u32(8)? as u8;
        }
    }
    let info = pic.mb_info_mut(mb_x, mb_y);
    info.qp_y = prev_qp;
    info.coded = true;
    info.intra = true;
    info.luma_nc = [16; 16];
    info.cb_nc = [16; 4];
    info.cr_nc = [16; 4];
    info.intra4x4_pred_mode = [INTRA_DC_FAKE; 16];
    Ok(())
}

// -----------------------------------------------------------------------------
// Luma — I_NxN.
// -----------------------------------------------------------------------------

fn decode_luma_intra_nxn(
    br: &mut BitReader<'_>,
    _sps: &Sps,
    _pps: &Pps,
    _sh: &SliceHeader,
    mb_x: u32,
    mb_y: u32,
    pic: &mut Picture,
    modes: &[u8; 16],
    cbp_luma: u8,
    qp_y: i32,
) -> Result<()> {
    let lstride = pic.luma_stride();
    let lo_mb = pic.luma_off(mb_x, mb_y);

    for blk in 0..16usize {
        let (br_row, br_col) = LUMA_BLOCK_RASTER[blk];
        let mode_v = modes[br_row * 4 + br_col];
        let mode = Intra4x4Mode::from_u8(mode_v)
            .ok_or_else(|| Error::invalid(format!("h264 mb: invalid intra4x4 mode {mode_v}")))?;
        let neigh = collect_intra4x4_neighbours(pic, mb_x, mb_y, br_row, br_col);
        let mut pred = [0u8; 16];
        predict_intra_4x4(&mut pred, mode, &neigh);

        let cbp_bit_idx = (br_row / 2) * 2 + (br_col / 2);
        let has_residual = (cbp_luma >> cbp_bit_idx) & 1 != 0;

        let mut residual = [0i32; 16];
        let mut total_coeff = 0u32;
        if has_residual {
            let nc = predict_nc_luma(pic, mb_x, mb_y, br_row, br_col);
            let blk = decode_residual_block(br, nc, BlockKind::Luma4x4)?;
            total_coeff = blk.total_coeff;
            residual = blk.coeffs;
            dequantize_4x4(&mut residual, qp_y);
            idct_4x4(&mut residual);
        }
        let lo = lo_mb + br_row * 4 * lstride + br_col * 4;
        for r in 0..4 {
            for c in 0..4 {
                let v = pred[r * 4 + c] as i32 + residual[r * 4 + c];
                pic.y[lo + r * lstride + c] = v.clamp(0, 255) as u8;
            }
        }
        pic.mb_info_mut(mb_x, mb_y).luma_nc[br_row * 4 + br_col] = total_coeff as u8;
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Luma — I_16x16.
// -----------------------------------------------------------------------------

fn decode_luma_intra_16x16(
    br: &mut BitReader<'_>,
    _sps: &Sps,
    _pps: &Pps,
    _sh: &SliceHeader,
    mb_x: u32,
    mb_y: u32,
    pic: &mut Picture,
    intra16x16_pred_mode: u8,
    cbp_luma: u8,
    qp_y: i32,
) -> Result<()> {
    let neigh = collect_intra16x16_neighbours(pic, mb_x, mb_y);
    let mut pred = [0u8; 256];
    let mode = Intra16x16Mode::from_u8(intra16x16_pred_mode).ok_or_else(|| {
        Error::invalid(format!(
            "h264 mb: invalid intra16x16 mode {intra16x16_pred_mode}"
        ))
    })?;
    predict_intra_16x16(&mut pred, mode, &neigh);

    // DC luma block — always coded for I_16x16.
    let nc_dc = predict_nc_luma(pic, mb_x, mb_y, 0, 0);
    let dc_block = decode_residual_block(br, nc_dc, BlockKind::Luma16x16Dc)?;
    let mut dc = dc_block.coeffs;
    inv_hadamard_4x4_dc(&mut dc, qp_y);

    let lstride = pic.luma_stride();
    let lo_mb = pic.luma_off(mb_x, mb_y);
    for blk in 0..16usize {
        let (br_row, br_col) = LUMA_BLOCK_RASTER[blk];
        let mut residual = [0i32; 16];
        let mut total_coeff = 0u32;
        if cbp_luma != 0 {
            let nc = predict_nc_luma(pic, mb_x, mb_y, br_row, br_col);
            let ac = decode_residual_block(br, nc, BlockKind::Luma16x16Ac)?;
            total_coeff = ac.total_coeff;
            residual = ac.coeffs;
            dequantize_4x4(&mut residual, qp_y);
        }
        // Insert DC sample at position 0 (already dequantised by the
        // Hadamard pass — it lives in the same scaled space as the AC
        // coefficients post-dequant).
        residual[0] = dc[br_row * 4 + br_col];
        idct_4x4(&mut residual);

        let lo = lo_mb + br_row * 4 * lstride + br_col * 4;
        for r in 0..4 {
            for c in 0..4 {
                let v = pred[(br_row * 4 + r) * 16 + (br_col * 4 + c)] as i32 + residual[r * 4 + c];
                pic.y[lo + r * lstride + c] = v.clamp(0, 255) as u8;
            }
        }
        pic.mb_info_mut(mb_x, mb_y).luma_nc[br_row * 4 + br_col] = total_coeff as u8;
    }

    Ok(())
}

// -----------------------------------------------------------------------------
// Chroma reconstruction.
// -----------------------------------------------------------------------------

fn decode_chroma(
    br: &mut BitReader<'_>,
    _sps: &Sps,
    pps: &Pps,
    _sh: &SliceHeader,
    mb_x: u32,
    mb_y: u32,
    pic: &mut Picture,
    chroma_mode: IntraChromaMode,
    cbp_chroma: u8,
    qp_y: i32,
) -> Result<()> {
    let qpc = chroma_qp(qp_y, pps.chroma_qp_index_offset);

    // Predict both planes.
    let neigh_cb = collect_chroma_neighbours(pic, mb_x, mb_y, true);
    let neigh_cr = collect_chroma_neighbours(pic, mb_x, mb_y, false);
    let mut pred_cb = [0u8; 64];
    let mut pred_cr = [0u8; 64];
    predict_intra_chroma(&mut pred_cb, chroma_mode, &neigh_cb);
    predict_intra_chroma(&mut pred_cr, chroma_mode, &neigh_cr);

    // DC blocks — present when cbp_chroma >= 1.
    let mut dc_cb = [0i32; 4];
    let mut dc_cr = [0i32; 4];
    if cbp_chroma >= 1 {
        let blk = decode_residual_block(br, 0, BlockKind::ChromaDc2x2)?;
        for i in 0..4 {
            dc_cb[i] = blk.coeffs[i];
        }
        let blk = decode_residual_block(br, 0, BlockKind::ChromaDc2x2)?;
        for i in 0..4 {
            dc_cr[i] = blk.coeffs[i];
        }
        inv_hadamard_2x2_chroma_dc(&mut dc_cb, qpc);
        inv_hadamard_2x2_chroma_dc(&mut dc_cr, qpc);
    }

    let cstride = pic.chroma_stride();
    let co = pic.chroma_off(mb_x, mb_y);

    // Reconstruct AC blocks for Cb then Cr.
    for plane_kind in [true, false] {
        let pred = if plane_kind { &pred_cb } else { &pred_cr };
        let dc = if plane_kind { &dc_cb } else { &dc_cr };
        let mut nc_arr = [0u8; 4];
        for blk_idx in 0..4u8 {
            let br_row = (blk_idx >> 1) as usize;
            let br_col = (blk_idx & 1) as usize;
            let mut res = [0i32; 16];
            let mut total_coeff = 0u32;
            if cbp_chroma == 2 {
                let nc = predict_nc_chroma(pic, mb_x, mb_y, plane_kind, br_row, br_col);
                let ac = decode_residual_block(br, nc, BlockKind::ChromaAc)?;
                total_coeff = ac.total_coeff;
                res = ac.coeffs;
                dequantize_4x4(&mut res, qpc);
            }
            res[0] = dc[(br_row << 1) | br_col];
            idct_4x4(&mut res);
            let off_in_mb = br_row * 4 * cstride + br_col * 4;
            let plane = if plane_kind { &mut pic.cb } else { &mut pic.cr };
            for r in 0..4 {
                for c in 0..4 {
                    let v = pred[(br_row * 4 + r) * 8 + (br_col * 4 + c)] as i32 + res[r * 4 + c];
                    plane[co + off_in_mb + r * cstride + c] = v.clamp(0, 255) as u8;
                }
            }
            nc_arr[(br_row << 1) | br_col] = total_coeff as u8;
        }
        let info = pic.mb_info_mut(mb_x, mb_y);
        if plane_kind {
            info.cb_nc = nc_arr;
        } else {
            info.cr_nc = nc_arr;
        }
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Predicted nC (§9.2.1.1).
// -----------------------------------------------------------------------------

fn predict_nc_luma(pic: &Picture, mb_x: u32, mb_y: u32, br_row: usize, br_col: usize) -> i32 {
    let info_here = pic.mb_info_at(mb_x, mb_y);
    let left = if br_col > 0 {
        Some(info_here.luma_nc[br_row * 4 + br_col - 1])
    } else if mb_x > 0 {
        let info = pic.mb_info_at(mb_x - 1, mb_y);
        if info.coded {
            Some(info.luma_nc[br_row * 4 + 3])
        } else {
            None
        }
    } else {
        None
    };
    let top = if br_row > 0 {
        Some(info_here.luma_nc[(br_row - 1) * 4 + br_col])
    } else if mb_y > 0 {
        let info = pic.mb_info_at(mb_x, mb_y - 1);
        if info.coded {
            Some(info.luma_nc[12 + br_col])
        } else {
            None
        }
    } else {
        None
    };
    nc_from_neighbours(left, top)
}

fn predict_nc_chroma(
    pic: &Picture,
    mb_x: u32,
    mb_y: u32,
    cb: bool,
    br_row: usize,
    br_col: usize,
) -> i32 {
    let pick = |info: &MbInfo, sub: usize| -> u8 {
        if cb {
            info.cb_nc[sub]
        } else {
            info.cr_nc[sub]
        }
    };
    let info_here = pic.mb_info_at(mb_x, mb_y);
    let left = if br_col > 0 {
        Some(pick(info_here, br_row * 2 + br_col - 1))
    } else if mb_x > 0 {
        let info = pic.mb_info_at(mb_x - 1, mb_y);
        if info.coded {
            Some(pick(info, br_row * 2 + 1))
        } else {
            None
        }
    } else {
        None
    };
    let top = if br_row > 0 {
        Some(pick(info_here, (br_row - 1) * 2 + br_col))
    } else if mb_y > 0 {
        let info = pic.mb_info_at(mb_x, mb_y - 1);
        if info.coded {
            Some(pick(info, 2 + br_col))
        } else {
            None
        }
    } else {
        None
    };
    nc_from_neighbours(left, top)
}

fn nc_from_neighbours(left: Option<u8>, top: Option<u8>) -> i32 {
    match (left, top) {
        (Some(l), Some(t)) => (l as i32 + t as i32 + 1) >> 1,
        (Some(l), None) => l as i32,
        (None, Some(t)) => t as i32,
        (None, None) => 0,
    }
}

// -----------------------------------------------------------------------------
// Intra 4×4 mode prediction.
// -----------------------------------------------------------------------------

fn predict_intra4x4_mode_with(
    pic: &Picture,
    mb_x: u32,
    mb_y: u32,
    br_row: usize,
    br_col: usize,
    in_progress_modes: &[u8; 16],
) -> u8 {
    // Within this MB we use in_progress_modes[] for previously coded blocks
    // (raster of 4×4 blocks in the spec coding order corresponds to
    // LUMA_BLOCK_RASTER above). For the left/top neighbour outside the MB we
    // inspect the picture's stored intra4x4 modes.
    let here_get = |row: usize, col: usize| in_progress_modes[row * 4 + col];

    let left_mode = if br_col > 0 {
        Some(here_get(br_row, br_col - 1))
    } else if mb_x > 0 {
        let li = pic.mb_info_at(mb_x - 1, mb_y);
        if li.coded && li.intra {
            Some(li.intra4x4_pred_mode[br_row * 4 + 3])
        } else {
            None
        }
    } else {
        None
    };
    let top_mode = if br_row > 0 {
        Some(here_get(br_row - 1, br_col))
    } else if mb_y > 0 {
        let ti = pic.mb_info_at(mb_x, mb_y - 1);
        if ti.coded && ti.intra {
            Some(ti.intra4x4_pred_mode[12 + br_col])
        } else {
            None
        }
    } else {
        None
    };
    match (left_mode, top_mode) {
        (Some(l), Some(t)) => l.min(t),
        _ => INTRA_DC_FAKE,
    }
}

// -----------------------------------------------------------------------------
// Neighbour collection.
// -----------------------------------------------------------------------------

fn collect_intra4x4_neighbours(
    pic: &Picture,
    mb_x: u32,
    mb_y: u32,
    br_row: usize,
    br_col: usize,
) -> Intra4x4Neighbours {
    let lstride = pic.luma_stride();
    let _ = pic.luma_off(mb_x, mb_y);

    let top_avail = br_row > 0 || mb_y > 0;
    let mut top = [0u8; 8];
    if top_avail {
        let row_y_global = if br_row > 0 {
            (mb_y as usize) * 16 + br_row * 4 - 1
        } else {
            (mb_y as usize) * 16 - 1
        };
        let row_off = row_y_global * lstride;
        for i in 0..4 {
            top[i] = pic.y[row_off + (mb_x as usize) * 16 + br_col * 4 + i];
        }
        let tr_avail = top_right_available_4x4(mb_x, mb_y, br_row, br_col, pic);
        if tr_avail {
            for i in 0..4 {
                top[4 + i] = pic.y[row_off + (mb_x as usize) * 16 + br_col * 4 + 4 + i];
            }
        } else {
            for i in 0..4 {
                top[4 + i] = top[3];
            }
        }
    }

    let left_avail = br_col > 0 || mb_x > 0;
    let mut left = [0u8; 4];
    if left_avail {
        let col_x_global: usize = if br_col > 0 {
            (mb_x as usize) * 16 + br_col * 4 - 1
        } else {
            (mb_x as usize) * 16 - 1
        };
        for i in 0..4 {
            let row = (mb_y as usize) * 16 + br_row * 4 + i;
            left[i] = pic.y[row * lstride + col_x_global];
        }
    }

    let tl_avail = top_avail && left_avail;
    let top_left = if tl_avail {
        let row_y_global: usize = if br_row > 0 {
            (mb_y as usize) * 16 + br_row * 4 - 1
        } else {
            (mb_y as usize) * 16 - 1
        };
        let col_x_global: usize = if br_col > 0 {
            (mb_x as usize) * 16 + br_col * 4 - 1
        } else {
            (mb_x as usize) * 16 - 1
        };
        pic.y[row_y_global * lstride + col_x_global]
    } else {
        0
    };

    Intra4x4Neighbours {
        top,
        left,
        top_left,
        top_available: top_avail,
        left_available: left_avail,
        top_left_available: tl_avail,
        top_right_available: false,
    }
}

/// Whether the 4 samples just above-right of this 4×4 block exist.
fn top_right_available_4x4(
    mb_x: u32,
    mb_y: u32,
    br_row: usize,
    br_col: usize,
    pic: &Picture,
) -> bool {
    // Per Figure 8-7 / §6.4.10:
    // - blocks where the upper-right 4×4 is *inside* the same MB and has
    //   already been decoded (i.e. its raster index < current's raster index)
    //   → available
    // - block (br_row=0, br_col=3): upper-right is in next MB to the right at
    //   block (3,0); available only if that MB sits in the row above.
    // - blocks at br_col == 3 with br_row > 0 → upper-right is in the next MB
    //   row, not yet decoded → unavailable.
    if br_col == 3 {
        if br_row == 0 {
            mb_x + 1 < pic.mb_width && mb_y > 0 && pic.mb_info_at(mb_x + 1, mb_y - 1).coded
        } else {
            false
        }
    } else {
        // Inside-the-MB cases.
        // Available: (0,0), (0,1), (0,2), (1,0), (1,1), (1,2), (2,0), (2,1), (2,2),
        //            (3,0), (3,1), (3,2). i.e. all (br_col != 3).
        // BUT — Figure 8-7 also marks blocks 3 (raster idx 3 = (1,1)),
        //   7 ((1,3)), 11 ((3,1)), 13 ((3,2)) as "n/a" for top-right.
        // Use the explicit table from the spec.
        const AVAIL: [[bool; 4]; 4] = [
            [true, true, true, false],
            [true, false, true, false],
            [true, true, true, false],
            [true, false, true, false],
        ];
        AVAIL[br_row][br_col]
    }
}

fn collect_intra16x16_neighbours(pic: &Picture, mb_x: u32, mb_y: u32) -> Intra16x16Neighbours {
    let lstride = pic.luma_stride();
    let lo_mb = pic.luma_off(mb_x, mb_y);
    let top_avail = mb_y > 0;
    let mut top = [0u8; 16];
    if top_avail {
        let off = lo_mb - lstride;
        for i in 0..16 {
            top[i] = pic.y[off + i];
        }
    }
    let left_avail = mb_x > 0;
    let mut left = [0u8; 16];
    if left_avail {
        for i in 0..16 {
            left[i] = pic.y[lo_mb + i * lstride - 1];
        }
    }
    let tl_avail = top_avail && left_avail;
    let top_left = if tl_avail {
        pic.y[lo_mb - lstride - 1]
    } else {
        0
    };
    Intra16x16Neighbours {
        top,
        left,
        top_left,
        top_available: top_avail,
        left_available: left_avail,
        top_left_available: tl_avail,
    }
}

fn collect_chroma_neighbours(
    pic: &Picture,
    mb_x: u32,
    mb_y: u32,
    cb: bool,
) -> IntraChromaNeighbours {
    let cstride = pic.chroma_stride();
    let co_mb = pic.chroma_off(mb_x, mb_y);
    let plane = if cb { &pic.cb } else { &pic.cr };
    let top_avail = mb_y > 0;
    let mut top = [0u8; 8];
    if top_avail {
        let off = co_mb - cstride;
        for i in 0..8 {
            top[i] = plane[off + i];
        }
    }
    let left_avail = mb_x > 0;
    let mut left = [0u8; 8];
    if left_avail {
        for i in 0..8 {
            left[i] = plane[co_mb + i * cstride - 1];
        }
    }
    let tl_avail = top_avail && left_avail;
    let top_left = if tl_avail {
        plane[co_mb - cstride - 1]
    } else {
        0
    };
    IntraChromaNeighbours {
        top,
        left,
        top_left,
        top_available: top_avail,
        left_available: left_avail,
        top_left_available: tl_avail,
    }
}
