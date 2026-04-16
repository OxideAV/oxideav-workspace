//! IVF container demuxer.
//!
//! IVF is a tiny, length-prefixed container used by libvpx and ffmpeg
//! for raw VP8/VP9/AV1 streams.
//!
//! 32-byte file header:
//!   0..4   "DKIF" magic
//!   4..6   version (u16-le, 0)
//!   6..8   header length (u16-le, 32)
//!   8..12  FourCC ("VP80" for VP8)
//!   12..14 width (u16-le)
//!   14..16 height (u16-le)
//!   16..20 frame rate numerator (u32-le)
//!   20..24 frame rate denominator (u32-le)
//!   24..28 frame count (u32-le, may be 0 = unknown)
//!   28..32 unused
//!
//! Each frame is preceded by:
//!   0..4 frame size in bytes (u32-le)
//!   4..12 pts in time-base units (u64-le)

use std::io::SeekFrom;

use oxideav_container::{ContainerRegistry, Demuxer, ProbeData, ReadSeek};
use oxideav_core::{
    CodecId, CodecParameters, Error, MediaType, Packet, PixelFormat, Rational, Result, StreamInfo,
    TimeBase,
};

const IVF_HEADER_LEN: usize = 32;
const FRAME_HEADER_LEN: usize = 12;

pub fn register(reg: &mut ContainerRegistry) {
    reg.register_demuxer("ivf", open);
    reg.register_extension("ivf", "ivf");
    reg.register_probe("ivf", probe);
}

fn probe(p: &ProbeData) -> u8 {
    if p.buf.len() < 12 {
        return 0;
    }
    if &p.buf[0..4] != b"DKIF" {
        return 0;
    }
    if &p.buf[8..12] == b"VP80" {
        return 100;
    }
    // Other FourCCs (VP90, AV01) are still IVF — but oxideav-vp8 only
    // claims VP8.
    0
}

fn open(mut input: Box<dyn ReadSeek>) -> Result<Box<dyn Demuxer>> {
    let mut hdr = [0u8; IVF_HEADER_LEN];
    read_exact(&mut input, &mut hdr)?;
    if &hdr[0..4] != b"DKIF" {
        return Err(Error::invalid("IVF: bad magic"));
    }
    let version = u16::from_le_bytes([hdr[4], hdr[5]]);
    if version != 0 {
        return Err(Error::invalid(format!(
            "IVF: unsupported version {version}"
        )));
    }
    let header_len = u16::from_le_bytes([hdr[6], hdr[7]]) as u64;
    if header_len < IVF_HEADER_LEN as u64 {
        return Err(Error::invalid("IVF: header length too small"));
    }
    let fourcc = &hdr[8..12];
    if fourcc != b"VP80" {
        return Err(Error::invalid(format!(
            "IVF: unsupported FourCC {:?}",
            std::str::from_utf8(fourcc).unwrap_or("???")
        )));
    }
    let width = u16::from_le_bytes([hdr[12], hdr[13]]) as u32;
    let height = u16::from_le_bytes([hdr[14], hdr[15]]) as u32;
    let fr_num = u32::from_le_bytes([hdr[16], hdr[17], hdr[18], hdr[19]]);
    let fr_den = u32::from_le_bytes([hdr[20], hdr[21], hdr[22], hdr[23]]);
    let _frame_count = u32::from_le_bytes([hdr[24], hdr[25], hdr[26], hdr[27]]);

    // Skip extra header bytes if any.
    if header_len > IVF_HEADER_LEN as u64 {
        input.seek(SeekFrom::Current(
            (header_len - IVF_HEADER_LEN as u64) as i64,
        ))?;
    }

    // IVF time_base = den / num seconds per tick.
    let (tb_num, tb_den) = if fr_num == 0 || fr_den == 0 {
        (1, 1000)
    } else {
        (fr_den as i64, fr_num as i64)
    };
    let time_base = TimeBase::new(tb_num, tb_den);

    let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    params.media_type = MediaType::Video;
    params.width = Some(width);
    params.height = Some(height);
    params.pixel_format = Some(PixelFormat::Yuv420P);
    if fr_num != 0 && fr_den != 0 {
        params.frame_rate = Some(Rational::new(fr_num as i64, fr_den as i64));
    }

    let stream = StreamInfo {
        index: 0,
        time_base,
        duration: None,
        start_time: Some(0),
        params,
    };

    Ok(Box::new(IvfDemuxer {
        input,
        stream,
        time_base,
    }))
}

struct IvfDemuxer {
    input: Box<dyn ReadSeek>,
    stream: StreamInfo,
    time_base: TimeBase,
}

impl Demuxer for IvfDemuxer {
    fn format_name(&self) -> &str {
        "ivf"
    }

    fn streams(&self) -> &[StreamInfo] {
        std::slice::from_ref(&self.stream)
    }

    fn next_packet(&mut self) -> Result<Packet> {
        let mut hdr = [0u8; FRAME_HEADER_LEN];
        match read_full(&mut self.input, &mut hdr) {
            Ok(true) => {}
            Ok(false) => return Err(Error::Eof),
            Err(e) => return Err(e),
        }
        let size = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let pts = u64::from_le_bytes([
            hdr[4], hdr[5], hdr[6], hdr[7], hdr[8], hdr[9], hdr[10], hdr[11],
        ]) as i64;
        let mut data = vec![0u8; size];
        read_exact(&mut self.input, &mut data)?;
        let mut pkt = Packet::new(0, self.time_base, data);
        pkt.pts = Some(pts);
        pkt.dts = Some(pts);
        // VP8 frame type lives in bit 0 of the first byte: 0 = key.
        pkt.flags.keyframe = !pkt.data.is_empty() && (pkt.data[0] & 1) == 0;
        Ok(pkt)
    }
}

fn read_exact(input: &mut Box<dyn ReadSeek>, buf: &mut [u8]) -> Result<()> {
    let mut got = 0;
    while got < buf.len() {
        match input.read(&mut buf[got..]) {
            Ok(0) => {
                return Err(Error::invalid(format!(
                    "IVF: unexpected EOF after {got} bytes"
                )))
            }
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Read up to `buf.len()` bytes. Returns `Ok(true)` if the buffer was
/// completely filled, `Ok(false)` if EOF was hit before any byte was
/// read, and `Err` if EOF was hit mid-buffer.
fn read_full(input: &mut Box<dyn ReadSeek>, buf: &mut [u8]) -> Result<bool> {
    let mut got = 0;
    while got < buf.len() {
        match input.read(&mut buf[got..]) {
            Ok(0) => {
                if got == 0 {
                    return Ok(false);
                } else {
                    return Err(Error::invalid("IVF: truncated frame header"));
                }
            }
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_recognises_dkif_vp80() {
        let mut buf = vec![0u8; 32];
        buf[0..4].copy_from_slice(b"DKIF");
        buf[8..12].copy_from_slice(b"VP80");
        let p = ProbeData {
            buf: &buf,
            ext: None,
        };
        assert_eq!(probe(&p), 100);
    }

    #[test]
    fn probe_rejects_other_fourcc() {
        let mut buf = vec![0u8; 32];
        buf[0..4].copy_from_slice(b"DKIF");
        buf[8..12].copy_from_slice(b"VP90");
        let p = ProbeData {
            buf: &buf,
            ext: None,
        };
        assert_eq!(probe(&p), 0);
    }

    #[test]
    fn probe_rejects_non_dkif() {
        let p = ProbeData {
            buf: b"RIFF................VP80",
            ext: None,
        };
        assert_eq!(probe(&p), 0);
    }
}
