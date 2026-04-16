//! Lookup tables that don't fit elsewhere — CBP mapping (Table 9-4) and
//! Intra4x4 mode neighbour tables.
//!
//! References: ITU-T H.264 §7.4.5, §9.1.2.

/// Table 9-4(b) — for I-slices, `coded_block_pattern` is a `me(v)` (mapped
/// Exp-Golomb) syntax element decoded as `ue(v)` then mapped through this
/// table. Index = ue value, Value = (cbp_luma | cbp_chroma << 4).
///
/// `cbp_luma` is the 4-bit pattern for the four 8×8 luma sub-blocks
/// (bits 0..=3); `cbp_chroma` is one of:
///   0 = all chroma AC + DC zero
///   1 = chroma DC only
///   2 = chroma DC + AC
pub const ME_INTRA_4_2_0: [u8; 48] = [
    47, 31, 15, 0, 23, 27, 29, 30, 7, 11, 13, 14, 39, 43, 45, 46, 16, 3, 5, 10, 12, 19, 21, 26, 28,
    35, 37, 42, 44, 1, 2, 4, 8, 17, 18, 20, 24, 6, 9, 22, 25, 32, 33, 34, 36, 40, 38, 41,
];

/// Decode CBP for an I-slice macroblock (4:2:0 chroma).
pub fn decode_cbp_intra(me_value: u32) -> Option<(u8, u8)> {
    let idx = me_value as usize;
    if idx >= ME_INTRA_4_2_0.len() {
        return None;
    }
    let v = ME_INTRA_4_2_0[idx];
    Some((v & 0x0F, v >> 4))
}
