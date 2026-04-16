//! Table B-16 / B-17 — texture coefficient VLCs (intra / inter).
//!
//! Transcribed from ISO/IEC 14496-2 Annex B, cross-checked against FFmpeg's
//! `ff_mpeg4_rl_intra` / `ff_h263_rl_inter` tables. The layout matches Tables
//! B-16 and B-17 exactly: the short-form VLC decodes to `(last, run, level)`
//! where `last==0` entries precede `last==1` entries in the natural "run
//! spreads slowest, level fastest" order.
//!
//! Both tables share a special escape code that uses the prefix `0000011` (7
//! bits) followed by one of three further-escape modes (§6.3.8, §7.4.1.3 —
//! Annex B describes it under "Escape code"). Our decoder consumes the
//! 7-bit prefix via the VLC table (as `TcoefSym::Escape`); the caller then
//! reads the remaining bits by hand.
//!
//! Sign encoding: every non-escape codeword is followed by a single sign bit
//! (`0` = positive, `1` = negative), consumed by the caller.

use std::sync::OnceLock;

use crate::tables::vlc::VlcEntry;

/// One decoded texture coefficient symbol.
#[derive(Clone, Copy, Debug)]
pub enum TcoefSym {
    /// `(last, run, level)` in short form — sign is in a following bit.
    RunLevel { last: bool, run: u8, level_abs: u8 },
    /// Escape codeword — the caller reads additional bits to get the actual
    /// `(last, run, level)` triple. MPEG-4 has three escape modes (§6.3.8
    /// types 1/2/3).
    Escape,
}

// Intra Annex B-16 tuples. Source: FFmpeg `ff_mpeg4_intra_vlc` / `ff_mpeg4_intra_run`
// / `ff_mpeg4_intra_level`. Each entry is `(bits, code, last, run, level)`.
// Index 0..=66 = last 0 (67 entries), 67..=101 = last 1 (35 entries).
// Entry 102 is the 7-bit escape (`0000011`), represented as a sentinel.
const INTRA_LAST0_VLC: [(u8, u32); 67] = [
    (2, 0x2),
    (3, 0x6),
    (4, 0xF),
    (5, 0xD),
    (5, 0xC),
    (6, 0x15),
    (6, 0x13),
    (6, 0x12),
    (7, 0x17),
    (8, 0x1F),
    (8, 0x1E),
    (8, 0x1D),
    (9, 0x25),
    (9, 0x24),
    (9, 0x23),
    (9, 0x21),
    (10, 0x21),
    (10, 0x20),
    (10, 0xF),
    (10, 0xE),
    (11, 0x7),
    (11, 0x6),
    (11, 0x20),
    (11, 0x21),
    (12, 0x50),
    (12, 0x51),
    (12, 0x52),
    (4, 0xE),
    (6, 0x14),
    (7, 0x16),
    (8, 0x1C),
    (9, 0x20),
    (9, 0x1F),
    (10, 0xD),
    (11, 0x22),
    (12, 0x53),
    (12, 0x55),
    (5, 0xB),
    (7, 0x15),
    (9, 0x1E),
    (10, 0xC),
    (12, 0x56),
    (6, 0x11),
    (8, 0x1B),
    (9, 0x1D),
    (10, 0xB),
    (6, 0x10),
    (9, 0x22),
    (10, 0xA),
    (6, 0xD),
    (9, 0x1C),
    (10, 0x8),
    (7, 0x12),
    (9, 0x1B),
    (12, 0x54),
    (7, 0x14),
    (9, 0x1A),
    (12, 0x57),
    (8, 0x19),
    (10, 0x9),
    (8, 0x18),
    (11, 0x23),
    (8, 0x17),
    (9, 0x19),
    (9, 0x18),
    (10, 0x7),
    (12, 0x58),
];
const INTRA_LAST0_RUN: [u8; 67] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 5, 5, 5, 6, 6, 6, 7, 7, 7, 8, 8, 9, 9, 10,
    11, 12, 13, 14,
];
const INTRA_LAST0_LEVEL: [u8; 67] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 1, 2, 3, 4, 5, 1, 2, 3, 4, 1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2,
    3, 1, 2, 1, 2, 1, 1, 1, 1, 1,
];

