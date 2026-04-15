//! FLAC frame header parsing.
//!
//! Format reference: <https://xiph.org/flac/format.html#frame_header>

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::crc;

/// Per-channel layout of the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelAssignment {
    /// Channels stored independently (1..=8 channels).
    Independent(u8),
    /// Stereo, channel 0 = left, channel 1 = side (right = left - side).
    LeftSide,
    /// Stereo, channel 0 = side, channel 1 = right (left = right + side).
    RightSide,
    /// Stereo, channel 0 = mid, channel 1 = side (decorrelated).
    MidSide,
}

impl ChannelAssignment {
    pub fn channel_count(&self) -> u8 {
        match self {
            Self::Independent(n) => *n,
            _ => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockingStrategy {
    Fixed,
    Variable,
}

#[derive(Clone, Debug)]
pub struct FrameHeader {
    pub blocking_strategy: BlockingStrategy,
    pub block_size: u32,
    /// Sample rate in Hz, or 0 if unknown (must be taken from STREAMINFO).
    pub sample_rate: u32,
    pub channels: ChannelAssignment,
    pub bits_per_sample: u8,
    /// Frame number (fixed-blocking) or sample number (variable-blocking).
    pub coded_number: u64,
    /// Number of bytes consumed by the header (including the trailing CRC-8).
    pub header_byte_len: usize,
}

impl FrameHeader {
    /// First sample number (within the stream) covered by this frame.
    /// Requires a streaminfo block size for fixed-blocking files.
    pub fn first_sample(&self, streaminfo_block_size: u32) -> u64 {
        match self.blocking_strategy {
            BlockingStrategy::Fixed => self.coded_number * streaminfo_block_size as u64,
            BlockingStrategy::Variable => self.coded_number,
        }
    }
}

/// Parse a frame header from `bytes`. Verifies the trailing CRC-8.
///
/// Returns the parsed header on success. The caller is responsible for
/// supplying enough bytes — this function returns `Error::NeedMore` when
/// `bytes` is short.
pub fn parse_frame_header(bytes: &[u8]) -> Result<FrameHeader> {
    if bytes.len() < 6 {
        return Err(Error::NeedMore);
    }
    if bytes[0] != 0xFF || (bytes[1] & 0xFE) != 0xF8 {
        return Err(Error::invalid("FLAC frame: missing sync"));
    }

    let mut br = BitReader::new(bytes);
    // Bits 0..15: sync. Bit 14 is the 14th sync bit.
    // We've already validated bytes[0..2] above, so just consume them.
    let _sync = br.read_u32(14)?;
    let _reserved_a = br.read_u32(1)?; // must be 0
    let blocking_bit = br.read_u32(1)?;
    let blocking_strategy = if blocking_bit == 0 {
        BlockingStrategy::Fixed
    } else {
        BlockingStrategy::Variable
    };

    let block_size_code = br.read_u32(4)? as u8;
    let sample_rate_code = br.read_u32(4)? as u8;
    let channel_code = br.read_u32(4)? as u8;
    let sample_size_code = br.read_u32(3)? as u8;
    let _reserved_b = br.read_u32(1)?; // must be 0

    let coded_number = br.read_utf8_u64()?;

    let block_size = match block_size_code {
        0 => return Err(Error::invalid("FLAC frame: reserved block size code 0")),
        1 => 192,
        2..=5 => 576 << (block_size_code - 2),
        6 => br.read_u32(8)? + 1,
        7 => br.read_u32(16)? + 1,
        8..=15 => 256u32 << (block_size_code - 8),
        _ => unreachable!(),
    };

    let sample_rate = match sample_rate_code {
        0 => 0, // get from streaminfo
        1 => 88_200,
        2 => 176_400,
        3 => 192_000,
        4 => 8_000,
        5 => 16_000,
        6 => 22_050,
        7 => 24_000,
        8 => 32_000,
        9 => 44_100,
        10 => 48_000,
        11 => 96_000,
        12 => br.read_u32(8)? * 1000,
        13 => br.read_u32(16)?,
        14 => br.read_u32(16)? * 10,
        15 => return Err(Error::invalid("FLAC frame: invalid sample rate code 15")),
        _ => unreachable!(),
    };

    let channels = match channel_code {
        0..=7 => ChannelAssignment::Independent(channel_code + 1),
        8 => ChannelAssignment::LeftSide,
        9 => ChannelAssignment::RightSide,
        10 => ChannelAssignment::MidSide,
        _ => {
            return Err(Error::invalid(format!(
                "FLAC frame: reserved channel assignment {channel_code}"
            )));
        }
    };

    let bits_per_sample = match sample_size_code {
        0 => 0, // get from streaminfo
        1 => 8,
        2 => 12,
        3 => return Err(Error::invalid("FLAC frame: reserved sample size code 3")),
        4 => 16,
        5 => 20,
        6 => 24,
        7 => return Err(Error::invalid("FLAC frame: reserved sample size code 7")),
        _ => unreachable!(),
    };

    // Header is now byte-aligned. The next byte is the CRC-8.
    if !br.is_byte_aligned() {
        return Err(Error::invalid("FLAC frame: header not byte aligned at CRC"));
    }
    let header_bytes_so_far = br.byte_position();
    if bytes.len() < header_bytes_so_far + 1 {
        return Err(Error::NeedMore);
    }
    let claimed_crc = bytes[header_bytes_so_far];
    let computed = crc::crc8(&bytes[..header_bytes_so_far]);
    if computed != claimed_crc {
        return Err(Error::invalid(format!(
            "FLAC frame header CRC-8 mismatch (got {:#04x}, want {:#04x})",
            computed, claimed_crc
        )));
    }

    Ok(FrameHeader {
        blocking_strategy,
        block_size,
        sample_rate,
        channels,
        bits_per_sample,
        coded_number,
        header_byte_len: header_bytes_so_far + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_minimal_header() {
        // Build a minimal valid frame header by hand:
        // sync 0xFFF8 (fixed blocking, reserved bits 0).
        // block_size_code = 0b0001 (192 samples)
        // sample_rate_code = 0b1010 (48000 Hz)
        // channel_code = 0b0000 (mono)
        // sample_size_code = 0b100 (16 bits)
        // reserved 0, frame number = 0 (1 byte UTF-8 = 0x00)
        // then CRC-8 over those 5 bytes.
        let mut hdr = vec![0xFF, 0xF8];
        hdr.push((0b0001 << 4) | 0b1010); // block_size + sample_rate
        hdr.push(0b100 << 1); // channel + sample_size + reserved
        hdr.push(0x00); // frame number = 0
        let c = crc::crc8(&hdr);
        hdr.push(c);
        let parsed = parse_frame_header(&hdr).unwrap();
        assert_eq!(parsed.block_size, 192);
        assert_eq!(parsed.sample_rate, 48_000);
        assert_eq!(parsed.bits_per_sample, 16);
        assert_eq!(parsed.channels, ChannelAssignment::Independent(1));
        assert_eq!(parsed.coded_number, 0);
        assert_eq!(parsed.blocking_strategy, BlockingStrategy::Fixed);
        assert_eq!(parsed.header_byte_len, hdr.len());
    }

    #[test]
    fn rejects_bad_crc() {
        let mut hdr = vec![0xFF, 0xF8, 0x14, 0x08, 0x00, 0xAA];
        // last byte is bogus CRC
        assert!(parse_frame_header(&hdr).is_err());
        // Fix it.
        let last = crc::crc8(&hdr[..5]);
        *hdr.last_mut().unwrap() = last;
        assert!(parse_frame_header(&hdr).is_ok());
    }
}
