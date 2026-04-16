//! MSB-first bit writer for H.263 bitstreams.
//!
//! Mirrors the layout assumed by the matching `BitReader` from
//! `oxideav-mpeg4video`: bits are packed MSB-first within each byte. After all
//! data is written, `finish()` zero-pads the trailing partial byte.
//!
//! Lifted verbatim from `oxideav-mpeg1video::bitwriter` — duplicated rather
//! than depended on so the H.263 crate does not cross-depend on MPEG-1.

pub struct BitWriter {
    buf: Vec<u8>,
    /// Bit accumulator: bits live in the low `nbits` positions and are shifted
    /// out from the MSB into the next output byte.
    acc: u64,
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

    /// Write the low `len` bits of `value` MSB-first. `len` must be ≤ 32.
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

    /// Current bit position from start of stream.
    pub fn bit_position(&self) -> u64 {
        self.buf.len() as u64 * 8 + self.nbits as u64
    }

    pub fn is_byte_aligned(&self) -> bool {
        self.nbits == 0
    }

    /// Append raw bytes verbatim. Caller must be byte-aligned.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        debug_assert!(self.is_byte_aligned());
        self.buf.extend_from_slice(bytes);
    }

    /// Flush any partial byte (zero-padded) and return the buffer.
    pub fn finish(mut self) -> Vec<u8> {
        self.align_to_byte();
        self.buf
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
    use oxideav_mpeg4video::bitreader::BitReader;

    #[test]
    fn round_trip_via_bitreader() {
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

    #[test]
    fn pad_zero_to_byte() {
        let mut bw = BitWriter::new();
        bw.write_bits(0b101, 3);
        let out = bw.finish();
        assert_eq!(out, vec![0b1010_0000]);
    }
}
