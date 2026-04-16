//! Reverse lookup tables for the H.263 encoder.
//!
//! For the I-picture encoder we only need three reverse lookups:
//! * MCBPC (intra) — value 0..=3 for `(Intra, cbpc)` pairs.
//!   (`mb_type=3`, no DQUANT, so we never hit value 4..=7.)
//! * CBPY — value 0..=15 (raw 4-bit pattern, no XOR for intra).
//! * TCOEF (inter Annex B-17 = H.263 Table 16) — `(last, run, level_abs)`
//!   with sign bit, plus a 7-bit `0000011` escape prefix for out-of-table
//!   tuples. The H.263 escape body is `last(1) + run(6) + level(8 signed
//!   two's complement, 0x00 and 0x80 forbidden)`.
//!
//! These tables are derived from the same tuple lists that
//! `oxideav-mpeg4video::tables::tcoef::inter_table` builds for decoding —
//! sourced from FFmpeg's `ff_inter_vlc` / `ff_inter_run` / `ff_inter_level`
//! arrays — so the encoded stream is bit-exact with what the decoder expects.

use crate::bitwriter::BitWriter;

// Mirror of `oxideav_mpeg4video::tables::tcoef::INTER_LAST0_*` /
// `INTER_LAST1_*`. Kept private — encoder-only consumers shouldn't need to
// poke individual entries.
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

/// Look up a `(last, run, level_abs)` triple in the inter TCOEF table. Returns
/// `Some((bits, code))` if found.
pub fn lookup_tcoef(last: bool, run: u8, level_abs: u8) -> Option<(u8, u32)> {
    let (vlc, runs, levels): (&[(u8, u32)], &[u8], &[u8]) = if last {
        (&INTER_LAST1_VLC, &INTER_LAST1_RUN, &INTER_LAST1_LEVEL)
    } else {
        (&INTER_LAST0_VLC, &INTER_LAST0_RUN, &INTER_LAST0_LEVEL)
    };
    for i in 0..vlc.len() {
        if runs[i] == run && levels[i] == level_abs {
            return Some(vlc[i]);
        }
    }
    None
}

/// Encode one `(last, run, level)` triple. `level` is signed and nonzero. If
/// the triple is in the VLC table, write the codeword + sign bit; otherwise
/// emit the H.263 fixed-length escape: `0000011 + last(1) + run(6) + level(8
/// signed)`.
///
/// Per spec the escape body forbids `level == 0` and `level == -128`. The
/// caller is responsible for not generating those.
pub fn write_tcoef(bw: &mut BitWriter, last: bool, run: u8, level: i32) {
    debug_assert!(level != 0);
    debug_assert!(run < 64);
    let abs = level.unsigned_abs();
    let sign = if level < 0 { 1 } else { 0 };
    if abs <= 255 {
        if let Some((bits, code)) = lookup_tcoef(last, run, abs as u8) {
            bw.write_bits(code, bits as u32);
            bw.write_bits(sign, 1);
            return;
        }
    }
    // Escape: 0000011 (7 bits) + last(1) + run(6) + level(8 signed)
    bw.write_bits(0b0000011, 7);
    bw.write_bits(last as u32, 1);
    bw.write_bits(run as u32 & 0x3F, 6);
    // 8-bit two's-complement: forbid 0 and -128.
    let level_byte = level & 0xFF;
    debug_assert!(level_byte != 0 && level_byte != 0x80);
    bw.write_bits(level_byte as u32, 8);
}

/// MCBPC intra: row table from `oxideav_mpeg4video::tables::mcbpc::I_ROWS`.
/// Indexed by value 0..=8 (8 = stuffing, never written by the encoder).
const I_MCBPC_VLC: [(u8, u32); 8] = [
    (1, 0b1),
    (3, 0b001),
    (3, 0b010),
    (3, 0b011),
    (4, 0b0001),
    (6, 0b00_0001),
    (6, 0b00_0010),
    (6, 0b00_0011),
];

/// Write the MCBPC for an intra (mb_type=3) MB with the given chroma CBP
/// (`cbpc` in 0..=3).
pub fn write_mcbpc_intra(bw: &mut BitWriter, cbpc: u8) {
    debug_assert!(cbpc < 4);
    let (bits, code) = I_MCBPC_VLC[cbpc as usize];
    bw.write_bits(code, bits as u32);
}

