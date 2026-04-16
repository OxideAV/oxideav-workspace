//! Token tree and DCT-coefficient context constants from RFC 6386 §13.
//!
//! VP8 codes residual coefficients as a tree of 12 tokens. The first 11 are
//! distinct numeric values; the 12th, "extra-bits" tokens, decode to ranges
//! of values requiring follow-on uniform bits.

// Token symbols (DCT_VAL_CATEGORY enum from RFC §13.3).
pub const DCT_0: i32 = 0;
pub const DCT_1: i32 = 1;
pub const DCT_2: i32 = 2;
pub const DCT_3: i32 = 3;
pub const DCT_4: i32 = 4;
pub const DCT_CAT1: i32 = 5;
pub const DCT_CAT2: i32 = 6;
pub const DCT_CAT3: i32 = 7;
pub const DCT_CAT4: i32 = 8;
pub const DCT_CAT5: i32 = 9;
pub const DCT_CAT6: i32 = 10;
pub const DCT_EOB: i32 = 11;

/// Coefficient tree from RFC 6386 §13.3 / §20.4 — `coef_tree`.
pub const COEF_TREE: [i8; 22] = [
    -DCT_EOB as i8,
    2,
    -DCT_0 as i8,
    4,
    -DCT_1 as i8,
    6,
    8,
    12,
    -DCT_2 as i8,
    10,
    -DCT_3 as i8,
    -DCT_4 as i8,
    14,
    16,
    -DCT_CAT1 as i8,
    -DCT_CAT2 as i8,
    18,
    20,
    -DCT_CAT3 as i8,
    -DCT_CAT4 as i8,
    -DCT_CAT5 as i8,
    -DCT_CAT6 as i8,
];

/// Probabilities for the trailing extra-bits in DCT_CAT* (RFC 6386 §13.2).
pub const PCAT1: [u8; 1] = [159];
pub const PCAT2: [u8; 2] = [165, 145];
pub const PCAT3: [u8; 3] = [173, 148, 140];
pub const PCAT4: [u8; 4] = [176, 155, 140, 135];
pub const PCAT5: [u8; 5] = [180, 157, 141, 134, 130];
pub const PCAT6: [u8; 11] = [254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129];

/// Base value (lower bound) for each DCT_CAT level.
pub const DCT_CAT_BASE: [i32; 6] = [5, 7, 11, 19, 35, 67];

/// Default zigzag for a 4×4 block (RFC 6386 §13.2 default_zig_zag1d).
pub const ZIGZAG: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

/// Number of plane categories for the coefficient probability table — 4.
pub const NUM_TYPES: usize = 4;
/// Number of coefficient bands — 8.
pub const NUM_BANDS: usize = 8;
/// Three context buckets per band.
pub const NUM_CTX: usize = 3;
/// 11 entropy probabilities per context (one per non-EOB tree branch above
/// the value range — covers the EOB / 0 / 1 / 2 / 3 / 4 / CAT1..CAT6 tree
/// branches).
pub const NUM_PROBS: usize = 11;

/// Mapping from raw zigzag-coefficient index (0..16) to coefficient band.
/// Band 0 = DC, bands 1..6 = AC bands, band 7 = beyond-the-band sink.
pub const COEF_BANDS: [usize; 16] = [0, 1, 2, 3, 6, 4, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7];
