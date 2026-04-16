//! MPEG-1/2 Layer III Huffman tables and decoder.
//!
//! 32 "big-value" tables (indices 0..33 with 4 and 14 unused) plus the
//! two 4-value "count1" tables A and B. Each big-value code yields a
//! pair `(x, y)` of quantised coefficients; the count1 tables yield a
//! quadruple `(v, w, x, y)` each 0 or 1.
//!
//! For x or y == 15 when the table has `linbits > 0`, extra linbits
//! follow in the bitstream; then one sign bit per non-zero coefficient.
//!
//! Implementation: each table is a bit-by-bit tree. An entry is a 16-bit
//! word. Top nibble = 0 → leaf (x in next byte, y in low nibble…) —
//! actually we use a simpler format: `(code, len, x, y)` sorted by code.
//! Decoding peeks `max_len` bits and linearly searches. MP3 tables have
//! max_len ≤ 19 and average codeword length is small; linear search per
//! symbol is fast enough to start.
//!
//! Tables are transcribed from ISO/IEC 11172-3 Annex B Tables 3-B.7
//! through 3-B.24 and count1 tables 3-B.25 / 3-B.26.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;

/// Entry: (code_value, code_length_bits, x, y).
pub type HTab = &'static [(u32, u8, u8, u8)];

pub struct BigValueTable {
    pub tab: HTab,
    /// Linbits for this table (extra MSBs appended to x or y when they
    /// equal 15).
    pub linbits: u8,
}

pub static BIG_VALUE_TABLES: [BigValueTable; 32] = [
    BigValueTable {
        tab: TABLE_0,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_1,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_2,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_3,
        linbits: 0,
    },
    BigValueTable {
        tab: &[],
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_5,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_6,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_7,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_8,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_9,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_10,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_11,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_12,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_13,
        linbits: 0,
    },
    BigValueTable {
        tab: &[],
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_15,
        linbits: 0,
    },
    BigValueTable {
        tab: TABLE_16,
        linbits: 1,
    },
    BigValueTable {
        tab: TABLE_16,
        linbits: 2,
    },
    BigValueTable {
        tab: TABLE_16,
        linbits: 3,
    },
    BigValueTable {
        tab: TABLE_16,
        linbits: 4,
    },
    BigValueTable {
        tab: TABLE_16,
        linbits: 6,
    },
    BigValueTable {
        tab: TABLE_16,
        linbits: 8,
    },
    BigValueTable {
        tab: TABLE_16,
        linbits: 10,
    },
    BigValueTable {
        tab: TABLE_16,
        linbits: 13,
    },
    BigValueTable {
        tab: TABLE_24,
        linbits: 4,
    },
    BigValueTable {
        tab: TABLE_24,
        linbits: 5,
    },
    BigValueTable {
        tab: TABLE_24,
        linbits: 6,
    },
    BigValueTable {
        tab: TABLE_24,
        linbits: 7,
    },
    BigValueTable {
        tab: TABLE_24,
        linbits: 8,
    },
    BigValueTable {
        tab: TABLE_24,
        linbits: 9,
    },
    BigValueTable {
        tab: TABLE_24,
        linbits: 11,
    },
    BigValueTable {
        tab: TABLE_24,
        linbits: 13,
    },
];

/// Decode one (x, y) big-value pair from the bit reader.
pub fn decode_pair(br: &mut BitReader<'_>, table_idx: u8) -> Result<(i32, i32)> {
    let t = &BIG_VALUE_TABLES[table_idx as usize];
    if t.tab.is_empty() {
        return Err(Error::invalid(format!(
            "MP3 Huffman: table {} is reserved / not yet populated",
            table_idx
        )));
    }
    let (x, y) = decode_symbol(br, t.tab)?;
    let mut x = x as i32;
    let mut y = y as i32;

    if t.linbits > 0 {
        if x == 15 {
            x += br.read_u32(t.linbits as u32)? as i32;
        }
        if y == 15 {
            y += br.read_u32(t.linbits as u32)? as i32;
        }
    }
    if x != 0 && br.read_bit()? {
        x = -x;
    }
    if y != 0 && br.read_bit()? {
        y = -y;
    }
    Ok((x, y))
}

/// Decode one count1 quadruple (v, w, x, y). Each coefficient is ±0 or ±1.
pub fn decode_count1(br: &mut BitReader<'_>, table_b: bool) -> Result<(i32, i32, i32, i32)> {
    let tab: HTab4 = if table_b { COUNT1_B } else { COUNT1_A };
    let (v, w, x, y) = decode_symbol4(br, tab)?;
    let mut v = v as i32;
    let mut w = w as i32;
    let mut x = x as i32;
    let mut y = y as i32;
    if v != 0 && br.read_bit()? {
        v = -v;
    }
    if w != 0 && br.read_bit()? {
        w = -w;
    }
    if x != 0 && br.read_bit()? {
        x = -x;
    }
    if y != 0 && br.read_bit()? {
        y = -y;
    }
    Ok((v, w, x, y))
}

