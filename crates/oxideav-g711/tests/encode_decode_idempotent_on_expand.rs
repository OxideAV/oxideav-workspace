//! Bounded-error roundtrip sweep for both µ-law and A-law.
//!
//! Because G.711 is an 8-bit companding codec targeting a nominally
//! 13-bit output range, the maximum round-trip quantisation error is
//! **one segment step** — which in the top segment equals `1 << 8` in the
//! 13-bit domain, or `1 << 11` = 2048 in the fully-left-justified S16
//! domain. However, most samples live in lower segments where the step
//! is much smaller. This test asserts a loose global bound and a tight
//! bound near silence.

use oxideav_g711::{alaw, mulaw};

const STEP: i32 = 7; // sparse-enough sweep to keep the test fast.

/// Top-segment step size in S16-domain LSBs for each law. These are the
/// *worst-case* round-trip errors; most samples are far better.
const MULAW_MAX_STEP_SHIFT: u32 = 11; // segment 7 step = 1 << 9 in 14-bit ≈ 2048 in S16.
const ALAW_MAX_STEP_SHIFT: u32 = 11;

#[test]
fn mulaw_roundtrip_bounded_error() {
    let max_allowed: i32 = 1 << MULAW_MAX_STEP_SHIFT;
    let mut worst: i32 = 0;
    for x in (-32768..=32767i32).step_by(STEP as usize) {
        let s = x as i16;
        let q = mulaw::decode_sample(mulaw::encode_sample(s)) as i32;
        let err = (q - x).abs();
        assert!(
            err <= max_allowed,
            "µ-law roundtrip error {err} > {max_allowed} at x={x} (q={q})"
        );
        worst = worst.max(err);
    }
    // Print a soft sanity check for humans running with --nocapture.
    eprintln!("µ-law worst-case roundtrip error: {worst} LSB");
}

#[test]
fn alaw_roundtrip_bounded_error() {
    let max_allowed: i32 = 1 << ALAW_MAX_STEP_SHIFT;
    let mut worst: i32 = 0;
    for x in (-32768..=32767i32).step_by(STEP as usize) {
        let s = x as i16;
        let q = alaw::decode_sample(alaw::encode_sample(s)) as i32;
        let err = (q - x).abs();
        assert!(
            err <= max_allowed,
            "A-law roundtrip error {err} > {max_allowed} at x={x} (q={q})"
        );
        worst = worst.max(err);
    }
    eprintln!("A-law worst-case roundtrip error: {worst} LSB");
}

/// Tight bound near silence — the smallest segment has step size ≤ 8 in
/// 13-bit i.e. 64 after <<3. This checks that low-amplitude samples
/// round-trip with far better fidelity than the global bound implies.
#[test]
fn alaw_low_amplitude_error_is_small() {
    // 13-bit domain smallest step = 8; after <<3 left-shift = 64.
    let tight_max: i32 = 128;
    for x in -2048..=2048i32 {
        let q = alaw::decode_sample(alaw::encode_sample(x as i16)) as i32;
        let err = (q - x).abs();
        assert!(
            err <= tight_max,
            "A-law low-amp roundtrip error {err} > {tight_max} at x={x} (q={q})"
        );
    }
}

#[test]
fn mulaw_low_amplitude_error_is_small() {
    // Smallest µ-law step in segment 0 is 2 in the pre-bias domain, << 2
    // left-justify = 8. After the (+bias / -bias) machinery the effective
    // S16-domain step for small samples is at most ~8 LSB.
    let tight_max: i32 = 32;
    for x in -512..=512i32 {
        let q = mulaw::decode_sample(mulaw::encode_sample(x as i16)) as i32;
        let err = (q - x).abs();
        assert!(
            err <= tight_max,
            "µ-law low-amp roundtrip error {err} > {tight_max} at x={x} (q={q})"
        );
    }
}
