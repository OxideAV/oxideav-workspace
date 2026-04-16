//! H.263 macroblock-level decoding for I- and P-pictures.
//!
//! For an I-picture, per-MB decode sequence (§5.3):
//! 1. **MCBPC** (Table 14/H.263) — picks `(mb_type, cbpc)`.
//!    `mb_type = 3` (Intra) or `4` (IntraQ); `cbpc` is the 2-bit chroma CBP.
//! 2. **CBPY** (Table 13/H.263, intra variant) — 4-bit luma CBP. We treat
//!    the decoded value directly as the bit-pattern of "block has AC
//!    coefficients" flags (matching `oxideav-mpeg4video`'s I-VOP convention,
//!    cross-checked against the `h263-rs` table).
//! 3. **DQUANT** — 2 signed bits, present iff `mb_type == IntraQ`. Adjusts
//!    QUANT by `[-1, -2, 1, 2]` for codes `0..=3`.
//! 4. **Per-block** (Y0..Y3, Cb, Cr): 8-bit INTRADC + optional TCOEF.
//!
//! For a P-picture, per-MB decode sequence (§5.3.1 / §5.3.5):
//! 1. **COD** — 1 bit. `1` means "not coded" → the MB is copied verbatim from
//!    the reference at the same position with MV(0,0).
//! 2. **MCBPC** (Table 16/H.263 inter) — picks `(mb_type, cbpc)`. We accept
//!    `Inter`, `InterQ`, `Intra`, `IntraQ`; `Inter4MV` / `Inter4MV+Q` are
//!    rejected (Annex F advanced prediction — out of scope).
//! 3. **CBPY** — for inter, bit-inverted of Table 13; for intra embedded in
//!    P, the raw (non-inverted) pattern.
//! 4. **DQUANT** — only if mb_type is `*Q`.
//! 5. **MV** — 2 half-pel components via `motion::decode_mv_component` using
//!    the median predictor over decoded neighbours.
//! 6. **Per-block texture**:
//!    * Inter: TCOEF at scan index 0 (no INTRADC), dequantise → IDCT →
//!      residual; add to the half-pel motion-compensated predictor then clip.
//!    * Intra-in-P: same path as I-pictures (INTRADC + AC).

use oxideav_core::{Error, Result};
use oxideav_mpeg4video::bitreader::BitReader;
use oxideav_mpeg4video::tables::{cbpy, mcbpc, vlc};

use crate::block::{decode_ac, decode_intradc, idct_and_clip};
use crate::interp::predict_block;
use crate::motion::{decode_mv_component, luma_to_chroma_mv, predict_mv, MbMotion, MvGrid};

/// Signed `dquant` adjustment — Table 12/H.263. 2-bit code indexes `[-1, -2, 1, 2]`.
const DQUANT_DELTA: [i32; 4] = [-1, -2, 1, 2];