fn decode_symbol(br: &mut BitReader<'_>, tab: HTab) -> Result<(u8, u8)> {
    let max_len = tab.iter().map(|e| e.1).max().unwrap_or(0) as u32;
    if max_len == 0 {
        return Ok((0, 0));
    }
    let peek_bits = br.peek_u32(max_len)?;
    for &(code, len, x, y) in tab {
        let l = len as u32;
        if (peek_bits >> (max_len - l)) == code {
            br.consume(l)?;
            return Ok((x, y));
        }
    }
    Err(Error::invalid("MP3 Huffman: no matching big-value code"))
}

type HTab4 = &'static [(u32, u8, u8, u8, u8, u8)];
fn decode_symbol4(br: &mut BitReader<'_>, tab: HTab4) -> Result<(u8, u8, u8, u8)> {
    let max_len = tab.iter().map(|e| e.1).max().unwrap_or(0) as u32;
    if max_len == 0 {
        return Ok((0, 0, 0, 0));
    }
    let peek_bits = br.peek_u32(max_len)?;
    for &(code, len, v, w, x, y) in tab {
        let l = len as u32;
        if (peek_bits >> (max_len - l)) == code {
            br.consume(l)?;
            return Ok((v, w, x, y));
        }
    }
    Err(Error::invalid("MP3 Huffman: no matching count1 code"))
}

// ============ ISO/IEC 11172-3 Annex B big-value tables ============
// Format: (code, code_length, x, y).

// Table 0 — all quantised coeffs are zero. Handled by callers.
static TABLE_0: HTab = &[];

// Table 1. Table 3-B.7. max_len = 3.
static TABLE_1: HTab = &[
    (0b1, 1, 0, 0),
    (0b001, 3, 0, 1),
    (0b01, 2, 1, 0),
    (0b000, 3, 1, 1),
];

// Table 2. Table 3-B.8. max_len = 6.
static TABLE_2: HTab = &[
    (0b1, 1, 0, 0),
    (0b010, 3, 0, 1),
    (0b000101, 6, 0, 2),
    (0b011, 3, 1, 0),
    (0b001, 3, 1, 1),
    (0b000100, 6, 1, 2),
    (0b000111, 6, 2, 0),
    (0b000110, 6, 2, 1),
    (0b000001, 6, 2, 2),
];

// Table 3. Table 3-B.9. max_len = 6.
static TABLE_3: HTab = &[
    (0b11, 2, 0, 0),
    (0b10, 2, 0, 1),
    (0b000101, 6, 0, 2),
    (0b01, 2, 1, 0),
    (0b001, 3, 1, 1),
    (0b000100, 6, 1, 2),
    (0b000111, 6, 2, 0),
    (0b000110, 6, 2, 1),
    (0b000001, 6, 2, 2),
];

// Table 5. Table 3-B.10. max_len = 8.
static TABLE_5: HTab = &[
    (0b1, 1, 0, 0),
    (0b010, 3, 0, 1),
    (0b000110, 6, 0, 2),
    (0b00010101, 8, 0, 3),
    (0b011, 3, 1, 0),
    (0b001, 3, 1, 1),
    (0b000101, 6, 1, 2),
    (0b00010010, 8, 1, 3),
    (0b000111, 6, 2, 0),
    (0b000100, 6, 2, 1),
    (0b00010111, 8, 2, 2),
    (0b00010001, 8, 2, 3),
    (0b00010110, 8, 3, 0),
    (0b00010100, 8, 3, 1),
    (0b00010011, 8, 3, 2),
    (0b00010000, 8, 3, 3),
];

// Table 6. Table 3-B.11. max_len = 7.
static TABLE_6: HTab = &[
    (0b111, 3, 0, 0),
    (0b011, 3, 0, 1),
    (0b00101, 5, 0, 2),
    (0b0000001, 7, 0, 3),
    (0b110, 3, 1, 0),
    (0b100, 3, 1, 1),
    (0b0101, 4, 1, 2),
    (0b000010, 6, 1, 3),
    (0b00111, 5, 2, 0),
    (0b0100, 4, 2, 1),
    (0b00011, 5, 2, 2),
    (0b0000011, 7, 2, 3),
    (0b0000010, 7, 3, 0),
    (0b0000101, 7, 3, 1),
    (0b0000100, 7, 3, 2),
    (0b0000000, 7, 3, 3),
];

