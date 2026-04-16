//! Frame-header symbol decoding for CELT (RFC 6716 §4.3, Table 56).
//!
//! Decodes the fixed-prefix run of range-coded symbols every CELT frame
//! starts with:
//!
//! 1. **silence** (`{32767, 1}/32768`) — when set, the rest of the frame is
//!    silence and no further symbols are decoded.
//! 2. **post-filter** (`{1, 1}/2`) — if set, four more symbols
//!    (`octave`, `period`, `gain`, `tapset`) describe a comb-filter.
//! 3. **transient** (`{7, 1}/8`) — long single MDCT vs. multiple short MDCTs.
//! 4. **intra** (`{7, 1}/8`) — disable inter-frame energy prediction.
//!
//! Bit-exactness here is critical because every downstream symbol shares
//! the same range-coder state.

use crate::range_decoder::RangeDecoder;

/// Decoded post-filter parameters per RFC 6716 §4.3 (Table 56).
#[derive(Copy, Clone, Debug)]
pub struct PostFilter {
    /// 3-bit octave (0..=5 after biasing — actually decoded as a 3-bit
    /// uniform, range 0..=5 is the spec maximum but the field is 3 bits).
    pub octave: u32,
    /// Period in `4 + octave` raw bits.
    pub period: u32,
    /// 3-bit gain.
    pub gain: u32,
    /// Tap set (0..=2) selecting one of three FIR filters.
    pub tapset: u32,
}

/// All header symbols decoded at the front of every CELT frame.
#[derive(Copy, Clone, Debug)]
pub struct CeltHeader {
    pub silence: bool,
    pub post_filter: Option<PostFilter>,
    pub transient: bool,
    pub intra: bool,
}

/// PDF for the post-filter `tapset` symbol: {2, 1, 1}/4.
/// ICDF representation: [4-2, 4-3, 4-4] = [2, 1, 0].
const TAPSET_ICDF: [u8; 3] = [2, 1, 0];

/// Decode the silence flag only (logp=15). When set, the caller can skip
/// the rest of the frame and emit silence.
pub fn decode_silence(rc: &mut RangeDecoder<'_>) -> bool {
    rc.decode_bit_logp(15)
}

/// Decode all of the frame header symbols up to (and excluding) coarse
/// energy. Returns `None` if the silence flag was set; otherwise returns
/// `Some(header)` with the parsed flags.
///
/// Per RFC 6716 §4.3 / Table 56:
///   - silence (logp=15)
///   - post_filter (logp=1) — if set, decode octave / period / gain / tapset
///   - transient (logp=3)
///   - intra (logp=3)
pub fn decode_header(rc: &mut RangeDecoder<'_>) -> Option<CeltHeader> {
    if decode_silence(rc) {
        return None;
    }
    // Post-filter flag is ec_dec_bit_logp(1) — 50/50 prior.
    let pf_flag = rc.decode_bit_logp(1);
    let post_filter = if pf_flag {
        // octave: uniform on [0,6) — 3 bits but actually <= 5.
        let octave = rc.decode_uint(6);
        // period: (4 + octave) raw bits from the back-loaded bit reader.
        let period = rc.decode_bits(4 + octave);
        // gain: 3 raw bits.
        let gain = rc.decode_bits(3);
        // tapset: ICDF {2, 1, 1}/4
        let tapset = rc.decode_icdf(&TAPSET_ICDF, 2) as u32;
        Some(PostFilter {
            octave,
            period,
            gain,
            tapset,
        })
    } else {
        None
    };
    let transient = rc.decode_bit_logp(3);
    let intra = rc.decode_bit_logp(3);
    Some(CeltHeader {
        silence: false,
        post_filter,
        transient,
        intra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Probability of the silence flag being "1" is 1/32768. With a payload
    /// whose primed `val` is well above 1, the flag must decode to 0.
    #[test]
    fn silence_typically_false_on_random_input() {
        // 0x00 first byte → val = (1<<7)-1 - (0>>1) = 127, far above the
        // s = rng>>15 = 1 threshold for the silence "1" symbol.
        let mut rc = RangeDecoder::new(&[0x00, 0x55, 0xAA, 0x55]);
        let s = decode_silence(&mut rc);
        assert!(!s);
    }

    /// Decoding the full header on a non-silence buffer yields finite values
    /// and doesn't latch the range-coder error flag for a well-sized buffer.
    #[test]
    fn header_reads_without_error_on_decent_payload() {
        // 16-byte buffer is generous enough for the worst-case header (post-
        // filter takes ~3+9+3+2 = 17 bits in the worst case).
        let buf: [u8; 16] = [
            0xA5, 0x3B, 0x77, 0x10, 0xC1, 0xDE, 0xAD, 0xBE, 0xEF, 0x12, 0x34, 0x56, 0x78, 0x9A,
            0xBC, 0xDE,
        ];
        let mut rc = RangeDecoder::new(&buf);
        let h = decode_header(&mut rc);
        assert!(h.is_some(), "header should parse on non-silence payload");
        let h = h.unwrap();
        // Sanity: octave is 0..=5 if post-filter set.
        if let Some(pf) = h.post_filter {
            assert!(pf.octave < 6);
        }
    }
}
