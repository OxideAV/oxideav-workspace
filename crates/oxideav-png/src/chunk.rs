//! PNG chunk framing (read + write).
//!
//! Every chunk has this layout (RFC 2083 §3.2):
//!
//! ```text
//!   4 bytes  length  (big-endian, *only* the data portion)
//!   4 bytes  type    (ASCII, case-sensitive)
//!   N bytes  data    (where N = length)
//!   4 bytes  CRC32   (over type + data, PNG flavour)
//! ```
//!
//! The 8-byte magic `\x89PNG\r\n\x1a\n` precedes the first chunk.

use oxideav_core::{Error, Result};

use crate::filter::crc32;

/// PNG file magic.
pub const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Maximum chunk length we'll accept from untrusted input. The spec allows
/// up to 2^31-1 bytes, but it's unreasonable in practice.
pub const MAX_CHUNK_LEN: u32 = 0x7FFF_FFFF;

/// A parsed chunk borrowed from a larger buffer.
#[derive(Debug, Clone, Copy)]
pub struct ChunkRef<'a> {
    pub chunk_type: [u8; 4],
    pub data: &'a [u8],
}

impl<'a> ChunkRef<'a> {
    pub fn type_str(&self) -> &str {
        std::str::from_utf8(&self.chunk_type).unwrap_or("????")
    }

    pub fn is_type(&self, t: &[u8; 4]) -> bool {
        &self.chunk_type == t
    }
}

/// Read one chunk starting at `buf[pos..]`, verify its CRC32, and return
/// the parsed `ChunkRef` + the updated position.
pub fn read_chunk<'a>(buf: &'a [u8], pos: usize) -> Result<(ChunkRef<'a>, usize)> {
    if pos + 8 > buf.len() {
        return Err(Error::invalid("PNG: truncated chunk header"));
    }
    let len = u32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]);
    if len > MAX_CHUNK_LEN {
        return Err(Error::invalid(format!(
            "PNG: chunk length {len} exceeds maximum"
        )));
    }
    let type_start = pos + 4;
    let data_start = type_start + 4;
    let data_end = data_start
        .checked_add(len as usize)
        .ok_or_else(|| Error::invalid("PNG: chunk length overflow"))?;
    let crc_end = data_end + 4;
    if crc_end > buf.len() {
        return Err(Error::invalid("PNG: chunk extends past end of buffer"));
    }
    let mut chunk_type = [0u8; 4];
    chunk_type.copy_from_slice(&buf[type_start..type_start + 4]);
    let data = &buf[data_start..data_end];

    let declared = u32::from_be_bytes([
        buf[data_end],
        buf[data_end + 1],
        buf[data_end + 2],
        buf[data_end + 3],
    ]);
    let computed = crc32(&buf[type_start..data_end]);
    if declared != computed {
        return Err(Error::invalid(format!(
            "PNG: bad CRC on chunk {:?} (expected {:08X}, got {:08X})",
            std::str::from_utf8(&chunk_type).unwrap_or("????"),
            declared,
            computed
        )));
    }

    Ok((ChunkRef { chunk_type, data }, crc_end))
}

/// Write one chunk to `out`. Appends: length, type, data, CRC32.
pub fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    let len = data.len() as u32;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    // CRC over type + data.
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(chunk_type);
    crc_input.extend_from_slice(data);
    let c = crc32(&crc_input);
    out.extend_from_slice(&c.to_be_bytes());
}

/// Iterator over chunks in a PNG file buffer (starting after the magic).
pub struct ChunkIter<'a> {
    buf: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> ChunkIter<'a> {
    pub fn new(buf: &'a [u8], start: usize) -> Self {
        Self {
            buf,
            pos: start,
            done: false,
        }
    }
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = Result<ChunkRef<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.pos >= self.buf.len() {
            return None;
        }
        match read_chunk(self.buf, self.pos) {
            Ok((c, next)) => {
                self.pos = next;
                if c.chunk_type == *b"IEND" {
                    self.done = true;
                }
                Some(Ok(c))
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrip() {
        let mut out = Vec::new();
        write_chunk(&mut out, b"IHDR", &[1, 2, 3, 4]);
        let (chunk, end) = read_chunk(&out, 0).unwrap();
        assert_eq!(&chunk.chunk_type, b"IHDR");
        assert_eq!(chunk.data, &[1, 2, 3, 4]);
        assert_eq!(end, out.len());
    }

    #[test]
    fn bad_crc_rejected() {
        let mut out = Vec::new();
        write_chunk(&mut out, b"IHDR", &[1, 2, 3, 4]);
        // Flip one CRC byte.
        let last = out.len() - 1;
        out[last] ^= 0x01;
        let err = read_chunk(&out, 0).unwrap_err();
        assert!(matches!(err, Error::InvalidData(_)));
    }
}
