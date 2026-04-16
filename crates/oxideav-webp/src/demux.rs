//! WebP container demuxer.
//!
//! WebP is a RIFF file whose top-level form-type is `WEBP`. Inside the
//! form we find one of three layouts:
//!
//! ```text
//! RIFF <size> WEBP VP8  <size> <VP8 keyframe bytes>       — simple lossy
//! RIFF <size> WEBP VP8L <size> <VP8L bytes>               — simple lossless
//! RIFF <size> WEBP VP8X <size> <flags+size hdr>
//!                 [ICCP|ANIM|ALPH|VP8 |VP8L|ANMF|EXIF|XMP ]* — extended
//! ```
//!
//! Chunks are even-padded the same way as other RIFF formats: if the payload
//! length is odd, one zero byte follows. All multi-byte integers are
//! little-endian.
//!
//! The demuxer emits each still frame (or each animation frame from an
//! `ANMF` chunk) as a single `Packet` on stream 0, with `codec_id = "webp"`
//! (a synthetic codec id local to this crate — the decoder handles all
//! three flavours transparently).

use std::io::{Read, SeekFrom};

use oxideav_container::{ContainerRegistry, Demuxer, ProbeData, ReadSeek};
use oxideav_core::{
    CodecId, CodecParameters, Error, MediaType, Packet, PixelFormat, Result, StreamInfo, TimeBase,
};

/// Codec id we attach to every packet emitted by this demuxer. The decoder
/// registered under the same id dispatches to the VP8, VP8L, or extended
/// path based on the chunk layout.
pub const WEBP_CODEC_ID: &str = "webp";

pub fn register(reg: &mut ContainerRegistry) {
    reg.register_demuxer("webp", open);
    reg.register_extension("webp", "webp");
    reg.register_probe("webp", probe);
}

fn probe(p: &ProbeData) -> u8 {
    if p.buf.len() < 12 {
        return 0;
    }
    if &p.buf[0..4] != b"RIFF" {
        return 0;
    }
    if &p.buf[8..12] != b"WEBP" {
        return 0;
    }
    // `VeryHigh` — the RIFF magic + WEBP form-type is unambiguous.
    100
}

/// Public wrapper over `open` so the decoder-side convenience API can
/// instantiate a demuxer without duplicating the boxing dance.
pub fn open_boxed(input: Box<dyn ReadSeek>) -> Result<Box<dyn Demuxer>> {
    open(input)
}

fn open(mut input: Box<dyn ReadSeek>) -> Result<Box<dyn Demuxer>> {
    // Read the whole file into memory. WebP stills are inherently small
    // (max 16384x16384 lossless / 16383x16383 VP8) and a full-buffer pass
    // simplifies chunk iteration + random access over the `ANMF` loop.
    let mut buf = Vec::new();
    input.seek(SeekFrom::Start(0))?;
    input.read_to_end(&mut buf)?;
    drop(input);

    if buf.len() < 12 || &buf[0..4] != b"RIFF" || &buf[8..12] != b"WEBP" {
        return Err(Error::invalid("WebP: bad RIFF/WEBP magic"));
    }
    let riff_size = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    // `riff_size` excludes the "RIFF" FourCC + the 4-byte size field, so
    // the total file is 8 + riff_size bytes. Clamp to the actual buffer to
    // survive files whose size field lies (we'd rather keep decoding).
    let end = (8 + riff_size).min(buf.len());
    let body = &buf[12..end];

    let parsed = parse_webp_body(body)?;
    // Width/height default to the dimensions declared by the first image
    // chunk (VP8/VP8L) or the VP8X canvas, whichever we saw first.
    let (w, h) = parsed.canvas;

    let mut params = CodecParameters::video(CodecId::new(WEBP_CODEC_ID));
    params.media_type = MediaType::Video;
    params.width = Some(w);
    params.height = Some(h);
    params.pixel_format = Some(PixelFormat::Rgba);

    // Time base: milliseconds. Animation chunk durations are already in ms.
    let time_base = TimeBase::new(1, 1000);
    let stream = StreamInfo {
        index: 0,
        time_base,
        duration: Some(parsed.total_duration_ms as i64),
        start_time: Some(0),
        params,
    };

    Ok(Box::new(WebpDemuxer {
        stream,
        packets: parsed.into_packets(time_base),
        pos: 0,
    }))
}

