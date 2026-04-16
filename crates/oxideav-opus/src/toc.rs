//! Opus packet framing — RFC 6716 §3.
//!
//! Every Opus packet begins with a **TOC** ("table of contents") byte that
//! describes the encoded mode, audio bandwidth, frame size, channel count and
//! framing code (how many frames are packed into this packet and how their
//! sizes are encoded).
//!
//! ```text
//!   0
//!   0 1 2 3 4 5 6 7
//!  +-+-+-+-+-+-+-+-+
//!  | config  |s| c |
//!  +-+-+-+-+-+-+-+-+
//! ```
//!
//! * `config` — 5-bit mode + bandwidth + frame-size descriptor (Table 2)
//! * `s` — stereo flag (0 = mono, 1 = stereo)
//! * `c` — framing code (0 = 1 frame, 1 = 2 equal-sized frames,
//!   2 = 2 differently-sized frames, 3 = signalled count).
//!
//! This module parses that byte, plus the remainder of the packet (the
//! per-frame length signalling when `c ∈ {1, 2, 3}`) so the downstream
//! decoder just sees a sequence of already-split frame byte slices.

use oxideav_core::{Error, Result};

/// Internal-decoder mode indicated by the TOC `config` field.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OpusMode {
    /// Speech-oriented, LPC-based (RFC 6716 §4.2).
    SilkOnly,
    /// SILK low band + CELT high band (RFC 6716 §4).
    Hybrid,
    /// Music-oriented, MDCT-based (RFC 6716 §4.3).
    CeltOnly,
}

/// Audio bandwidth category implied by the TOC `config`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OpusBandwidth {
    /// 4 kHz — Narrowband.
    Narrowband,
    /// 6 kHz — Mediumband (SILK-only).
    Mediumband,
    /// 8 kHz — Wideband.
    Wideband,
    /// 12 kHz — Super-wideband.
    SuperWideband,
    /// 20 kHz — Fullband.
    Fullband,
}

impl OpusBandwidth {
    /// Nominal cut-off frequency in Hz.
    pub fn cutoff_hz(self) -> u32 {
        match self {
            Self::Narrowband => 4_000,
            Self::Mediumband => 6_000,
            Self::Wideband => 8_000,
            Self::SuperWideband => 12_000,
            Self::Fullband => 20_000,
        }
    }
}

/// Decoded TOC byte.
#[derive(Copy, Clone, Debug)]
pub struct Toc {
    pub config: u8,
    pub mode: OpusMode,
    pub bandwidth: OpusBandwidth,
    /// Frame duration in 1/48000 units (audio always exits the decoder at
    /// 48 kHz). A 20-ms frame is 960 samples; a 2.5-ms frame is 120 samples.
    pub frame_samples_48k: u32,
    pub stereo: bool,
    /// Framing-code field (0..=3, RFC §3.2.1).
    pub code: u8,
}

impl Toc {
    /// Parse just the TOC byte. The `config` field is looked up against
    /// RFC 6716 Table 2 to populate `mode`, `bandwidth`, and frame duration.
    pub fn parse(b: u8) -> Self {
        let config = b >> 3;
        let stereo = (b >> 2) & 1 != 0;
        let code = b & 0x3;
        let (mode, bandwidth, frame_samples_48k) = decode_config(config);
        Self {
            config,
            mode,
            bandwidth,
            frame_samples_48k,
            stereo,
            code,
        }
    }

    /// Channel count encoded in the TOC (`1` mono or `2` stereo).
    pub fn channels(&self) -> u16 {
        if self.stereo {
            2
        } else {
            1
        }
    }
}

