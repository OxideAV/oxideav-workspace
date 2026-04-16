//! CAVLC residual block decoding — ITU-T H.264 §9.2.
//!
//! Tables taken straight from the H.264 specification (Tables 9-5, 9-7, 9-9.1,
//! 9-9.2, 9-10).
//!
//! Pipeline per block:
//!
//! 1. Read `coeff_token` → (`total_coeff`, `trailing_ones`) using the table
//!    selected by predicted neighbour count `nC` (§9.2.1.1).
//! 2. Read each trailing-one sign as 1 bit (§9.2.2).
//! 3. Read remaining levels via `level_prefix` + `level_suffix` (§9.2.2).
//! 4. Read `total_zeros` (§9.2.3) followed by per-coefficient `run_before`
//!    (§9.2.3.2).
//! 5. Reverse-scan the levels into coded-order positions, then apply zig-zag
//!    inverse mapping (Table 8-12) to produce a 4×4 raster of coefficients.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;

/// 4×4 zig-zag scan (frame coding, §8.5.6 Table 8-12). Maps coded-order
/// index → raster index.
pub const ZIGZAG_4X4: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

/// Inverse of `ZIGZAG_4X4` — maps raster index → coded-order index.
pub const INVERSE_ZIGZAG_4X4: [usize; 16] = {
    let mut out = [0usize; 16];
    let mut i = 0;
    while i < 16 {
        out[ZIGZAG_4X4[i]] = i;
        i += 1;
    }
    out
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// Standard 4×4 luma block (16 coefficients).
    Luma4x4,
    /// Intra16×16 DC luma block (16 coefficients fed to a Hadamard).
    Luma16x16Dc,
    /// Intra16×16 AC block (15 coefficients — DC excluded).
    Luma16x16Ac,
    /// Chroma AC block (15 coefficients — DC excluded).
    ChromaAc,
    /// 4:2:0 chroma DC block (4 coefficients).
    ChromaDc2x2,
}

impl BlockKind {
    pub fn max_num_coeff(self) -> usize {
        match self {
            BlockKind::Luma4x4 | BlockKind::Luma16x16Dc => 16,
            BlockKind::Luma16x16Ac | BlockKind::ChromaAc => 15,
            BlockKind::ChromaDc2x2 => 4,
        }
    }
}

/// Decoded residual block.
#[derive(Clone, Debug)]
pub struct ResidualBlock {
    /// Coefficients in raster order. The exact layout depends on `kind`:
    /// - `Luma4x4` / `Luma16x16Dc`: 4×4 raster (`row*4 + col`).
    /// - `Luma16x16Ac` / `ChromaAc`: 4×4 raster with AC-only positions
    ///   (coefficient at raster index 0 is always 0; the DC of the macroblock
    ///   lives in a separate `Luma16x16Dc`/`ChromaDc2x2` block).
    /// - `ChromaDc2x2`: positions 0..=3, others are zero.
    pub coeffs: [i32; 16],
    pub total_coeff: u32,
    pub trailing_ones: u32,
}

impl Default for ResidualBlock {
    #[inline]
    fn default() -> Self {
        Self {
            coeffs: [0; 16],
            total_coeff: 0,
            trailing_ones: 0,
        }
    }
}