/// Result of parsing a `WEBP` form body.
#[derive(Debug)]
pub(crate) struct ParsedContainer {
    /// (width, height) — final rendered canvas size.
    pub canvas: (u32, u32),
    /// Frames in presentation order. Each frame has an already-extracted
    /// payload chunk (VP8 or VP8L) plus optional ALPH and offset/disposal
    /// info for animations. Static files produce exactly one entry.
    pub frames: Vec<ParsedFrame>,
    pub total_duration_ms: u32,
}

#[derive(Debug)]
pub(crate) struct ParsedFrame {
    /// Raw payload of the image chunk — VP8 keyframe or VP8L bitstream.
    pub image: ImagePayload,
    /// Optional ALPH chunk (extended-format alpha plane).
    pub alph: Option<AlphChunk>,
    pub x_offset: u32,
    pub y_offset: u32,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u32,
    /// True → dispose to background colour before rendering the next frame.
    pub dispose_to_background: bool,
    /// True → blend with the canvas (false = overwrite).
    pub blend_with_previous: bool,
}

#[derive(Debug)]
pub(crate) enum ImagePayload {
    Vp8(Vec<u8>),
    Vp8l(Vec<u8>),
}

#[derive(Debug)]
pub(crate) struct AlphChunk {
    pub pre_processing: u8,
    pub filtering: u8,
    pub compression: u8,
    pub data: Vec<u8>,
}

impl ParsedContainer {
    fn into_packets(self, tb: TimeBase) -> Vec<Packet> {
        let mut pkts = Vec::with_capacity(self.frames.len());
        let mut pts: i64 = 0;
        let canvas = self.canvas;
        for (i, f) in self.frames.into_iter().enumerate() {
            let duration = f.duration_ms;
            let data = encode_frame_payload(&f, canvas);
            let mut pkt = Packet::new(0, tb, data);
            pkt.pts = Some(pts);
            pkt.dts = Some(pts);
            pkt.duration = Some(duration.max(1) as i64);
            pkt.flags.keyframe = i == 0;
            pts += duration.max(1) as i64;
            pkts.push(pkt);
        }
        pkts
    }
}