/// Reconstructed I-picture: three pel planes (Y, Cb, Cr), MB-aligned, stride
/// equal to MB-aligned width.
pub struct IPicture {
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

impl IPicture {
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

/// Decode one I-picture intra macroblock. Returns the (possibly updated)
/// quantiser.
pub fn decode_intra_mb(
    br: &mut BitReader<'_>,
    mb_x: usize,
    mb_y: usize,
    quant_in: u32,
    pic: &mut IPicture,
) -> Result<u32> {
    // 1. MCBPC — loop over stuffing.
    let mcbpc_v = loop {
        let v = vlc::decode(br, mcbpc::i_table())?;
        if v != mcbpc::STUFFING {
            break v;
        }
    };
    let (is_intra_q, cbpc) = if mcbpc_v < 4 {
        (false, mcbpc_v)
    } else if mcbpc_v < 8 {
        (true, mcbpc_v - 4)
    } else {
        return Err(Error::invalid("h263 MB: invalid MCBPC value"));
    };

    // 2. CBPY (intra variant — direct, no XOR).
    let cbpy = vlc::decode(br, cbpy::table())?;

    // 3. DQUANT.
    let mut quant = quant_in;
    if is_intra_q {
        let d = br.read_u32(2)? as usize;
        let new_q = (quant as i32) + DQUANT_DELTA[d];
        quant = new_q.clamp(1, 31) as u32;
    }

    // 4. Per-block decode.
    // CBPY bit 3 (MSB) -> block 0, bit 0 (LSB) -> block 3 (per spec ordering).
    let luma_coded = [
        (cbpy >> 3) & 1 != 0,
        (cbpy >> 2) & 1 != 0,
        (cbpy >> 1) & 1 != 0,
        cbpy & 1 != 0,
    ];
    let chroma_coded = [(cbpc >> 1) & 1 != 0, cbpc & 1 != 0];

    for block_idx in 0..6usize {
        let coded = if block_idx < 4 {
            luma_coded[block_idx]
        } else {
            chroma_coded[block_idx - 4]
        };
        decode_one_intra_block(br, block_idx, coded, mb_x, mb_y, quant, pic)?;
    }

    Ok(quant)
}

fn decode_one_intra_block(
    br: &mut BitReader<'_>,
    block_idx: usize,
    has_ac: bool,
    mb_x: usize,
    mb_y: usize,
    quant: u32,
    pic: &mut IPicture,
) -> Result<()> {
    // INTRADC always present for intra blocks.
    let dc = decode_intradc(br)?;
    let mut coeffs = [0i32; 64];
    coeffs[0] = dc;

    if has_ac {
        decode_ac(br, &mut coeffs, 1, quant)?;
    }

    // Saturate the DC coefficient to spec range.
    coeffs[0] = coeffs[0].clamp(-2048, 2047);

    // IDCT + clip.
    let mut out = [0u8; 64];
    idct_and_clip(&mut coeffs, &mut out);

    write_block_to_picture(pic, block_idx, mb_x, mb_y, &out);
    Ok(())
}

/// Write the 8×8 reconstructed block into the picture buffer.
fn write_block_to_picture(
    pic: &mut IPicture,
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
    out: &[u8; 64],
) {
    let (plane, stride, px, py) = block_dst(pic, block_idx, mb_x, mb_y);
    for dy in 0..8 {
        for dx in 0..8 {
            plane[(py + dy) * stride + (px + dx)] = out[dy * 8 + dx];
        }
    }
}

/// Block-layout helper: return the plane slice + stride + top-left pel for
/// block `block_idx` (0..=5) of MB `(mb_x, mb_y)`.
fn block_dst(
    pic: &mut IPicture,
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
) -> (&mut [u8], usize, usize, usize) {
    match block_idx {
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
    }
}

/// Decode one P-picture macroblock at `(mb_x, mb_y)`. Reads from `br`,
/// writes decoded pels into `pic`, consults `reference` for motion
/// compensation, and updates `mv_grid` so subsequent MBs see the correct
/// median predictor.
///
/// Returns the (possibly updated) quantiser.
pub fn decode_p_mb(
    br: &mut BitReader<'_>,
    mb_x: usize,
    mb_y: usize,
    quant_in: u32,
    pic: &mut IPicture,
    reference: &IPicture,
    mv_grid: &mut MvGrid,
) -> Result<u32> {
    // 1. COD flag (§5.3.1). 1 → not_coded: copy from reference, MV(0,0).
    let cod = br.read_u1()?;
    if cod == 1 {
        copy_skipped_mb(pic, reference, mb_x, mb_y);
        mv_grid.set(
            mb_x,
            mb_y,
            MbMotion {
                mv: (0, 0),
                coded: false,
                intra: false,
            },
        );
        return Ok(quant_in);
    }

    // 2. MCBPC inter — Table 16/H.263 (identical to MPEG-4 Table B-13).
    let mcbpc_v = loop {
        let v = vlc::decode(br, mcbpc::p_table())?;
        if v != mcbpc::INTER_STUFFING {
            break v;
        }
    };
    let (mb_type, cbpc) = mcbpc::decompose_inter(mcbpc_v);
    use mcbpc::PMbType;
    if matches!(mb_type, PMbType::Inter4MV | PMbType::Inter4MVQ) {
        return Err(Error::unsupported(
            "h263 P-MB: Inter4MV (Annex F advanced prediction): follow-up",
        ));
    }
    let is_intra = matches!(mb_type, PMbType::Intra | PMbType::IntraQ);
    let needs_dquant = matches!(mb_type, PMbType::InterQ | PMbType::IntraQ);

    // 3. CBPY. Inter MBs invert the 4-bit pattern (§5.3.3) — `0` in the
    //    decoded VLC corresponds to "AC coded" for inter. Intra (embedded in
    //    P) uses the raw pattern.
    let cbpy_raw = vlc::decode(br, cbpy::table())?;
    let cbpy = if is_intra { cbpy_raw } else { cbpy_raw ^ 0xF };

    // 4. DQUANT — 2 signed bits, if mb_type is `*Q`.
    let mut quant = quant_in;
    if needs_dquant {
        const DQUANT_DELTA: [i32; 4] = [-1, -2, 1, 2];
        let d = br.read_u32(2)? as usize;
        let new_q = (quant as i32) + DQUANT_DELTA[d];
        quant = new_q.clamp(1, 31) as u32;
    }

    // 5. Motion vector (only for inter MBs). Intra MBs carry MV(0,0) as a
    //    neighbour predictor and their texture is decoded via the intra
    //    path.
    let mv_half: (i32, i32) = if is_intra {
        mv_grid.set(
            mb_x,
            mb_y,
            MbMotion {
                mv: (0, 0),
                coded: true,
                intra: true,
            },
        );
        (0, 0)
    } else {
        let (px, py) = predict_mv(mv_grid, mb_x, mb_y);
        let mvx = decode_mv_component(br, px)?;
        let mvy = decode_mv_component(br, py)?;
        mv_grid.set(
            mb_x,
            mb_y,
            MbMotion {
                mv: (mvx, mvy),
                coded: true,
                intra: false,
            },
        );
        (mvx, mvy)
    };

    // 6. Per-block texture.
    let luma_coded = [
        (cbpy >> 3) & 1 != 0,
        (cbpy >> 2) & 1 != 0,
        (cbpy >> 1) & 1 != 0,
        cbpy & 1 != 0,
    ];
    let chroma_coded = [(cbpc >> 1) & 1 != 0, cbpc & 1 != 0];

    if is_intra {
        for block_idx in 0..6usize {
            let coded = if block_idx < 4 {
                luma_coded[block_idx]
            } else {
                chroma_coded[block_idx - 4]
            };
            decode_one_intra_block_in_p(br, block_idx, coded, mb_x, mb_y, quant, pic)?;
        }
        return Ok(quant);
    }

    // Inter path: predict from reference with MV, add AC residual.
    decode_inter_mb_texture(
        br,
        mb_x,
        mb_y,
        quant,
        pic,
        reference,
        mv_half,
        &luma_coded,
        &chroma_coded,
    )?;
    Ok(quant)
}

/// Decode the 6 blocks of an inter MB: MC predictor + (optional) residual for
/// each. `luma_coded` / `chroma_coded` are the CBP bits after the inter XOR.
#[allow(clippy::too_many_arguments)]
fn decode_inter_mb_texture(
    br: &mut BitReader<'_>,
    mb_x: usize,
    mb_y: usize,
    quant: u32,
    pic: &mut IPicture,
    reference: &IPicture,
    mv_half: (i32, i32),
    luma_coded: &[bool; 4],
    chroma_coded: &[bool; 2],
) -> Result<()> {
    let ref_y_h = reference.y.len() / reference.y_stride;
    let ref_c_h = reference.cb.len() / reference.c_stride;
    let (mvx, mvy) = mv_half;

    for block_idx in 0..4usize {
        let (sub_x, sub_y) = match block_idx {
            0 => (0, 0),
            1 => (8, 0),
            2 => (0, 8),
            3 => (8, 8),
            _ => unreachable!(),
        };
        let blk_px = (mb_x * 16 + sub_x) as i32;
        let blk_py = (mb_y * 16 + sub_y) as i32;
        let mut pred = [0u8; 64];
        predict_block(
            &reference.y,
            reference.y_stride,
            reference.y_stride as i32,
            ref_y_h as i32,
            blk_px,
            blk_py,
            mvx,
            mvy,
            8,
            &mut pred,
            8,
        );

        if luma_coded[block_idx] {
            let mut coeffs = [0i32; 64];
            decode_ac(br, &mut coeffs, 0, quant)?;
            let mut resid = [0i32; 64];
            crate::block::idct_signed(&mut coeffs, &mut resid);
            let (plane, stride, px, py) = block_dst(pic, block_idx, mb_x, mb_y);
            for j in 0..8 {
                for i in 0..8 {
                    let s = pred[j * 8 + i] as i32 + resid[j * 8 + i];
                    plane[(py + j) * stride + (px + i)] = s.clamp(0, 255) as u8;
                }
            }
        } else {
            let (plane, stride, px, py) = block_dst(pic, block_idx, mb_x, mb_y);
            for j in 0..8 {
                for i in 0..8 {
                    plane[(py + j) * stride + (px + i)] = pred[j * 8 + i];
                }
            }
        }
    }

    // Chroma: single MV scaled to chroma grid.
    let cmx = luma_to_chroma_mv(mvx);
    let cmy = luma_to_chroma_mv(mvy);
    for (plane_idx, block_idx) in (4..6usize).enumerate() {
        let blk_px = (mb_x * 8) as i32;
        let blk_py = (mb_y * 8) as i32;
        let mut pred = [0u8; 64];
        let (ref_plane, ref_stride) = if plane_idx == 0 {
            (&reference.cb, reference.c_stride)
        } else {
            (&reference.cr, reference.c_stride)
        };
        predict_block(
            ref_plane,
            ref_stride,
            ref_stride as i32,
            ref_c_h as i32,
            blk_px,
            blk_py,
            cmx,
            cmy,
            8,
            &mut pred,
            8,
        );
        if chroma_coded[plane_idx] {
            let mut coeffs = [0i32; 64];
            decode_ac(br, &mut coeffs, 0, quant)?;
            let mut resid = [0i32; 64];
            crate::block::idct_signed(&mut coeffs, &mut resid);
            let (plane, stride, px, py) = block_dst(pic, block_idx, mb_x, mb_y);
            for j in 0..8 {
                for i in 0..8 {
                    let s = pred[j * 8 + i] as i32 + resid[j * 8 + i];
                    plane[(py + j) * stride + (px + i)] = s.clamp(0, 255) as u8;
                }
            }
        } else {
            let (plane, stride, px, py) = block_dst(pic, block_idx, mb_x, mb_y);
            for j in 0..8 {
                for i in 0..8 {
                    plane[(py + j) * stride + (px + i)] = pred[j * 8 + i];
                }
            }
        }
    }
    Ok(())
}

/// Intra block decode when the MB is an embedded intra inside a P-picture.
/// Identical to `decode_one_intra_block` (I-path) — factored as a separate
/// function so the intra-in-P caller doesn't depend on the I-path's private
/// helper name.
fn decode_one_intra_block_in_p(
    br: &mut BitReader<'_>,
    block_idx: usize,
    has_ac: bool,
    mb_x: usize,
    mb_y: usize,
    quant: u32,
    pic: &mut IPicture,
) -> Result<()> {
    let dc = decode_intradc(br)?;
    let mut coeffs = [0i32; 64];
    coeffs[0] = dc;

    if has_ac {
        decode_ac(br, &mut coeffs, 1, quant)?;
    }
    coeffs[0] = coeffs[0].clamp(-2048, 2047);

    let mut out = [0u8; 64];
    idct_and_clip(&mut coeffs, &mut out);

    write_block_to_picture(pic, block_idx, mb_x, mb_y, &out);
    Ok(())
}

/// Copy a skipped MB from the reference frame verbatim. MV(0,0).
fn copy_skipped_mb(pic: &mut IPicture, reference: &IPicture, mb_x: usize, mb_y: usize) {
    let py = mb_y * 16;
    let px = mb_x * 16;
    for j in 0..16 {
        let dst_off = (py + j) * pic.y_stride + px;
        let src_off = (py + j) * reference.y_stride + px;
        pic.y[dst_off..dst_off + 16].copy_from_slice(&reference.y[src_off..src_off + 16]);
    }
    let cy = mb_y * 8;
    let cx = mb_x * 8;
    for j in 0..8 {
        let dst_off = (cy + j) * pic.c_stride + cx;
        let src_off = (cy + j) * reference.c_stride + cx;
        pic.cb[dst_off..dst_off + 8].copy_from_slice(&reference.cb[src_off..src_off + 8]);
        pic.cr[dst_off..dst_off + 8].copy_from_slice(&reference.cr[src_off..src_off + 8]);
    }
}
