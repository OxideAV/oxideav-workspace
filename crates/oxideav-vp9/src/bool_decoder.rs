//! VP9 boolean (range) decoder.
//!
//! Reference: VP9 Bitstream & Decoding Process Specification, version 0.7
//! (2017), §9.2 ("Boolean decoding process").
//!
//! Implementation note: the spec phrases everything in terms of a bit-stream
//! `f(1)` reader that the bool engine pulls from. We model the underlying
//! bit pump as a tiny MSB-first reader that lazily refills a byte buffer
//! when its 8-bit window runs out. This keeps the bool engine itself
//! straightforward to read against §9.2.2.

use oxideav_core::{Error, Result};

/// Tiny MSB-first bit pump backing the bool engine. Reads `f(1)` /
/// `f(8)` style fields from the underlying byte slice.
struct BitPump<'a> {
    data: &'a [u8],
    byte_pos: usize,
    /// Number of unread bits in the current byte (1..=8). When 0, we need
    /// to fetch another byte.
    bits_left: u32,
    /// Current byte buffered (low 8 bits).
    cur: u32,
}

impl<'a> BitPump<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bits_left: 0,
            cur: 0,
        }
    }

    fn read_bit(&mut self) -> Result<u32> {
        if self.bits_left == 0 {
            if self.byte_pos >= self.data.len() {
                // §9.2 final paragraph: zero-pad past end.
                return Ok(0);
            }
            self.cur = self.data[self.byte_pos] as u32;
            self.byte_pos += 1;
            self.bits_left = 8;
        }
        self.bits_left -= 1;
        Ok((self.cur >> self.bits_left) & 1)
    }

    fn read_bits(&mut self, n: u32) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.read_bit()?;
        }
        Ok(v)
    }

    fn pos(&self) -> usize {
        self.byte_pos
    }
}

pub struct BoolDecoder<'a> {
    pump: BitPump<'a>,
    /// `BoolRange` — current range [128, 255].
    range: u32,
    /// `BoolValue` — 8-bit window into the renormalised value buffer.
    value: u32,
}

impl<'a> BoolDecoder<'a> {
    /// `init_bool( sz )` from VP9 §9.2.1.
    pub fn new(data: &'a [u8]) -> Result<Self> {
        if data.is_empty() {
            return Err(Error::invalid("vp9 bool decoder: empty payload"));
        }
        let mut pump = BitPump::new(data);
        let value = pump.read_bits(8)?;
        // marker bit f(1) must be 0
        let marker = pump.read_bit()?;
        if marker != 0 {
            return Err(Error::invalid(
                "vp9 bool decoder: §9.2.1 marker bit must be zero",
            ));
        }
        Ok(Self {
            pump,
            range: 255,
            value,
        })
    }

    /// `boolean( p )` — VP9 §9.2.2. Returns the decoded bit (0 or 1) for
    /// the 8-bit "probability that the bit is 0" `p`.
    pub fn read(&mut self, p: u8) -> Result<u32> {
        let split = 1 + (((self.range - 1) * p as u32) >> 8);
        let bit = if self.value < split {
            self.range = split;
            0
        } else {
            self.range -= split;
            self.value -= split;
            1
        };
        // Renormalise: shift in bits until range >= 128.
        while self.range < 128 {
            self.range <<= 1;
            self.value = (self.value << 1) | self.pump.read_bit()?;
        }
        Ok(bit)
    }

    /// Convenience returning a `bool`.
    pub fn read_bool(&mut self, p: u8) -> Result<bool> {
        Ok(self.read(p)? != 0)
    }

    /// `decode_literal(n)` — read `n` equiprobable bits, MSB-first.
    pub fn read_literal(&mut self, n: u32) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.read(128)?;
        }
        Ok(v)
    }

    /// `decode_uniform()` — read an unsigned integer in `[0, n)` assuming
    /// uniform distribution.
    pub fn read_uniform(&mut self, n: u32) -> Result<u32> {
        if n <= 1 {
            return Ok(0);
        }
        let l = 32 - (n - 1).leading_zeros();
        let m = (1u32 << l) - n;
        let mut v = 0u32;
        for _ in 0..(l - 1) {
            v = (v << 1) | self.read_literal(1)?;
        }
        if v < m {
            Ok(v)
        } else {
            let extra = self.read_literal(1)?;
            Ok((v << 1) - m + extra)
        }
    }

    /// Number of bytes consumed from the input so far.
    pub fn pos(&self) -> usize {
        self.pump.pos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_marker_must_be_zero() {
        // Init reads f(8) into BoolValue, then f(1) marker. With our pump
        // (byte-granular MSB-first), the first 8 bits are byte 0 and the
        // 9th bit is the MSB of byte 1. So marker=0 requires byte 1's
        // MSB to be 0, e.g. 0x00..0x7F.
        assert!(BoolDecoder::new(&[0xAB, 0x00, 0xCD]).is_ok());
        assert!(BoolDecoder::new(&[0xAB, 0xFF, 0xCD]).is_err());
    }

    /// Read several bits at p=128. Should not panic / EOF.
    #[test]
    fn read_literals_smoke() {
        let buf = [0xABu8, 0x10, 0xCD, 0xEF, 0x12, 0x34, 0x56, 0x78];
        let mut bd = BoolDecoder::new(&buf).unwrap();
        for _ in 0..32 {
            bd.read(128).unwrap();
        }
    }
}