/// P-picture MCBPC inter table (Table 16/H.263 — rows for the mb-types we
/// actually emit: Inter (0..=3), Intra (4..=7), InterQ (8..=11), IntraQ
/// (12..=15)). Mirrors `oxideav_mpeg4video::tables::mcbpc::P_ROWS`.
///
/// Indexed by the encoded value `group * 4 + cbpc` with `group` = 0 (Inter),
/// 1 (Intra), 2 (InterQ), 3 (IntraQ). Inter4MV rows are NOT emitted by this
/// baseline encoder.
const P_MCBPC_VLC: [(u8, u32); 16] = [
    (1, 0b1),         // Inter,  cbpc=00
    (4, 0b0011),      // Inter,  cbpc=01
    (4, 0b0010),      // Inter,  cbpc=10
    (6, 0b000101),    // Inter,  cbpc=11
    (5, 0b00011),     // Intra,  cbpc=00
    (8, 0b00000100),  // Intra,  cbpc=01
    (8, 0b00000011),  // Intra,  cbpc=10
    (7, 0b0000011),   // Intra,  cbpc=11
    (3, 0b011),       // InterQ, cbpc=00
    (7, 0b0000111),   // InterQ, cbpc=01
    (7, 0b0000110),   // InterQ, cbpc=10
    (9, 0b000000101), // InterQ, cbpc=11
    (6, 0b000100),    // IntraQ, cbpc=00
    (9, 0b000000100), // IntraQ, cbpc=01
    (9, 0b000000011), // IntraQ, cbpc=10
    (9, 0b000000010), // IntraQ, cbpc=11
];

/// P-picture mb_type selector for `write_mcbpc_inter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PMbKind {
    /// `Inter` (mb_type 0 in Table 16) — no DQUANT, 1 MV.
    Inter,
    /// `Intra` (mb_type 1) — embedded intra inside a P-picture, no DQUANT.
    Intra,
    /// `InterQ` (mb_type 2) — 1 MV with DQUANT.
    InterQ,
    /// `IntraQ` (mb_type 3) — embedded intra with DQUANT.
    IntraQ,
}

/// Write the MCBPC for a P-picture MB.
pub fn write_mcbpc_inter(bw: &mut BitWriter, kind: PMbKind, cbpc: u8) {
    debug_assert!(cbpc < 4);
    let group = match kind {
        PMbKind::Inter => 0u8,
        PMbKind::Intra => 1,
        PMbKind::InterQ => 2,
        PMbKind::IntraQ => 3,
    };
    let idx = (group * 4 + cbpc) as usize;
    let (bits, code) = P_MCBPC_VLC[idx];
    bw.write_bits(code, bits as u32);
}

/// CBPY VLC table (raw 4-bit values 0..=15). Mirrors
/// `oxideav_mpeg4video::tables::cbpy::ROWS`.
const CBPY_VLC: [(u8, u32); 16] = [
    (4, 0b0011),
    (5, 0b00101),
    (5, 0b00100),
    (4, 0b1001),
    (5, 0b00011),
    (4, 0b0111),
    (6, 0b000010),
    (4, 0b1011),
    (5, 0b00010),
    (6, 0b000011),
    (4, 0b0101),
    (4, 0b1010),
    (4, 0b0100),
    (4, 0b1000),
    (4, 0b0110),
    (2, 0b11),
];