/// Decode one residual block.
///
/// `nc` is the predicted neighbour count from §9.2.1.1 (use the average of
/// left + top neighbour totals, with the standard rounding rules); for chroma
/// DC pass `BlockKind::ChromaDc2x2` (the function ignores `nc` then).
pub fn decode_residual_block(
    br: &mut BitReader<'_>,
    nc: i32,
    kind: BlockKind,
) -> Result<ResidualBlock> {
    let max_num_coeff = kind.max_num_coeff();

    let (total_coeff, trailing_ones) = read_coeff_token(br, nc, kind)?;
    let mut block = ResidualBlock {
        total_coeff,
        trailing_ones,
        ..Default::default()
    };
    if total_coeff == 0 {
        return Ok(block);
    }
    debug_assert!(total_coeff as usize <= max_num_coeff);
    debug_assert!(trailing_ones <= 3 && trailing_ones <= total_coeff);

    // Levels — §9.2.2.
    let mut levels = [0i32; 16];
    // Trailing ones: signs only.
    for i in 0..trailing_ones as usize {
        let sign = br.read_u1()?;
        levels[i] = if sign == 1 { -1 } else { 1 };
    }
    // Remaining levels.
    let mut suffix_length: u32 = if total_coeff > 10 && trailing_ones < 3 {
        1
    } else {
        0
    };
    for i in trailing_ones as usize..total_coeff as usize {
        let level = read_level(br, &mut suffix_length, i, trailing_ones)?;
        levels[i] = level;
    }

    // Runs — §9.2.3.
    let mut runs = [0u32; 16];
    let zeros_left = if (total_coeff as usize) < max_num_coeff {
        read_total_zeros(br, total_coeff, kind)?
    } else {
        0
    };
    let mut zl = zeros_left;
    for i in 0..total_coeff as usize - 1 {
        let run = if zl > 0 {
            let r = read_run_before(br, zl)?;
            zl = zl.saturating_sub(r);
            r
        } else {
            0
        };
        runs[i] = run;
    }
    runs[total_coeff as usize - 1] = zl;

    // Place levels into coded-order array. `levels[0]` is the
    // highest-frequency non-zero coefficient and `runs[0]` is the run that
    // precedes it (in scan order). The spec algorithm (§9.2.3.2) walks i
    // from TC-1 down to 0, accumulating run+1 each step.
    let coded_len = total_coeff as usize + zeros_left as usize;
    debug_assert!(coded_len <= max_num_coeff);
    let mut coded = [0i32; 16];
    let mut coeff_num: i32 = -1;
    for i in (0..total_coeff as usize).rev() {
        coeff_num += runs[i] as i32 + 1;
        let target = coeff_num as usize;
        if target >= max_num_coeff {
            return Err(Error::invalid("h264 cavlc: coefficient index overflow"));
        }
        coded[target] = levels[i];
    }

    // Map coded order → raster according to block kind.
    match kind {
        BlockKind::Luma4x4 | BlockKind::Luma16x16Dc => {
            for i in 0..16 {
                let raster = ZIGZAG_4X4[i];
                block.coeffs[raster] = coded[i];
            }
        }
        BlockKind::Luma16x16Ac | BlockKind::ChromaAc => {
            // Coded indices 0..=14 map to raster indices ZIGZAG_4X4[1..=15];
            // the DC slot (raster 0) stays zero — it's filled separately
            // from the DC block.
            for i in 0..15 {
                let raster = ZIGZAG_4X4[i + 1];
                block.coeffs[raster] = coded[i];
            }
        }
        BlockKind::ChromaDc2x2 => {
            for i in 0..4 {
                block.coeffs[i] = coded[i];
            }
        }
    }

    Ok(block)
}

// ---------------------------------------------------------------------------
// coeff_token (Table 9-5)
// ---------------------------------------------------------------------------
//
// Indexed by [nC_class][TotalCoeff*4 + TrailingOnes]. nC_class:
//   0 = (0 <= nC < 2)
//   1 = (2 <= nC < 4)
//   2 = (4 <= nC < 8)
//   3 = (nC == -2 or nC >= 8) -- FLC
//
// All four tables share the layout: row 0 is (TC=0,T1=0); the meaningless
// (TC < T1) entries have len=0.

const COEFF_TOKEN_LEN: [[u8; 68]; 4] = [
    [
        1, 0, 0, 0, 6, 2, 0, 0, 8, 6, 3, 0, 9, 8, 7, 5, 10, 9, 8, 6, 11, 10, 9, 7, 13, 11, 10, 8,
        13, 13, 11, 9, 13, 13, 13, 10, 14, 14, 13, 11, 14, 14, 14, 13, 15, 15, 14, 14, 15, 15, 15,
        14, 16, 15, 15, 15, 16, 16, 16, 15, 16, 16, 16, 16, 16, 16, 16, 16,
    ],
    [
        2, 0, 0, 0, 6, 2, 0, 0, 6, 5, 3, 0, 7, 6, 6, 4, 8, 6, 6, 4, 8, 7, 7, 5, 9, 8, 8, 6, 11, 9,
        9, 6, 11, 11, 11, 7, 12, 11, 11, 9, 12, 12, 12, 11, 12, 12, 12, 11, 13, 13, 13, 12, 13, 13,
        13, 13, 13, 14, 13, 13, 14, 14, 14, 13, 14, 14, 14, 14,
    ],
    [
        4, 0, 0, 0, 6, 4, 0, 0, 6, 5, 4, 0, 6, 5, 5, 4, 7, 5, 5, 4, 7, 5, 5, 4, 7, 6, 6, 4, 7, 6,
        6, 4, 8, 7, 7, 5, 8, 8, 7, 6, 9, 8, 8, 7, 9, 9, 8, 8, 9, 9, 9, 8, 10, 9, 9, 9, 10, 10, 10,
        10, 10, 10, 10, 10, 10, 10, 10, 10,
    ],
    [
        6, 0, 0, 0, 6, 6, 0, 0, 6, 6, 6, 0, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
        6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
        6, 6, 6, 6, 6, 6, 6, 6,
    ],
];