const INTRA_LAST1_VLC: [(u8, u32); 35] = [
    (4, 0x7),
    (6, 0xC),
    (8, 0x16),
    (9, 0x17),
    (10, 0x6),
    (11, 0x5),
    (11, 0x4),
    (12, 0x59),
    (6, 0xF),
    (9, 0x16),
    (10, 0x5),
    (6, 0xE),
    (10, 0x4),
    (7, 0x11),
    (11, 0x24),
    (7, 0x10),
    (11, 0x25),
    (7, 0x13),
    (12, 0x5A),
    (8, 0x15),
    (12, 0x5B),
    (8, 0x14),
    (8, 0x13),
    (8, 0x1A),
    (9, 0x15),
    (9, 0x14),
    (9, 0x13),
    (9, 0x12),
    (9, 0x11),
    (11, 0x26),
    (11, 0x27),
    (12, 0x5C),
    (12, 0x5D),
    (12, 0x5E),
    (12, 0x5F),
];
const INTRA_LAST1_RUN: [u8; 35] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20,
];
const INTRA_LAST1_LEVEL: [u8; 35] = [
    1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1,
];

// Escape code (Annex B §6.3.8): `0000011` = 7 bits, value 0x03.
const TCOEF_ESCAPE_BITS: u8 = 7;
const TCOEF_ESCAPE_CODE: u32 = 0x03;

/// Returns the intra tcoef table (Table B-16).
pub fn intra_table() -> &'static [VlcEntry<TcoefSym>] {
    static CELL: OnceLock<Vec<VlcEntry<TcoefSym>>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut out = Vec::with_capacity(103);
        for i in 0..INTRA_LAST0_VLC.len() {
            let (bits, code) = INTRA_LAST0_VLC[i];
            out.push(VlcEntry::new(
                bits,
                code,
                TcoefSym::RunLevel {
                    last: false,
                    run: INTRA_LAST0_RUN[i],
                    level_abs: INTRA_LAST0_LEVEL[i],
                },
            ));
        }
        for i in 0..INTRA_LAST1_VLC.len() {
            let (bits, code) = INTRA_LAST1_VLC[i];
            out.push(VlcEntry::new(
                bits,
                code,
                TcoefSym::RunLevel {
                    last: true,
                    run: INTRA_LAST1_RUN[i],
                    level_abs: INTRA_LAST1_LEVEL[i],
                },
            ));
        }
        out.push(VlcEntry::new(
            TCOEF_ESCAPE_BITS,
            TCOEF_ESCAPE_CODE,
            TcoefSym::Escape,
        ));
        out
    })
    .as_slice()
}

// Inter Annex B-17 tuples. Source: FFmpeg `ff_inter_vlc` / `ff_inter_run` /
// `ff_inter_level`. Index 0..=57 = last 0 (58 entries), 58..=101 = last 1 (44
// entries). Entry 102 is the 7-bit escape.
const INTER_LAST0_VLC: [(u8, u32); 58] = [
    (2, 0x2),
    (4, 0xF),
    (6, 0x15),
    (7, 0x17),
    (8, 0x1F),
    (9, 0x25),
    (9, 0x24),
    (10, 0x21),
    (10, 0x20),
    (11, 0x7),
    (11, 0x6),
    (11, 0x20),
    (3, 0x6),
    (6, 0x14),
    (8, 0x1E),
    (10, 0xF),
    (11, 0x21),
    (12, 0x50),
    (4, 0xE),
    (8, 0x1D),
    (10, 0xE),
    (12, 0x51),
    (5, 0xD),
    (9, 0x23),
    (10, 0xD),
    (5, 0xC),
    (9, 0x22),
    (12, 0x52),
    (5, 0xB),
    (10, 0xC),
    (12, 0x53),
    (6, 0x13),
    (10, 0xB),
    (12, 0x54),
    (6, 0x12),
    (10, 0xA),
    (6, 0x11),
    (10, 0x9),
    (6, 0x10),
    (10, 0x8),
    (7, 0x16),
    (12, 0x55),
    (7, 0x15),
    (7, 0x14),
    (8, 0x1C),
    (8, 0x1B),
    (9, 0x21),
    (9, 0x20),
    (9, 0x1F),
    (9, 0x1E),
    (9, 0x1D),
    (9, 0x1C),
    (9, 0x1B),
    (9, 0x1A),
    (11, 0x22),
    (11, 0x23),
    (12, 0x56),
    (12, 0x57),
];
const INTER_LAST0_RUN: [u8; 58] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 5, 6,
    6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
];
const INTER_LAST0_LEVEL: [u8; 58] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 1, 2, 3, 4, 5, 6, 1, 2, 3, 4, 1, 2, 3, 1, 2, 3, 1, 2, 3,
    1, 2, 3, 1, 2, 1, 2, 1, 2, 1, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];

