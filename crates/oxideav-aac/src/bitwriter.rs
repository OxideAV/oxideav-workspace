//! MSB-first bit writer — the inverse of [`crate::bitreader::BitReader`].
//!
//! AAC bitstreams (ISO/IEC 14496-3) pack all multi-bit fields most-
//! significant-bit first within each byte. Huffman codewords are also
//! emitted MSB-first.

/// MSB-first bit writer over an internal byte buffer.
pub struct BitWriter {
    data: Vec<u8>,
    /// Bits buffered at the *high* end of `acc` (next-to-emit at top).
    acc: u64,
    /// Number of valid bits currently in `acc` (0..=64).
    bits_in_acc: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            acc: 0,
            bits_in_acc: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            data: Vec::with_capacity(cap),
            acc: 0,
            bits_in_acc: 0,
        }
    }

    /// Total bits written so far (including any in the unflushed accumulator).
    pub fn bit_position(&self) -> u64 {
        self.data.len() as u64 * 8 + self.bits_in_acc as u64
    }

    /// Append `n` bits (0..=32) from the low `n` bits of `value`, MSB first.
    pub fn write_u32(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32, "BitWriter::write_u32 supports up to 32 bits");
        if n == 0 {
            return;
        }
        let mask: u32 = if n == 32 { u32::MAX } else { (1u32 << n) - 1 };
        let v = (value & mask) as u64;
        // Make room at the high end. After this `bits_in_acc + n` <= 64
        // because we drain to bytes whenever the top byte is full.
        let shift = 64 - self.bits_in_acc - n;
        self.acc |= v << shift;
        self.bits_in_acc += n;
        while self.bits_in_acc >= 8 {
            let byte = (self.acc >> 56) as u8;
            self.data.push(byte);
            self.acc <<= 8;
            self.bits_in_acc -= 8;
        }
    }

    pub fn write_u64(&mut self, value: u64, n: u32) {
        debug_assert!(n <= 64);
        if n <= 32 {
            self.write_u32(value as u32, n);
        } else {
            // High bits first.
            self.write_u32((value >> 32) as u32, n - 32);
            self.write_u32(value as u32, 32);
        }
    }

    pub fn write_bit(&mut self, bit: bool) {
        self.write_u32(bit as u32, 1);
    }

    /// Pad to the next byte boundary with zero bits.
    pub fn align_to_byte(&mut self) {
        let pad = (8 - self.bits_in_acc % 8) % 8;
        if pad > 0 {
            self.write_u32(0, pad);
        }
    }

    /// Borrow the bytes accumulated so far (excluding any unflushed partial byte).
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Pad with zero bits to the next byte boundary, then return the bytes.
    pub fn finish(mut self) -> Vec<u8> {
        if self.bits_in_acc > 0 {
            let byte = (self.acc >> 56) as u8;
            self.data.push(byte);
            self.acc = 0;
            self.bits_in_acc = 0;
        }
        self.data
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
    use crate::bitreader::BitReader;

    #[test]
    fn roundtrip_msb_first_byte() {
        let mut w = BitWriter::new();
        // Write the bits of 0xA5 = 10100101 MSB first.
        for &b in &[1u32, 0, 1, 0, 0, 1, 0, 1] {
            w.write_u32(b, 1);
        }
        assert_eq!(w.finish(), vec![0xA5]);
    }

    #[test]
    fn roundtrip_with_reader() {
        let mut w = BitWriter::new();
        w.write_u32(5, 3);
        w.write_u32(0xABCD, 16);
        w.write_u32(0x1234567, 27);
        w.write_bit(true);
        let bytes = w.finish();

        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_u32(3).unwrap(), 5);
        assert_eq!(r.read_u32(16).unwrap(), 0xABCD);
        assert_eq!(r.read_u32(27).unwrap(), 0x1234567);
        assert!(r.read_bit().unwrap());
    }

    #[test]
    fn align_pads_to_byte() {
        let mut w = BitWriter::new();
        w.write_u32(0b101, 3);
        w.align_to_byte();
        assert_eq!(w.finish(), vec![0b10100000]);
    }

    #[test]
    fn varied_widths_roundtrip() {
        let mut bw = BitWriter::new();
        let writes: Vec<(u32, u32)> = vec![
            (0b1, 1),
            (0b10101, 5),
            (0b111100001111, 12),
            (0xDEADBEEF, 32),
            (0b001, 3),
            (0xC, 4),
            (0xABCD, 16),
            (0x12345, 20),
            (0, 8),
            (0xFFFFFFFF, 32),
        ];
        for &(v, n) in &writes {
            bw.write_u32(v, n);
        }
        let bytes = bw.finish();
        let mut br = BitReader::new(&bytes);
        for &(v, n) in &writes {
            let got = br.read_u32(n).unwrap();
            let mask = if n == 32 { u32::MAX } else { (1 << n) - 1 };
            assert_eq!(got, v & mask, "mismatch for ({v:#x}, {n})");
        }
    }

    #[test]
    fn many_bits() {
        let mut w = BitWriter::new();
        for i in 0..100 {
            w.write_u32(i & 1, 1);
        }
        let out = w.finish();
        // 100 bits → 13 bytes.
        assert_eq!(out.len(), 13);
        // First byte: bits 0..8 = 0,1,0,1,0,1,0,1 → 0x55.
        assert_eq!(out[0], 0x55);
    }
}