/// Map the 5-bit `config` field to (mode, bandwidth, samples-per-frame at 48 kHz).
/// Exact layout from RFC 6716 Table 2.
fn decode_config(config: u8) -> (OpusMode, OpusBandwidth, u32) {
    // Note: SILK frames are natively at 8/12/16 kHz, CELT at 48 kHz, hybrid
    // at 48 kHz with SILK @ 16 kHz internally. At the decoder output they
    // are all resampled to 48 kHz; the `frame_samples_48k` reflects that.
    match config {
        // SILK-only, NB, 10/20/40/60 ms — 480/960/1920/2880 samples @48k
        0 => (OpusMode::SilkOnly, OpusBandwidth::Narrowband, 480),
        1 => (OpusMode::SilkOnly, OpusBandwidth::Narrowband, 960),
        2 => (OpusMode::SilkOnly, OpusBandwidth::Narrowband, 1920),
        3 => (OpusMode::SilkOnly, OpusBandwidth::Narrowband, 2880),
        // SILK-only, MB
        4 => (OpusMode::SilkOnly, OpusBandwidth::Mediumband, 480),
        5 => (OpusMode::SilkOnly, OpusBandwidth::Mediumband, 960),
        6 => (OpusMode::SilkOnly, OpusBandwidth::Mediumband, 1920),
        7 => (OpusMode::SilkOnly, OpusBandwidth::Mediumband, 2880),
        // SILK-only, WB
        8 => (OpusMode::SilkOnly, OpusBandwidth::Wideband, 480),
        9 => (OpusMode::SilkOnly, OpusBandwidth::Wideband, 960),
        10 => (OpusMode::SilkOnly, OpusBandwidth::Wideband, 1920),
        11 => (OpusMode::SilkOnly, OpusBandwidth::Wideband, 2880),
        // Hybrid, SWB, 10/20 ms
        12 => (OpusMode::Hybrid, OpusBandwidth::SuperWideband, 480),
        13 => (OpusMode::Hybrid, OpusBandwidth::SuperWideband, 960),
        // Hybrid, FB, 10/20 ms
        14 => (OpusMode::Hybrid, OpusBandwidth::Fullband, 480),
        15 => (OpusMode::Hybrid, OpusBandwidth::Fullband, 960),
        // CELT-only, NB, 2.5/5/10/20 ms = 120/240/480/960 @48k
        16 => (OpusMode::CeltOnly, OpusBandwidth::Narrowband, 120),
        17 => (OpusMode::CeltOnly, OpusBandwidth::Narrowband, 240),
        18 => (OpusMode::CeltOnly, OpusBandwidth::Narrowband, 480),
        19 => (OpusMode::CeltOnly, OpusBandwidth::Narrowband, 960),
        // CELT-only, WB
        20 => (OpusMode::CeltOnly, OpusBandwidth::Wideband, 120),
        21 => (OpusMode::CeltOnly, OpusBandwidth::Wideband, 240),
        22 => (OpusMode::CeltOnly, OpusBandwidth::Wideband, 480),
        23 => (OpusMode::CeltOnly, OpusBandwidth::Wideband, 960),
        // CELT-only, SWB
        24 => (OpusMode::CeltOnly, OpusBandwidth::SuperWideband, 120),
        25 => (OpusMode::CeltOnly, OpusBandwidth::SuperWideband, 240),
        26 => (OpusMode::CeltOnly, OpusBandwidth::SuperWideband, 480),
        27 => (OpusMode::CeltOnly, OpusBandwidth::SuperWideband, 960),
        // CELT-only, FB
        28 => (OpusMode::CeltOnly, OpusBandwidth::Fullband, 120),
        29 => (OpusMode::CeltOnly, OpusBandwidth::Fullband, 240),
        30 => (OpusMode::CeltOnly, OpusBandwidth::Fullband, 480),
        31 => (OpusMode::CeltOnly, OpusBandwidth::Fullband, 960),
        // `config` is 5 bits, so this is unreachable.
        _ => (OpusMode::CeltOnly, OpusBandwidth::Fullband, 960),
    }
}

/// A single Opus packet after TOC parsing. Holds zero-copy slices into the
/// caller's buffer.
#[derive(Debug)]
pub struct OpusPacket<'a> {
    pub toc: Toc,
    /// Per-frame byte slices. Length is 1..=48.
    pub frames: Vec<&'a [u8]>,
    /// Optional padding bytes (discarded content, RFC §3.2.5).
    pub padding: &'a [u8],
}

/// Read a single RFC 6716 §3.2.1 length value (one or two bytes).
///
/// Returns `(length, bytes_consumed)`.
fn read_length(data: &[u8]) -> Result<(usize, usize)> {
    let b0 = *data
        .first()
        .ok_or_else(|| Error::invalid("Opus frame length truncated"))?;
    if b0 < 252 {
        Ok((b0 as usize, 1))
    } else {
        let b1 = *data
            .get(1)
            .ok_or_else(|| Error::invalid("Opus frame length truncated (2-byte)"))?;
        Ok(((b1 as usize * 4) + b0 as usize, 2))
    }
}

/// Parse a full Opus packet (TOC + framing) per RFC 6716 §3.
///
/// The returned `frames` slices reference `data` directly — no copies.
pub fn parse_packet(data: &[u8]) -> Result<OpusPacket<'_>> {
    if data.is_empty() {
        return Err(Error::invalid("Opus packet empty (needs ≥ 1 TOC byte)"));
    }
    let toc = Toc::parse(data[0]);
    let body = &data[1..];

    match toc.code {
        // Code 0: one single frame fills the remaining bytes.
        0 => Ok(OpusPacket {
            toc,
            frames: vec![body],
            padding: &[],
        }),
        // Code 1: two equal-sized frames, remaining bytes split in half.
        1 => {
            if body.len() % 2 != 0 {
                return Err(Error::invalid(
                    "Opus code-1 packet: body length must be even",
                ));
            }
            let half = body.len() / 2;
            Ok(OpusPacket {
                toc,
                frames: vec![&body[..half], &body[half..]],
                padding: &[],
            })
        }
        // Code 2: two frames of possibly-different sizes. Length of first
        // frame is coded in 1..=2 bytes; second frame takes the rest.
        2 => {
            let (n1, used) = read_length(body)?;
            let rest = &body[used..];
            if n1 > rest.len() {
                return Err(Error::invalid(
                    "Opus code-2 packet: frame-1 length exceeds payload",
                ));
            }
            Ok(OpusPacket {
                toc,
                frames: vec![&rest[..n1], &rest[n1..]],
                padding: &[],
            })
        }
        // Code 3: N frames, with a dedicated framing byte.
        3 => parse_code3(toc, body),
        _ => unreachable!(),
    }
}