const COEFF_TOKEN_BITS: [[u8; 68]; 4] = [
    [
        1, 0, 0, 0, 5, 1, 0, 0, 7, 4, 1, 0, 7, 6, 5, 3, 7, 6, 5, 3, 7, 6, 5, 4, 15, 6, 5, 4, 11,
        14, 5, 4, 8, 10, 13, 4, 15, 14, 9, 4, 11, 10, 13, 12, 15, 14, 9, 12, 11, 10, 13, 8, 15, 1,
        9, 12, 11, 14, 13, 8, 7, 10, 9, 12, 4, 6, 5, 8,
    ],
    [
        3, 0, 0, 0, 11, 2, 0, 0, 7, 7, 3, 0, 7, 10, 9, 5, 7, 6, 5, 4, 4, 6, 5, 6, 7, 6, 5, 8, 15,
        6, 5, 4, 11, 14, 13, 4, 15, 10, 9, 4, 11, 14, 13, 12, 8, 10, 9, 8, 15, 14, 13, 12, 11, 10,
        9, 12, 7, 11, 6, 8, 9, 8, 10, 1, 7, 6, 5, 4,
    ],
    [
        15, 0, 0, 0, 15, 14, 0, 0, 11, 15, 13, 0, 8, 12, 14, 12, 15, 10, 11, 11, 11, 8, 9, 10, 9,
        14, 13, 9, 8, 10, 9, 8, 15, 14, 13, 13, 11, 14, 10, 12, 15, 10, 13, 12, 11, 14, 9, 12, 8,
        10, 13, 8, 13, 7, 9, 12, 9, 12, 11, 10, 5, 8, 7, 6, 1, 4, 3, 2,
    ],
    [
        3, 0, 0, 0, 0, 1, 0, 0, 4, 5, 6, 0, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
        22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44,
        45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63,
    ],
];

// Chroma DC coeff_token (4:2:0) — Table 9-5(d). 5 rows × 4 cols (TC*4+T1).
const CHROMA_DC_COEFF_TOKEN_LEN: [u8; 20] =
    [2, 0, 0, 0, 6, 1, 0, 0, 6, 6, 3, 0, 6, 7, 7, 6, 6, 8, 8, 7];
const CHROMA_DC_COEFF_TOKEN_BITS: [u8; 20] =
    [1, 0, 0, 0, 7, 1, 0, 0, 4, 6, 1, 0, 3, 3, 2, 5, 2, 3, 2, 0];

fn nc_class(nc: i32) -> usize {
    if nc < 0 {
        // chroma 2x2 DC handled separately, this is only used as a guard.
        0
    } else if nc < 2 {
        0
    } else if nc < 4 {
        1
    } else if nc < 8 {
        2
    } else {
        3
    }
}

/// Read coeff_token. Returns `(total_coeff, trailing_ones)`.
pub fn read_coeff_token(br: &mut BitReader<'_>, nc: i32, kind: BlockKind) -> Result<(u32, u32)> {
    if matches!(kind, BlockKind::ChromaDc2x2) {
        let (idx, _) =
            lookup_in_table_pairs(br, &CHROMA_DC_COEFF_TOKEN_LEN, &CHROMA_DC_COEFF_TOKEN_BITS)?;
        return Ok(((idx / 4) as u32, (idx % 4) as u32));
    }
    let cls = nc_class(nc);
    let (idx, _) = lookup_in_table_pairs(br, &COEFF_TOKEN_LEN[cls], &COEFF_TOKEN_BITS[cls])?;
    Ok(((idx / 4) as u32, (idx % 4) as u32))
}

/// Walk a (length-array, code-array) pair returned by FFmpeg-style tables.
/// Returns the index in the array of the matching entry plus its bit length.
fn lookup_in_table_pairs(br: &mut BitReader<'_>, lens: &[u8], bits: &[u8]) -> Result<(usize, u32)> {
    debug_assert_eq!(lens.len(), bits.len());
    // Find max nonzero length to peek.
    let mut max_len = 0u32;
    for &l in lens.iter() {
        if l as u32 > max_len {
            max_len = l as u32;
        }
    }
    if max_len == 0 {
        return Err(Error::invalid("h264 cavlc: empty VLC table"));
    }
    let peek = br.peek_u32_lax(max_len);
    for i in 0..lens.len() {
        let l = lens[i] as u32;
        if l == 0 {
            continue;
        }
        let code = bits[i] as u32;
        let shift = max_len - l;
        if (peek >> shift) == code {
            br.skip(l)?;
            return Ok((i, l));
        }
    }
    Err(Error::invalid("h264 cavlc: no matching VLC code"))
}

