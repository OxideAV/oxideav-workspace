//! Static CELT tables from RFC 6716 §4.3 (informative reference: libopus
//! `static_modes_float.h`, `quant_bands.c`, `rate.c`).
//!
//! Only the tables needed by the parts of the decoder that are landed in
//! this crate are present. Adding more tables (e.g. PVQ codebooks,
//! `cache_caps50`, fine bits LUT) is a follow-up tracked alongside the
//! corresponding decoder stages.

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
