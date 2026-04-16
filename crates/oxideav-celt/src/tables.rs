//! Static CELT tables from RFC 6716 §4.3 (informative reference: libopus
//! `static_modes_float.h`, `quant_bands.c`, `rate.c`).
//!
//! All tables are pure constants transcribed verbatim from the libopus
//! reference (data, not code logic). The CELT decoder needs:
//!
//! * `EBAND_5MS` — band edges (RFC 6716 §4.3 Table 55).
//! * `E_PROB_MODEL` — Laplace parameters per (LM, intra, band) for coarse
//!   energy decoding (RFC §4.3.2.1).
//! * `PRED_COEF` / `BETA_COEF` / `BETA_INTRA` — inter/intra prediction
//!   coefficients in Q15.
//! * `BAND_ALLOCATION` — base bit budget per band per quality level
//!   (RFC §4.3.3 / Table 57).
//! * `LOG2_FRAC_TABLE` — fractional log2 lookup (rate.c).
//! * `CACHE_INDEX50` / `CACHE_BITS50` / `CACHE_CAPS50` — PVQ pulse-count
//!   thresholds and per-band caps (RFC §4.3.3).

/// CELT band edges in units of MDCT bins for a 5-ms (LM=0) frame at 48 kHz.
/// Each entry is the *start* of the band; the next entry is the start of the
/// next band, so band `i` spans `[eband_5ms[i], eband_5ms[i+1])`.
///
/// At LM>0 (longer frames) the same table is multiplied by `1 << LM` to
/// expand the per-band MDCT-bin count.
///
/// Source: libopus `static_modes_float.h` (eband5ms[]).
pub const EBAND_5MS: [u16; 22] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 20, 24, 28, 34, 40, 48, 60, 78, 100,
];

/// Number of bands actually decoded for each [bandwidth][end_band_table_idx]
/// combination. CELT-only modes use the full table up to the bandwidth limit.
///
/// Maps `OpusBandwidth` → upper-band index used at the decoder. The lower
/// edge `start` is always 0 for CELT-only frames (Hybrid uses 17).
pub fn end_band_for_bandwidth_celt(cutoff_hz: u32) -> usize {
    // Per RFC 6716 §4.3 + libopus `mode.c` `compute_ebands`:
    //   NB: 13 bands  (≤ 4 kHz)
    //   WB: 17 bands  (≤ 8 kHz)
    //   SWB: 19 bands (≤ 12 kHz)
    //   FB: 21 bands  (≤ 20 kHz)
    match cutoff_hz {
        0..=4_000 => 13,
        4_001..=8_000 => 17,
        8_001..=12_000 => 19,
        _ => 21,
    }
}

/// log2(frame_samples_48k / 120) — the "LM" shift used throughout RFC §4.3.
pub fn lm_for_frame_samples(frame_samples_48k: u32) -> u32 {
    match frame_samples_48k {
        120 => 0,
        240 => 1,
        480 => 2,
        960 => 3,
        _ => 0,
    }
}

/// Number of CELT bands (always 21 for the standard mode).
pub const NB_EBANDS: usize = 21;

/// Inter-frame prediction coefficients for coarse energy (Q15), one per LM.
/// libopus `pred_coef`.
pub const PRED_COEF_Q15: [i16; 4] = [29440, 26112, 21248, 16384];

/// Inter-frame prediction beta coefficients (Q15), one per LM.
/// libopus `beta_coef`.
pub const BETA_COEF_Q15: [i16; 4] = [30147, 22282, 12124, 6554];

/// Intra-frame prediction beta coefficient (Q15). libopus `beta_intra`.
pub const BETA_INTRA_Q15: i16 = 4915;

/// Floating-point versions for ergonomic use in non-fixed-point code.
pub const PRED_COEF_F32: [f32; 4] = [
    29440.0 / 32768.0,
    26112.0 / 32768.0,
    21248.0 / 32768.0,
    16384.0 / 32768.0,
];
pub const BETA_COEF_F32: [f32; 4] = [
    30147.0 / 32768.0,
    22282.0 / 32768.0,
    12124.0 / 32768.0,
    6554.0 / 32768.0,
];
pub const BETA_INTRA_F32: f32 = 4915.0 / 32768.0;

