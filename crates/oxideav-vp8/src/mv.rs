//! Motion-vector decoding — RFC 6386 §16 + §17.
//!
//! A VP8 motion vector is a signed pair `(row, col)` in 1/8-pel units.
//! Each component is either "short" (magnitude 0..=7) or "long"
//! (magnitude 8..=1023) and is coded using a dedicated set of 19
//! per-component probabilities (`MvContext`).

use crate::bool_decoder::BoolDecoder;
use crate::tables::mv::{MvContext, MV_SHORT_TREE};
use crate::tables::trees::decode_tree;

/// Signed MV component in 1/8-pel units.
pub type MvComponent = i16;

/// A 2D motion vector, row then column, in 1/8-pel units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mv {
    pub row: MvComponent,
    pub col: MvComponent,
}

impl Mv {
    pub const ZERO: Self = Self { row: 0, col: 0 };

    pub fn new(row: i32, col: i32) -> Self {
        Self {
            row: row.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            col: col.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        }
    }
}

/// Decode a single MV component. Returns a signed integer in 1/8-pel
/// units.
pub fn decode_mv_component(d: &mut BoolDecoder<'_>, probs: &MvContext) -> i32 {
    // Probability at index 0 indicates "is large" (long). When true we take
    // the long path, otherwise use the 3-bit short tree.
    let large = d.read_bool(probs[0] as u32);
    let mut mag = if large {
        // Bits 0..=9 individually except bit 3 is deferred to the end.
        let mut v = 0i32;
        for i in 0..3 {
            v += (d.read_bool(probs[9 + i] as u32) as i32) << i;
        }
        for i in (4..=9).rev() {
            v += (d.read_bool(probs[9 + i] as u32) as i32) << i;
        }
        // Bit 3 (LSB-4) — only relevant if at least one of bits 4..9 is
        // non-zero; otherwise implicit. See RFC 6386 §17.1 pseudo-code.
        if (v & 0xfff0) == 0 || d.read_bool(probs[9 + 3] as u32) {
            v += 8;
        }
        v
    } else {
        // 3-bit short magnitude using bits at indices 2..=8.
        decode_tree(d, &MV_SHORT_TREE, &probs[2..9])
    };
    if mag != 0 && d.read_bool(probs[1] as u32) {
        mag = -mag;
    }
    mag
}

/// Decode a full MV given a pair of component probability contexts
/// (row-first then col — matching libvpx's layout of the default context).
pub fn decode_mv(d: &mut BoolDecoder<'_>, mv_probs: &[MvContext; 2]) -> Mv {
    let row = decode_mv_component(d, &mv_probs[0]);
    let col = decode_mv_component(d, &mv_probs[1]);
    Mv::new(row, col)
}
