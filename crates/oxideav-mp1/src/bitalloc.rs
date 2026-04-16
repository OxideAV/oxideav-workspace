//! MPEG-1 Audio Layer I bit allocation and requantization constants.
//!
//! Reference: ISO/IEC 11172-3:1993 §2.4.2.3 / §2.4.3.1 / Table 3-B.1.
//!
//! **Allocation field.** Each of the 32 subbands carries a 4-bit allocation
//! value. Values 1..14 indicate the number of bits per sample minus one
//! (so nb = allocation + 1 ∈ {2..15}); value 0 means "no samples in this
//! subband" (scalefactor is also skipped); value 15 is forbidden.
//!
//! **Scalefactor.** A 6-bit index selects one of 64 entries from
//! SCALE = `2 * 2^(-i/3)` for i = 0..62. Entry 63 is reserved by the
//! spec but we fill it to keep lookups branch-free; encoders do not use
//! it.
//!
//! **Requantization.** After reading the nb-bit unsigned sample `s`, the
//! spec's MSB-inversion + c/d path simplifies to
//!
//! ```text
//!   sample_pcm = (s - 2^(nb-1) + 1) * 2 / (2^nb - 1) * SCALE[scf]
//! ```
//!
//! We precompute the `dequant[nb][scf] = 2 / (2^nb - 1) * SCALE[scf]`
//! table so the decoder inner loop is one add + one multiply per sample.

use std::sync::OnceLock;

/// Number of subbands in Layer I / II (constant across the format).
pub const SBLIMIT: usize = 32;

/// Number of sample blocks per frame per subband in Layer I.
/// Each block holds 12 samples per subband, times 32 subbands = 384 PCM
/// samples per channel (one Layer I frame).
pub const LAYER1_BLOCKS_PER_FRAME: usize = 1;
/// Number of samples per subband per block.
pub const SAMPLES_PER_SUBBAND: usize = 12;

/// Number of bits to read for a given 4-bit allocation code. `None`
/// means "no samples" (allocation 0); `Some(forbidden=15)` is reported
/// by the decoder as an error.
#[inline]
pub fn bits_per_sample(alloc: u8) -> Option<u8> {
    match alloc {
        0 => None,
        1..=14 => Some(alloc + 1),
        _ => None, // 15 is forbidden; caller should reject before this
    }
}

/// SCALE[i] = 2 * 2^(-i/3) for i = 0..62, with index 63 extrapolated
/// (reserved in the spec but included so lookups can't go out of bounds).
pub fn scale_table() -> &'static [f32; 64] {
    static TBL: OnceLock<[f32; 64]> = OnceLock::new();
    TBL.get_or_init(|| {
        let mut t = [0.0f32; 64];
        for (i, slot) in t.iter_mut().enumerate() {
            // exact geometric series: 2 * 2^(-i/3)
            *slot = (2.0_f64 * (-(i as f64) / 3.0).exp2()) as f32;
        }
        t
    })
}

/// Precomputed dequantization table: `DEQUANT[nb][scf]` where
/// `nb ∈ 2..=15` and `scf ∈ 0..64`. For nb outside that range the
/// entries are zero.
///
/// Final PCM for a subband sample = `DEQUANT[nb][scf] * raw_signed_int`
/// with `raw_signed_int = sample_bits - 2^(nb-1) + 1` in
/// range [-2^(nb-1) + 1, 2^(nb-1)].
pub fn dequant_table() -> &'static [[f32; 64]; 16] {
    static TBL: OnceLock<[[f32; 64]; 16]> = OnceLock::new();
    TBL.get_or_init(|| {
        let sc = scale_table();
        let mut t = [[0.0f32; 64]; 16];
        for nb in 2..=15usize {
            let denom = (1u64 << nb) - 1; // 2^nb - 1
            let k = 2.0_f64 / denom as f64;
            for scf in 0..64 {
                t[nb][scf] = (k * sc[scf] as f64) as f32;
            }
        }
        t
    })
}

/// Convenience: dequantize one sample from raw `sample_bits` (unsigned,
/// `nb` bits wide) and scalefactor index `scf`.
///
/// `nb` must be in `2..=15`. Caller is responsible for rejecting `nb=16`
/// (allocation=15 forbidden).
#[inline]
pub fn dequantize_sample(sample_bits: u32, nb: u8, scf: u8) -> f32 {
    let level = sample_bits as i32 - (1 << (nb - 1)) + 1;
    dequant_table()[nb as usize][scf as usize] * level as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_table_endpoints() {
        let s = scale_table();
        assert!((s[0] - 2.0).abs() < 1e-6);
        // s[3] = 2 * 2^(-1) = 1.0
        assert!((s[3] - 1.0).abs() < 1e-5);
        // Monotonically decreasing
        for i in 1..63 {
            assert!(s[i] < s[i - 1]);
            assert!(s[i] > 0.0);
        }
    }

    #[test]
    fn bits_per_sample_mapping() {
        assert_eq!(bits_per_sample(0), None);
        assert_eq!(bits_per_sample(1), Some(2));
        assert_eq!(bits_per_sample(14), Some(15));
        assert_eq!(bits_per_sample(15), None); // forbidden
    }

    #[test]
    fn dequantize_basic_roundtrip() {
        // For nb = 15 (highest resolution), scf such that SCALE = 1.0
        // (index 3). Midpoint sample (2^14 = 16384) should map to 1/(2^15-1).
        // sample = 2^14 gives level = 2^14 - 2^14 + 1 = 1.
        let v = dequantize_sample(1 << 14, 15, 3);
        let expected = 2.0_f32 / ((1 << 15) - 1) as f32 * 1.0;
        assert!((v - expected).abs() < 1e-6, "got {v} expected {expected}");
    }

    #[test]
    fn dequantize_nb2_levels() {
        // nb=2: 4 levels {-1, 0, 1, 2}. With scf=3 (SCALE=1) the
        // dequant factor is 2/(2^2-1) = 2/3.
        let d = dequant_table()[2][3];
        let want = 2.0_f32 / 3.0;
        assert!((d - want).abs() < 1e-6);
        // Check level mapping
        assert!((dequantize_sample(0, 2, 3) - -2.0 / 3.0).abs() < 1e-6); // level -1
        assert!((dequantize_sample(1, 2, 3) - 0.0).abs() < 1e-6); // level  0
        assert!((dequantize_sample(2, 2, 3) - 2.0 / 3.0).abs() < 1e-6); // level +1
        assert!((dequantize_sample(3, 2, 3) - 4.0 / 3.0).abs() < 1e-6); // level +2
    }
}
