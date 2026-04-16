//! Table B-12 — motion vector VLC for MPEG-4 Part 2 P-VOPs (and B-VOPs).
//!
//! The table encodes the unsigned magnitude `|motion_code|` (a value in
//! 0..=32). Sign and `motion_residual` (low bits selected by `f_code`) are
//! transmitted separately by the caller. The table is identical to the H.263
//! motion-VLC.
//!
//! Source: ISO/IEC 14496-2 Annex B Table B-12, cross-checked against FFmpeg's
//! `ff_mvtab` in libavcodec/h263data.c.
//!
//! The table contains 33 entries (codes 0..=32). Value `0` carries no sign
//! bit; all other entries are followed by a single sign bit (`0`=positive,
//! `1`=negative) read by the caller.

use std::sync::OnceLock;

use crate::tables::vlc::VlcEntry;

/// (bits, code) pairs for Table B-12, indexed by magnitude `|motion_code|`.
///
/// Cross-checked against FFmpeg `ff_mvtab[33][2] = {(code, bits), ...}`:
const ROWS: [(u8, u32); 33] = [
    (1, 1),   // 0  -> '1'
    (2, 1),   // 1  -> '01'
    (3, 1),   // 2  -> '001'
    (4, 1),   // 3  -> '0001'
    (6, 3),   // 4  -> '000011'
    (7, 5),   // 5  -> '0000101'
    (7, 4),   // 6  -> '0000100'
    (7, 3),   // 7  -> '0000011'
    (9, 11),  // 8  -> '000001011'
    (9, 10),  // 9  -> '000001010'
    (9, 9),   // 10 -> '000001001'
    (10, 17), // 11 -> '0000010001'
    (10, 16), // 12 -> '0000010000'
    (10, 15), // 13 -> '0000001111'
    (10, 14), // 14 -> '0000001110'
    (10, 13), // 15 -> '0000001101'
    (10, 12), // 16 -> '0000001100'
    (10, 11), // 17 -> '0000001011'
    (10, 10), // 18 -> '0000001010'
    (10, 9),  // 19 -> '0000001001'
    (10, 8),  // 20 -> '0000001000'
    (10, 7),  // 21 -> '0000000111'
    (10, 6),  // 22 -> '0000000110'
    (10, 5),  // 23 -> '0000000101'
    (10, 4),  // 24 -> '0000000100'
    (11, 7),  // 25 -> '00000000111'
    (11, 6),  // 26 -> '00000000110'
    (11, 5),  // 27 -> '00000000101'
    (11, 4),  // 28 -> '00000000100'
    (11, 3),  // 29 -> '00000000011'
    (11, 2),  // 30 -> '00000000010'
    (12, 3),  // 31 -> '000000000011'
    (12, 2),  // 32 -> '000000000010'
];

/// Returns the motion VLC table. Decoded value is the unsigned magnitude
/// `|motion_code|` (0..=32). The caller reads a sign bit when magnitude > 0.
pub fn table() -> &'static [VlcEntry<u8>] {
    static CELL: OnceLock<Vec<VlcEntry<u8>>> = OnceLock::new();
    CELL.get_or_init(|| {
        ROWS.iter()
            .enumerate()
            .map(|(i, &(b, c))| VlcEntry::new(b, c, i as u8))
            .collect()
    })
    .as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_prefix_code() {
        let t = table();
        for i in 0..t.len() {
            for j in 0..t.len() {
                if i == j {
                    continue;
                }
                let a = &t[i];
                let b = &t[j];
                if a.bits <= b.bits {
                    let shift = b.bits - a.bits;
                    if (b.code >> shift) == a.code {
                        panic!(
                            "entry value={} bits={} code=0x{:x} is prefix of value={} bits={} code=0x{:x}",
                            a.value, a.bits, a.code, b.value, b.bits, b.code
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn table_size() {
        assert_eq!(table().len(), 33);
    }
}
