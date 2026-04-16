//! VP8 frame tag + uncompressed data chunk — RFC 6386 §9.1.
//!
//! The first 3 bytes of every frame contain:
//!   bit 0     — frame type: 0 = I (key) frame, 1 = P (inter) frame.
//!   bits 1-3  — version (3 bits).
//!   bit 4     — show_frame.
//!   bits 5-23 — first partition size (19 bits).
//!
//! For key-frames an additional 7 bytes follow:
//!   3-byte sync code   `9d 01 2a`
//!   2-byte width       (lower 14 bits little-endian: 13 bits width + 2 bits scale)
//!   2-byte height      (lower 14 bits little-endian: 13 bits height + 2 bits scale)

use oxideav_core::{Error, Result};

pub const KEYFRAME_SYNC_CODE: [u8; 3] = [0x9d, 0x01, 0x2a];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameType {
    Key,
    Inter,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameTag {
    pub frame_type: FrameType,
    pub version: u8,
    pub show_frame: bool,
    pub first_partition_size: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct KeyframeHeader {
    pub width: u16,
    pub width_scale: u8,
    pub height: u16,
    pub height_scale: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct ParsedHeader {
    pub tag: FrameTag,
    /// Present only on key-frames.
    pub keyframe: Option<KeyframeHeader>,
    /// Offset of the first compressed (boolean-coded) byte from the start
    /// of the input buffer. 3 for inter, 10 for keyframe.
    pub compressed_offset: usize,
}

/// Parse the 3-byte frame tag plus, for keyframes, the 7-byte uncompressed
/// chunk. Returns the parsed values and the byte offset where the
/// boolean-coded compressed data begins.
pub fn parse_header(buf: &[u8]) -> Result<ParsedHeader> {
    if buf.len() < 3 {
        return Err(Error::invalid("VP8: frame tag truncated"));
    }
    let b0 = buf[0] as u32;
    let b1 = buf[1] as u32;
    let b2 = buf[2] as u32;
    let tag_word = b0 | (b1 << 8) | (b2 << 16);

    let frame_type = if tag_word & 1 == 0 {
        FrameType::Key
    } else {
        FrameType::Inter
    };
    let version = ((tag_word >> 1) & 0b111) as u8;
    let show_frame = (tag_word >> 4) & 1 == 1;
    let first_partition_size = (tag_word >> 5) & ((1 << 19) - 1);

    let tag = FrameTag {
        frame_type,
        version,
        show_frame,
        first_partition_size,
    };

    if matches!(frame_type, FrameType::Inter) {
        return Ok(ParsedHeader {
            tag,
            keyframe: None,
            compressed_offset: 3,
        });
    }

    if buf.len() < 10 {
        return Err(Error::invalid("VP8 keyframe header truncated"));
    }
    if buf[3..6] != KEYFRAME_SYNC_CODE {
        return Err(Error::invalid("VP8 keyframe sync code mismatch"));
    }
    let w_word = u16::from_le_bytes([buf[6], buf[7]]);
    let h_word = u16::from_le_bytes([buf[8], buf[9]]);
    let width = w_word & 0x3fff;
    let width_scale = (w_word >> 14) as u8;
    let height = h_word & 0x3fff;
    let height_scale = (h_word >> 14) as u8;

    Ok(ParsedHeader {
        tag,
        keyframe: Some(KeyframeHeader {
            width,
            width_scale,
            height,
            height_scale,
        }),
        compressed_offset: 10,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-encoded key-frame: type=0, version=0, show=1, partition=0x12345.
    /// Tag word = 0 | 0<<1 | 1<<4 | 0x12345<<5 = 0x2468a0 + 0x10
    ///         = 0x2468b0  (low byte 0xb0, then 0x68, 0x24)
    #[test]
    fn parse_keyframe_tag() {
        let part_size = 0x12345u32;
        let tag = 0u32 | (0 << 1) | (1 << 4) | (part_size << 5);
        let buf = vec![
            (tag & 0xff) as u8,
            ((tag >> 8) & 0xff) as u8,
            ((tag >> 16) & 0xff) as u8,
            0x9d,
            0x01,
            0x2a,
            // width 320 (=0x140), scale 0
            0x40,
            0x01,
            // height 240 (=0xf0), scale 0
            0xf0,
            0x00,
        ];
        let p = parse_header(&buf).unwrap();
        assert!(matches!(p.tag.frame_type, FrameType::Key));
        assert_eq!(p.tag.version, 0);
        assert!(p.tag.show_frame);
        assert_eq!(p.tag.first_partition_size, part_size);
        let kf = p.keyframe.unwrap();
        assert_eq!(kf.width, 320);
        assert_eq!(kf.height, 240);
        assert_eq!(kf.width_scale, 0);
        assert_eq!(kf.height_scale, 0);
        assert_eq!(p.compressed_offset, 10);
    }

    #[test]
    fn parse_inter_tag() {
        let part_size = 7u32;
        let tag = 1u32 | (0 << 1) | (1 << 4) | (part_size << 5);
        let buf = vec![
            (tag & 0xff) as u8,
            ((tag >> 8) & 0xff) as u8,
            ((tag >> 16) & 0xff) as u8,
        ];
        let p = parse_header(&buf).unwrap();
        assert!(matches!(p.tag.frame_type, FrameType::Inter));
        assert_eq!(p.tag.first_partition_size, 7);
        assert!(p.keyframe.is_none());
        assert_eq!(p.compressed_offset, 3);
    }

    #[test]
    fn rejects_bad_sync() {
        let buf = vec![0x10, 0x00, 0x00, 0xaa, 0xbb, 0xcc, 0, 0, 0, 0];
        assert!(parse_header(&buf).is_err());
    }
}