// ---------------------------------------------------------------------------
// total_zeros (Tables 9-7 / 9-9.1 luma, 9-9.2 chroma DC)
// ---------------------------------------------------------------------------

const TOTAL_ZEROS_LEN: [&[u8]; 15] = [
    &[1, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 9],
    &[3, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 6, 6, 6, 6],
    &[4, 3, 3, 3, 4, 4, 3, 3, 4, 5, 5, 6, 5, 6],
    &[5, 3, 4, 4, 3, 3, 3, 4, 3, 4, 5, 5, 5],
    &[4, 4, 4, 3, 3, 3, 3, 3, 4, 5, 4, 5],
    &[6, 5, 3, 3, 3, 3, 3, 3, 4, 3, 6],
    &[6, 5, 3, 3, 3, 2, 3, 4, 3, 6],
    &[6, 4, 5, 3, 2, 2, 3, 3, 6],
    &[6, 6, 4, 2, 2, 3, 2, 5],
    &[5, 5, 3, 2, 2, 2, 4],
    &[4, 4, 3, 3, 1, 3],
    &[4, 4, 2, 1, 3],
    &[3, 3, 1, 2],
    &[2, 2, 1],
    &[1, 1],
];

const TOTAL_ZEROS_BITS: [&[u8]; 15] = [
    &[1, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 1],
    &[7, 6, 5, 4, 3, 5, 4, 3, 2, 3, 2, 3, 2, 1, 0],
    &[5, 7, 6, 5, 4, 3, 4, 3, 2, 3, 2, 1, 1, 0],
    &[3, 7, 5, 4, 6, 5, 4, 3, 3, 2, 2, 1, 0],
    &[5, 4, 3, 7, 6, 5, 4, 3, 2, 1, 1, 0],
    &[1, 1, 7, 6, 5, 4, 3, 2, 1, 1, 0],
    &[1, 1, 5, 4, 3, 3, 2, 1, 1, 0],
    &[1, 1, 1, 3, 3, 2, 2, 1, 0],
    &[1, 0, 1, 3, 2, 1, 1, 1],
    &[1, 0, 1, 3, 2, 1, 1],
    &[0, 1, 1, 2, 1, 3],
    &[0, 1, 1, 1, 1],
    &[0, 1, 1, 1],
    &[0, 1, 1],
    &[0, 1],
];

const CHROMA_DC_TOTAL_ZEROS_LEN: [[u8; 4]; 3] = [[1, 2, 3, 3], [1, 2, 2, 0], [1, 1, 0, 0]];
const CHROMA_DC_TOTAL_ZEROS_BITS: [[u8; 4]; 3] = [[1, 1, 1, 0], [1, 1, 0, 0], [1, 0, 0, 0]];

fn read_total_zeros(br: &mut BitReader<'_>, total_coeff: u32, kind: BlockKind) -> Result<u32> {
    let row = (total_coeff - 1) as usize;
    if matches!(kind, BlockKind::ChromaDc2x2) {
        let lens = &CHROMA_DC_TOTAL_ZEROS_LEN[row];
        let bits = &CHROMA_DC_TOTAL_ZEROS_BITS[row];
        let (idx, _) = lookup_in_table_pairs(br, lens, bits)?;
        return Ok(idx as u32);
    }
    let lens = TOTAL_ZEROS_LEN[row];
    let bits = TOTAL_ZEROS_BITS[row];
    let (idx, _) = lookup_in_table_pairs(br, lens, bits)?;
    Ok(idx as u32)
}

// ---------------------------------------------------------------------------
// run_before (Table 9-10)
// ---------------------------------------------------------------------------

const RUN_LEN: [&[u8]; 7] = [
    &[1, 1],
    &[1, 2, 2],
    &[2, 2, 2, 2],
    &[2, 2, 2, 3, 3],
    &[2, 2, 3, 3, 3, 3],
    &[2, 3, 3, 3, 3, 3, 3],
    &[3, 3, 3, 3, 3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11],
];
const RUN_BITS: [&[u8]; 7] = [
    &[1, 0],
    &[1, 1, 0],
    &[3, 2, 1, 0],
    &[3, 2, 1, 1, 0],
    &[3, 2, 3, 2, 1, 0],
    &[3, 0, 1, 3, 2, 5, 4],
    &[7, 6, 5, 4, 3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1],
];

