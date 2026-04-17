//! Still-image JPEG container (`.jpg` / `.jpeg`).
//!
//! A JPEG file is simply a single SOI..EOI byte stream — there is no
//! container wrapping around the codec payload. Registering this as a
//! container lets the oxideav pipeline treat a standalone `.jpg` as a
//! one-frame "video" whose stream uses the existing `mjpeg` codec id
//! (Motion JPEG = concatenated stills, so a still is the N=1 case).
//!
//! * Demuxer: reads the file, walks its marker segments to discover the
//!   canvas dimensions (SOF0/SOF1/SOF2/SOF3) and the SOI..EOI bounds
//!   (robust against trailing garbage like some cameras append), and
//!   emits the whole frame as one packet tagged `codec_id = "mjpeg"`.
//! * Muxer: writes the encoded packet's bytes verbatim. JPEG has no
//!   container wrapping, so this is a pass-through.
//! * Probe: three-byte magic `FF D8 FF` at offset 0 → score 100.
//!
//! Caveats / notes:
//!
//! - EXIF / XMP / ICC data lives in APP1 / APP2 segments; those are
//!   preserved as part of the packet payload because we copy the full
//!   SOI..EOI span. No parsing is done on them here.
//! - We accept SOF0/1/2/3 for dimension discovery so that probing /
//!   metadata works even if the underlying decoder can't yet decode the
//!   bitstream (progressive JPEGs still round-trip at the container
//!   level).
//! - Trailing bytes past EOI are dropped. Leading bytes before SOI are
//!   not allowed (that would be a different file format).

use std::io::{Read, SeekFrom, Write};

use oxideav_container::{ContainerRegistry, Demuxer, Muxer, ProbeData, ReadSeek, WriteSeek};
use oxideav_core::{
    CodecId, CodecParameters, Error, MediaType, Packet, PixelFormat, Result, StreamInfo, TimeBase,
};

use crate::jpeg::markers::{self, EOI, SOI};
use crate::jpeg::parser::{parse_sof, SofInfo};

/// Register the still-image JPEG container under the name `jpeg`.
pub fn register(reg: &mut ContainerRegistry) {
    reg.register_demuxer("jpeg", open_demuxer);
    reg.register_muxer("jpeg", open_muxer);
    reg.register_extension("jpg", "jpeg");
    reg.register_extension("jpeg", "jpeg");
    reg.register_extension("jpe", "jpeg");
    reg.register_extension("jfif", "jpeg");
    reg.register_probe("jpeg", probe);
}

/// Probe: `FF D8 FF` at offset 0 is an unambiguous JPEG signature.
pub fn probe(p: &ProbeData) -> u8 {
    if p.buf.len() >= 3 && p.buf[0] == 0xFF && p.buf[1] == 0xD8 && p.buf[2] == 0xFF {
        100
    } else {
        0
    }
}

// --- Demuxer ---------------------------------------------------------------

fn open_demuxer(mut input: Box<dyn ReadSeek>) -> Result<Box<dyn Demuxer>> {
    input.seek(SeekFrom::Start(0))?;
    let mut buf = Vec::new();
    input.read_to_end(&mut buf)?;
    drop(input);

    // Validate SOI at offset 0 and locate the EOI that terminates this JPEG,
    // trimming any trailing garbage some cameras append (MPF thumbnails, etc.).
    let (start, end) = find_soi_eoi(&buf)?;
    if start != 0 {
        // We insist the file *starts* with SOI — a JPEG with leading junk
        // is probably a different container (or a TIFF-with-embedded-JPEG).
        return Err(Error::invalid("JPEG: SOI not at offset 0"));
    }
    let data = buf[start..end].to_vec();

    // Sniff frame size (SOF) so the stream can report width/height up-front.
    let sof = scan_for_sof(&data)?;

    let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    params.media_type = MediaType::Video;
    params.width = Some(sof.width as u32);
    params.height = Some(sof.height as u32);
    params.pixel_format = Some(pixel_format_for_sof(&sof));

    let time_base = TimeBase::new(1, 1);
    let stream = StreamInfo {
        index: 0,
        time_base,
        duration: Some(1),
        start_time: Some(0),
        params,
    };

    let mut pkt = Packet::new(0, time_base, data);
    pkt.pts = Some(0);
    pkt.dts = Some(0);
    pkt.duration = Some(1);
    pkt.flags.keyframe = true;

    Ok(Box::new(JpegDemuxer {
        stream,
        pending: Some(pkt),
    }))
}

/// Return `(start, end_exclusive)` of the first complete SOI..EOI JPEG in
/// `buf`. The returned span includes the SOI and EOI markers themselves.
fn find_soi_eoi(buf: &[u8]) -> Result<(usize, usize)> {
    if buf.len() < 4 {
        return Err(Error::invalid("JPEG: file too short"));
    }
    if buf[0] != 0xFF || buf[1] != SOI {
        return Err(Error::invalid("JPEG: missing SOI"));
    }
    // Scan forward for an EOI marker. We use the same rules as the decoder's
    // marker walker: `0xFF 0x00` is a stuffed literal (inside a scan), and a
    // run of 0xFF fill bytes before a marker byte is legal. RST* markers are
    // also part of the scan.
    let mut i = 2;
    while i + 1 < buf.len() {
        if buf[i] != 0xFF {
            i += 1;
            continue;
        }
        // Collapse any run of 0xFF fill bytes.
        let mut j = i + 1;
        while j < buf.len() && buf[j] == 0xFF {
            j += 1;
        }
        if j >= buf.len() {
            break;
        }
        let m = buf[j];
        if m == 0x00 {
            // Stuffed zero — skip and keep looking.
            i = j + 1;
            continue;
        }
        if markers::is_rst(m) {
            i = j + 1;
            continue;
        }
        if m == EOI {
            return Ok((0, j + 1));
        }
        // For any other marker just advance past the marker byte. Marker
        // segments outside the scan carry a length prefix but we don't need
        // to parse them — walking byte-by-byte still lands on the EOI
        // eventually. The SOS scan data is handled implicitly: its stuffed
        // 0xFF00 bytes and RST markers are filtered above.
        i = j + 1;
    }
    Err(Error::invalid("JPEG: no EOI marker found"))
}

