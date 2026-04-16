//! AV1 bitstream reader.
//!
//! AV1 (AOMedia Video 1, AOM Bitstream & Decoding Process Specification 2019)
//! is read MSB-first inside each byte. The spec defines several primitive
//! readers used by every higher-level parser:
//!
//! * `f(n)` — unsigned `n`-bit field, big-endian. §4.10.2.
//! * `su(n)` — signed two's-complement `n`-bit field. §4.10.5.
//! * `uvlc()` — universal variable-length code. §4.10.3.
//! * `leb128()` — little-endian variable-length unsigned integer carrying
//!   continuation bit `0x80`; up to 8 bytes / 56 useful bits. §4.10.5.
//!
//! This module provides a `BitReader` over a borrowed `&[u8]` plus
//! convenience wrappers for the AV1-specific encodings. Byte-align is also
//! exposed because OBU payload boundaries are byte-aligned.

use oxideav_core::{Error, Result};

/// MSB-first bit reader over a byte slice.
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Next byte to refill from `data`.
    byte_pos: usize,
    /// Accumulator: high bits-in-acc bits hold the unread MSB-first bits.
    acc: u64,
    bits_in_acc: u32,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            acc: 0,
            bits_in_acc: 0,
        }
    }

    /// Bit position relative to the start of the slice. Useful for §5.9.4
    /// `payload_bits` calculations.
    pub fn bit_position(&self) -> u64 {
        self.byte_pos as u64 * 8 - self.bits_in_acc as u64
    }

    pub fn is_byte_aligned(&self) -> bool {
        self.bits_in_acc % 8 == 0
    }

    pub fn align_to_byte(&mut self) {
        let drop = self.bits_in_acc % 8;
        self.acc <<= drop;
        self.bits_in_acc -= drop;
    }

    pub fn bits_remaining(&self) -> u64 {
        (self.data.len() as u64 - self.byte_pos as u64) * 8 + self.bits_in_acc as u64
    }

    fn refill(&mut self) {
        while self.bits_in_acc <= 56 && self.byte_pos < self.data.len() {
            self.acc |= (self.data[self.byte_pos] as u64) << (56 - self.bits_in_acc);
            self.bits_in_acc += 8;
            self.byte_pos += 1;
        }
    }

    /// `f(n)` — read an unsigned `n`-bit field MSB-first. `n` must be ≤ 32.
    pub fn f(&mut self, n: u32) -> Result<u32> {
        debug_assert!(n <= 32);
        if n == 0 {
            return Ok(0);
        }
        if self.bits_in_acc < n {
            self.refill();
            if self.bits_in_acc < n {
                return Err(Error::invalid("av1 bitreader: out of bits"));
            }
        }
        let v = (self.acc >> (64 - n)) as u32;
        self.acc <<= n;
        self.bits_in_acc -= n;
        Ok(v)
    }

    /// `f(n)` for n in 33..=64.
    pub fn f64(&mut self, n: u32) -> Result<u64> {
        debug_assert!(n <= 64);
        if n <= 32 {
            return self.f(n).map(|v| v as u64);
        }
        let high = self.f(n - 32)? as u64;
        let low = self.f(32)? as u64;
        Ok((high << 32) | low)
    }

    /// `su(n)` — signed two's-complement `n`-bit field.
    pub fn su(&mut self, n: u32) -> Result<i32> {
        let v = self.f(n)?;
        if n == 0 {
            return Ok(0);
        }
        let sign = 1u32 << (n - 1);
        if v & sign != 0 {
            Ok(v as i32 - (1i64 << n) as i32)
        } else {
            Ok(v as i32)
        }
    }

    /// Read 1 bit.
    pub fn bit(&mut self) -> Result<bool> {
        Ok(self.f(1)? != 0)
    }

    /// `uvlc()` — universal variable-length code (§4.10.3).
    /// `leadingZeros` zero bits followed by a `1` and then `leadingZeros` payload
    /// bits. Returns `value = (1 << leadingZeros) - 1 + read_bits(leadingZeros)`.
    /// The spec caps leadingZeros at 32 (any 32-or-more leading zeros indicates
    /// a value of `0xFFFF_FFFF`).
    pub fn uvlc(&mut self) -> Result<u32> {
        let mut leading_zeros = 0u32;
        loop {
            if leading_zeros >= 32 {
                return Ok(u32::MAX);
            }
            let b = self.f(1)?;
            if b == 1 {
                break;
            }
            leading_zeros += 1;
        }
        if leading_zeros == 32 {
            return Ok(u32::MAX);
        }
        let value = self.f(leading_zeros)?;
        Ok(value + ((1u32 << leading_zeros) - 1))
    }

    /// `leb128()` — little-endian unsigned variable-length integer.
    /// Up to 8 bytes are consumed; each byte's low 7 bits contribute payload,
    /// MSB is the continuation bit. The value is byte-aligned in the bitstream.
    pub fn leb128(&mut self) -> Result<u64> {
        if !self.is_byte_aligned() {
            return Err(Error::invalid("av1 leb128: not byte-aligned"));
        }
        // Drain accumulator first if any whole bytes are buffered.
        let mut value: u64 = 0;
        let mut leb128_bytes: u32 = 0;
        for i in 0..8u32 {
            let b = self.f(8)? as u64;
            value |= (b & 0x7f) << (i * 7);
            leb128_bytes += 1;
            if (b & 0x80) == 0 {
                break;
            }
        }
        let _ = leb128_bytes;
        Ok(value)
    }

    /// `ns(n)` — non-symmetric uniform integer (§4.10.6). Used in tile column /
    /// row counts. `n` MUST be > 0.
    pub fn ns(&mut self, n: u32) -> Result<u32> {
        if n == 0 {
            return Err(Error::invalid("av1 ns: n must be > 0"));
        }
        let w = ceil_log2(n);
        let m = (1u32 << w) - n;
        let v = self.f(w - 1)?;
        if v < m {
            return Ok(v);
        }
        let extra_bit = self.f(1)?;
        Ok((v << 1) - m + extra_bit)
    }

    /// `trailing_bits(nbBits)` — verify the trailing 1 + zeros pattern (§5.3.4).
    pub fn trailing_bits(&mut self, nb_bits: u32) -> Result<()> {
        if nb_bits == 0 {
            return Ok(());
        }
        let one = self.f(1)?;
        if one != 1 {
            return Err(Error::invalid("av1: trailing_bits: missing trailing one"));
        }
        for _ in 1..nb_bits {
            let z = self.f(1)?;
            if z != 0 {
                return Err(Error::invalid("av1: trailing_bits: nonzero zero-bit"));
            }
        }
        Ok(())
    }

    /// `byte_alignment()` — pad with zero bits up to the next byte boundary
    /// (§5.3.5). Bits MUST be zero per spec — we tolerate non-zero values
    /// for robustness in real-world streams.
    pub fn byte_alignment(&mut self) -> Result<()> {
        while !self.is_byte_aligned() {
            self.f(1)?;
        }
        Ok(())
    }
}

