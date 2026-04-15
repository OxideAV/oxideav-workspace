//! MSB-first bit reader for FLAC bitstreams.
//!
//! FLAC stores all multi-bit fields with the most-significant bit first within
//! each byte. The reader keeps a small accumulator so callers can request
//! arbitrary widths up to 32 bits in one go.

use oxideav_core::{Error, Result};

pub struct BitReader<'a> {
    data: &'a [u8],
    /// Index of the next byte to load into the accumulator.
    byte_pos: usize,
    /// Bits buffered from `data`, left-aligned in `acc` (high bits = next).
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

    /// Number of bits already consumed from the stream.
    pub fn bit_position(&self) -> u64 {
        self.byte_pos as u64 * 8 - self.bits_in_acc as u64
    }

    /// Bytes already consumed (assumes byte-aligned position).
    pub fn byte_position(&self) -> usize {
        debug_assert_eq!(
            self.bits_in_acc % 8,
            0,
            "byte_position requires byte alignment"
        );
        self.byte_pos - (self.bits_in_acc as usize) / 8
    }

    /// True if the reader is positioned on a byte boundary.
    pub fn is_byte_aligned(&self) -> bool {
        self.bits_in_acc % 8 == 0
    }

    /// Skip remaining bits in the current byte, leaving the reader byte-aligned.
    pub fn align_to_byte(&mut self) {
        let drop = self.bits_in_acc % 8;
        self.acc <<= drop;
        self.bits_in_acc -= drop;
    }

    /// Reload the accumulator from the underlying slice.
    fn refill(&mut self) -> Result<()> {
        while self.bits_in_acc <= 56 && self.byte_pos < self.data.len() {
            self.acc |= (self.data[self.byte_pos] as u64) << (56 - self.bits_in_acc);
            self.bits_in_acc += 8;
            self.byte_pos += 1;
        }
        Ok(())
    }

    /// Read `n` bits (0..=32) as an unsigned integer.
    pub fn read_u32(&mut self, n: u32) -> Result<u32> {
        debug_assert!(n <= 32, "BitReader::read_u32 supports up to 32 bits");
        if n == 0 {
            return Ok(0);
        }
        if self.bits_in_acc < n {
            self.refill()?;
            if self.bits_in_acc < n {
                return Err(Error::invalid("BitReader: out of bits"));
            }
        }
        let v = (self.acc >> (64 - n)) as u32;
        self.acc <<= n;
        self.bits_in_acc -= n;
        Ok(v)
    }

    /// Read `n` bits as a signed integer (sign-extended from the high bit).
    pub fn read_i32(&mut self, n: u32) -> Result<i32> {
        if n == 0 {
            return Ok(0);
        }
        let raw = self.read_u32(n)? as i32;
        // Sign-extend.
        let shift = 32 - n;
        Ok((raw << shift) >> shift)
    }

    /// Read a single bit as bool.
    pub fn read_bit(&mut self) -> Result<bool> {
        Ok(self.read_u32(1)? != 0)
    }

    /// Read a unary-coded value: count of leading zero bits, terminated by a 1.
    pub fn read_unary(&mut self) -> Result<u32> {
        let mut count = 0u32;
        loop {
            if self.bits_in_acc == 0 {
                self.refill()?;
                if self.bits_in_acc == 0 {
                    return Err(Error::invalid("BitReader: out of bits in unary code"));
                }
            }
            // Count leading zeros in the accumulator's high bits.
            let lz_total = self.acc.leading_zeros();
            let lz_avail = lz_total.min(self.bits_in_acc);
            count = count
                .checked_add(lz_avail)
                .ok_or_else(|| Error::invalid("BitReader: unary count overflow"))?;
            self.acc <<= lz_avail;
            self.bits_in_acc -= lz_avail;
            if lz_avail < lz_total {
                // We hit the end of the buffered bits without finding a 1 —
                // loop and refill.
                continue;
            }
            // Consume the terminating 1 bit.
            self.acc <<= 1;
            self.bits_in_acc -= 1;
            return Ok(count);
        }
    }

    /// Read a UTF-8-encoded variable-length integer (FLAC frame/sample number).
    /// Same prefix-byte scheme as UTF-8, supports up to 36-bit FLAC sample
    /// numbers (lead byte 0xFE → 6 continuation bytes × 6 bits = 36 payload
    /// bits with 0 payload bits in the lead).
    pub fn read_utf8_u64(&mut self) -> Result<u64> {
        if !self.is_byte_aligned() {
            return Err(Error::invalid(
                "BitReader: read_utf8_u64 requires byte alignment",
            ));
        }
        let b0 = self.read_u32(8)? as u8;
        // Lead byte starts with N ones followed by a 0. Payload bits in lead = 7 - N.
        let (n_extra, lead_payload_bits) = match b0 {
            0x00..=0x7F => (0u32, 7u32), // 0xxxxxxx
            0xC0..=0xDF => (1, 5),       // 110xxxxx
            0xE0..=0xEF => (2, 4),       // 1110xxxx
            0xF0..=0xF7 => (3, 3),       // 11110xxx
            0xF8..=0xFB => (4, 2),       // 111110xx
            0xFC..=0xFD => (5, 1),       // 1111110x
            0xFE => (6, 0),              // 11111110
            _ => return Err(Error::invalid("invalid UTF-8 leading byte")),
        };
        let lead_mask: u8 = if lead_payload_bits == 0 {
            0
        } else {
            ((1u16 << lead_payload_bits) - 1) as u8
        };
        let mut value = (b0 & lead_mask) as u64;
        for _ in 0..n_extra {
            let cont = self.read_u32(8)? as u8;
            if cont & 0xC0 != 0x80 {
                return Err(Error::invalid("invalid UTF-8 continuation byte"));
            }
            value = (value << 6) | ((cont & 0x3F) as u64);
        }
        Ok(value)
    }

    /// Read `n` bytes from the stream (requires byte alignment).
    pub fn read_bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        if !self.is_byte_aligned() {
            return Err(Error::invalid(
                "BitReader: read_bytes requires byte alignment",
            ));
        }
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.read_u32(8)? as u8);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_u32_small() {
        // 0b1010_0101_1100_0011 = 0xA5C3
        let mut br = BitReader::new(&[0xA5, 0xC3]);
        assert_eq!(br.read_u32(4).unwrap(), 0xA);
        assert_eq!(br.read_u32(4).unwrap(), 0x5);
        assert_eq!(br.read_u32(8).unwrap(), 0xC3);
    }

    #[test]
    fn read_i32_negative() {
        // 4 bits: 0b1111 should be -1 when read as signed.
        let mut br = BitReader::new(&[0xFF]);
        assert_eq!(br.read_i32(4).unwrap(), -1);
        // Next 4 bits: 0b1111 → -1 again.
        assert_eq!(br.read_i32(4).unwrap(), -1);
    }

    #[test]
    fn unary_codes() {
        // Bits: 0b11110_001_1... = unary 0, 0, 0, 0, then unary 3 (000 then 1)
        let mut br = BitReader::new(&[0b11110_001, 0b1_0000000]);
        assert_eq!(br.read_unary().unwrap(), 0);
        assert_eq!(br.read_unary().unwrap(), 0);
        assert_eq!(br.read_unary().unwrap(), 0);
        assert_eq!(br.read_unary().unwrap(), 0);
        assert_eq!(br.read_unary().unwrap(), 3);
    }

    #[test]
    fn utf8_round_trip() {
        // Encode a sample number 0x12345 by hand: 5 hex digits → 21 bits → 4-byte UTF-8.
        // 11110xxx 10xxxxxx 10xxxxxx 10xxxxxx — 3+6+6+6 = 21 bits.
        let v = 0x12345u32;
        let b0 = 0xF0u8 | ((v >> 18) & 0x07) as u8;
        let b1 = 0x80u8 | ((v >> 12) & 0x3F) as u8;
        let b2 = 0x80u8 | ((v >> 6) & 0x3F) as u8;
        let b3 = 0x80u8 | (v & 0x3F) as u8;
        let bytes = [b0, b1, b2, b3];
        let mut br = BitReader::new(&bytes);
        assert_eq!(br.read_utf8_u64().unwrap() as u32, v);
    }

    #[test]
    fn alignment() {
        let mut br = BitReader::new(&[0xFF, 0xFF]);
        let _ = br.read_u32(3).unwrap();
        assert!(!br.is_byte_aligned());
        br.align_to_byte();
        assert!(br.is_byte_aligned());
        assert_eq!(br.read_u32(8).unwrap(), 0xFF);
    }
}