/// Walk the marker segments until we find any SOF marker and parse it.
/// Returns the canvas size and component layout so the demuxer can report
/// width/height on its stream.
fn scan_for_sof(data: &[u8]) -> Result<SofInfo> {
    // Skip past SOI.
    let body = &data[2..];
    let mut walker = crate::jpeg::parser::MarkerWalker::new(body);
    loop {
        let Some(marker) = walker.next_marker()? else {
            return Err(Error::invalid("JPEG: SOF not found before EOF"));
        };
        if markers::is_sof(marker) {
            let payload = walker.read_segment_payload()?;
            return parse_sof(payload);
        }
        // SOI / EOI / RST have no length field. Everything else does.
        if marker == SOI || marker == EOI || markers::is_rst(marker) {
            continue;
        }
        // Length-prefixed segment — consume and ignore.
        let _ = walker.read_segment_payload()?;
    }
}

fn pixel_format_for_sof(sof: &SofInfo) -> PixelFormat {
    match sof.components.len() {
        1 => PixelFormat::Gray8,
        3 => {
            // Default to 4:2:0 if we can read the sampling factors; fall back
            // to 4:4:4 otherwise. The decoder double-checks and rejects
            // truly weird layouts.
            let y = sof.components[0];
            match (y.h_factor, y.v_factor) {
                (2, 2) => PixelFormat::Yuv420P,
                (2, 1) => PixelFormat::Yuv422P,
                (1, 1) => PixelFormat::Yuv444P,
                _ => PixelFormat::Yuv420P,
            }
        }
        _ => PixelFormat::Yuv420P,
    }
}

struct JpegDemuxer {
    stream: StreamInfo,
    pending: Option<Packet>,
}

impl Demuxer for JpegDemuxer {
    fn format_name(&self) -> &str {
        "jpeg"
    }

    fn streams(&self) -> &[StreamInfo] {
        std::slice::from_ref(&self.stream)
    }

    fn next_packet(&mut self) -> Result<Packet> {
        self.pending.take().ok_or(Error::Eof)
    }
}

// --- Muxer -----------------------------------------------------------------

fn open_muxer(output: Box<dyn WriteSeek>, streams: &[StreamInfo]) -> Result<Box<dyn Muxer>> {
    if streams.len() != 1 {
        return Err(Error::invalid(
            "JPEG container holds exactly one video stream",
        ));
    }
    let s = &streams[0];
    if s.params.media_type != MediaType::Video {
        return Err(Error::invalid("JPEG container: stream must be video"));
    }
    if s.params.codec_id.as_str() != crate::CODEC_ID_STR {
        return Err(Error::invalid(format!(
            "JPEG container requires codec_id={} (got {})",
            crate::CODEC_ID_STR,
            s.params.codec_id
        )));
    }
    Ok(Box::new(JpegMuxer {
        output,
        header_written: false,
        wrote_packet: false,
        trailer_written: false,
    }))
}

struct JpegMuxer {
    output: Box<dyn WriteSeek>,
    header_written: bool,
    wrote_packet: bool,
    trailer_written: bool,
}

impl Muxer for JpegMuxer {
    fn format_name(&self) -> &str {
        "jpeg"
    }

    fn write_header(&mut self) -> Result<()> {
        if self.header_written {
            return Err(Error::other("JPEG muxer: write_header called twice"));
        }
        // No container wrapping — nothing to emit here.
        self.header_written = true;
        Ok(())
    }

    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        if !self.header_written {
            return Err(Error::other("JPEG muxer: write_header not called"));
        }
        if self.wrote_packet {
            return Err(Error::invalid(
                "JPEG container is single-frame: exactly one packet per file",
            ));
        }
        self.output.write_all(&packet.data)?;
        self.wrote_packet = true;
        Ok(())
    }

    fn write_trailer(&mut self) -> Result<()> {
        if self.trailer_written {
            return Ok(());
        }
        self.output.flush()?;
        self.trailer_written = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_accepts_soi_ff() {
        let p = ProbeData {
            buf: &[0xFF, 0xD8, 0xFF, 0xE0],
            ext: None,
        };
        assert_eq!(probe(&p), 100);
    }

    #[test]
    fn probe_rejects_short_buffer() {
        let p = ProbeData {
            buf: &[0xFF, 0xD8],
            ext: None,
        };
        assert_eq!(probe(&p), 0);
    }

    #[test]
    fn probe_rejects_non_jpeg() {
        let p = ProbeData {
            buf: &[0x00, 0x00, 0x00, 0x00],
            ext: None,
        };
        assert_eq!(probe(&p), 0);
    }

    #[test]
    fn find_soi_eoi_trims_trailing_garbage() {
        // Minimal SOI..EOI with 4 bytes of trailing garbage.
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9, 0x00, 0x11, 0x22, 0x33];
        let (s, e) = find_soi_eoi(&jpeg).expect("span");
        assert_eq!((s, e), (0, 4));
    }

    #[test]
    fn find_soi_eoi_rejects_missing_soi() {
        let not_jpeg = [0x00, 0x01, 0x02, 0x03];
        assert!(find_soi_eoi(&not_jpeg).is_err());
    }
}