/// Code-3 packet parsing per RFC 6716 §3.2.5.
fn parse_code3<'a>(toc: Toc, body: &'a [u8]) -> Result<OpusPacket<'a>> {
    if body.is_empty() {
        return Err(Error::invalid(
            "Opus code-3 packet: missing frame-count byte",
        ));
    }
    let fc = body[0];
    let vbr = fc & 0x80 != 0;
    let has_padding = fc & 0x40 != 0;
    let n_frames = (fc & 0x3F) as usize;
    if n_frames == 0 {
        return Err(Error::invalid("Opus code-3 packet: frame count is 0"));
    }
    // Soft spec limit: total frame duration <= 120 ms. With config-dependent
    // frame sizes that never exceeds 48 frames even at the shortest size.
    if n_frames > 48 {
        return Err(Error::invalid(
            "Opus code-3 packet: frame count exceeds 48 (>120 ms)",
        ));
    }
    let samples_per = toc.frame_samples_48k as usize;
    if samples_per * n_frames > 5760 {
        return Err(Error::invalid(
            "Opus code-3 packet: total duration > 120 ms",
        ));
    }

    let mut cursor = 1usize;

    // Padding length (if signalled). The padding-length field itself can
    // span multiple bytes: each byte 0xFF contributes 254 and indicates
    // "another byte follows", and the final byte contributes its own value.
    let mut pad_bytes = 0usize;
    if has_padding {
        loop {
            if cursor >= body.len() {
                return Err(Error::invalid(
                    "Opus code-3 packet: padding length truncated",
                ));
            }
            let p = body[cursor];
            cursor += 1;
            if p == 255 {
                pad_bytes += 254;
            } else {
                pad_bytes += p as usize;
                break;
            }
        }
    }

    // Per-frame length table when VBR is on; otherwise every frame has the
    // same size = (remaining_after_padding) / n_frames.
    let mut frame_sizes = Vec::with_capacity(n_frames);
    if vbr {
        for _ in 0..n_frames - 1 {
            let (len, used) = read_length(&body[cursor..])?;
            frame_sizes.push(len);
            cursor += used;
        }
    }

    // Calculate where frame data actually begins and how much is left for it.
    let data_end = body
        .len()
        .checked_sub(pad_bytes)
        .ok_or_else(|| Error::invalid("Opus code-3 packet: padding exceeds body"))?;
    if cursor > data_end {
        return Err(Error::invalid(
            "Opus code-3 packet: headers overflow past data region",
        ));
    }
    let data_region = &body[cursor..data_end];

    if !vbr {
        if data_region.len() % n_frames != 0 {
            return Err(Error::invalid(
                "Opus code-3 CBR packet: body not divisible by frame count",
            ));
        }
        let each = data_region.len() / n_frames;
        for _ in 0..n_frames {
            frame_sizes.push(each);
        }
    } else {
        // For VBR, the last frame consumes the remainder.
        let explicit_total: usize = frame_sizes.iter().sum();
        if explicit_total > data_region.len() {
            return Err(Error::invalid(
                "Opus code-3 VBR packet: coded lengths exceed body size",
            ));
        }
        frame_sizes.push(data_region.len() - explicit_total);
    }

    // Split the data region into per-frame slices.
    let mut frames = Vec::with_capacity(n_frames);
    let mut pos = 0usize;
    for &sz in &frame_sizes {
        if pos + sz > data_region.len() {
            return Err(Error::invalid(
                "Opus code-3 packet: frame slice runs past body",
            ));
        }
        frames.push(&data_region[pos..pos + sz]);
        pos += sz;
    }

    Ok(OpusPacket {
        toc,
        frames,
        padding: &body[data_end..],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_config_celt_20ms_fb_stereo() {
        // config=31 (CELT-only FB 20 ms), stereo=1, code=0.
        let b = (31 << 3) | (1 << 2);
        let t = Toc::parse(b);
        assert_eq!(t.config, 31);
        assert_eq!(t.mode, OpusMode::CeltOnly);
        assert_eq!(t.bandwidth, OpusBandwidth::Fullband);
        assert_eq!(t.frame_samples_48k, 960);
        assert!(t.stereo);
        assert_eq!(t.code, 0);
    }

    #[test]
    fn toc_silk_nb_mono() {
        let t = Toc::parse(0);
        assert_eq!(t.mode, OpusMode::SilkOnly);
        assert_eq!(t.bandwidth, OpusBandwidth::Narrowband);
        assert!(!t.stereo);
    }

    #[test]
    fn toc_hybrid_fb() {
        // config=15 — Hybrid FB 20 ms.
        let t = Toc::parse(15 << 3);
        assert_eq!(t.mode, OpusMode::Hybrid);
        assert_eq!(t.bandwidth, OpusBandwidth::Fullband);
        assert_eq!(t.frame_samples_48k, 960);
    }

    #[test]
    fn parse_code0_single_frame() {
        let mut p = vec![31u8 << 3]; // TOC: config=31, stereo=0, code=0
        p.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        let pkt = parse_packet(&p).unwrap();
        assert_eq!(pkt.frames.len(), 1);
        assert_eq!(pkt.frames[0], &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn parse_code1_two_equal_frames() {
        let mut p = vec![(31u8 << 3) | 1];
        p.extend_from_slice(&[1, 2, 3, 4]);
        let pkt = parse_packet(&p).unwrap();
        assert_eq!(pkt.frames.len(), 2);
        assert_eq!(pkt.frames[0], &[1, 2]);
        assert_eq!(pkt.frames[1], &[3, 4]);
    }

    #[test]
    fn parse_code1_rejects_odd_body() {
        let mut p = vec![(31u8 << 3) | 1];
        p.extend_from_slice(&[1, 2, 3]);
        assert!(parse_packet(&p).is_err());
    }

    #[test]
    fn parse_code2_two_frames_short_length() {
        // TOC=code-2, length byte = 3, frame-1 bytes, then frame-2 bytes.
        let mut p = vec![(31u8 << 3) | 2, 3];
        p.extend_from_slice(&[1, 2, 3]); // frame 1
        p.extend_from_slice(&[9, 9]); // frame 2
        let pkt = parse_packet(&p).unwrap();
        assert_eq!(pkt.frames.len(), 2);
        assert_eq!(pkt.frames[0], &[1, 2, 3]);
        assert_eq!(pkt.frames[1], &[9, 9]);
    }

    #[test]
    fn parse_code2_two_frames_long_length() {
        // Long length = 252 + 4*1 = 256 bytes.
        let mut p = vec![(31u8 << 3) | 2, 252, 1];
        p.extend(std::iter::repeat_n(0x5A, 256));
        p.extend_from_slice(&[0xAA, 0xBB]);
        let pkt = parse_packet(&p).unwrap();
        assert_eq!(pkt.frames.len(), 2);
        assert_eq!(pkt.frames[0].len(), 256);
        assert_eq!(pkt.frames[1], &[0xAA, 0xBB]);
    }

    #[test]
    fn parse_code3_cbr_three_frames() {
        // TOC=code-3 | CELT 2.5ms config 16, fc=3 frames, no VBR, no padding.
        let mut p = vec![(16u8 << 3) | 3, 3];
        p.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let pkt = parse_packet(&p).unwrap();
        assert_eq!(pkt.frames.len(), 3);
        assert_eq!(pkt.frames[0], &[1, 2]);
        assert_eq!(pkt.frames[1], &[3, 4]);
        assert_eq!(pkt.frames[2], &[5, 6]);
    }

    #[test]
    fn parse_code3_vbr_three_frames_with_padding() {
        // TOC=code-3 | CELT 2.5ms NB; fc=0xC3 (VBR, padding, count=3);
        // pad=2; f1 len=1; f2 len=2; then data f1/f2/f3 then padding.
        let mut p = vec![(16u8 << 3) | 3, 0xC3, 2, 1, 2, 0xA];
        p.extend_from_slice(&[0xB, 0xC]);
        p.extend_from_slice(&[0xD, 0xD, 0xD, 0xD]);
        // padding bytes
        p.extend_from_slice(&[0, 0]);
        let pkt = parse_packet(&p).unwrap();
        assert_eq!(pkt.frames.len(), 3);
        assert_eq!(pkt.frames[0], &[0xA]);
        assert_eq!(pkt.frames[1], &[0xB, 0xC]);
        assert_eq!(pkt.frames[2], &[0xD, 0xD, 0xD, 0xD]);
        assert_eq!(pkt.padding, &[0, 0]);
    }

    #[test]
    fn parse_code3_rejects_zero_frames() {
        let p = vec![(16u8 << 3) | 3, 0x00];
        assert!(parse_packet(&p).is_err());
    }

    #[test]
    fn parse_empty_packet_errors() {
        assert!(parse_packet(&[]).is_err());
    }
}