/// Serialise a parsed frame into a self-contained payload the decoder can
/// consume without touching the original file. The layout is a tiny custom
/// TLV — a 32-byte header followed by the VP8/VP8L bitstream and an
/// optional ALPH payload.
///
/// This is local to the crate; it never escapes into `Packet::data`'s
/// public consumers because WebP packets only travel from `WebpDemuxer` to
/// `WebpDecoder` in the same process.
pub(crate) fn encode_frame_payload(f: &ParsedFrame, canvas: (u32, u32)) -> Vec<u8> {
    let img_bytes = match &f.image {
        ImagePayload::Vp8(v) | ImagePayload::Vp8l(v) => v,
    };
    let mut out = Vec::with_capacity(
        64 + img_bytes.len() + f.alph.as_ref().map(|a| a.data.len() + 16).unwrap_or(0),
    );
    // Magic "OWEB" + version byte.
    out.extend_from_slice(b"OWEB");
    out.push(1);
    // Flags: bit0 = has_alph, bit1 = is_vp8l.
    let mut flags = 0u8;
    if f.alph.is_some() {
        flags |= 0x01;
    }
    if matches!(f.image, ImagePayload::Vp8l(_)) {
        flags |= 0x02;
    }
    if f.dispose_to_background {
        flags |= 0x04;
    }
    if f.blend_with_previous {
        flags |= 0x08;
    }
    out.push(flags);
    // Canvas + frame bbox, 6 x u32.
    for v in [
        canvas.0, canvas.1, f.x_offset, f.y_offset, f.width, f.height,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    // Duration.
    out.extend_from_slice(&f.duration_ms.to_le_bytes());
    // Image chunk length + data.
    out.extend_from_slice(&(img_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(img_bytes);
    // ALPH chunk (optional): pre/filter/comp bytes + length + data.
    if let Some(a) = &f.alph {
        out.push(a.pre_processing);
        out.push(a.filtering);
        out.push(a.compression);
        out.extend_from_slice(&(a.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&a.data);
    }
    out
}

/// Counterpart to `encode_frame_payload`. Used by the decoder.
pub(crate) fn decode_frame_payload(buf: &[u8]) -> Result<DecodedPayload<'_>> {
    if buf.len() < 4 + 1 + 1 + 6 * 4 + 4 + 4 {
        return Err(Error::invalid("WebP: frame payload too short"));
    }
    if &buf[0..4] != b"OWEB" {
        return Err(Error::invalid("WebP: bad frame payload magic"));
    }
    if buf[4] != 1 {
        return Err(Error::invalid("WebP: unknown frame payload version"));
    }
    let flags = buf[5];
    let mut p = 6usize;
    let read_u32 = |p: &mut usize, buf: &[u8]| -> u32 {
        let v = u32::from_le_bytes([buf[*p], buf[*p + 1], buf[*p + 2], buf[*p + 3]]);
        *p += 4;
        v
    };
    let canvas_w = read_u32(&mut p, buf);
    let canvas_h = read_u32(&mut p, buf);
    let x_off = read_u32(&mut p, buf);
    let y_off = read_u32(&mut p, buf);
    let frame_w = read_u32(&mut p, buf);
    let frame_h = read_u32(&mut p, buf);
    let duration_ms = read_u32(&mut p, buf);
    let img_len = read_u32(&mut p, buf) as usize;
    if p + img_len > buf.len() {
        return Err(Error::invalid("WebP: image chunk extends past payload"));
    }
    let image = &buf[p..p + img_len];
    p += img_len;
    let alph = if flags & 0x01 != 0 {
        if p + 3 + 4 > buf.len() {
            return Err(Error::invalid("WebP: truncated ALPH header"));
        }
        let pre = buf[p];
        let filt = buf[p + 1];
        let comp = buf[p + 2];
        p += 3;
        let alen = read_u32(&mut p, buf) as usize;
        if p + alen > buf.len() {
            return Err(Error::invalid("WebP: ALPH data extends past payload"));
        }
        let a = &buf[p..p + alen];
        Some(DecodedAlph {
            pre_processing: pre,
            filtering: filt,
            compression: comp,
            data: a,
        })
    } else {
        None
    };
    Ok(DecodedPayload {
        is_vp8l: flags & 0x02 != 0,
        dispose_to_background: flags & 0x04 != 0,
        blend_with_previous: flags & 0x08 != 0,
        canvas: (canvas_w, canvas_h),
        x_offset: x_off,
        y_offset: y_off,
        width: frame_w,
        height: frame_h,
        duration_ms,
        image,
        alph,
    })
}

pub(crate) struct DecodedPayload<'a> {
    pub is_vp8l: bool,
    pub dispose_to_background: bool,
    pub blend_with_previous: bool,
    pub canvas: (u32, u32),
    pub x_offset: u32,
    pub y_offset: u32,
    pub width: u32,
    pub height: u32,
    #[allow(dead_code)]
    pub duration_ms: u32,
    pub image: &'a [u8],
    pub alph: Option<DecodedAlph<'a>>,
}

pub(crate) struct DecodedAlph<'a> {
    #[allow(dead_code)]
    pub pre_processing: u8,
    pub filtering: u8,
    pub compression: u8,
    pub data: &'a [u8],
}

fn parse_webp_body(body: &[u8]) -> Result<ParsedContainer> {
    let mut chunks = RiffChunks::new(body);
    // Peek the first chunk to distinguish simple vs extended layout.
    let first = chunks
        .next()
        .transpose()?
        .ok_or_else(|| Error::invalid("WebP: empty RIFF body"))?;

    match &first.id {
        b"VP8 " => {
            // Simple lossy still.
            let (w, h) = parse_vp8_keyframe_dims(first.data)?;
            let frame = ParsedFrame {
                image: ImagePayload::Vp8(first.data.to_vec()),
                alph: None,
                x_offset: 0,
                y_offset: 0,
                width: w,
                height: h,
                duration_ms: 0,
                dispose_to_background: false,
                blend_with_previous: false,
            };
            Ok(ParsedContainer {
                canvas: (w, h),
                frames: vec![frame],
                total_duration_ms: 0,
            })
        }
        b"VP8L" => {
            let (w, h) = parse_vp8l_dims(first.data)?;
            let frame = ParsedFrame {
                image: ImagePayload::Vp8l(first.data.to_vec()),
                alph: None,
                x_offset: 0,
                y_offset: 0,
                width: w,
                height: h,
                duration_ms: 0,
                dispose_to_background: false,
                blend_with_previous: false,
            };
            Ok(ParsedContainer {
                canvas: (w, h),
                frames: vec![frame],
                total_duration_ms: 0,
            })
        }
        b"VP8X" => parse_extended(first.data, &mut chunks),
        other => Err(Error::invalid(format!(
            "WebP: unexpected first chunk {:?}",
            std::str::from_utf8(other).unwrap_or("???")
        ))),
    }
}

fn parse_extended(vp8x: &[u8], chunks: &mut RiffChunks<'_>) -> Result<ParsedContainer> {
    if vp8x.len() < 10 {
        return Err(Error::invalid("WebP: VP8X chunk too short"));
    }
    // VP8X layout: 1 byte flags, 3 bytes reserved, 3 bytes canvas_w-1, 3 bytes canvas_h-1.
    let flags = vp8x[0];
    let has_anim = flags & 0x02 != 0;
    let canvas_w = (u32::from_le_bytes([vp8x[4], vp8x[5], vp8x[6], 0]) & 0x00FF_FFFF) + 1;
    let canvas_h = (u32::from_le_bytes([vp8x[7], vp8x[8], vp8x[9], 0]) & 0x00FF_FFFF) + 1;

    let mut frames: Vec<ParsedFrame> = Vec::new();
    // Static extended WebP state — we accumulate the VP8/VP8L chunk and
    // optional ALPH, and emit one frame when we've seen an image.
    let mut pending_alph: Option<AlphChunk> = None;
    let mut pending_image: Option<ImagePayload> = None;

    let mut total_duration = 0u32;

    while let Some(c) = chunks.next().transpose()? {
        match &c.id {
            b"VP8 " => {
                pending_image = Some(ImagePayload::Vp8(c.data.to_vec()));
            }
            b"VP8L" => {
                pending_image = Some(ImagePayload::Vp8l(c.data.to_vec()));
            }
            b"ALPH" => {
                if c.data.is_empty() {
                    return Err(Error::invalid("WebP: ALPH chunk empty"));
                }
                let hdr = c.data[0];
                let pre = (hdr >> 4) & 0x3;
                let filt = (hdr >> 2) & 0x3;
                let comp = hdr & 0x3;
                pending_alph = Some(AlphChunk {
                    pre_processing: pre,
                    filtering: filt,
                    compression: comp,
                    data: c.data[1..].to_vec(),
                });
            }
            b"ANMF" => {
                let anmf = parse_anmf(c.data)?;
                let f = anmf.into_frame();
                total_duration = total_duration.saturating_add(f.duration_ms);
                frames.push(f);
            }
            // Ignored auxiliary chunks:
            b"ANIM" | b"ICCP" | b"EXIF" | b"XMP " => {}
            _ => {
                // Unknown chunk — skip silently per the spec.
            }
        }
    }

    if !has_anim {
        let image = pending_image
            .ok_or_else(|| Error::invalid("WebP: extended file has no image chunk"))?;
        let (w, h) = match &image {
            ImagePayload::Vp8(v) => parse_vp8_keyframe_dims(v).unwrap_or((canvas_w, canvas_h)),
            ImagePayload::Vp8l(v) => parse_vp8l_dims(v).unwrap_or((canvas_w, canvas_h)),
        };
        let frame = ParsedFrame {
            image,
            alph: pending_alph.take(),
            x_offset: 0,
            y_offset: 0,
            width: w,
            height: h,
            duration_ms: 0,
            dispose_to_background: false,
            blend_with_previous: false,
        };
        frames.push(frame);
    }

    Ok(ParsedContainer {
        canvas: (canvas_w, canvas_h),
        frames,
        total_duration_ms: total_duration,
    })
}

struct AnmfBundle {
    x_offset: u32,
    y_offset: u32,
    width: u32,
    height: u32,
    duration_ms: u32,
    dispose_to_background: bool,
    blend_with_previous: bool,
    image: ImagePayload,
    alph: Option<AlphChunk>,
}

impl AnmfBundle {
    fn into_frame(self) -> ParsedFrame {
        ParsedFrame {
            image: self.image,
            alph: self.alph,
            x_offset: self.x_offset,
            y_offset: self.y_offset,
            width: self.width,
            height: self.height,
            duration_ms: self.duration_ms,
            dispose_to_background: self.dispose_to_background,
            blend_with_previous: self.blend_with_previous,
        }
    }
}

fn parse_anmf(data: &[u8]) -> Result<AnmfBundle> {
    // ANMF: 3 bytes X/2, 3 bytes Y/2, 3 bytes w-1, 3 bytes h-1, 3 bytes duration,
    //       1 byte flags (bit0 = blending=overwrite, bit1 = dispose-to-bg).
    //       Then nested sub-chunks (ALPH? + VP8/VP8L).
    if data.len() < 16 {
        return Err(Error::invalid("WebP: ANMF header too short"));
    }
    let x_off = u32::from_le_bytes([data[0], data[1], data[2], 0]) & 0x00FF_FFFF;
    let y_off = u32::from_le_bytes([data[3], data[4], data[5], 0]) & 0x00FF_FFFF;
    let w = (u32::from_le_bytes([data[6], data[7], data[8], 0]) & 0x00FF_FFFF) + 1;
    let h = (u32::from_le_bytes([data[9], data[10], data[11], 0]) & 0x00FF_FFFF) + 1;
    let dur = u32::from_le_bytes([data[12], data[13], data[14], 0]) & 0x00FF_FFFF;
    let flags = data[15];
    // Spec: bit0 = blending_method (1 = "no blend" = overwrite),
    //       bit1 = disposal_method (1 = dispose to BG).
    let blend_with_previous = flags & 0x02 == 0;
    let dispose_to_background = flags & 0x01 != 0;

    let mut chunks = RiffChunks::new(&data[16..]);
    let mut image: Option<ImagePayload> = None;
    let mut alph: Option<AlphChunk> = None;
    while let Some(c) = chunks.next().transpose()? {
        match &c.id {
            b"VP8 " => image = Some(ImagePayload::Vp8(c.data.to_vec())),
            b"VP8L" => image = Some(ImagePayload::Vp8l(c.data.to_vec())),
            b"ALPH" => {
                if !c.data.is_empty() {
                    let hdr = c.data[0];
                    alph = Some(AlphChunk {
                        pre_processing: (hdr >> 4) & 0x3,
                        filtering: (hdr >> 2) & 0x3,
                        compression: hdr & 0x3,
                        data: c.data[1..].to_vec(),
                    });
                }
            }
            _ => {}
        }
    }
    let image = image.ok_or_else(|| Error::invalid("WebP: ANMF has no image chunk"))?;
    Ok(AnmfBundle {
        x_offset: x_off * 2, // spec: multiples of 2 → stored /2
        y_offset: y_off * 2,
        width: w,
        height: h,
        duration_ms: dur,
        dispose_to_background,
        blend_with_previous,
        image,
        alph,
    })
}

fn parse_vp8_keyframe_dims(vp8: &[u8]) -> Result<(u32, u32)> {
    // Bare VP8 keyframe tag: 3 byte frame tag + 3 byte start code + 4 byte hdr.
    if vp8.len() < 10 {
        return Err(Error::invalid("WebP: VP8 chunk too short"));
    }
    if vp8[3] != 0x9d || vp8[4] != 0x01 || vp8[5] != 0x2a {
        return Err(Error::invalid("WebP: missing VP8 keyframe start code"));
    }
    let w = u16::from_le_bytes([vp8[6], vp8[7]]) as u32 & 0x3FFF;
    let h = u16::from_le_bytes([vp8[8], vp8[9]]) as u32 & 0x3FFF;
    Ok((w, h))
}

fn parse_vp8l_dims(vp8l: &[u8]) -> Result<(u32, u32)> {
    // VP8L: signature byte 0x2f then 14 bit width-1, 14 bit height-1, ...
    if vp8l.len() < 5 {
        return Err(Error::invalid("WebP: VP8L chunk too short"));
    }
    if vp8l[0] != 0x2f {
        return Err(Error::invalid("WebP: bad VP8L signature"));
    }
    let bits = u32::from_le_bytes([vp8l[1], vp8l[2], vp8l[3], vp8l[4]]);
    let w = (bits & 0x3FFF) + 1;
    let h = ((bits >> 14) & 0x3FFF) + 1;
    Ok((w, h))
}

/// Iterator over RIFF chunks inside a body. Borrows the body slice.
struct RiffChunks<'a> {
    body: &'a [u8],
    pos: usize,
}

