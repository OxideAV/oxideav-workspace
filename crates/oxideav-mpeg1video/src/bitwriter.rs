//! MSB-first bit writer for MPEG-1 video bitstreams.
//!
//! Mirrors the layout assumed by the matching `BitReader`: bits are packed
//! MSB-first within each byte. After all data is written, `finish()` zero-pads
//! the trailing partial byte.

pub struct BitWriter {
    buf: Vec<u8>,
    /// Bit accumulator. Bits are stored in the low `nbits` positions, ready
    /// to be shifted into the next output byte from the MSB side.
    acc: u64,
    /// How many bits currently held in `acc`.
    nbits: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(4096),
            acc: 0,
            nbits: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
            acc: 0,
            nbits: 0,
        }
    }

    /// Write the low `len` bits of `value`, MSB-first. `len` must be ≤ 32.
    pub fn write_bits(&mut self, value: u32, len: u32) {
        debug_assert!(len <= 32);
        if len == 0 {
            return;
        }
        let mask = if len == 32 {
            u32::MAX
        } else {
            (1u32 << len) - 1
        };
        let v = value & mask;
        self.acc = (self.acc << len) | v as u64;
        self.nbits += len;
        while self.nbits >= 8 {
            self.nbits -= 8;
            let b = ((self.acc >> self.nbits) & 0xFF) as u8;
            self.buf.push(b);
        }
    }

    /// Pad to the next byte boundary with zero bits.
    pub fn align_to_byte(&mut self) {
        if self.nbits > 0 {
            let pad = 8 - self.nbits;
            self.write_bits(0, pad);
        }
    }

    /// Returns the current bit position from start of stream.
    pub fn bit_position(&self) -> u64 {
        self.buf.len() as u64 * 8 + self.nbits as u64
    }

    /// True if the next write would land on a byte boundary.
    pub fn is_byte_aligned(&self) -> bool {
        self.nbits == 0
    }

    /// Append a raw byte. Caller must ensure the writer is byte-aligned.
    pub fn write_byte(&mut self, b: u8) {
        debug_assert!(self.is_byte_aligned());
        self.buf.push(b);
    }

    /// Append raw bytes verbatim (e.g. for start codes). Caller must be
    /// byte-aligned.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        debug_assert!(self.is_byte_aligned());
        self.buf.extend_from_slice(bytes);
    }

    /// Flush any partial byte (zero-padded) and return the buffer.
    pub fn finish(mut self) -> Vec<u8> {
        self.align_to_byte();
        self.buf
    }

    /// Borrow the byte buffer without consuming. Only meaningful when byte-
    /// aligned.
    pub fn buffer(&self) -> &[u8] {
        debug_assert!(self.is_byte_aligned());
        &self.buf
    }
}

impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_msb_first() {
        // Write `1`, `01`, `10001`, then `01010101` — bits 1+2+5+8 = 16 bits.
        // Same pattern as bitreader::tests::basic_msb_first.
        let mut bw = BitWriter::new();
        bw.write_bits(1, 1);
        bw.write_bits(0b01, 2);
        bw.write_bits(0b1_0001, 5);
        bw.write_bits(0b0101_0101, 8);
        let out = bw.finish();
        assert_eq!(out, vec![0b1011_0001, 0b0101_0101]);
    }

    #[test]
    fn pad_zero_to_byte() {
        let mut bw = BitWriter::new();
        bw.write_bits(0b101, 3);
        let out = bw.finish();
        // 101 pad with 5 zero bits → 1010_0000
        assert_eq!(out, vec![0b1010_0000]);
    }

    #[test]
    fn writes_long_value() {
        let mut bw = BitWriter::new();
        bw.write_bits(0xDEAD_BEEF, 32);
        let out = bw.finish();
        assert_eq!(out, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn round_trip_via_bitreader() {
        use crate::bitreader::BitReader;
        let mut bw = BitWriter::new();
        let payload = [
            (0xAB, 8u32),
            (0b101, 3),
            (0b1100, 4),
            (0xFFFF_FFFF, 32),
            (0, 1),
            (0x1, 1),
        ];
        for (v, n) in payload.iter().copied() {
            bw.write_bits(v, n);
        }
        let bytes = bw.finish();
        let mut br = BitReader::new(&bytes);
        for (v, n) in payload.iter().copied() {
            let got = br.read_u32(n).unwrap();
            assert_eq!(got, v, "round-trip mismatch on {n}-bit field");
        }
    }
}
