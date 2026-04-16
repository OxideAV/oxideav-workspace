//! MPEG-1 Audio Layer II bit-allocation tables (ISO/IEC 11172-3 §2.4, Annex B).
//!
//! The tables describe, per-subband, the number of bits used to transmit the
//! allocation index (`nbal`) and, for each allocation value, the number of
//! quantisation levels and the per-sample codeword width. Layer II selects
//! one of four tables (Table 3-B.2a/b/c/d) according to the (sample_rate,
//! bitrate, channel_mode) triplet, as encoded in Table 3-B.2.
//!
//! Table layout is adapted from libmpg123's `l2tables.h` flat
//! `struct al_table { bits; d }` representation — which is a faithful
//! transcription of the ISO tables. Numerical constants in an ISO standard
//! are not copyrightable; the representation here was re-derived by this
//! author and verified bit-for-bit against the spec.
//!
//! # Entry semantics
//!
//! `AllocEntry { bits, d }`:
//! - The first entry of each subband block has `bits = nbal` (number of
//!   bits carrying the allocation index for this subband). `d` is unused
//!   on the header entry (always 0 in the spec).
//! - Subsequent `(1 << nbal) - 1` entries describe the quantisation class
//!   selected by allocation indices 1, 2, …:
//!     * `bits`: codeword width in bits.
//!     * `d`: if positive, the codeword is a packed triplet for the
//!       (3|5|9)-level grouped quantiser — `d` is the grouping size (3, 5
//!       or 9). If negative, the codeword is a plain `|d| + 1`-level
//!       unsigned integer; the decoder subtracts `|d|` to recentre the
//!       sample around zero (equivalent to adding `d`).
//! - Allocation index 0 always means "no samples transmitted" (this entry
//!   is implicit — the block stores `nbal + (2^nbal − 1)` entries since
//!   alloc=0 needs no row; libmpg123 stores it the same way).

/// One entry in the allocation table. See module docs for semantics.
#[derive(Clone, Copy, Debug)]
pub struct AllocEntry {
    /// For the first entry of a subband block: number of bits used for the
    /// allocation index (`nbal`). For subsequent entries: codeword width.
    pub bits: i8,
    /// For subsequent entries: grouping size (3/5/9) when positive, or
    /// `-(levels − 1) / 2` when negative (equivalently, the additive
    /// centring offset for a `levels`-level unsigned codeword).
    pub d: i16,
}

/// A Layer-II allocation table: an sblimit, a flat sequence of entries per
/// subband, and a per-subband index pointing at the header entry of each
/// subband's block in the flat array.
pub struct AllocTable {
    pub sblimit: usize,
    /// Flat entries `[sb0_header, sb0_alloc1, ..., sb1_header, ...]`.
    pub entries: &'static [AllocEntry],
    /// Byte-offsets into `entries` for each subband. `offsets[sb]` is the
    /// index of subband `sb`'s header entry; entries `offsets[sb]+1..` up
    /// through `offsets[sb] + (1 << nbal)` are the class entries 1..`2^nbal`.
    pub offsets: &'static [usize],
}

impl AllocTable {
    /// `nbal[sb]` — width in bits of the allocation index for subband `sb`.
    pub fn nbal(&self, sb: usize) -> u32 {
        self.entries[self.offsets[sb]].bits as u32
    }

    /// For allocation index `a` (>= 1) of subband `sb`, return
    /// `(codeword_bits, d)` as defined above.
    pub fn class(&self, sb: usize, a: u32) -> (u32, i32) {
        let e = self.entries[self.offsets[sb] + a as usize];
        (e.bits as u32, e.d as i32)
    }
}

// ---------------------------------------------------------------------------
// Compact macro that, given nbal and a list of `(bits, d)` per allocation
// value 1..(2^nbal − 1), emits the (2^nbal) flat entries (header + rows).
// Allocation 0 ("no samples") is implicit and therefore contributes no row
// — so a block of nbal=4 has 1 header + 15 rows = 16 entries.
// ---------------------------------------------------------------------------