/// Write the CBPY field. `cbpy` is the raw 4-bit pattern (no XOR for intra).
pub fn write_cbpy(bw: &mut BitWriter, cbpy: u8) {
    debug_assert!(cbpy < 16);
    let (bits, code) = CBPY_VLC[cbpy as usize];
    bw.write_bits(code, bits as u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_mpeg4video::bitreader::BitReader;
    use oxideav_mpeg4video::tables::{
        cbpy as dec_cbpy, mcbpc as dec_mcbpc, tcoef as dec_tcoef, vlc,
    };

    #[test]
    fn mcbpc_round_trip() {
        for cbpc in 0u8..4 {
            let mut bw = BitWriter::new();
            write_mcbpc_intra(&mut bw, cbpc);
            let bytes = bw.finish();
            let mut br = BitReader::new(&bytes);
            let v = vlc::decode(&mut br, dec_mcbpc::i_table()).unwrap();
            assert_eq!(v, cbpc, "MCBPC intra round-trip for cbpc={cbpc}");
        }
    }

    #[test]
    fn mcbpc_inter_round_trip() {
        // Every (kind, cbpc) should round-trip through the inter table.
        use PMbKind::*;
        let cases: &[(PMbKind, u8, dec_mcbpc::PMbType)] = &[
            (Inter, 0, dec_mcbpc::PMbType::Inter),
            (Inter, 3, dec_mcbpc::PMbType::Inter),
            (Intra, 0, dec_mcbpc::PMbType::Intra),
            (Intra, 3, dec_mcbpc::PMbType::Intra),
            (InterQ, 0, dec_mcbpc::PMbType::InterQ),
            (InterQ, 3, dec_mcbpc::PMbType::InterQ),
            (IntraQ, 0, dec_mcbpc::PMbType::IntraQ),
            (IntraQ, 3, dec_mcbpc::PMbType::IntraQ),
        ];
        for &(kind, cbpc, want_ty) in cases {
            let mut bw = BitWriter::new();
            write_mcbpc_inter(&mut bw, kind, cbpc);
            let bytes = bw.finish();
            let mut br = BitReader::new(&bytes);
            let v = vlc::decode(&mut br, dec_mcbpc::p_table()).unwrap();
            let (ty, cbpc_r) = dec_mcbpc::decompose_inter(v);
            assert_eq!(ty, want_ty, "MCBPC inter kind {kind:?} cbpc={cbpc}");
            assert_eq!(cbpc_r, cbpc);
        }
    }

    #[test]
    fn cbpy_round_trip() {
        for c in 0u8..16 {
            let mut bw = BitWriter::new();
            write_cbpy(&mut bw, c);
            let bytes = bw.finish();
            let mut br = BitReader::new(&bytes);
            let v = vlc::decode(&mut br, dec_cbpy::table()).unwrap();
            assert_eq!(v, c, "CBPY round-trip for {c}");
        }
    }

    #[test]
    fn tcoef_short_round_trip() {
        // Pick a sample (last=false, run=0, level=1): the most common.
        let mut bw = BitWriter::new();
        write_tcoef(&mut bw, false, 0, 1);
        let bytes = bw.finish();
        let mut br = BitReader::new(&bytes);
        let sym = vlc::decode(&mut br, dec_tcoef::inter_table()).unwrap();
        match sym {
            dec_tcoef::TcoefSym::RunLevel {
                last,
                run,
                level_abs,
            } => {
                assert!(!last);
                assert_eq!(run, 0);
                assert_eq!(level_abs, 1);
                let sign = br.read_u32(1).unwrap();
                assert_eq!(sign, 0);
            }
            _ => panic!("wrong sym"),
        }
    }

    #[test]
    fn tcoef_negative_round_trip() {
        // (last=false, run=0, level=-2) is a short codeword + sign.
        let mut bw = BitWriter::new();
        write_tcoef(&mut bw, false, 0, -2);
        let bytes = bw.finish();
        let mut br = BitReader::new(&bytes);
        let sym = vlc::decode(&mut br, dec_tcoef::inter_table()).unwrap();
        if let dec_tcoef::TcoefSym::RunLevel {
            last,
            run,
            level_abs,
        } = sym
        {
            assert!(!last);
            assert_eq!(run, 0);
            assert_eq!(level_abs, 2);
            let sign = br.read_u32(1).unwrap();
            assert_eq!(sign, 1);
        } else {
            panic!("expected RunLevel symbol");
        }
    }

    #[test]
    fn tcoef_escape_round_trip() {
        // (last=false, run=0, level=100) is far past the table maximum (12) —
        // escape path.
        let mut bw = BitWriter::new();
        write_tcoef(&mut bw, false, 0, 100);
        let bytes = bw.finish();
        let mut br = BitReader::new(&bytes);
        let sym = vlc::decode(&mut br, dec_tcoef::inter_table()).unwrap();
        assert!(matches!(sym, dec_tcoef::TcoefSym::Escape));
        let last = br.read_u32(1).unwrap();
        let run = br.read_u32(6).unwrap();
        let lvl = br.read_u32(8).unwrap();
        assert_eq!(last, 0);
        assert_eq!(run, 0);
        assert_eq!(lvl, 100);
    }

    #[test]
    fn tcoef_escape_negative() {
        let mut bw = BitWriter::new();
        write_tcoef(&mut bw, true, 25, -50);
        let bytes = bw.finish();
        let mut br = BitReader::new(&bytes);
        let sym = vlc::decode(&mut br, dec_tcoef::inter_table()).unwrap();
        assert!(matches!(sym, dec_tcoef::TcoefSym::Escape));
        let last = br.read_u32(1).unwrap();
        let run = br.read_u32(6).unwrap();
        let lvl = br.read_u32(8).unwrap();
        assert_eq!(last, 1);
        assert_eq!(run, 25);
        assert_eq!(lvl, 256 - 50); // -50 in 8-bit two's complement
    }
}
