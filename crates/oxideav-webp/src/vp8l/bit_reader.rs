//! LSB-first bit reader for VP8L.
//!
//! VP8L packs bits with the low bit of each byte coming out first — the
//! opposite convention from VP8's boolean arithmetic decoder. This reader
//! is a straightforward buffered shift accumulator: whenever fewer than 32
//! bits remain in the buffer we pull one more byte in.

use oxideav_core::Result;

pub struct BitReader<'a> {
    buf: &'a [u8],
    /// Next byte to consume.
    byte_pos: usize,
    /// Accumulator of pending bits (LSB-first).
    bits: u64,
    /// Valid bits currently in `bits`.
    nbits: u32,
}

impl<'a> BitReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            byte_pos: 0,
            bits: 0,
            nbits: 0,
        }
    }

    /// Read `n` bits (0..=32) and return them as a u32, LSB-first. Past
    /// end-of-buffer reads return zero bits — matching libwebp's
    /// well-defined trailing-zero behaviour. Callers that care about
    /// catching truncation should watch [`Self::at_end`].
    pub fn read_bits(&mut self, n: u8) -> Result<u32> {
        debug_assert!(n <= 32);
        while self.nbits < n as u32 {
            if self.byte_pos >= self.buf.len() {
                // Inject zeros — we deliberately do *not* error here.
                self.nbits += 8;
                continue;
            }
            self.bits |= (self.buf[self.byte_pos] as u64) << self.nbits;
            self.byte_pos += 1;
            self.nbits += 8;
        }
        let mask = if n == 0 { 0u64 } else { (1u64 << n) - 1 };
        let v = (self.bits & mask) as u32;
        self.bits >>= n;
        self.nbits -= n as u32;
        Ok(v)
    }

    /// Read exactly one bit.
    pub fn read_bit(&mut self) -> Result<u32> {
        self.read_bits(1)
    }

    /// True if we've read past the physical end of the underlying buffer.
    pub fn at_end(&self) -> bool {
        self.byte_pos >= self.buf.len()
    }

    /// Current byte position (useful for debugging).
    pub fn byte_pos(&self) -> usize {
        self.byte_pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lsb_first() {
        // 0b1011_0001 → LSB-first yields 1, 0, 0, 0, 1, 1, 0, 1
        let buf = [0b1011_0001u8];
        let mut br = BitReader::new(&buf);
        assert_eq!(br.read_bits(1).unwrap(), 1);
        assert_eq!(br.read_bits(1).unwrap(), 0);
        assert_eq!(br.read_bits(1).unwrap(), 0);
        assert_eq!(br.read_bits(1).unwrap(), 0);
        assert_eq!(br.read_bits(4).unwrap(), 0b1011);
    }

    #[test]
    fn crosses_byte_boundaries() {
        let buf = [0xff, 0x01];
        let mut br = BitReader::new(&buf);
        let v = br.read_bits(12).unwrap();
        assert_eq!(v, 0x1ff);
    }

    #[test]
    fn read_16_bits() {
        let buf = [0x34, 0x12];
        let mut br = BitReader::new(&buf);
        assert_eq!(br.read_bits(16).unwrap(), 0x1234);
    }
}