const INTER_LAST1_VLC: [(u8, u32); 44] = [
    (4, 0x7),
    (9, 0x19),
    (11, 0x5),
    (6, 0xF),
    (11, 0x4),
    (6, 0xE),
    (6, 0xD),
    (6, 0xC),
    (7, 0x13),
    (7, 0x12),
    (7, 0x11),
    (7, 0x10),
    (8, 0x1A),
    (8, 0x19),
    (8, 0x18),
    (8, 0x17),
    (8, 0x16),
    (8, 0x15),
    (8, 0x14),
    (8, 0x13),
    (9, 0x18),
    (9, 0x17),
    (9, 0x16),
    (9, 0x15),
    (9, 0x14),
    (9, 0x13),
    (9, 0x12),
    (9, 0x11),
    (10, 0x7),
    (10, 0x6),
    (10, 0x5),
    (10, 0x4),
    (11, 0x24),
    (11, 0x25),
    (11, 0x26),
    (11, 0x27),
    (12, 0x58),
    (12, 0x59),
    (12, 0x5A),
    (12, 0x5B),
    (12, 0x5C),
    (12, 0x5D),
    (12, 0x5E),
    (12, 0x5F),
];
const INTER_LAST1_RUN: [u8; 44] = [
    0, 0, 0, 1, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
    24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
];
const INTER_LAST1_LEVEL: [u8; 44] = [
    1, 2, 3, 1, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];

/// Returns the inter tcoef table (Table B-17).
pub fn inter_table() -> &'static [VlcEntry<TcoefSym>] {
    static CELL: OnceLock<Vec<VlcEntry<TcoefSym>>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut out = Vec::with_capacity(103);
        for i in 0..INTER_LAST0_VLC.len() {
            let (bits, code) = INTER_LAST0_VLC[i];
            out.push(VlcEntry::new(
                bits,
                code,
                TcoefSym::RunLevel {
                    last: false,
                    run: INTER_LAST0_RUN[i],
                    level_abs: INTER_LAST0_LEVEL[i],
                },
            ));
        }
        for i in 0..INTER_LAST1_VLC.len() {
            let (bits, code) = INTER_LAST1_VLC[i];
            out.push(VlcEntry::new(
                bits,
                code,
                TcoefSym::RunLevel {
                    last: true,
                    run: INTER_LAST1_RUN[i],
                    level_abs: INTER_LAST1_LEVEL[i],
                },
            ));
        }
        out.push(VlcEntry::new(
            TCOEF_ESCAPE_BITS,
            TCOEF_ESCAPE_CODE,
            TcoefSym::Escape,
        ));
        out
    })
    .as_slice()
}

/// Look up the maximum `level` (1-indexed, positive) for a given `(last, run)`
/// under the intra table. Used by "first escape" mode to recover a level that
/// was encoded as `table_level + max_level[last][run]`.
///
/// Returns `0` if `(last, run)` has no entry — callers should treat this as an
/// invalid stream.
pub fn intra_max_level(last: bool, run: u8) -> u8 {
    let rows: &[u8] = if last {
        &INTRA_LAST1_RUN
    } else {
        &INTRA_LAST0_RUN
    };
    let levels: &[u8] = if last {
        &INTRA_LAST1_LEVEL
    } else {
        &INTRA_LAST0_LEVEL
    };
    let mut max = 0u8;
    for i in 0..rows.len() {
        if rows[i] == run {
            max = max.max(levels[i]);
        }
    }
    max
}

