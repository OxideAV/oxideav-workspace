//! MSB-first bit reader used to parse the VP9 uncompressed header.
//!
//! Reference: VP9 Bitstream & Decoding Process Specification, version 0.7
//! (2017), §4.10.2 ("f(n)" — MSB-first n-bit unsigned literal).

use oxideav_core::{Error, Result};

/// MSB-first bit reader, matching VP9 §4.10.2 `f(n)`.
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Byte offset of the next byte to fetch.
    byte_pos: usize,
    /// Buffered bits, high-aligned (next bit to emit is bit 63 of `acc`).
    acc: u64,
    /// Number of valid bits currently in `acc` (0..=64).
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

    /// Total bits consumed so far (VP9 spec uses this to compute
    /// `header_size` payload offset).
    pub fn bit_position(&self) -> u64 {
        self.byte_pos as u64 * 8 - self.bits_in_acc as u64
    }

    /// Byte position rounded up to the next byte boundary — used after
    /// uncompressed-header parsing to find the start of the compressed
    /// header (VP9 §6.2).
    pub fn byte_aligned_position(&self) -> usize {
        let bits = self.bit_position();
        bits.div_ceil(8) as usize
    }

    fn refill(&mut self) {
        while self.bits_in_acc <= 56 && self.byte_pos < self.data.len() {
            self.acc |= (self.data[self.byte_pos] as u64) << (56 - self.bits_in_acc);
            self.bits_in_acc += 8;
            self.byte_pos += 1;
        }
    }

    /// `f(n)` — read up to 32 bits MSB-first.
    pub fn f(&mut self, n: u32) -> Result<u32> {
        debug_assert!(n <= 32);
        if n == 0 {
            return Ok(0);
        }
        if self.bits_in_acc < n {
            self.refill();
            if self.bits_in_acc < n {
                return Err(Error::Eof);
            }
        }
        let v = (self.acc >> (64 - n)) as u32;
        self.acc <<= n;
        self.bits_in_acc -= n;
        Ok(v)
    }

    /// `s(n)` — read an `n`-bit unsigned magnitude followed by a 1-bit
    /// sign (VP9 §4.10.6).
    pub fn s(&mut self, n: u32) -> Result<i32> {
        let mag = self.f(n)? as i32;
        let sign = self.f(1)?;
        Ok(if sign == 0 { mag } else { -mag })
    }

    /// Convenience for a single bit.
    pub fn bit(&mut self) -> Result<bool> {
        Ok(self.f(1)? != 0)
    }

    /// Skip `n` bits.
    pub fn skip(&mut self, n: u32) -> Result<()> {
        let mut left = n;
        while left > 32 {
            self.f(32)?;
            left -= 32;
        }
        if left > 0 {
            self.f(left)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msb_first() {
        let mut br = BitReader::new(&[0xA5]);
        assert_eq!(br.f(1).unwrap(), 1);
        assert_eq!(br.f(1).unwrap(), 0);
        assert_eq!(br.f(2).unwrap(), 0b10);
        assert_eq!(br.f(4).unwrap(), 0b0101);
    }

    #[test]
    fn signed_magnitude() {
        // bits: 0000_0010 1 -> magnitude=2, sign=1 -> -2
        let mut br = BitReader::new(&[0b0000_0010, 0b1000_0000]);
        assert_eq!(br.s(8).unwrap(), -2);
    }
}
