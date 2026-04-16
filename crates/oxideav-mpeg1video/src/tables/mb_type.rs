//! Tables B-2 / B-3 / B-4 — macroblock_type VLC per picture type.

use crate::vlc::VlcEntry;

/// Decoded `macroblock_type` flags.
#[derive(Clone, Copy, Debug, Default)]
pub struct MbTypeFlags {
    pub quant: bool,
    pub motion_forward: bool,
    pub motion_backward: bool,
    pub pattern: bool,
    pub intra: bool,
}

impl MbTypeFlags {
    pub const fn new(quant: bool, fwd: bool, bwd: bool, pat: bool, intra: bool) -> Self {
        Self {
            quant,
            motion_forward: fwd,
            motion_backward: bwd,
            pattern: pat,
            intra,
        }
    }
}

/// Table B-2 — macroblock_type in I-pictures.
/// 1     → Intra
/// 01    → Intra, quant
pub const I_TABLE: &[VlcEntry<MbTypeFlags>] = &[
    VlcEntry::new(1, 0b1, MbTypeFlags::new(false, false, false, false, true)),
    VlcEntry::new(2, 0b01, MbTypeFlags::new(true, false, false, false, true)),
];

/// Table B-3 — macroblock_type in P-pictures.
/// Codes from the spec (MSB-first):
///   1        → MC, Coded                 (fwd + pattern)
///   01       → No MC, Coded              (pattern)
///   001      → MC, Not Coded             (fwd)
///   0001 1   → Intra
///   0001 0   → MC, Coded, Quant          (fwd + pattern + quant)
///   0000 1   → No MC, Coded, Quant       (pattern + quant)
///   0000 01  → Intra, Quant
pub const P_TABLE: &[VlcEntry<MbTypeFlags>] = &[
    VlcEntry::new(1, 0b1, MbTypeFlags::new(false, true, false, true, false)),
    VlcEntry::new(2, 0b01, MbTypeFlags::new(false, false, false, true, false)),
    VlcEntry::new(3, 0b001, MbTypeFlags::new(false, true, false, false, false)),
    VlcEntry::new(
        5,
        0b00011,
        MbTypeFlags::new(false, false, false, false, true),
    ),
    VlcEntry::new(5, 0b00010, MbTypeFlags::new(true, true, false, true, false)),
    VlcEntry::new(
        5,
        0b00001,
        MbTypeFlags::new(true, false, false, true, false),
    ),
    VlcEntry::new(
        6,
        0b000001,
        MbTypeFlags::new(true, false, false, false, true),
    ),
];

/// Table B-4 — macroblock_type in B-pictures (codes/flags verified against
/// libavcodec `table_mb_btype`, which lists each entry's bit semantics as
/// {MB_INTRA, MB_PAT, MB_BACK, MB_FOR, MB_QUANT}).
///
/// ```text
///   10      → Interpolated, Not Coded     (fwd + bwd)
///   11      → Interpolated, Coded         (fwd + bwd + pattern)
///   010     → Backward, Not Coded         (bwd)
///   011     → Backward, Coded             (bwd + pattern)
///   0010    → Forward, Not Coded          (fwd)
///   0011    → Forward, Coded              (fwd + pattern)
///   00010   → Interpolated, Coded, Quant  (quant + fwd + bwd + pattern)
///   00011   → Intra
///   000001  → Intra, Quant
///   000010  → Backward, Coded, Quant
///   000011  → Forward, Coded, Quant
/// ```
pub const B_TABLE: &[VlcEntry<MbTypeFlags>] = &[
    // 10 → interpolated (fwd + bwd), no pattern
    VlcEntry::new(2, 0b10, MbTypeFlags::new(false, true, true, false, false)),
    // 11 → interpolated, coded
    VlcEntry::new(2, 0b11, MbTypeFlags::new(false, true, true, true, false)),
    // 010 → backward only
    VlcEntry::new(3, 0b010, MbTypeFlags::new(false, false, true, false, false)),
    // 011 → backward, coded
    VlcEntry::new(3, 0b011, MbTypeFlags::new(false, false, true, true, false)),
    // 0010 → forward only
    VlcEntry::new(
        4,
        0b0010,
        MbTypeFlags::new(false, true, false, false, false),
    ),
    // 0011 → forward, coded
    VlcEntry::new(4, 0b0011, MbTypeFlags::new(false, true, false, true, false)),
    // 00010 → interpolated, coded, quant
    VlcEntry::new(5, 0b00010, MbTypeFlags::new(true, true, true, true, false)),
    // 00011 → intra
    VlcEntry::new(
        5,
        0b00011,
        MbTypeFlags::new(false, false, false, false, true),
    ),
    // 000001 → intra, quant
    VlcEntry::new(
        6,
        0b000001,
        MbTypeFlags::new(true, false, false, false, true),
    ),
    // 000010 → backward, coded, quant
    VlcEntry::new(
        6,
        0b000010,
        MbTypeFlags::new(true, false, true, true, false),
    ),
    // 000011 → forward, coded, quant
    VlcEntry::new(
        6,
        0b000011,
        MbTypeFlags::new(true, true, false, true, false),
    ),
];
