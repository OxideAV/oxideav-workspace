//! H.263 macroblock-level decoding for I-pictures.
//!
//! Per-MB decode sequence (§5.3):
//! 1. **MCBPC** (Table 14/H.263) — picks `(mb_type, cbpc)`.
//!    `mb_type = 3` (Intra) or `4` (IntraQ); `cbpc` is the 2-bit chroma CBP.
//! 2. **CBPY** (Table 13/H.263, intra variant) — 4-bit luma CBP. We treat
//!    the decoded value directly as the bit-pattern of "block has AC
//!    coefficients" flags (matching `oxideav-mpeg4video`'s I-VOP convention,
//!    cross-checked against the `h263-rs` table).
//! 3. **DQUANT** — 2 signed bits, present iff `mb_type == IntraQ`. Adjusts
//!    QUANT by `[-1, -2, 1, 2]` for codes `0..=3`.
//! 4. **Per-block** (Y0..Y3, Cb, Cr): 8-bit INTRADC + optional TCOEF.

use oxideav_core::{Error, Result};
use oxideav_mpeg4video::bitreader::BitReader;
use oxideav_mpeg4video::tables::{cbpy, mcbpc, vlc};

use crate::block::{decode_ac, decode_intradc, idct_and_clip};

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
            plane[(py + dy) * stride + (px + dx)] = out[dy * 8 + dx];
        }
    }
}
