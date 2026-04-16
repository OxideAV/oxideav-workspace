//! LSB-first bit writer — the inverse of [`crate::bitreader::BitReader`].
//!
//! Vorbis packs bits LSB-first within each byte (Vorbis I §2.1.4). When the
//! writer accumulates N bits they are packed into the low N bits of the
//! current byte; overflow spills into the next byte's low bits.

pub struct BitWriter {
    data: Vec<u8>,
    /// Bits held over from the last partial byte, low-aligned.
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

    /// Total bits written so far.
    pub fn bit_position(&self) -> u64 {
        self.data.len() as u64 * 8 + self.bits_in_acc as u64
    }

    /// Append `n` bits from the low `n` bits of `value`.
    pub fn write_u32(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32, "BitWriter::write_u32 supports up to 32 bits");
        if n == 0 {
            return;
        }
        let mask: u32 = if n == 32 { u32::MAX } else { (1u32 << n) - 1 };
        let v = value & mask;
        self.acc |= (v as u64) << self.bits_in_acc;
        self.bits_in_acc += n;
        while self.bits_in_acc >= 8 {
            self.data.push((self.acc & 0xFF) as u8);
            self.acc >>= 8;
            self.bits_in_acc -= 8;
        }
    }

    pub fn write_u64(&mut self, value: u64, n: u32) {
        debug_assert!(n <= 64);
        if n <= 32 {
            self.write_u32(value as u32, n);
        } else {
            self.write_u32(value as u32, 32);
            self.write_u32((value >> 32) as u32, n - 32);
        }
    }

    pub fn write_bit(&mut self, bit: bool) {
        self.write_u32(bit as u32, 1);
    }

    /// Pad with zero bits to the next byte boundary, then return the
    /// accumulated bytes. Subsequent writes start a fresh stream.
    pub fn finish(mut self) -> Vec<u8> {
        if self.bits_in_acc > 0 {
            self.data.push((self.acc & 0xFF) as u8);
            self.acc = 0;
            self.bits_in_acc = 0;
        }
        self.data
    }

    /// Pad to byte boundary but keep writing afterwards. Useful when a
    /// Vorbis header demands byte alignment before the next section.
    pub fn align_to_byte(&mut self) {
        let pad = (8 - self.bits_in_acc % 8) % 8;
        self.write_u32(0, pad);
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
    fn roundtrip_lsb_first_byte() {
        let mut w = BitWriter::new();
        // Write the pattern 10100101 as 8 one-bit writes, LSB-first. That's
        // bits 1, 0, 1, 0, 0, 1, 0, 1 → byte 0xA5 under LSB-first packing.
        for &b in &[1u32, 0, 1, 0, 0, 1, 0, 1] {
            w.write_u32(b, 1);
        }
        assert_eq!(w.finish(), vec![0xA5]);
    }

    #[test]
    fn roundtrip_multi_byte() {
        let mut w = BitWriter::new();
        w.write_u32(0x3412, 16);
        assert_eq!(w.finish(), vec![0x12, 0x34]);
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
    fn tail_padded_to_byte() {
        let mut w = BitWriter::new();
        w.write_u32(1, 3);
        let bytes = w.finish();
        // Bits: 001 (3 bits) → byte 0x01 after zero-padding the high 5 bits.
        assert_eq!(bytes, vec![0x01]);
    }
}