fn read_run_before(br: &mut BitReader<'_>, zeros_left: u32) -> Result<u32> {
    let row = zeros_left.min(7) as usize - 1;
    let lens = RUN_LEN[row];
    let bits = RUN_BITS[row];
    let (idx, _) = lookup_in_table_pairs(br, lens, bits)?;
    Ok(idx as u32)
}

// ---------------------------------------------------------------------------
// level_prefix + level_suffix (§9.2.2)
// ---------------------------------------------------------------------------

fn read_level(
    br: &mut BitReader<'_>,
    suffix_length: &mut u32,
    coeff_idx_among_nontrailing: usize,
    trailing_ones: u32,
) -> Result<i32> {
    // §9.2.2.1: level_prefix is unary (zeros terminated by a 1).
    let mut level_prefix = 0u32;
    loop {
        if level_prefix > 25 {
            return Err(Error::invalid("h264 cavlc: level_prefix overflow"));
        }
        let b = br.read_u1()?;
        if b == 1 {
            break;
        }
        level_prefix += 1;
    }

    // §9.2.2.1 — derivation of levelSuffixSize.
    let level_suffix_size = if level_prefix == 14 && *suffix_length == 0 {
        4u32
    } else if level_prefix >= 15 {
        level_prefix - 3
    } else {
        *suffix_length
    };

    let level_suffix = if level_suffix_size > 0 {
        br.read_u32(level_suffix_size)? as i32
    } else {
        0
    };

    // §9.2.2.1 levelCode derivation.
    let mut level_code: i32 = (level_prefix.min(15) << *suffix_length) as i32 + level_suffix;
    if level_prefix >= 14 && *suffix_length == 0 {
        level_code += 15;
    }
    if level_prefix >= 15 {
        // For 8-bit profile, levelSuffixSize = level_prefix - 3, so
        // (1 << levelSuffixSize) - 4096 with 12-bit suffix = 0.
        level_code += (1i32 << (level_prefix - 3)) - 4096;
    }
    if coeff_idx_among_nontrailing == trailing_ones as usize && trailing_ones < 3 {
        level_code += 2;
    }

    let level = if level_code & 1 == 0 {
        (level_code + 2) >> 1
    } else {
        -((level_code + 1) >> 1)
    };

    // suffix_length update §9.2.2.1.
    if *suffix_length == 0 {
        *suffix_length = 1;
    }
    if level.unsigned_abs() > (3u32 << (*suffix_length - 1)) && *suffix_length < 6 {
        *suffix_length += 1;
    }

    Ok(level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coeff_token_zero_token_nc_low() {
        // NC<2: '1' (1 bit) means TC=0, T1=0.
        let data = [0x80];
        let mut br = BitReader::new(&data);
        let (tc, t1) = read_coeff_token(&mut br, 0, BlockKind::Luma4x4).unwrap();
        assert_eq!((tc, t1), (0, 0));
    }

    #[test]
    fn run_before_zl1_examples() {
        // zerosLeft=1: '1' -> 0, '0' -> 1.
        let data = [0b1000_0000u8];
        let mut br = BitReader::new(&data);
        assert_eq!(read_run_before(&mut br, 1).unwrap(), 0);
        let data = [0b0000_0000u8];
        let mut br = BitReader::new(&data);
        assert_eq!(read_run_before(&mut br, 1).unwrap(), 1);
    }

    #[test]
    fn coeff_token_tc1_t1_nc_low() {
        // NC<2: pattern "000101" = (TC=1, T1=0) per Table 9-5.
        let data = [0b0001_0100u8];
        let mut br = BitReader::new(&data);
        let (tc, t1) = read_coeff_token(&mut br, 0, BlockKind::Luma4x4).unwrap();
        assert_eq!((tc, t1), (1, 0));
        // And "01" (2 bits) = (TC=1, T1=1).
        let data = [0b0100_0000u8];
        let mut br = BitReader::new(&data);
        let (tc, t1) = read_coeff_token(&mut br, 0, BlockKind::Luma4x4).unwrap();
        assert_eq!((tc, t1), (1, 1));
    }

    #[test]
    fn total_zeros_tc1_zero() {
        // Table 9-7 row tc=1, zeros=0 → bits=1, len=1 → "1".
        let data = [0x80];
        let mut br = BitReader::new(&data);
        assert_eq!(read_total_zeros(&mut br, 1, BlockKind::Luma4x4).unwrap(), 0);
    }
}
