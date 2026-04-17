//! A-law integration tests — encode S16 → A-law → decode, and spot-check
//! against the canonical ITU-T G.711 lookup at endpoint samples.

use oxideav_g711::alaw::{decode_sample, encode_sample};
use oxideav_g711::tables::ALAW_XOR;

#[test]
fn alaw_canonical_endpoints() {
    // Smallest-magnitude A-law codes in the "bit 7 ⇒ positive" convention:
    // 0xD5 → +8, 0x55 → −8 (in the 13-bit linear domain that maps directly
    // into S16).
    assert_eq!(decode_sample(0xD5), 8);
    assert_eq!(decode_sample(0x55), -8);

    for b in 0u8..=255 {
        let a = decode_sample(b) as i32;
        let c = decode_sample(b ^ 0x80) as i32;
        assert_eq!(a, -c, "A-law sign symmetry violated at byte {:#x}", b);
    }
    // Peak codes: top segment / top mantissa is pre-XOR 0xFF (positive
    // peak, since sign bit = 1). On the wire that's 0xFF ^ 0x55 = 0xAA.
    // Magnitude = ((0xF << 4) + 0x108) << 6 = 0x1F8 << 6 = 32256.
    assert_eq!(decode_sample(0xFF ^ ALAW_XOR), 32256); // wire byte 0xAA
    assert_eq!(decode_sample(0x7F ^ ALAW_XOR), -32256); // wire byte 0x2A
}

#[test]
fn alaw_encode_silence_is_zero_magnitude() {
    // S16 silence encodes to the positive zero-magnitude code:
    // pre-XOR 0x80 (sign=1, exp=0, mant=0) → wire byte 0xD5.
    assert_eq!(encode_sample(0), 0xD5);
    // Saturation: i16::MAX → positive peak 0xAA; i16::MIN → negative peak
    // 0x2A.
    assert_eq!(encode_sample(i16::MAX), 0xAA);
    assert_eq!(encode_sample(i16::MIN), 0x2A);
}

#[test]
fn alaw_roundtrip_is_deterministic() {
    for x in (-32768..=32767i32).step_by(37) {
        let s = x as i16;
        let q1 = decode_sample(encode_sample(s));
        let q2 = decode_sample(encode_sample(q1));
        assert_eq!(q1, q2, "A-law not idempotent at sample {s}");
    }
}

#[test]
fn alaw_monotonic_encoding() {
    let mut prev = i16::MIN as i32;
    for x in (-32768..=32767i32).step_by(101) {
        let q = decode_sample(encode_sample(x as i16)) as i32;
        assert!(q >= prev, "A-law not monotonic: prev={prev}, q={q}, x={x}");
        prev = q;
    }
}