macro_rules! ae {
    ($b:expr, $d:expr) => {
        AllocEntry { bits: $b, d: $d }
    };
}

// Row templates reused across subbands. Each of these is the (2^nbal − 1)
// non-zero allocation classes plus the nbal header entry prepended.

/// Row 0–1 of alloc_0 (B.2a) and alloc_1 (B.2b). 4-bit nbal, 15 non-zero
/// classes. Excludes the {10,9} (9-level grouping) — this is the
/// "standard 15-level alloc ladder" used for subbands 0..2.
const BLOCK_4BIT_NO9: [AllocEntry; 16] = [
    ae!(4, 0),
    ae!(5, 3),
    ae!(3, -3),
    ae!(4, -7),
    ae!(5, -15),
    ae!(6, -31),
    ae!(7, -63),
    ae!(8, -127),
    ae!(9, -255),
    ae!(10, -511),
    ae!(11, -1023),
    ae!(12, -2047),
    ae!(13, -4095),
    ae!(14, -8191),
    ae!(15, -16383),
    ae!(16, -32767),
];

/// Row 3–10 of alloc_0/alloc_1: 4-bit nbal, 15 non-zero classes including
/// the {10,9} and {7,5} grouped quantisers. Used for mid subbands.
const BLOCK_4BIT_WITH_GRP: [AllocEntry; 16] = [
    ae!(4, 0),
    ae!(5, 3),
    ae!(7, 5),
    ae!(3, -3),
    ae!(10, 9),
    ae!(4, -7),
    ae!(5, -15),
    ae!(6, -31),
    ae!(7, -63),
    ae!(8, -127),
    ae!(9, -255),
    ae!(10, -511),
    ae!(11, -1023),
    ae!(12, -2047),
    ae!(13, -4095),
    ae!(16, -32767),
];

/// Rows with nbal = 3 (alloc_0/alloc_1 upper subbands): 7 non-zero classes.
const BLOCK_3BIT: [AllocEntry; 8] = [
    ae!(3, 0),
    ae!(5, 3),
    ae!(7, 5),
    ae!(3, -3),
    ae!(10, 9),
    ae!(4, -7),
    ae!(5, -15),
    ae!(16, -32767),
];

/// Rows with nbal = 2 (alloc_0 highest subbands): 3 non-zero classes.
const BLOCK_2BIT_ALLOC0: [AllocEntry; 4] = [ae!(2, 0), ae!(5, 3), ae!(7, 5), ae!(16, -32767)];

/// Rows with nbal = 2 (alloc_1 highest subbands): 3 non-zero classes — same
/// as alloc_0's 2-bit block.
const BLOCK_2BIT_ALLOC1: [AllocEntry; 4] = [ae!(2, 0), ae!(5, 3), ae!(7, 5), ae!(16, -32767)];

/// 4-bit block for alloc_2/alloc_3 (48 kHz low-bitrate). 15 non-zero
/// classes; the 3-level quantiser (d=-3) is *absent* here per ISO.
const BLOCK_4BIT_48K: [AllocEntry; 16] = [
    ae!(4, 0),
    ae!(5, 3),
    ae!(7, 5),
    ae!(10, 9),
    ae!(4, -7),
    ae!(5, -15),
    ae!(6, -31),
    ae!(7, -63),
    ae!(8, -127),
    ae!(9, -255),
    ae!(10, -511),
    ae!(11, -1023),
    ae!(12, -2047),
    ae!(13, -4095),
    ae!(14, -8191),
    ae!(15, -16383),
];

/// 3-bit block for alloc_2/alloc_3 upper subbands: 7 non-zero classes.
const BLOCK_3BIT_48K: [AllocEntry; 8] = [
    ae!(3, 0),
    ae!(5, 3),
    ae!(7, 5),
    ae!(10, 9),
    ae!(4, -7),
    ae!(5, -15),
    ae!(6, -31),
    ae!(7, -63),
];