/// Ceiling of log2(x). Returns 0 for `x <= 1`.
pub fn ceil_log2(x: u32) -> u32 {
    if x <= 1 {
        return 0;
    }
    32 - (x - 1).leading_zeros()
}

/// Floor of log2(x). Returns 0 for `x == 0`.
pub fn floor_log2(x: u32) -> u32 {
    if x == 0 {
        return 0;
    }
    31 - x.leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f_msb_first() {
        let data = [0b1011_0001u8, 0b0101_0101];
        let mut br = BitReader::new(&data);
        assert_eq!(br.f(1).unwrap(), 1);
        assert_eq!(br.f(2).unwrap(), 0b01);
        assert_eq!(br.f(5).unwrap(), 0b1_0001);
        assert_eq!(br.f(8).unwrap(), 0b0101_0101);
    }

    #[test]
    fn leb128_roundtrip() {
        // 300 → 0xAC 0x02
        let data = [0xAC, 0x02];
        let mut br = BitReader::new(&data);
        assert_eq!(br.leb128().unwrap(), 300);
    }

    #[test]
    fn leb128_single_byte() {
        let data = [0x10];
        let mut br = BitReader::new(&data);
        assert_eq!(br.leb128().unwrap(), 0x10);
    }

    #[test]
    fn uvlc_zero() {
        // "1" — leading_zeros=0, no payload bits, value=0
        let data = [0b1000_0000];
        let mut br = BitReader::new(&data);
        assert_eq!(br.uvlc().unwrap(), 0);
    }

    #[test]
    fn uvlc_big() {
        // leading_zeros = 3 (000), then 1, then 3 bits = 0b101 → value = (1<<3)-1 + 5 = 12
        let data = [0b0001_1010];
        let mut br = BitReader::new(&data);
        assert_eq!(br.uvlc().unwrap(), 12);
    }

    #[test]
    fn ceil_log2_basic() {
        assert_eq!(ceil_log2(0), 0);
        assert_eq!(ceil_log2(1), 0);
        assert_eq!(ceil_log2(2), 1);
        assert_eq!(ceil_log2(3), 2);
        assert_eq!(ceil_log2(4), 2);
        assert_eq!(ceil_log2(5), 3);
        assert_eq!(ceil_log2(8), 3);
        assert_eq!(ceil_log2(9), 4);
    }
}