/// Laplace probability-model parameters per (LM, intra, band-pair).
/// 4 frame sizes × 2 prediction types × 21 (prob, decay) pairs.
/// libopus `e_prob_model` (quant_bands.c).
#[rustfmt::skip]
pub const E_PROB_MODEL: [[[u8; 42]; 2]; 4] = [
    /* 120-sample frames (LM=0) */
    [
        /* Inter */
        [
            72, 127, 65, 129, 66, 128, 65, 128, 64, 128, 62, 128, 64, 128,
            64, 128, 92, 78, 92, 79, 92, 78, 90, 79, 116, 41, 115, 40,
            114, 40, 132, 26, 132, 26, 145, 17, 161, 12, 176, 10, 177, 11,
        ],
        /* Intra */
        [
            24, 179, 48, 138, 54, 135, 54, 132, 53, 134, 56, 133, 55, 132,
            55, 132, 61, 114, 70, 96, 74, 88, 75, 88, 87, 74, 89, 66,
            91, 67, 100, 59, 108, 50, 120, 40, 122, 37, 97, 43, 78, 50,
        ],
    ],
    /* 240-sample frames (LM=1) */
    [
        /* Inter */
        [
            83, 78, 84, 81, 88, 75, 86, 74, 87, 71, 90, 73, 93, 74,
            93, 74, 109, 40, 114, 36, 117, 34, 117, 34, 143, 17, 145, 18,
            146, 19, 162, 12, 165, 10, 178, 7, 189, 6, 190, 8, 177, 9,
        ],
        /* Intra */
        [
            23, 178, 54, 115, 63, 102, 66, 98, 69, 99, 74, 89, 71, 91,
            73, 91, 78, 89, 86, 80, 92, 66, 93, 64, 102, 59, 103, 60,
            104, 60, 117, 52, 123, 44, 138, 35, 133, 31, 97, 38, 77, 45,
        ],
    ],
    /* 480-sample frames (LM=2) */
    [
        /* Inter */
        [
            61, 90, 93, 60, 105, 42, 107, 41, 110, 45, 116, 38, 113, 38,
            112, 38, 124, 26, 132, 27, 136, 19, 140, 20, 155, 14, 159, 16,
            158, 18, 170, 13, 177, 10, 187, 8, 192, 6, 175, 9, 159, 10,
        ],
        /* Intra */
        [
            21, 178, 59, 110, 71, 86, 75, 85, 84, 83, 91, 66, 88, 73,
            87, 72, 92, 75, 98, 72, 105, 58, 107, 54, 115, 52, 114, 55,
            112, 56, 129, 51, 132, 40, 150, 33, 140, 29, 98, 35, 77, 42,
        ],
    ],
    /* 960-sample frames (LM=3) */
    [
        /* Inter */
        [
            42, 121, 96, 66, 108, 43, 111, 40, 117, 44, 123, 32, 120, 36,
            119, 33, 127, 33, 134, 34, 139, 21, 147, 23, 152, 20, 158, 25,
            154, 26, 166, 21, 173, 16, 184, 13, 184, 10, 150, 13, 139, 15,
        ],
        /* Intra */
        [
            22, 178, 63, 114, 74, 82, 84, 83, 92, 82, 103, 62, 96, 72,
            96, 67, 101, 73, 107, 72, 113, 55, 118, 52, 125, 52, 118, 52,
            117, 55, 135, 49, 137, 39, 157, 32, 145, 29, 97, 33, 77, 40,
        ],
    ],
];

/// ICDF for the small-energy (≤1-bit budget) fallback in coarse energy
/// decoding. libopus `small_energy_icdf`.
pub const SMALL_ENERGY_ICDF: [u8; 3] = [2, 1, 0];

/// Bit allocation table (`band_allocation` in libopus `modes.c`).
/// 11 quality levels × 21 bands. Units are 1/32 bit/sample.
pub const BITALLOC_SIZE: usize = 11;
#[rustfmt::skip]
pub const BAND_ALLOCATION: [u8; BITALLOC_SIZE * 21] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    90, 80, 75, 69, 63, 56, 49, 40, 34, 29, 20, 18, 10, 0, 0, 0, 0, 0, 0, 0, 0,
    110, 100, 90, 84, 78, 71, 65, 58, 51, 45, 39, 32, 26, 20, 12, 0, 0, 0, 0, 0, 0,
    118, 110, 103, 93, 86, 80, 75, 70, 65, 59, 53, 47, 40, 31, 23, 15, 4, 0, 0, 0, 0,
    126, 119, 112, 104, 95, 89, 83, 78, 72, 66, 60, 54, 47, 39, 32, 25, 17, 12, 1, 0, 0,
    134, 127, 120, 114, 103, 97, 91, 85, 78, 72, 66, 60, 54, 47, 41, 35, 29, 23, 16, 10, 1,
    144, 137, 130, 124, 113, 107, 101, 95, 88, 82, 76, 70, 64, 57, 51, 45, 39, 33, 26, 15, 1,
    152, 145, 138, 132, 123, 117, 111, 105, 98, 92, 86, 80, 74, 67, 61, 55, 49, 43, 36, 20, 1,
    162, 155, 148, 142, 133, 127, 121, 115, 108, 102, 96, 90, 84, 77, 71, 65, 59, 53, 46, 30, 1,
    172, 165, 158, 152, 143, 137, 131, 125, 118, 112, 106, 100, 94, 87, 81, 75, 69, 63, 56, 45, 20,
    200, 200, 200, 200, 200, 200, 200, 200, 198, 193, 188, 183, 178, 173, 168, 163, 158, 153, 148, 129, 104,
];

/// Fractional log2 lookup (libopus `LOG2_FRAC_TABLE`). Values are log2(1+i/8)
/// in 1/8th-bit units, indexed 0..24.
pub const LOG2_FRAC_TABLE: [u8; 24] = [
    0, 8, 13, 16, 19, 21, 23, 24, 26, 27, 28, 29, 30, 31, 32, 32, 33, 34, 34, 35, 36, 36, 37, 37,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eband_5ms_is_monotonic_and_ends_at_100() {
        for w in EBAND_5MS.windows(2) {
            assert!(w[0] < w[1], "EBAND_5MS not strictly increasing");
        }
        assert_eq!(*EBAND_5MS.last().unwrap(), 100);
    }

    #[test]
    fn lm_is_log2_frame_size_over_120() {
        assert_eq!(lm_for_frame_samples(120), 0);
        assert_eq!(lm_for_frame_samples(240), 1);
        assert_eq!(lm_for_frame_samples(480), 2);
        assert_eq!(lm_for_frame_samples(960), 3);
    }

    #[test]
    fn end_band_increases_with_bandwidth() {
        assert!(end_band_for_bandwidth_celt(4_000) < end_band_for_bandwidth_celt(8_000));
        assert!(end_band_for_bandwidth_celt(8_000) < end_band_for_bandwidth_celt(12_000));
        assert!(end_band_for_bandwidth_celt(12_000) < end_band_for_bandwidth_celt(20_000));
        assert_eq!(end_band_for_bandwidth_celt(20_000), 21);
    }
}
