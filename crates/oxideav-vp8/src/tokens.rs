//! Token (residual coefficient) decoding — RFC 6386 §13.
//!
//! The flat decoder structure mirrors libvpx's `GetCoeffs()` exactly.
//! Each call reads probabilities `p[0..10]` directly rather than walking
//! the canonical tree — every branch in the tree maps to a fixed `p[i]`
//! lookup, so the unrolled form is both faster and unambiguous about
//! the EOB-skip-after-zero special case.

use crate::bool_decoder::BoolDecoder;
use crate::tables::coeff_probs::CoeffProbs;
use crate::tables::token_tree::{COEF_BANDS, ZIGZAG};

/// Block category (RFC §13.2). Maps to plane type for probability lookup.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    /// Block type 0 — Y after Y2.
    YAfterY2 = 0,
    /// Block type 1 — Y2.
    Y2 = 1,
    /// Block type 2 — UV.
    UV = 2,
    /// Block type 3 — Y when no Y2 (B_PRED MBs).
    YNoY2 = 3,
}

const PCAT3: [u8; 4] = [173, 148, 140, 0];
const PCAT4: [u8; 5] = [176, 155, 140, 135, 0];
const PCAT5: [u8; 6] = [180, 157, 141, 134, 130, 0];
const PCAT6: [u8; 12] = [254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129, 0];
const PCAT3456: [&[u8]; 4] = [&PCAT3, &PCAT4, &PCAT5, &PCAT6];

/// Decode the coefficients of one transform block. Returns the number
/// of coefficients decoded (i.e. position of last non-zero + 1, or 0 if
/// EOB at the very start).
pub fn decode_block(
    d: &mut BoolDecoder<'_>,
    probs: &CoeffProbs,
    block_type: BlockType,
    nctx: u8,
    coeffs: &mut [i16; 16],
    start: usize,
) -> u8 {
    coeffs.iter_mut().for_each(|c| *c = 0);
    let plane = block_type as usize;
    let plane_probs = &probs[plane];
    let mut n = start;
    let mut ctx = nctx as usize;
    // `p` is the current 11-entry prob array.
    let mut p: &[u8; 11] = &plane_probs[COEF_BANDS[n]][ctx];
    // First read: p[0] is the EOB / CBP bit. If 0, block is empty.
    if !d.read_bool(p[0] as u32) {
        return 0;
    }
    loop {
        // Advance position; if we're at end-of-block, this read decides
        // whether a coefficient is present (NOT EOB).
        loop {
            // Read DCT_0 vs non-zero.
            if !d.read_bool(p[1] as u32) {
                // Zero coefficient. Switch band, ctx = 0.
                n += 1;
                if n == 16 {
                    return 16;
                }
                p = &plane_probs[COEF_BANDS[n]][0];
            } else {
                break;
            }
        }
        // Non-zero coefficient. Decode magnitude.
        let v = if !d.read_bool(p[2] as u32) {
            // DCT_1
            ctx = 1;
            1i32
        } else {
            ctx = 2;
            if !d.read_bool(p[3] as u32) {
                if !d.read_bool(p[4] as u32) {
                    2
                } else {
                    3 + d.read_bool(p[5] as u32) as i32
                }
            } else if !d.read_bool(p[6] as u32) {
                if !d.read_bool(p[7] as u32) {
                    5 + d.read_bool(159) as i32
                } else {
                    let mut v = 7 + 2 * d.read_bool(165) as i32;
                    v += d.read_bool(145) as i32;
                    v
                }
            } else {
                // CAT3..CAT6
                let bit1 = d.read_bool(p[8] as u32) as usize;
                let bit0 = d.read_bool(p[9 + bit1] as u32) as usize;
                let cat = 2 * bit1 + bit0;
                let mut v = 0i32;
                let tab = PCAT3456[cat];
                let mut i = 0;
                while i < tab.len() && tab[i] != 0 {
                    v = v + v + d.read_bool(tab[i] as u32) as i32;
                    i += 1;
                }
                v + 3 + (8 << cat)
            }
        };
        // Apply sign.
        let signed = if d.read_bool(128) { -v } else { v };
        coeffs[ZIGZAG[n]] = signed as i16;
        n += 1;
        if n == 16 {
            return 16;
        }
        // Switch context for the next prob lookup; check end-of-block EOB.
        p = &plane_probs[COEF_BANDS[n]][ctx];
        if !d.read_bool(p[0] as u32) {
            return n as u8;
        }
    }
}