// ---------------------------------------------------------------------------
// Table B.2a (alloc_0) — 32 kHz / 44.1 kHz, sblimit = 27.
//
//   sb  0..2    nbal = 4  (BLOCK_4BIT_NO9)
//   sb  3..10   nbal = 4  (BLOCK_4BIT_WITH_GRP)
//   sb 11..22   nbal = 3  (BLOCK_3BIT)
//   sb 23..26   nbal = 2  (BLOCK_2BIT_ALLOC0)
// ---------------------------------------------------------------------------

/// Build the flat entry table and offsets for alloc_0 (sblimit=27).
const fn build_alloc_0() -> ([AllocEntry; ALLOC0_LEN], [usize; 27]) {
    let mut out = [AllocEntry { bits: 0, d: 0 }; ALLOC0_LEN];
    let mut off = [0usize; 27];
    let mut pos = 0usize;
    let mut sb = 0usize;
    // sb 0..2: 4-bit no-9 (3 subbands × 16 entries = 48).
    while sb < 3 {
        off[sb] = pos;
        let mut k = 0;
        while k < 16 {
            out[pos] = BLOCK_4BIT_NO9[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    // sb 3..10: 4-bit with-grp (8 × 16 = 128).
    while sb < 11 {
        off[sb] = pos;
        let mut k = 0;
        while k < 16 {
            out[pos] = BLOCK_4BIT_WITH_GRP[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    // sb 11..22: 3-bit (12 × 8 = 96).
    while sb < 23 {
        off[sb] = pos;
        let mut k = 0;
        while k < 8 {
            out[pos] = BLOCK_3BIT[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    // sb 23..26: 2-bit (4 × 4 = 16).
    while sb < 27 {
        off[sb] = pos;
        let mut k = 0;
        while k < 4 {
            out[pos] = BLOCK_2BIT_ALLOC0[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    (out, off)
}

const ALLOC0_LEN: usize = 3 * 16 + 8 * 16 + 12 * 8 + 4 * 4; // = 48 + 128 + 96 + 16 = 288
const ALLOC0: ([AllocEntry; ALLOC0_LEN], [usize; 27]) = build_alloc_0();
pub const TABLE_B2A: AllocTable = AllocTable {
    sblimit: 27,
    entries: &ALLOC0.0,
    offsets: &ALLOC0.1,
};

// ---------------------------------------------------------------------------
// Table B.2b (alloc_1) — 32 kHz / 44.1 kHz, sblimit = 30.
//
//   sb  0..2    nbal = 4  (BLOCK_4BIT_NO9)
//   sb  3..10   nbal = 4  (BLOCK_4BIT_WITH_GRP)
//   sb 11..22   nbal = 3  (BLOCK_3BIT)
//   sb 23..29   nbal = 2  (BLOCK_2BIT_ALLOC1)   — 7 subbands.
// ---------------------------------------------------------------------------

const ALLOC1_LEN: usize = 3 * 16 + 8 * 16 + 12 * 8 + 7 * 4; // = 48 + 128 + 96 + 28 = 300

const fn build_alloc_1() -> ([AllocEntry; ALLOC1_LEN], [usize; 30]) {
    let mut out = [AllocEntry { bits: 0, d: 0 }; ALLOC1_LEN];
    let mut off = [0usize; 30];
    let mut pos = 0usize;
    let mut sb = 0usize;
    while sb < 3 {
        off[sb] = pos;
        let mut k = 0;
        while k < 16 {
            out[pos] = BLOCK_4BIT_NO9[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    while sb < 11 {
        off[sb] = pos;
        let mut k = 0;
        while k < 16 {
            out[pos] = BLOCK_4BIT_WITH_GRP[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    while sb < 23 {
        off[sb] = pos;
        let mut k = 0;
        while k < 8 {
            out[pos] = BLOCK_3BIT[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    while sb < 30 {
        off[sb] = pos;
        let mut k = 0;
        while k < 4 {
            out[pos] = BLOCK_2BIT_ALLOC1[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    (out, off)
}

const ALLOC1: ([AllocEntry; ALLOC1_LEN], [usize; 30]) = build_alloc_1();
pub const TABLE_B2B: AllocTable = AllocTable {
    sblimit: 30,
    entries: &ALLOC1.0,
    offsets: &ALLOC1.1,
};

// ---------------------------------------------------------------------------
// Table B.2c (alloc_2) — 48 kHz low-bitrate, sblimit = 8.
//
//   sb 0..1  nbal = 4   (BLOCK_4BIT_48K — 2 × 16)
//   sb 2..7  nbal = 3   (BLOCK_3BIT_48K — 6 × 8)
// ---------------------------------------------------------------------------

const ALLOC2_LEN: usize = 2 * 16 + 6 * 8; // = 32 + 48 = 80

const fn build_alloc_2() -> ([AllocEntry; ALLOC2_LEN], [usize; 8]) {
    let mut out = [AllocEntry { bits: 0, d: 0 }; ALLOC2_LEN];
    let mut off = [0usize; 8];
    let mut pos = 0usize;
    let mut sb = 0usize;
    while sb < 2 {
        off[sb] = pos;
        let mut k = 0;
        while k < 16 {
            out[pos] = BLOCK_4BIT_48K[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    while sb < 8 {
        off[sb] = pos;
        let mut k = 0;
        while k < 8 {
            out[pos] = BLOCK_3BIT_48K[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    (out, off)
}

const ALLOC2: ([AllocEntry; ALLOC2_LEN], [usize; 8]) = build_alloc_2();
pub const TABLE_B2C: AllocTable = AllocTable {
    sblimit: 8,
    entries: &ALLOC2.0,
    offsets: &ALLOC2.1,
};

// ---------------------------------------------------------------------------
// Table B.2d (alloc_3) — 48 kHz low-bitrate, sblimit = 12.
//
//   sb 0..1  nbal = 4   (BLOCK_4BIT_48K — 2 × 16)
//   sb 2..11 nbal = 3   (BLOCK_3BIT_48K — 10 × 8)
// ---------------------------------------------------------------------------

const ALLOC3_LEN: usize = 2 * 16 + 10 * 8; // = 32 + 80 = 112

const fn build_alloc_3() -> ([AllocEntry; ALLOC3_LEN], [usize; 12]) {
    let mut out = [AllocEntry { bits: 0, d: 0 }; ALLOC3_LEN];
    let mut off = [0usize; 12];
    let mut pos = 0usize;
    let mut sb = 0usize;
    while sb < 2 {
        off[sb] = pos;
        let mut k = 0;
        while k < 16 {
            out[pos] = BLOCK_4BIT_48K[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    while sb < 12 {
        off[sb] = pos;
        let mut k = 0;
        while k < 8 {
            out[pos] = BLOCK_3BIT_48K[k];
            pos += 1;
            k += 1;
        }
        sb += 1;
    }
    (out, off)
}

const ALLOC3: ([AllocEntry; ALLOC3_LEN], [usize; 12]) = build_alloc_3();
pub const TABLE_B2D: AllocTable = AllocTable {
    sblimit: 12,
    entries: &ALLOC3.0,
    offsets: &ALLOC3.1,
};

// ---------------------------------------------------------------------------
// Table selection — ISO/IEC 11172-3 Table 3-B.2 (§2.4.2.3).
//
// Maps (sample_rate_index, is_stereo, bitrate_index) → one of {B.2a, B.2b,
// B.2c, B.2d}. The constants here are the transcription of libmpg123's
// `translate[]` lookup, which is a direct expression of Table 3-B.2.
//
// Bitrate_index: 1..=14 (0 = free, 15 = reserved — both rejected upstream).
// Sample-frequency index: 0 = 44.1 kHz, 1 = 48 kHz, 2 = 32 kHz.
//
// For joint-stereo and dual-channel, the decoder uses the stereo column
// (is_stereo = true).
// ---------------------------------------------------------------------------

/// Decoder-internal index: 0 = B.2a, 1 = B.2b, 2 = B.2c, 3 = B.2d.
pub fn select_alloc_table_index(srf_index: u32, stereo: bool, bitrate_index: u32) -> u32 {
    // translate[sampling_frequency_index][mode_col][bitrate_index]
    //   where mode_col 0 = stereo (or joint/dual), 1 = mono.
    // These three tables correspond respectively to 44.1, 48 and 32 kHz.
    const TRANSLATE: [[[u8; 16]; 2]; 3] = [
        // 0 = 44.1 kHz
        [
            // stereo
            [0, 2, 2, 2, 2, 2, 2, 0, 0, 0, 1, 1, 1, 1, 1, 0],
            // mono
            [0, 2, 2, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0],
        ],
        // 1 = 48 kHz
        [
            // stereo
            [0, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            // mono
            [0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ],
        // 2 = 32 kHz
        [
            // stereo
            [0, 3, 3, 3, 3, 3, 3, 0, 0, 0, 1, 1, 1, 1, 1, 0],
            // mono
            [0, 3, 3, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0],
        ],
    ];
    let mode_col = if stereo { 0 } else { 1 };
    TRANSLATE[srf_index as usize][mode_col][bitrate_index as usize] as u32
}

/// Returns the allocation table for the given (sample_rate, stereo,
/// bitrate_index). `bitrate_index` is the raw 4-bit field from the header
/// (1..=14 valid — 0 is free-format, 15 is reserved).
pub fn select_alloc_table(
    sample_rate: u32,
    stereo: bool,
    bitrate_index: u32,
) -> &'static AllocTable {
    let srf = match sample_rate {
        44_100 => 0,
        48_000 => 1,
        32_000 => 2,
        _ => 0,
    };
    let t = select_alloc_table_index(srf, stereo, bitrate_index);
    match t {
        0 => &TABLE_B2A,
        1 => &TABLE_B2B,
        2 => &TABLE_B2C,
        3 => &TABLE_B2D,
        _ => &TABLE_B2A,
    }
}

// ---------------------------------------------------------------------------
// Requantisation factor table (ISO/IEC 11172-3 Table 3-B.1).
//
// 64 scalefactor indices (0..=62 valid; 63 is reserved) map to a floating
// point magnitude 2.0 * 2^(-index/3). Index 0 → 2.0, index 63 → ~4.7e-7.
// ---------------------------------------------------------------------------

/// Decoded scalefactor magnitudes. `SCALEFACTORS[i] = 2.0 * 2^(-i/3)`.
pub fn scalefactor_magnitude(index: u8) -> f32 {
    SCALEFACTORS[index as usize]
}

const SCALEFACTORS: [f32; 64] = compute_scalefactors();

const fn compute_scalefactors() -> [f32; 64] {
    // 2.0 * 2^(-i/3). Evaluate at compile time using the integer triplet
    // representation. For i = 3k + r with r ∈ {0, 1, 2}:
    //   value = 2 * 2^(-k) * 2^(-r/3) = 2^(1 - k) * cuberoot(2^(-r))
    // To keep this const-evaluable we hard-code the cube-root factors.
    //   r=0 → 1
    //   r=1 → 2^(-1/3) ≈ 0.79370052598409979
    //   r=2 → 2^(-2/3) ≈ 0.62996052494743658
    let r_factors: [f64; 3] = [
        1.0,
        0.7937005259840998, // 2^(-1/3)
        0.6299605249474366, // 2^(-2/3)
    ];
    let mut out = [0.0f32; 64];
    let mut i = 0usize;
    while i < 63 {
        let k = (i / 3) as i32;
        let r = i % 3;
        // 2 * 2^(-k) * r_factor[r]   — implemented with a runtime-friendly
        // const loop: build 2^(-k) by repeated halving.
        let mut mag = 2.0f64;
        let mut kk = k;
        while kk > 0 {
            mag *= 0.5;
            kk -= 1;
        }
        while kk < 0 {
            mag *= 2.0;
            kk += 1;
        }
        mag *= r_factors[r];
        out[i] = mag as f32;
        i += 1;
    }
    // Index 63 is reserved; set to 0.0 so any stray reference produces
    // silence rather than a denormal/NaN.
    out[63] = 0.0;
    out
}

/// Number of subband samples per subband per frame (Layer II = 36, grouped
/// as 3 × 12 granules).
pub const SAMPLES_PER_SUBBAND: usize = 36;
/// PCM samples per channel per Layer II frame (32 * 36).
pub const PCM_PER_CHANNEL: usize = 1152;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b2a_is_27_subbands() {
        assert_eq!(TABLE_B2A.sblimit, 27);
        assert_eq!(TABLE_B2A.offsets.len(), 27);
    }

    #[test]
    fn b2a_headers_have_correct_nbal() {
        // Expected nbal layout for B.2a.
        let expected: [u32; 27] = [
            4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2,
        ];
        for sb in 0..27 {
            assert_eq!(TABLE_B2A.nbal(sb), expected[sb], "nbal[{sb}]");
        }
    }

    #[test]
    fn b2b_is_30_subbands_mostly_same_nbal() {
        assert_eq!(TABLE_B2B.sblimit, 30);
        let expected: [u32; 30] = [
            4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2,
            2,
        ];
        for sb in 0..30 {
            assert_eq!(TABLE_B2B.nbal(sb), expected[sb]);
        }
    }

    #[test]
    fn b2c_is_8_subbands_all_nbal4_or_3() {
        assert_eq!(TABLE_B2C.sblimit, 8);
        let expected: [u32; 8] = [4, 4, 3, 3, 3, 3, 3, 3];
        for sb in 0..8 {
            assert_eq!(TABLE_B2C.nbal(sb), expected[sb]);
        }
    }

    #[test]
    fn b2d_is_12_subbands() {
        assert_eq!(TABLE_B2D.sblimit, 12);
        let expected: [u32; 12] = [4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3];
        for sb in 0..12 {
            assert_eq!(TABLE_B2D.nbal(sb), expected[sb]);
        }
    }

    #[test]
    fn scalefactor_endpoints() {
        // ISO 11172-3 Table 3-B.1: SF[0] = 2.000000000, SF[1] = 1.58740105...
        assert!((scalefactor_magnitude(0) - 2.0).abs() < 1e-6);
        assert!((scalefactor_magnitude(1) - 1.587_401).abs() < 1e-5);
        // SF[62] is smallest non-reserved entry.
        let sf62 = scalefactor_magnitude(62);
        assert!(sf62 > 0.0 && sf62 < 1e-5);
    }

    #[test]
    fn select_table_matches_reference() {
        // At 44.1 kHz, stereo, 128 kbps (bitrate_index=9) => table 0 (B.2a).
        assert_eq!(select_alloc_table_index(0, true, 9), 0);
        // At 48 kHz, stereo, 128 kbps => 0 (B.2a).
        assert_eq!(select_alloc_table_index(1, true, 9), 0);
        // At 48 kHz, stereo, 64 kbps (bri=5) => 2 (B.2c).
        assert_eq!(select_alloc_table_index(1, true, 5), 2);
        // At 32 kHz, stereo, 64 kbps (bri=5) => 3 (B.2d).
        assert_eq!(select_alloc_table_index(2, true, 5), 3);
        // At 44.1 kHz, mono, 64 kbps (bri=5) => 0.
        assert_eq!(select_alloc_table_index(0, false, 5), 0);
        // At 44.1 kHz, stereo, 160 kbps (bri=10) => 1 (B.2b).
        assert_eq!(select_alloc_table_index(0, true, 10), 1);
    }
}
