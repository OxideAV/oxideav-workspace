//! MSB-first bit writer, the inverse of [`crate::bitreader::BitReader`].

pub struct BitWriter {
    buf: Vec<u8>,
    /// Bits queued for output, left-aligned in `acc` (high bits first).
    acc: u64,
    /// Number of valid bits currently in `acc` (0..64).
    bits_in_acc: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            acc: 0,
            bits_in_acc: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
            acc: 0,
            bits_in_acc: 0,
        }
    }

    /// Append `n` bits (0..=32) of `value`. High bits of `value` beyond `n`
    /// are ignored.
    pub fn write_u32(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32, "BitWriter::write_u32 supports up to 32 bits");
        if n == 0 {
            return;
        }
        let mask: u64 = if n == 32 {
            0xFFFF_FFFF
        } else {
            (1u64 << n) - 1
        };
        let v = (value as u64) & mask;
        // Place value at top of free bits in acc.
        // bits_in_acc + n may not exceed 64.
        debug_assert!(self.bits_in_acc + n <= 64);
        self.acc |= v << (64 - n - self.bits_in_acc);
        self.bits_in_acc += n;
        while self.bits_in_acc >= 8 {
            let byte = (self.acc >> 56) as u8;
            self.buf.push(byte);
            self.acc <<= 8;
            self.bits_in_acc -= 8;
        }
    }

    /// Append `n` bits representing a signed integer (two's complement).
    pub fn write_i32(&mut self, value: i32, n: u32) {
        let mask: u32 = if n == 32 {
            0xFFFF_FFFF
        } else {
            (1u32 << n) - 1
        };
        self.write_u32((value as u32) & mask, n);
    }

    pub fn write_bit(&mut self, b: bool) {
        self.write_u32(if b { 1 } else { 0 }, 1);
    }

    /// Write a unary-coded value: `n` zero bits followed by a single `1`.
    pub fn write_unary(&mut self, n: u32) {
        let mut remaining = n;
        while remaining > 0 {
            let chunk = remaining.min(32);
            self.write_u32(0, chunk);
            remaining -= chunk;
        }
        self.write_u32(1, 1);
    }

    pub fn is_byte_aligned(&self) -> bool {
        self.bits_in_acc % 8 == 0
    }

    /// Pad the current byte with zero bits so the writer is byte-aligned.
    pub fn align_to_byte(&mut self) {
        let pad = (8 - self.bits_in_acc % 8) % 8;
        if pad > 0 {
            self.write_u32(0, pad);
        }
    }

    /// Read-only view of the currently emitted bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Number of bytes already emitted to the buffer (does not include
    /// partial bits still in the accumulator).
    pub fn byte_len(&self) -> usize {
        self.buf.len()
    }

    /// Consume the writer, padding any partial byte with zeros, and return
    /// the final byte buffer.
    pub fn into_bytes(mut self) -> Vec<u8> {
        self.align_to_byte();
        self.buf
    }

    /// Append a FLAC-style UTF-8 variable-length integer (same encoding as
    /// UTF-8 bytes, supporting up to 36-bit values with a 0xFE lead byte).
    pub fn write_utf8_u64(&mut self, value: u64) {
        debug_assert!(self.is_byte_aligned());
        // Determine how many payload bits we need.
        let bits_needed = if value == 0 {
            1
        } else {
            64 - value.leading_zeros()
        };
        let (lead_bits, n_extra, lead_prefix, lead_payload_bits): (u32, u32, u8, u32) =
            match bits_needed {
                0..=7 => (8, 0, 0x00, 7),
                8..=11 => (8, 1, 0xC0, 5),
                12..=16 => (8, 2, 0xE0, 4),
                17..=21 => (8, 3, 0xF0, 3),
                22..=26 => (8, 4, 0xF8, 2),
                27..=31 => (8, 5, 0xFC, 1),
                32..=36 => (8, 6, 0xFE, 0),
                _ => panic!("UTF-8 varint value exceeds 36 bits"),
            };
        let _ = lead_bits;
        let total_payload_bits = lead_payload_bits + n_extra * 6;
        // Compose the full byte sequence.
        if n_extra == 0 {
            // Plain ASCII-style byte.
            self.write_u32((value as u32) & 0x7F, 8);
        } else {
            let lead_payload =
                ((value >> (n_extra * 6)) & ((1u64 << lead_payload_bits) - 1)) as u32;
            self.write_u32(lead_prefix as u32 | lead_payload, 8);
            for i in (0..n_extra).rev() {
                let chunk = ((value >> (i * 6)) & 0x3F) as u32;
                self.write_u32(0x80 | chunk, 8);
            }
        }
        let _ = total_payload_bits;
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
    fn round_trip_u32() {
        let mut w = BitWriter::new();
        w.write_u32(0xA, 4);
        w.write_u32(0x5, 4);
        w.write_u32(0xC3, 8);
        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_u32(4).unwrap(), 0xA);
        assert_eq!(r.read_u32(4).unwrap(), 0x5);
        assert_eq!(r.read_u32(8).unwrap(), 0xC3);
    }

    #[test]
    fn round_trip_signed() {
        let mut w = BitWriter::new();
        w.write_i32(-1, 4);
        w.write_i32(-1, 4);
        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_i32(4).unwrap(), -1);
        assert_eq!(r.read_i32(4).unwrap(), -1);
    }

    #[test]
    fn unary_round_trip() {
        let mut w = BitWriter::new();
        for v in &[0u32, 1, 2, 5, 10, 40] {
            w.write_unary(*v);
        }
        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes);
        for v in &[0u32, 1, 2, 5, 10, 40] {
            assert_eq!(r.read_unary().unwrap(), *v);
        }
    }

    #[test]
    fn utf8_round_trip() {
        for v in &[0u64, 1, 127, 128, 1024, 0x12345, 0x1234_5678, 0xFFFF_FFFF] {
            let mut w = BitWriter::new();
            w.write_utf8_u64(*v);
            let bytes = w.into_bytes();
            let mut r = BitReader::new(&bytes);
            assert_eq!(r.read_utf8_u64().unwrap(), *v, "v={v:x}");
        }
    }

    #[test]
    fn align_pads_with_zeros() {
        let mut w = BitWriter::new();
        w.write_u32(0b101, 3);
        w.align_to_byte();
        assert_eq!(w.bytes(), &[0b10100000]);
    }
}
