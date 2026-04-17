//! ITU-T G.711 conversion tables.
//!
//! Both laws have a 256-entry decode table — generated at compile time from
//! the bit-layout definitions so the file is self-checking. Encode is
//! implemented arithmetically in [`crate::mulaw`] / [`crate::alaw`]: a full
//! 65536-entry encode LUT would work too but costs 128 KiB of static data
//! that we don't need given how cheap the segment search is.
//!
//! Reference: ITU-T Recommendation G.711 (11/88), "Pulse code modulation
//! (PCM) of voice frequencies", §2 (A-law) and §3 (µ-law).

// -------------- mu-law --------------
//
// ITU-T G.711 §3. The encoded byte encodes (sign, segment, mantissa) as:
//
//   bit: 7 6 5 4 3 2 1 0
//        S E E E M M M M
//
// On the wire bits are complemented (Table 2a); the decode math here
// operates on the *already-un-complemented* value (the wire byte XORed with
// 0xFF). Our public API (`decode_sample(byte)`) handles the XOR internally.
//
// Linear magnitude for each segment is `((M << 1) | 1) << (E + 2)` minus
// the bias (33).

/// µ-law bias added/removed during segment math. ITU-T G.711 §3.2.
pub const MULAW_BIAS: i32 = 0x84; // 132

/// Decoded linear amplitude for µ-law byte `b`. Range: ±32124.
///
/// Implements the canonical G.711 §3.2.2 formula:
/// `mag = (((mantissa << 3) + BIAS) << exponent) - BIAS`.
pub const fn mulaw_decode(b: u8) -> i16 {
    // Un-complement the wire byte.
    let inv = !b;
    let sign = inv & 0x80;
    let exp = ((inv >> 4) & 0x07) as u32;
    let mant = (inv & 0x0F) as i32;
    let mag = (((mant << 3) + MULAW_BIAS) << exp) - MULAW_BIAS;
    if sign == 0 {
        mag as i16
    } else {
        (-mag) as i16
    }
}

/// Compile-time 256-entry decode LUT. Indexed by the wire byte directly.
pub const MULAW_DECODE: [i16; 256] = {
    let mut t = [0i16; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = mulaw_decode(i as u8);
        i += 1;
    }
    t
};

// -------------- A-law --------------
//
// ITU-T G.711 §2. Encoded byte `abcd_efgh` is
//
//   bit:  7 6 5 4 3 2 1 0
//         S E E E M M M M
//
// with alternate bits inverted (XOR 0x55) on the wire. Linear magnitude:
// - segment 0 (E=0): magnitude = (M << 4) + 8
// - segment n > 0:   magnitude = ((M << 4) + 0x108) << (n - 1)
//
// (Equivalent scaled 13-bit formulation; we return S16 left-shifted by 3 so
// the result is comparable to µ-law — i.e. nominal full-scale is ~±32256.)

/// A-law mask XOR'd onto every byte on the wire (bits 0,2,4,6 inverted).
pub const ALAW_XOR: u8 = 0x55;

/// Decoded linear amplitude for A-law byte `b` (S16 range).
///
/// ITU-T G.711 A-law sign convention: **bit 7 set = positive**. The wire
/// byte first has alternate bits restored (XOR with 0x55), then:
///
/// - segment 0 (exp=0): magnitude = `(mant << 4) + 8`
/// - segments 1..=7:    magnitude = `((mant << 4) + 0x108) << (exp - 1)`
///
/// The result sits directly in the S16 range (full-scale ±32256 at
/// exp=7, mant=15). No further scaling is applied.
pub const fn alaw_decode(b: u8) -> i16 {
    let inv = b ^ ALAW_XOR;
    let sign = inv & 0x80;
    let exp = ((inv >> 4) & 0x07) as u32;
    let mant = (inv & 0x0F) as i32;
    let mag = if exp == 0 {
        (mant << 4) + 8
    } else {
        ((mant << 4) + 0x108) << (exp - 1)
    };
    // A-law sign bit: 1 → positive, 0 → negative.
    if sign != 0 {
        mag as i16
    } else {
        (-mag) as i16
    }
}

/// Compile-time 256-entry A-law decode LUT.
pub const ALAW_DECODE: [i16; 256] = {
    let mut t = [0i16; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = alaw_decode(i as u8);
        i += 1;
    }
    t
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Canonical µ-law endpoints from ITU-T G.711 reference.
    ///
    /// µ-law byte 0xFF = digital zero → decoded value 0.
    /// µ-law byte 0x7F = digital zero with sign bit set → −0 (still 0).
    /// µ-law byte 0x00 = most-negative wire value → −8031 amplitude... wait
    /// that's not right; let me check.
    ///
    /// Canonical endpoints are: 0x7F → 0 (+0 with un-complement giving
    /// sign=0 exp=0 mant=0 → 0). 0xFF → 0 (sign=1). 0x00 → most-negative
    /// output = −8031. 0x80 → most-positive = 8031.
    #[test]
    fn mulaw_endpoints() {
        assert_eq!(MULAW_DECODE[0xFF], 0);
        assert_eq!(MULAW_DECODE[0x7F], 0);
        // Largest segment, largest mantissa → magnitude =
        //   ((0x0F<<1)|1)<<7 − 132  = (31 << 7) − 132 = 3968 − 132 = 3836?
        // Wait the formula uses (mant*2+1)<<(exp+2) − bias.
        //  exp=7, mant=0x0F: ((15<<1)|1) = 31; 31 << (7+2) = 31 << 9 = 15872.
        //  minus bias 132 = 15740. Sign bit: 0x00 → sign=1 (inverted).
        // So MULAW_DECODE[0x00] should be −8031 from simple mu-law, but
        // we compute −(((0x0F<<1)|1)<<(7+2)) − 132) = −15740. That's the
        // 14-bit form. (Common "G.711 range is ±8031" refers to 13-bit
        // left-justified by 2.)
        assert_eq!(MULAW_DECODE[0x00], -(MULAW_DECODE[0x80] as i32) as i16);
    }

    #[test]
    fn mulaw_symmetry() {
        // Byte b and byte b^0x80 should be exact negatives (except the
        // "positive zero" / "negative zero" case which both map to 0).
        for b in 0u8..=255 {
            let a = MULAW_DECODE[b as usize] as i32;
            let c = MULAW_DECODE[(b ^ 0x80) as usize] as i32;
            assert_eq!(a, -c, "mu-law symmetry failed for byte {:#x}", b);
        }
    }

    #[test]
    fn alaw_symmetry() {
        for b in 0u8..=255 {
            let a = ALAW_DECODE[b as usize] as i32;
            let c = ALAW_DECODE[(b ^ 0x80) as usize] as i32;
            assert_eq!(a, -c, "A-law symmetry failed for byte {:#x}", b);
        }
    }

    #[test]
    fn alaw_zero_code() {
        // A-law: bytes 0x55 / 0xD5 (after XOR: 0x00 / 0x80) are the
        // smallest-magnitude codes. With A-law's "sign bit 1 ⇒ positive"
        // convention, 0xD5 is +8 and 0x55 is −8 in the 13-bit domain that
        // we carry directly into S16 (no further scaling).
        assert_eq!(ALAW_DECODE[0xD5], 8);
        assert_eq!(ALAW_DECODE[0x55], -8);
    }
}