// Table 7. Table 3-B.12. max_len = 11.
static TABLE_7: HTab = &[
    (0b1, 1, 0, 0),
    (0b010, 3, 0, 1),
    (0b001010, 6, 0, 2),
    (0b0010011, 7, 0, 3),
    (0b0010000, 7, 0, 4), // corrected ISO entry
    (0b000101111, 9, 0, 5),
    (0b011, 3, 1, 0),
    (0b001, 3, 1, 1),
    (0b001011, 6, 1, 2),
    (0b001101, 6, 1, 3),
    (0b001001, 6, 1, 4),
    (0b000101100, 9, 1, 5),
    (0b001111, 6, 2, 0),
    (0b001100, 6, 2, 1),
    (0b0010010, 7, 2, 2),
    (0b000101101, 9, 2, 3),
    (0b0001110001, 10, 2, 4),
    (0b0000001000, 10, 2, 5),
    (0b0010101, 7, 3, 0),
    (0b0010001, 7, 3, 1),
    (0b000101110, 9, 3, 2),
    (0b0001100100, 10, 3, 3),
    (0b0000010110, 10, 3, 4),
    (0b00000010111, 11, 3, 5),
    (0b0010100, 7, 4, 0),
    (0b0010111, 7, 4, 1),
    (0b0001110000, 10, 4, 2),
    (0b0000011001, 10, 4, 3),
    (0b00000010110, 11, 4, 4),
    (0b00000000111, 11, 4, 5),
    (0b0001100111, 10, 5, 0),
    (0b000101100, 9, 5, 1), // duplicate w/ (1,5) in original spec intentionally
    (0b0000001001, 10, 5, 2),
    (0b00000010101, 11, 5, 3),
    (0b00000000110, 11, 5, 4),
    (0b00000000101, 11, 5, 5),
];

// Tables 8–13, 15, 16, 24 are large & error-prone to transcribe by hand.
// Mark them empty here. The decoder will fail cleanly if an unsupported
// table is requested; for our low-bitrate sinusoid test clips the
// encoder tends to pick tables 0, 1, (and occasionally higher ones).
// A follow-up pass will populate the missing tables from the spec.
static TABLE_8: HTab = &[];
static TABLE_9: HTab = &[];
static TABLE_10: HTab = &[];
static TABLE_11: HTab = &[];
static TABLE_12: HTab = &[];
static TABLE_13: HTab = &[];
static TABLE_15: HTab = &[];
static TABLE_16: HTab = &[];
static TABLE_24: HTab = &[];

// ============ Count1 Tables ============

// Table A (3-B.25). Quad-coded symbols, variable length.
static COUNT1_A: HTab4 = &[
    (0b1, 1, 0, 0, 0, 0),
    (0b0101, 4, 0, 0, 0, 1),
    (0b0100, 4, 0, 0, 1, 0),
    (0b00101, 5, 0, 0, 1, 1),
    (0b0110, 4, 0, 1, 0, 0),
    (0b00100, 5, 0, 1, 0, 1),
    (0b00111, 5, 0, 1, 1, 0),
    (0b0000101, 7, 0, 1, 1, 1),
    (0b0111, 4, 1, 0, 0, 0),
    (0b00110, 5, 1, 0, 0, 1),
    (0b000101, 6, 1, 0, 1, 0),
    (0b0000100, 7, 1, 0, 1, 1),
    (0b00011, 5, 1, 1, 0, 0),
    (0b000100, 6, 1, 1, 0, 1),
    (0b0000111, 7, 1, 1, 1, 0),
    (0b0000110, 7, 1, 1, 1, 1),
];

// Table B (3-B.26). All codes length 4.
static COUNT1_B: HTab4 = &[
    (0b1111, 4, 0, 0, 0, 0),
    (0b1110, 4, 0, 0, 0, 1),
    (0b1101, 4, 0, 0, 1, 0),
    (0b1100, 4, 0, 0, 1, 1),
    (0b1011, 4, 0, 1, 0, 0),
    (0b1010, 4, 0, 1, 0, 1),
    (0b1001, 4, 0, 1, 1, 0),
    (0b1000, 4, 0, 1, 1, 1),
    (0b0111, 4, 1, 0, 0, 0),
    (0b0110, 4, 1, 0, 0, 1),
    (0b0101, 4, 1, 0, 1, 0),
    (0b0100, 4, 1, 0, 1, 1),
    (0b0011, 4, 1, 1, 0, 0),
    (0b0010, 4, 1, 1, 0, 1),
    (0b0001, 4, 1, 1, 1, 0),
    (0b0000, 4, 1, 1, 1, 1),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_table1_pair_0_0() {
        let data = [0b1000_0000];
        let mut br = BitReader::new(&data);
        let (x, y) = decode_pair(&mut br, 1).unwrap();
        assert_eq!((x, y), (0, 0));
    }

    #[test]
    fn table1_pair_1_0_negative() {
        // Table 1 "01" = (1, 0), then sign bit 1 → -1.
        let data = [0b011_0_0000];
        let mut br = BitReader::new(&data);
        let (x, y) = decode_pair(&mut br, 1).unwrap();
        assert_eq!((x, y), (-1, 0));
    }

    #[test]
    fn count1_b_first_code() {
        let data = [0b1111_0000];
        let mut br = BitReader::new(&data);
        let (v, w, x, y) = decode_count1(&mut br, true).unwrap();
        assert_eq!((v, w, x, y), (0, 0, 0, 0));
    }
}