impl<'a> RiffChunks<'a> {
    fn new(body: &'a [u8]) -> Self {
        Self { body, pos: 0 }
    }
}

struct ChunkRef<'a> {
    id: [u8; 4],
    data: &'a [u8],
}

impl<'a> Iterator for RiffChunks<'a> {
    type Item = Result<ChunkRef<'a>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + 8 > self.body.len() {
            // Dangling trailing bytes <8 bytes long — treat as clean EOF
            // to survive tolerant muxers.
            return None;
        }
        let id = [
            self.body[self.pos],
            self.body[self.pos + 1],
            self.body[self.pos + 2],
            self.body[self.pos + 3],
        ];
        let size = u32::from_le_bytes([
            self.body[self.pos + 4],
            self.body[self.pos + 5],
            self.body[self.pos + 6],
            self.body[self.pos + 7],
        ]) as usize;
        let payload_start = self.pos + 8;
        let payload_end = payload_start.saturating_add(size);
        if payload_end > self.body.len() {
            return Some(Err(Error::invalid("WebP: chunk extends past RIFF body")));
        }
        let data = &self.body[payload_start..payload_end];
        let padded = (size + (size & 1)).min(self.body.len().saturating_sub(payload_start));
        self.pos = payload_start + padded;
        Some(Ok(ChunkRef { id, data }))
    }
}

