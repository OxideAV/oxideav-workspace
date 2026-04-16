//! SILK-specific range-coder helpers.
//!
//! SILK shares the exact same arithmetic coder as CELT (RFC 6716 §4.1)
//! — we re-export [`RangeDecoder`] from the CELT crate and add a few
//! small adapters to match the naming conventions in RFC §4.2.
//!
//! Only thin wrappers here so that the rest of the SILK module reads
//! like the RFC pseudocode.

pub use oxideav_celt::range_decoder::RangeDecoder;

/// Decode an ICDF symbol from a table with `ft = 256`.
pub fn dec_icdf8(rc: &mut RangeDecoder<'_>, icdf: &[u8]) -> usize {
    rc.decode_icdf(icdf, 8)
}

/// Decode a uniform integer in [0, n).
pub fn dec_uniform(rc: &mut RangeDecoder<'_>, n: u32) -> u32 {
    rc.decode_uint(n)
}

/// Decode a single binary symbol with 50% probability.
pub fn dec_bit(rc: &mut RangeDecoder<'_>) -> bool {
    rc.decode_bit_logp(1)
}
