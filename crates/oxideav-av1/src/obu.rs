//! AV1 Open Bitstream Unit (OBU) parser — §5.3.
//!
//! Every AV1 bitstream is a concatenation of OBUs. Each OBU is a 1-byte
//! header (with optional 1-byte extension), an optional LEB128 size, and a
//! payload whose interpretation depends on `obu_type`.
//!
//! The parser is a streaming iterator over a buffer: it produces
//! `(header, payload_slice)` pairs without copying. It accepts both the
//! "low-overhead" (no obu_size) framing used inside `av1C` configOBUs and
//! the standard sized framing.

use oxideav_core::{Error, Result};

/// AV1 OBU types — §6.2.1, Table 6-2.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObuType {
    Reserved0 = 0,
    SequenceHeader = 1,
    TemporalDelimiter = 2,
    FrameHeader = 3,
    TileGroup = 4,
    Metadata = 5,
    Frame = 6,
    RedundantFrameHeader = 7,
    TileList = 8,
    Reserved9 = 9,
    Reserved10 = 10,
    Reserved11 = 11,
    Reserved12 = 12,
    Reserved13 = 13,
    Reserved14 = 14,
    Padding = 15,
}

impl ObuType {
    pub fn from_u8(v: u8) -> Self {
        match v & 0x0f {
            1 => Self::SequenceHeader,
            2 => Self::TemporalDelimiter,
            3 => Self::FrameHeader,
            4 => Self::TileGroup,
            5 => Self::Metadata,
            6 => Self::Frame,
            7 => Self::RedundantFrameHeader,
            8 => Self::TileList,
            15 => Self::Padding,
            0 => Self::Reserved0,
            9 => Self::Reserved9,
            10 => Self::Reserved10,
            11 => Self::Reserved11,
            12 => Self::Reserved12,
            13 => Self::Reserved13,
            _ => Self::Reserved14,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SequenceHeader => "OBU_SEQUENCE_HEADER",
            Self::TemporalDelimiter => "OBU_TEMPORAL_DELIMITER",
            Self::FrameHeader => "OBU_FRAME_HEADER",
            Self::TileGroup => "OBU_TILE_GROUP",
            Self::Metadata => "OBU_METADATA",
            Self::Frame => "OBU_FRAME",
            Self::RedundantFrameHeader => "OBU_REDUNDANT_FRAME_HEADER",
            Self::TileList => "OBU_TILE_LIST",
            Self::Padding => "OBU_PADDING",
            _ => "OBU_RESERVED",
        }
    }
}

/// Decoded OBU header (§5.3.2 + §5.3.3 extension).
#[derive(Clone, Copy, Debug)]
pub struct ObuHeader {
    pub obu_type: ObuType,
    pub extension_flag: bool,
    pub has_size_field: bool,
    pub temporal_id: u8,
    pub spatial_id: u8,
}

/// One parsed OBU referencing a slice of the source buffer.
#[derive(Clone, Copy, Debug)]
pub struct Obu<'a> {
    pub header: ObuHeader,
    /// Total length of header + size + payload in the source buffer.
    pub total_len: usize,
    /// Byte offset in the source buffer where this OBU starts.
    pub offset: usize,
    /// Payload bytes, slice into the source buffer.
    pub payload: &'a [u8],
}

/// Parse the 1-byte OBU header and (if present) the 1-byte extension byte.
/// Does NOT consume the optional LEB128 size — see `read_obu`.
pub fn parse_obu_header(byte: u8, ext: Option<u8>) -> Result<ObuHeader> {
    if byte & 0x80 != 0 {
        return Err(Error::invalid("av1: OBU forbidden bit set"));
    }
    let obu_type = ObuType::from_u8((byte >> 3) & 0x0f);
    let extension_flag = (byte & 0x04) != 0;
    let has_size_field = (byte & 0x02) != 0;
    if (byte & 0x01) != 0 {
        return Err(Error::invalid("av1: OBU reserved bit set"));
    }
    let (temporal_id, spatial_id) = if extension_flag {
        let e = ext.ok_or_else(|| Error::invalid("av1: OBU extension byte missing"))?;
        let t = (e >> 5) & 0x07;
        let s = (e >> 3) & 0x03;
        if e & 0x07 != 0 {
            return Err(Error::invalid("av1: OBU extension reserved bits set"));
        }
        (t, s)
    } else {
        (0, 0)
    };
    Ok(ObuHeader {
        obu_type,
        extension_flag,
        has_size_field,
        temporal_id,
        spatial_id,
    })
}