/// Look up the maximum `run` for a given `(last, level)` under the intra
/// table. Used by "second escape" mode to recover a run that was encoded as
/// `table_run + max_run[last][level] + 1`.
pub fn intra_max_run(last: bool, level: u8) -> u8 {
    let rows: &[u8] = if last {
        &INTRA_LAST1_RUN
    } else {
        &INTRA_LAST0_RUN
    };
    let levels: &[u8] = if last {
        &INTRA_LAST1_LEVEL
    } else {
        &INTRA_LAST0_LEVEL
    };
    let mut max = 0u8;
    let mut found = false;
    for i in 0..rows.len() {
        if levels[i] == level {
            if !found || rows[i] > max {
                max = rows[i];
            }
            found = true;
        }
    }
    max
}

/// Same as [`intra_max_level`] but indexes the inter table.
pub fn inter_max_level(last: bool, run: u8) -> u8 {
    let rows: &[u8] = if last {
        &INTER_LAST1_RUN
    } else {
        &INTER_LAST0_RUN
    };
    let levels: &[u8] = if last {
        &INTER_LAST1_LEVEL
    } else {
        &INTER_LAST0_LEVEL
    };
    let mut max = 0u8;
    for i in 0..rows.len() {
        if rows[i] == run {
            max = max.max(levels[i]);
        }
    }
    max
}

/// Same as [`intra_max_run`] but indexes the inter table.
pub fn inter_max_run(last: bool, level: u8) -> u8 {
    let rows: &[u8] = if last {
        &INTER_LAST1_RUN
    } else {
        &INTER_LAST0_RUN
    };
    let levels: &[u8] = if last {
        &INTER_LAST1_LEVEL
    } else {
        &INTER_LAST0_LEVEL
    };
    let mut max = 0u8;
    let mut found = false;
    for i in 0..rows.len() {
        if levels[i] == level {
            if !found || rows[i] > max {
                max = rows[i];
            }
            found = true;
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify no codeword is a prefix of another — mandatory for a valid VLC.
    fn assert_prefix_unique(table: &[VlcEntry<TcoefSym>]) {
        for i in 0..table.len() {
            for j in 0..table.len() {
                if i == j {
                    continue;
                }
                let a = &table[i];
                let b = &table[j];
                if a.bits <= b.bits {
                    let shift = b.bits - a.bits;
                    if (b.code >> shift) == a.code {
                        panic!(
                            "entry ({} bits, {:x}) is a prefix of ({} bits, {:x})",
                            a.bits, a.code, b.bits, b.code
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn intra_table_is_prefix_code() {
        assert_prefix_unique(intra_table());
    }

    #[test]
    fn inter_table_is_prefix_code() {
        assert_prefix_unique(inter_table());
    }

    #[test]
    fn intra_has_expected_entry_count() {
        // 67 last=0 + 35 last=1 + 1 escape = 103
        assert_eq!(intra_table().len(), 103);
    }

    #[test]
    fn inter_has_expected_entry_count() {
        // 58 last=0 + 44 last=1 + 1 escape = 103
        assert_eq!(inter_table().len(), 103);
    }

    #[test]
    fn max_level_and_run_smoke() {
        // Intra: (last=0, run=0) has levels 1..=27 -> max=27.
        assert_eq!(intra_max_level(false, 0), 27);
        // Intra: (last=0, level=1) max_run = 14 (the last last0 entry with level=1 is run=14).
        assert_eq!(intra_max_run(false, 1), 14);
        // Intra: (last=1, run=0) levels 1..=8 -> max=8.
        assert_eq!(intra_max_level(true, 0), 8);
        // Inter: (last=0, run=0) levels 1..=12 -> max=12.
        assert_eq!(inter_max_level(false, 0), 12);
    }
}
