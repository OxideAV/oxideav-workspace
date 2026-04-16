//! MPEG-1 Layer III scalefactor band tables.
//!
//! From ISO/IEC 11172-3 Table 3-B.8 (long-block sfb widths) and 3-B.8.2
//! (short-block sfb widths). Each sample rate has its own partition of
//! the 576 (or 192×3) coefficient positions into scalefactor bands.
//!
//! The tables below give the *starting offset* of each sfb. The number
//! of entries is 23 (22 bands + sentinel) for long and 14 (13 + sentinel)
//! for short.

/// Long-block sfb boundaries by sample rate (Hz). Returns offsets into
/// the 576-sample granule.
pub fn sfband_long(sample_rate: u32) -> &'static [u16] {
    match sample_rate {
        44_100 => &SFB_LONG_44100,
        48_000 => &SFB_LONG_48000,
        32_000 => &SFB_LONG_32000,
        22_050 => &SFB_LONG_22050,
        24_000 => &SFB_LONG_24000,
        16_000 => &SFB_LONG_16000,
        11_025 => &SFB_LONG_11025,
        12_000 => &SFB_LONG_12000,
        8_000 => &SFB_LONG_8000,
        _ => &SFB_LONG_44100, // fallback
    }
}

/// Short-block sfb boundaries (offsets within a 192-sample window).
pub fn sfband_short(sample_rate: u32) -> &'static [u16] {
    match sample_rate {
        44_100 => &SFB_SHORT_44100,
        48_000 => &SFB_SHORT_48000,
        32_000 => &SFB_SHORT_32000,
        22_050 => &SFB_SHORT_22050,
        24_000 => &SFB_SHORT_24000,
        16_000 => &SFB_SHORT_16000,
        _ => &SFB_SHORT_44100,
    }
}

// Long-block sfb boundaries. 23 entries (0..=576) from ISO 3-B.8(a).
pub const SFB_LONG_44100: [u16; 23] = [
    0, 4, 8, 12, 16, 20, 24, 30, 36, 44, 52, 62, 74, 90, 110, 134, 162, 196, 238, 288, 342, 418,
    576,
];
pub const SFB_LONG_48000: [u16; 23] = [
    0, 4, 8, 12, 16, 20, 24, 30, 36, 42, 50, 60, 72, 88, 106, 128, 156, 190, 230, 276, 330, 384,
    576,
];
pub const SFB_LONG_32000: [u16; 23] = [
    0, 4, 8, 12, 16, 20, 24, 30, 36, 44, 54, 66, 82, 102, 126, 156, 194, 240, 296, 364, 448, 550,
    576,
];
pub const SFB_LONG_22050: [u16; 23] = [
    0, 6, 12, 18, 24, 30, 36, 44, 54, 66, 80, 96, 116, 140, 168, 200, 238, 284, 336, 396, 464, 522,
    576,
];
pub const SFB_LONG_24000: [u16; 23] = [
    0, 6, 12, 18, 24, 30, 36, 44, 54, 66, 80, 96, 114, 136, 162, 194, 232, 278, 330, 394, 464, 540,
    576,
];
pub const SFB_LONG_16000: [u16; 23] = [
    0, 6, 12, 18, 24, 30, 36, 44, 54, 66, 80, 96, 116, 140, 168, 200, 238, 284, 336, 396, 464, 522,
    576,
];
pub const SFB_LONG_11025: [u16; 23] = [
    0, 6, 12, 18, 24, 30, 36, 44, 54, 66, 80, 96, 116, 140, 168, 200, 238, 284, 336, 396, 464, 522,
    576,
];
pub const SFB_LONG_12000: [u16; 23] = [
    0, 6, 12, 18, 24, 30, 36, 44, 54, 66, 80, 96, 116, 140, 168, 200, 238, 284, 336, 396, 464, 522,
    576,
];
pub const SFB_LONG_8000: [u16; 23] = [
    0, 12, 24, 36, 48, 60, 72, 88, 108, 132, 160, 192, 232, 280, 336, 400, 476, 566, 568, 570, 572,
    574, 576,
];

// Short-block sfb widths (ISO 3-B.8(b)). These apply per-window;
// granule has 3 windows so actual width = 3 * w. Table of start offsets
// with 14 entries (0..=192).
pub const SFB_SHORT_44100: [u16; 14] = [0, 4, 8, 12, 16, 22, 30, 40, 52, 66, 84, 106, 136, 192];
pub const SFB_SHORT_48000: [u16; 14] = [0, 4, 8, 12, 16, 22, 28, 38, 50, 64, 80, 100, 126, 192];
pub const SFB_SHORT_32000: [u16; 14] = [0, 4, 8, 12, 16, 22, 30, 42, 58, 78, 104, 138, 180, 192];
pub const SFB_SHORT_22050: [u16; 14] = [0, 4, 8, 12, 18, 24, 32, 42, 56, 74, 100, 132, 174, 192];
pub const SFB_SHORT_24000: [u16; 14] = [0, 4, 8, 12, 18, 26, 36, 48, 62, 80, 104, 136, 180, 192];
pub const SFB_SHORT_16000: [u16; 14] = [0, 4, 8, 12, 18, 26, 36, 48, 62, 80, 104, 134, 174, 192];
