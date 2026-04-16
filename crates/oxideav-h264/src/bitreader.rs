//! MSB-first bit reader for H.264 RBSP data, with Exp-Golomb support.
//!
//! The reader operates on the **RBSP** (raw byte sequence payload) — i.e. the
//! NAL unit payload after `0x03` emulation-prevention bytes have been
//! stripped. See [`crate::nal`] for that pre-processing.
//!
//! References: ITU-T H.264 (07/2019) §7.2 (CAVLC parsing process for
//! syntax-element categories), §9.1 (parsing of `ue(v)` / `se(v)` /
//! `te(v)` / `me(v)`).

use oxideav_core::{Error, Result};

/// MSB-first bit reader over a byte slice.
pub struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
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

    /// Total bit position from the start of the input slice.
    pub fn bit_position(&self) -> u64 {
        self.byte_pos as u64 * 8 - self.bits_in_acc as u64
    }

    /// Bits left to read (data + accumulator).
    pub fn bits_remaining(&self) -> u64 {
        (self.data.len() - self.byte_pos) as u64 * 8 + self.bits_in_acc as u64
    }

    pub fn is_byte_aligned(&self) -> bool {
        self.bits_in_acc % 8 == 0
    }

    fn refill(&mut self) {
        while self.bits_in_acc <= 56 && self.byte_pos < self.data.len() {
            self.acc |= (self.data[self.byte_pos] as u64) << (56 - self.bits_in_acc);
            self.bits_in_acc += 8;
            self.byte_pos += 1;
        }
    }

    /// Read up to 32 unsigned bits, MSB-first.
    pub fn read_u32(&mut self, n: u32) -> Result<u32> {
        debug_assert!(n <= 32);
        if n == 0 {
            return Ok(0);
        }
        if self.bits_in_acc < n {
            self.refill();
            if self.bits_in_acc < n {
                return Err(Error::invalid("h264 bitreader: out of bits"));
            }
        }
        let v = (self.acc >> (64 - n)) as u32;
        self.acc <<= n;
        self.bits_in_acc -= n;
        Ok(v)
    }

    pub fn read_u1(&mut self) -> Result<u32> {
        self.read_u32(1)
    }

    /// Read a flag (`u(1)` syntax element).
    pub fn read_flag(&mut self) -> Result<bool> {
        Ok(self.read_u1()? != 0)
    }

    /// Peek the next `n` bits without consuming them.
    pub fn peek_u32(&mut self, n: u32) -> Result<u32> {
        debug_assert!(n <= 32);
        if self.bits_in_acc < n {
            self.refill();
            if self.bits_in_acc < n {
                return Err(Error::invalid("h264 bitreader: peek past EOF"));
            }
        }
        Ok((self.acc >> (64 - n)) as u32)
    }

    /// Peek up to `n` bits, padding with zeros if fewer bits are available.
    /// Returns the raw bits left-aligned in the returned `u32` width `n`.
    pub fn peek_u32_lax(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if self.bits_in_acc < n {
            self.refill();
        }
        if self.bits_in_acc == 0 {
            return 0;
        }
        // Mask to the top `n` bits (those after the cursor); the accumulator
        // holds bits at the most significant end.
        if n <= self.bits_in_acc {
            (self.acc >> (64 - n)) as u32
        } else {
            // Shift so the available bits land at the top of an `n`-bit field.
            let avail = self.bits_in_acc;
            let v = (self.acc >> (64 - avail)) as u32;
            v << (n - avail)
        }
    }

    /// Unsigned Exp-Golomb code (`ue(v)`), §9.1.
    ///
    /// Reads leading zero bits (`leadingZeroBits`), then a 1, then
    /// `leadingZeroBits` more bits. The resulting value is
    /// `(1 << leadingZeroBits) - 1 + read_bits`.
    pub fn read_ue(&mut self) -> Result<u32> {
        let mut leading_zeros = 0u32;
        loop {
            if leading_zeros > 32 {
                return Err(Error::invalid("h264 ue(v): >32 leading zeros"));
            }
            let bit = self.read_u1()?;
            if bit == 1 {
                break;
            }
            leading_zeros += 1;
        }
        if leading_zeros == 0 {
            return Ok(0);
        }
        let suffix = self.read_u32(leading_zeros)?;
        // (1 << leading_zeros) - 1 + suffix; saturate at u32::MAX
        let base = (1u64 << leading_zeros) - 1;
        Ok((base + suffix as u64) as u32)
    }

    /// Signed Exp-Golomb (`se(v)`), §9.1.1.
    pub fn read_se(&mut self) -> Result<i32> {
        let code = self.read_ue()?;
        // mapping: 0 -> 0; odd k -> (k+1)/2; even k -> -(k/2)
        if code & 1 == 1 {
            Ok((code.div_ceil(2)) as i32)
        } else {
            Ok(-((code / 2) as i32))
        }
    }

    /// Truncated Exp-Golomb (`te(v)`), §9.1.2.
    /// `x` is the upper bound of the syntax element.
    pub fn read_te(&mut self, x: u32) -> Result<u32> {
        if x == 1 {
            // 1-bit, with reverse polarity: 0 -> 1, 1 -> 0
            Ok(1 - self.read_u1()?)
        } else {
            self.read_ue()
        }
    }

    /// Skip `n` bits.
    pub fn skip(&mut self, n: u32) -> Result<()> {
        let mut remaining = n;
        while remaining > 32 {
            self.read_u32(32)?;
            remaining -= 32;
        }
        self.read_u32(remaining)?;
        Ok(())
    }

    /// True if more RBSP data remains. §7.2 / §9.1.
    ///
    /// `more_rbsp_data()` returns false when the only bits remaining are the
    /// `rbsp_trailing_bits` syntax: a single `1` followed by zero or more
    /// `0`s up to the next byte boundary. Otherwise true.
    ///
    /// Implementation: locate the last `1` bit in the entire input (the
    /// rbsp stop bit) and return true iff there is any other `1` bit at a
    /// position `>= cursor` and `< last_one`.
    pub fn more_rbsp_data(&mut self) -> Result<bool> {
        let cursor = self.bit_position();
        let last_one = match find_last_one_bit(self.data) {
            Some(p) => p,
            None => return Ok(false),
        };
        if cursor >= last_one {
            return Ok(false);
        }
        // Scan bits in [cursor, last_one) for any set bit.
        let start_byte = (cursor / 8) as usize;
        let end_byte = (last_one / 8) as usize;
        for i in start_byte..=end_byte {
            if i >= self.data.len() {
                break;
            }
            let b = self.data[i];
            for j in 0..8u64 {
                let pos = (i as u64) * 8 + j;
                if pos < cursor {
                    continue;
                }
                if pos >= last_one {
                    break;
                }
                if b & (1u8 << (7 - j as u32)) != 0 {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Skip the trailing rbsp_trailing_bits: a `1` then zero alignment.
    /// §7.3.2.11.
    pub fn read_rbsp_trailing_bits(&mut self) -> Result<()> {
        let one = self.read_u1()?;
        if one != 1 {
            return Err(Error::invalid("h264 rbsp_trailing_bits: missing stop bit"));
        }
        while !self.is_byte_aligned() {
            let z = self.read_u1()?;
            if z != 0 {
                return Err(Error::invalid(
                    "h264 rbsp_trailing_bits: non-zero alignment bit",
                ));
            }
        }
        Ok(())
    }
}

/// Find the bit position (MSB-first, 0-indexed from the start of the slice)
/// of the very last set bit, or `None` if `data` is all zeros.
fn find_last_one_bit(data: &[u8]) -> Option<u64> {
    for i in (0..data.len()).rev() {
        let b = data[i];
        if b != 0 {
            // Position of the LSB-most set bit within this byte.
            let trail = b.trailing_zeros();
            let bit_in_byte = 7 - trail;
            return Some((i as u64) * 8 + bit_in_byte as u64);
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)]
mod tests {
    use super::*;

    #[test]
    fn ue_basic() {
        // Bit pattern from H.264 §9.1.1 example:
        // 1          -> 0
        // 010        -> 1
        // 011        -> 2
        // 00100      -> 3
        // 00101      -> 4
        // 00110      -> 5
        // 00111      -> 6
        // 0001000    -> 7
        let data = [
            0b1_010_011_0u8,
            0b0100_0010,
            0b1_00110_00,
            0b111_00010,
            0b00_000000,
        ];
        let mut br = BitReader::new(&data);
        assert_eq!(br.read_ue().unwrap(), 0);
        assert_eq!(br.read_ue().unwrap(), 1);
        assert_eq!(br.read_ue().unwrap(), 2);
        assert_eq!(br.read_ue().unwrap(), 3);
        assert_eq!(br.read_ue().unwrap(), 4);
        assert_eq!(br.read_ue().unwrap(), 5);
        assert_eq!(br.read_ue().unwrap(), 6);
        assert_eq!(br.read_ue().unwrap(), 7);
    }

    #[test]
    fn se_basic() {
        // se mapping: 0->0, 1->1, 2->-1, 3->2, 4->-2, 5->3, 6->-3
        // ue codes for 0..=6: 1, 010, 011, 00100, 00101, 00110, 00111
        let data = [0b1_010_011_0u8, 0b0100_0010, 0b1_00110_00, 0b111_00000];
        let mut br = BitReader::new(&data);
        assert_eq!(br.read_se().unwrap(), 0);
        assert_eq!(br.read_se().unwrap(), 1);
        assert_eq!(br.read_se().unwrap(), -1);
        assert_eq!(br.read_se().unwrap(), 2);
        assert_eq!(br.read_se().unwrap(), -2);
        assert_eq!(br.read_se().unwrap(), 3);
        assert_eq!(br.read_se().unwrap(), -3);
    }

    #[test]
    fn read_u32_msb_first() {
        let data = [0xAB, 0xCD];
        let mut br = BitReader::new(&data);
        assert_eq!(br.read_u32(4).unwrap(), 0xA);
        assert_eq!(br.read_u32(8).unwrap(), 0xBC);
        assert_eq!(br.read_u32(4).unwrap(), 0xD);
    }
}