/// Read one OBU starting at `data[offset..]`. Returns the parsed OBU plus
/// the cumulative cursor advance.
///
/// `default_size`, if `Some`, is used when the OBU header has
/// `obu_has_size_field == 0` — this is the case for OBUs embedded in the
/// `av1C` configOBUs where the framing relies on the surrounding container.
pub fn read_obu<'a>(data: &'a [u8], offset: usize, default_size: Option<usize>) -> Result<Obu<'a>> {
    if offset >= data.len() {
        return Err(Error::invalid("av1: read past end of buffer"));
    }
    let mut p = offset;
    let header_byte = data[p];
    p += 1;
    let extension_flag = (header_byte & 0x04) != 0;
    let ext_byte = if extension_flag {
        if p >= data.len() {
            return Err(Error::invalid("av1: missing OBU extension byte"));
        }
        let b = data[p];
        p += 1;
        Some(b)
    } else {
        None
    };
    let header = parse_obu_header(header_byte, ext_byte)?;

    let payload_size = if header.has_size_field {
        // LEB128 inline. We can decode from a tiny helper without going
        // through BitReader because we know we're byte-aligned here.
        let (value, consumed) = read_leb128(&data[p..])?;
        p += consumed;
        value as usize
    } else {
        default_size.ok_or_else(|| {
            Error::invalid("av1: OBU has_size_field=0 but no surrounding length given")
        })?
    };

    if p.checked_add(payload_size)
        .map_or(true, |end| end > data.len())
    {
        return Err(Error::invalid("av1: OBU payload exceeds buffer"));
    }
    let payload = &data[p..p + payload_size];
    let total_len = (p - offset) + payload_size;
    Ok(Obu {
        header,
        total_len,
        offset,
        payload,
    })
}

/// Iterate every OBU in `data`. Each OBU MUST carry its own size field
/// (low-overhead framing via `default_size` is not appropriate at the
/// stream level). For configOBUs use `parse_config_obus` instead.
pub fn iter_obus(data: &[u8]) -> ObuIter<'_> {
    ObuIter { data, offset: 0 }
}

pub struct ObuIter<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for ObuIter<'a> {
    type Item = Result<Obu<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.data.len() {
            return None;
        }
        match read_obu(self.data, self.offset, None) {
            Ok(o) => {
                self.offset += o.total_len;
                Some(Ok(o))
            }
            Err(e) => {
                // Make iteration stop on the first error.
                self.offset = self.data.len();
                Some(Err(e))
            }
        }
    }
}

/// Parse the `configOBUs` field of an `AV1CodecConfigurationRecord`. These
/// OBUs are stored back-to-back and may use either framed (has_size=1) or
/// unframed (has_size=0) layout — when unframed, the OBU size is implicitly
/// the remainder of the byte buffer (only one such OBU may appear, and it
/// must be last per the AV1-in-ISOBMFF spec).
pub fn parse_config_obus(data: &[u8]) -> Result<Vec<Obu<'_>>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < data.len() {
        let remaining = data.len() - offset;
        // Peek the header to decide whether to fall back to "rest of buffer"
        // sizing.
        let header_byte = data[offset];
        let extension_flag = (header_byte & 0x04) != 0;
        let has_size = (header_byte & 0x02) != 0;
        let ext_len = if extension_flag { 1usize } else { 0 };
        if has_size {
            let obu = read_obu(data, offset, None)?;
            offset += obu.total_len;
            out.push(obu);
        } else {
            // Unframed: payload runs to end of buffer.
            let header_len = 1 + ext_len;
            if header_len > remaining {
                return Err(Error::invalid("av1 configOBUs: truncated OBU header"));
            }
            let payload_size = remaining - header_len;
            let obu = read_obu(data, offset, Some(payload_size))?;
            offset += obu.total_len;
            out.push(obu);
        }
    }
    Ok(out)
}