struct WebpDemuxer {
    stream: StreamInfo,
    packets: Vec<Packet>,
    pos: usize,
}

impl Demuxer for WebpDemuxer {
    fn format_name(&self) -> &str {
        "webp"
    }

    fn streams(&self) -> &[StreamInfo] {
        std::slice::from_ref(&self.stream)
    }

    fn next_packet(&mut self) -> Result<Packet> {
        if self.pos >= self.packets.len() {
            return Err(Error::Eof);
        }
        let pkt = self.packets[self.pos].clone();
        self.pos += 1;
        Ok(pkt)
    }

    fn duration_micros(&self) -> Option<i64> {
        self.stream.duration.map(|d| d * 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_recognises_webp() {
        let mut buf = vec![0u8; 16];
        buf[..4].copy_from_slice(b"RIFF");
        buf[8..12].copy_from_slice(b"WEBP");
        let p = ProbeData {
            buf: &buf,
            ext: None,
        };
        assert_eq!(probe(&p), 100);
    }

    #[test]
    fn probe_rejects_non_webp_riff() {
        let mut buf = vec![0u8; 16];
        buf[..4].copy_from_slice(b"RIFF");
        buf[8..12].copy_from_slice(b"AVI ");
        let p = ProbeData {
            buf: &buf,
            ext: None,
        };
        assert_eq!(probe(&p), 0);
    }
}
