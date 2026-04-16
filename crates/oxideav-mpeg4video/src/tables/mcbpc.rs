//! Table B-10 / B-13 — MCBPC (macroblock-type + coded-block-pattern for chroma)
//! VLCs.
//!
//! * **I-VOP** uses Table B-10 — small table, intra MB types only.
//! * **P-VOP** uses Table B-13 — covers Inter (mb_type=0), Intra (3),
//!   InterQ (1), IntraQ (4), Inter4MV (2) and Inter4MV+Q (rare). The
//!   decoded value here is a flat 0..=27 index; the caller decomposes it.
//!
//! Layout for the inter (P) table (mirrors FFmpeg's `ff_h263_inter_MCBPC_*`):
//! * 0..=3 — Inter, cbpc = idx & 3
//! * 4..=7 — Intra, cbpc = idx & 3
//! * 8..=11 — InterQ, cbpc = idx & 3
//! * 12..=15 — IntraQ, cbpc = idx & 3
//! * 16..=19 — Inter4MV, cbpc = idx & 3
//! * 20 — stuffing
//! * 24..=27 — Inter4MV+Q, cbpc = idx & 3
//!
//! Stuffing codeword for the inter table (Table B-13) is the 9-bit
//! `000000001`.

use std::sync::OnceLock;

use crate::tables::vlc::VlcEntry;

/// Table B-10 rows for I-VOPs. (bits, code, value).
const I_ROWS: [(u8, u32, u8); 9] = [
    (1, 0b1, 0),
    (3, 0b001, 1),
    (3, 0b010, 2),
    (3, 0b011, 3),
    (4, 0b0001, 4),
    (6, 0b00_0001, 5),
    (6, 0b00_0010, 6),
    (6, 0b00_0011, 7),
    (9, 0b000_000_001, 8), // stuffing
];

/// Intra MCBPC stuffing codeword value.
pub const STUFFING: u8 = 8;

/// Inter MCBPC stuffing codeword value (Table B-13).
pub const INTER_STUFFING: u8 = 20;

pub fn i_table() -> &'static [VlcEntry<u8>] {
    static CELL: OnceLock<Vec<VlcEntry<u8>>> = OnceLock::new();
    CELL.get_or_init(|| {
        I_ROWS
            .iter()
            .map(|&(b, c, v)| VlcEntry::new(b, c, v))
            .collect()
    })
    .as_slice()
}

/// Table B-13 inter MCBPC rows. (bits, code, value).
/// Cross-checked against FFmpeg `ff_h263_inter_MCBPC_code/_bits`.
const P_ROWS: [(u8, u32, u8); 25] = [
    (1, 0b1, 0),               // Inter,   cbpc=00
    (4, 0b0011, 1),            // Inter,   cbpc=01
    (4, 0b0010, 2),            // Inter,   cbpc=10
    (6, 0b000101, 3),          // Inter,   cbpc=11
    (5, 0b00011, 4),           // Intra,   cbpc=00
    (8, 0b00000100, 5),        // Intra,   cbpc=01
    (8, 0b00000011, 6),        // Intra,   cbpc=10
    (7, 0b0000011, 7),         // Intra,   cbpc=11
    (3, 0b011, 8),             // InterQ,  cbpc=00
    (7, 0b0000111, 9),         // InterQ,  cbpc=01
    (7, 0b0000110, 10),        // InterQ,  cbpc=10
    (9, 0b000000101, 11),      // InterQ,  cbpc=11
    (6, 0b000100, 12),         // IntraQ,  cbpc=00
    (9, 0b000000100, 13),      // IntraQ,  cbpc=01
    (9, 0b000000011, 14),      // IntraQ,  cbpc=10
    (9, 0b000000010, 15),      // IntraQ,  cbpc=11
    (3, 0b010, 16),            // Inter4,  cbpc=00
    (7, 0b0000101, 17),        // Inter4,  cbpc=01
    (7, 0b0000100, 18),        // Inter4,  cbpc=10
    (8, 0b00000101, 19),       // Inter4,  cbpc=11
    (9, 0b000000001, 20),      // Stuffing
    (11, 0b00000000010, 24),   // Inter4Q, cbpc=00
    (13, 0b0000000001100, 25), // Inter4Q, cbpc=01
    (13, 0b0000000001110, 26), // Inter4Q, cbpc=10
    (13, 0b0000000001111, 27), // Inter4Q, cbpc=11
];

pub fn p_table() -> &'static [VlcEntry<u8>] {
    static CELL: OnceLock<Vec<VlcEntry<u8>>> = OnceLock::new();
    CELL.get_or_init(|| {
        P_ROWS
            .iter()
            .map(|&(b, c, v)| VlcEntry::new(b, c, v))
            .collect()
    })
    .as_slice()
}

/// Decoded P-MB type, from MCBPC inter index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PMbType {
    /// Inter — single forward MV, no dquant.
    Inter,
    /// InterQ — single forward MV, with dquant.
    InterQ,
    /// Inter4MV — four forward MVs (one per luma block), no dquant.
    Inter4MV,
    /// Inter4MV+Q — four forward MVs, with dquant.
    Inter4MVQ,
    /// Intra macroblock embedded in a P-VOP (handled by intra path).
    Intra,
    /// IntraQ macroblock embedded in a P-VOP (with dquant).
    IntraQ,
}

/// Decompose a Table B-13 value into `(mb_type, cbpc)`.
pub fn decompose_inter(value: u8) -> (PMbType, u8) {
    let cbpc = value & 0x3;
    let group = value >> 2;
    let ty = match group {
        0 => PMbType::Inter,
        1 => PMbType::Intra,
        2 => PMbType::InterQ,
        3 => PMbType::IntraQ,
        4 => PMbType::Inter4MV,
        6 => PMbType::Inter4MVQ,
        _ => PMbType::Inter, // unreachable in practice (5 = stuffing handled by caller)
    };
    (ty, cbpc)
}