/// Read a byte-aligned LEB128. Returns `(value, bytes_consumed)`. Up to 8
/// bytes are consumed.
pub fn read_leb128(buf: &[u8]) -> Result<(u64, usize)> {
    let mut value: u64 = 0;
    let mut consumed = 0usize;
    for i in 0..8u32 {
        if consumed >= buf.len() {
            return Err(Error::invalid("av1 leb128: truncated"));
        }
        let b = buf[consumed] as u64;
        value |= (b & 0x7f) << (i * 7);
        consumed += 1;
        if (b & 0x80) == 0 {
            return Ok((value, consumed));
        }
    }
    // 8 bytes consumed, last byte must have continuation=0.
    Err(Error::invalid("av1 leb128: more than 8 bytes"))
}

/// Encode a value as a LEB128 byte string. Used by tests and metadata writers.
pub fn write_leb128(value: u64, fixed_len: Option<usize>) -> Vec<u8> {
    let mut out = Vec::with_capacity(fixed_len.unwrap_or(2));
    let mut v = value;
    let mut i = 0usize;
    let target_len = fixed_len.unwrap_or(0);
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        i += 1;
        let more = if let Some(n) = fixed_len {
            i < n
        } else {
            v != 0
        };
        if more {
            byte |= 0x80;
        }
        out.push(byte);
        if !more {
            break;
        }
        if i >= 8 {
            break;
        }
    }
    while out.len() < target_len && out.len() < 8 {
        // Pad with continuation+0.
        if let Some(b) = out.last_mut() {
            *b |= 0x80;
        }
        out.push(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_header() {
        // Sequence header: type=1, no extension, has_size=1
        // bits: 0 0001 0 1 0 = 0x0A
        let h = parse_obu_header(0x0A, None).unwrap();
        assert_eq!(h.obu_type, ObuType::SequenceHeader);
        assert!(!h.extension_flag);
        assert!(h.has_size_field);
    }

    #[test]
    fn parse_with_extension() {
        // type=6 (frame), extension=1, has_size=1 → 0011 0 1 1 0 = 0x36
        let ext = (2u8 << 5) | (1u8 << 3); // temporal_id=2, spatial_id=1
        let h = parse_obu_header(0x36, Some(ext)).unwrap();
        assert_eq!(h.obu_type, ObuType::Frame);
        assert!(h.extension_flag);
        assert_eq!(h.temporal_id, 2);
        assert_eq!(h.spatial_id, 1);
    }

    #[test]
    fn read_obu_inline_size() {
        // Minimal sequence header OBU: header 0x0A, size 0x05, payload 5 bytes
        let data = [0x0A, 0x05, 1, 2, 3, 4, 5];
        let obu = read_obu(&data, 0, None).unwrap();
        assert_eq!(obu.header.obu_type, ObuType::SequenceHeader);
        assert_eq!(obu.payload, &[1, 2, 3, 4, 5]);
        assert_eq!(obu.total_len, 7);
    }

    #[test]
    fn iter_two_obus() {
        // OBU 1: temporal delim type=2, has_size=1, payload=0
        // OBU 2: sequence header type=1, has_size=1, payload=2 bytes
        let data = [0x12, 0x00, 0x0A, 0x02, 0xAB, 0xCD];
        let v: Vec<_> = iter_obus(&data).collect();
        assert_eq!(v.len(), 2);
        let o0 = v[0].as_ref().unwrap();
        assert_eq!(o0.header.obu_type, ObuType::TemporalDelimiter);
        assert!(o0.payload.is_empty());
        let o1 = v[1].as_ref().unwrap();
        assert_eq!(o1.header.obu_type, ObuType::SequenceHeader);
        assert_eq!(o1.payload, &[0xAB, 0xCD]);
    }

    #[test]
    fn leb128_roundtrip() {
        for v in [0u64, 1, 0x7F, 0x80, 300, 0xFFFF, 0x1FFFFF] {
            let enc = write_leb128(v, None);
            let (dec, _) = read_leb128(&enc).unwrap();
            assert_eq!(dec, v);
        }
    }

    #[test]
    fn config_obus_unframed_last() {
        // configOBUs from /tmp/av1.mp4: header 0x0A (seq hdr, has_size=1) +
        // size byte + 10-byte payload. With has_size=1 it's just framed.
        let data = vec![
            0x0A, 0x0A, 0x00, 0x00, 0x00, 0x02, 0xAF, 0xFF, 0x9B, 0x5F, 0x30, 0x08,
        ];
        let v = parse_config_obus(&data).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].header.obu_type, ObuType::SequenceHeader);
        assert_eq!(v[0].payload.len(), 10);
    }
}
