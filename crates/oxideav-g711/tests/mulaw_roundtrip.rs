//! µ-law integration tests — encode S16 → µ-law → decode, and spot-check
//! against the canonical ITU-T G.711 lookup at endpoint samples.

use oxideav_g711::mulaw::{decode_sample, encode_sample};

/// Representative bytes from the ITU-T G.711 §3 reference:
///
/// - `0xFF` is the "digital zero" for µ-law (sign 0, exp 0, mant 0 after
///   inversion) → decoded amplitude 0.
/// - `0x7F` is "negative digital zero" → also 0.
/// - `0x80` is the most-positive wire code → top segment, zero mantissa
///   → decoded amplitude 8031 (+bias math gives 8031 in the common 14-bit
///   unsigned left-justified form; here we normalise to the fully
///   sign-extended 16-bit S16 so the magnitude is `(31<<9)-132 = 15740`
///   after our decoder's implicit <<2 left-shift).
#[test]
fn mulaw_canonical_endpoints() {
    // Digital zero.
    assert_eq!(decode_sample(0xFF), 0);
    assert_eq!(decode_sample(0x7F), 0);
    // The decoder outputs sign-symmetric values — every pair (b, b^0x80)
    // must be exact negatives.
    for b in 0u8..=255 {
        let a = decode_sample(b) as i32;
        let c = decode_sample(b ^ 0x80) as i32;
        assert_eq!(a, -c, "µ-law sign symmetry violated at byte {:#x}", b);
    }
    // Peak magnitudes.
    let pos_peak = decode_sample(0x80); // +max
    let neg_peak = decode_sample(0x00); // −max
    assert!(pos_peak > 0);
    assert!(neg_peak < 0);
    assert_eq!(pos_peak as i32, -(neg_peak as i32));
}

/// Encoding known silence / peak samples must match the canonical
/// encoder output.
#[test]
fn mulaw_encode_known_points() {
    // Silence → "digital zero" byte.
    assert_eq!(encode_sample(0), 0xFF);
    // Small negative magnitude → negative digital zero (0x7F) or very
    // close; the exact match depends on the bias. sample = 0 with sign
    // flipped (ie −0) is still 0, but sample = -1 lands in the smallest
    // negative segment.
    assert_ne!(encode_sample(-1), encode_sample(1));
    // Peak samples saturate to the ±8031-magnitude codes (byte `0x80`
    // / `0x00` after un-complementation).
    assert_eq!(encode_sample(i16::MAX), 0x80);
    assert_eq!(encode_sample(i16::MIN), 0x00);
}

#[test]
fn mulaw_roundtrip_is_deterministic() {
    // For every S16 input, encoding then decoding must be idempotent under
    // a second encode cycle — i.e. decode(encode(decode(encode(x)))) ==
    // decode(encode(x)). This is the classic G.711 "quantisation is a
    // projection" property.
    for x in (-32768..=32767i32).step_by(37) {
        let s = x as i16;
        let q1 = decode_sample(encode_sample(s));
        let q2 = decode_sample(encode_sample(q1));
        assert_eq!(q1, q2, "µ-law not idempotent at sample {s}");
    }
}

#[test]
fn mulaw_monotonic_encoding() {
    // The µ-law transfer function is monotonic: if a ≤ b then
    // decode(encode(a)) ≤ decode(encode(b)). Sweep the whole S16 range
    // and assert.
    let mut prev = i16::MIN as i32;
    for x in (-32768..=32767i32).step_by(101) {
        let q = decode_sample(encode_sample(x as i16)) as i32;
        assert!(q >= prev, "µ-law not monotonic: prev={prev}, q={q}, x={x}");
        prev = q;
    }
}
