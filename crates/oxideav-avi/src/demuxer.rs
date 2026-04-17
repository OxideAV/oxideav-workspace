//! AVI (RIFF/AVI) demuxer.
//!
//! On `open()`:
//! 1. Verify the top-level `RIFF`…`AVI ` header.
//! 2. Locate the `hdrl` LIST, parse `avih` (main header) and each `strl`
//!    LIST → `strh` (stream header) + `strf` (stream format).
//! 3. Locate the `movi` LIST. Remember its start offset and size so we can
//!    walk packet chunks lazily.
//! 4. If an `idx1` top-level chunk is present, parse it into an in-memory
//!    seek table (see [`IdxEntry`]). OpenDML `indx`/`ix##` super-indexes
//!    are out of scope — `seek_to` returns `Unsupported` when no `idx1`
//!    was seen.
//!
//! `next_packet()` walks chunks inside `movi`. Each payload chunk name is
//! `NNxx` where `NN` is a two-ASCII-digit stream index and `xx` is one of
//! `dc` (compressed video), `db` (uncompressed video), `wb` (audio), or
//! something else which we skip. Unknown or out-of-range indexes are skipped
//! so we can tolerate files with embedded junk (`JUNK`, `ix##`, unsupported
//! streams).

use std::io::{Seek, SeekFrom};

use oxideav_container::{Demuxer, ReadSeek};
use oxideav_core::{
    CodecId, CodecParameters, Error, MediaType, Packet, Rational, Result, SampleFormat, StreamInfo,
    TimeBase,
};

use crate::codec_map::{audio_codec_id, video_codec_id};
use crate::riff::{read_chunk_header, read_form_type, skip_chunk, skip_pad, AVI_FORM, LIST, RIFF};
use crate::stream_format::{parse_bitmap_info_header, parse_waveformatex};

/// Factory registered with the container registry.
pub fn open(mut input: Box<dyn ReadSeek>) -> Result<Box<dyn Demuxer>> {
    // Top-level RIFF chunk.
    let top = match read_chunk_header(&mut *input)? {
        Some(h) => h,
        None => return Err(Error::invalid("AVI: empty file")),
    };
    if top.id != RIFF {
        return Err(Error::invalid("AVI: not a RIFF file"));
    }
    let form = read_form_type(&mut *input)?;
    if form != AVI_FORM {
        return Err(Error::invalid("AVI: RIFF form type is not AVI"));
    }

    // Walk top-level nested chunks until we've processed both hdrl and movi.
    let mut streams: Vec<StreamInfo> = Vec::new();
    let mut packet_chunk_suffix: Vec<[u8; 2]> = Vec::new();
    let mut movi_start: Option<u64> = None;
    let mut movi_end: Option<u64> = None;
    let mut avih: Option<AviMainHeader> = None;
    let mut metadata: Vec<(String, String)> = Vec::new();
    let mut idx1_raw: Option<Vec<u8>> = None;

    while let Some(hdr) = read_chunk_header(&mut *input)? {
        if hdr.id == LIST {
            let list_type = read_form_type(&mut *input)?;
            let body_len = hdr.size.saturating_sub(4);
            let body_start = input.stream_position()?;
            let body_end = body_start + body_len as u64;
            match &list_type {
                b"hdrl" => {
                    let (main, stream_infos, suffixes) = parse_hdrl(&mut *input, body_end)?;
                    avih = Some(main);
                    streams = stream_infos;
                    packet_chunk_suffix = suffixes;
                }
                b"movi" => {
                    movi_start = Some(body_start);
                    movi_end = Some(body_end);
                }
                b"INFO" => {
                    let mut buf = vec![0u8; body_len as usize];
                    input.read_exact(&mut buf)?;
                    parse_info_list(&buf, &mut metadata);
                }
                _ => {}
            }
            // Jump to end of list (skips contents we didn't consume) + pad.
            input.seek(SeekFrom::Start(body_end))?;
            skip_pad(&mut *input, hdr.size)?;
        } else if &hdr.id == b"idx1" {
            // Legacy AVI 1.0 index. Read the body now so we can build a
            // keyframe seek table; fall back silently if it's malformed.
            let mut buf = vec![0u8; hdr.size as usize];
            input.read_exact(&mut buf)?;
            skip_pad(&mut *input, hdr.size)?;
            idx1_raw = Some(buf);
        } else {
            // Non-list top-level chunks (JUNK, etc.).
            skip_chunk(&mut *input, &hdr)?;
        }
    }

    let movi_start = movi_start.ok_or_else(|| Error::invalid("AVI: missing movi list"))?;
    let movi_end = movi_end.ok_or_else(|| Error::invalid("AVI: missing movi list"))?;
    if streams.is_empty() {
        return Err(Error::invalid("AVI: no streams"));
    }

    // Duration: the AVI main header carries microseconds-per-frame and
    // total-frame-count for the primary (first) video stream. Multiply.
    let duration_micros: i64 = match avih {
        Some(h) if h.micro_sec_per_frame > 0 && h.total_frames > 0 => {
            (h.total_frames as i64) * (h.micro_sec_per_frame as i64)
        }
        _ => 0,
    };

    // Build the seek table from idx1 (if present). `build_idx_table` resolves
    // the per-file offset base (file-absolute vs movi-relative) by probing
    // the first entry against the known chunk header.
    let idx_table = if let Some(raw) = idx1_raw {
        build_idx_table(&mut *input, &raw, movi_start, &streams)?
    } else {
        Vec::new()
    };

    // Seek to start of movi body for next_packet.
    input.seek(SeekFrom::Start(movi_start))?;

    Ok(Box::new(AviDemuxer {
        input,
        streams,
        packet_chunk_suffix,
        movi_start,
        movi_end,
        per_stream_counter: Vec::new(),
        metadata,
        duration_micros,
        idx_table,
    }))
}

/// Parse a `LIST INFO` body (the 4-byte "INFO" form-type has already been
/// consumed). Each child is a 4-CC chunk whose payload is a NUL-terminated
/// string. Maps to standard metadata keys.
fn parse_info_list(buf: &[u8], out: &mut Vec<(String, String)>) {
    let mut i = 0usize;
    while i + 8 <= buf.len() {
        let id: [u8; 4] = [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]];
        let size = u32::from_le_bytes([buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]]) as usize;
        i += 8;
        if i + size > buf.len() {
            break;
        }
        let raw = &buf[i..i + size];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let value = String::from_utf8_lossy(&raw[..end]).trim().to_string();
        let key = info_id_to_key(&id);
        if !value.is_empty() {
            if let Some(k) = key {
                out.push((k.to_string(), value));
            }
        }
        i += size;
        if size % 2 == 1 {
            i += 1;
        }
    }
}

fn info_id_to_key(id: &[u8; 4]) -> Option<&'static str> {
    match id {
        b"INAM" => Some("title"),
        b"IART" => Some("artist"),
        b"IPRD" => Some("album"),
        b"ICMT" => Some("comment"),
        b"ICRD" => Some("date"),
        b"IGNR" => Some("genre"),
        b"ICOP" => Some("copyright"),
        b"IENG" => Some("engineer"),
        b"ITCH" => Some("technician"),
        b"ISFT" => Some("encoder"),
        b"ISBJ" => Some("subject"),
        b"ITRK" => Some("track"),
        _ => None,
    }
}

/// Decoded AVIMAINHEADER (dwMicroSecPerFrame / … struct).
///
/// Most fields are retained for future use (seek tables, buffer sizing) even
/// though the current demuxer consumes them only during parsing.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
struct AviMainHeader {
    micro_sec_per_frame: u32,
    #[allow(dead_code)]
    max_bytes_per_sec: u32,
    #[allow(dead_code)]
    flags: u32,
    total_frames: u32,
    #[allow(dead_code)]
    initial_frames: u32,
    streams: u32,
    #[allow(dead_code)]
    suggested_buffer_size: u32,
    width: u32,
    height: u32,
}

/// Parse the AVIMAINHEADER body (should be 56 bytes).
fn parse_avih(buf: &[u8]) -> Result<AviMainHeader> {
    if buf.len() < 40 {
        return Err(Error::invalid("AVI: avih too short"));
    }
    Ok(AviMainHeader {
        micro_sec_per_frame: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
        max_bytes_per_sec: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        // dwPaddingGranularity at offset 8 is ignored.
        flags: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
        total_frames: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
        initial_frames: u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
        streams: u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]),
        suggested_buffer_size: u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]),
        width: u32::from_le_bytes([buf[32], buf[33], buf[34], buf[35]]),
        height: u32::from_le_bytes([buf[36], buf[37], buf[38], buf[39]]),
    })
}

/// Parse the `hdrl` LIST body.
///
/// Reads `avih`, then walks each nested `strl` LIST to build one `StreamInfo`
/// per stream. Returns also the list of expected packet-chunk suffixes (e.g.
/// `b"dc"`, `b"wb"`) so the demuxer can recognise packets.
fn parse_hdrl<R: ReadSeek + ?Sized>(
    r: &mut R,
    end_pos: u64,
) -> Result<(AviMainHeader, Vec<StreamInfo>, Vec<[u8; 2]>)> {
    let mut main = AviMainHeader::default();
    let mut streams: Vec<StreamInfo> = Vec::new();
    let mut suffixes: Vec<[u8; 2]> = Vec::new();

    while r.stream_position()? < end_pos {
        let hdr = match read_chunk_header(r)? {
            Some(h) => h,
            None => break,
        };
        match &hdr.id {
            b"avih" => {
                let body = read_body_bounded(r, hdr.size)?;
                main = parse_avih(&body)?;
                skip_pad(r, hdr.size)?;
            }
            b"LIST" => {
                let list_type = read_form_type(r)?;
                let body_len = hdr.size.saturating_sub(4);
                let body_start = r.stream_position()?;
                let body_end = body_start + body_len as u64;
                if &list_type == b"strl" {
                    let (si, suf) = parse_strl(r, body_end, streams.len() as u32)?;
                    if let Some(si) = si {
                        streams.push(si);
                        suffixes.push(suf.unwrap_or(*b"xx"));
                    }
                }
                r.seek(SeekFrom::Start(body_end))?;
                skip_pad(r, hdr.size)?;
            }
            _ => {
                skip_chunk(r, &hdr)?;
            }
        }
    }
    Ok((main, streams, suffixes))
}

/// Parse a `strl` LIST. Returns the `StreamInfo` and expected packet suffix.
fn parse_strl<R: ReadSeek + ?Sized>(
    r: &mut R,
    end_pos: u64,
    index: u32,
) -> Result<(Option<StreamInfo>, Option<[u8; 2]>)> {
    let mut strh_buf: Option<Vec<u8>> = None;
    let mut strf_buf: Option<Vec<u8>> = None;
    while r.stream_position()? < end_pos {
        let hdr = match read_chunk_header(r)? {
            Some(h) => h,
            None => break,
        };
        match &hdr.id {
            b"strh" => {
                strh_buf = Some(read_body_bounded(r, hdr.size)?);
                skip_pad(r, hdr.size)?;
            }
            b"strf" => {
                strf_buf = Some(read_body_bounded(r, hdr.size)?);
                skip_pad(r, hdr.size)?;
            }
            _ => {
                skip_chunk(r, &hdr)?;
            }
        }
    }
    let strh = match strh_buf {
        Some(b) => b,
        None => return Ok((None, None)),
    };
    let strf = strf_buf.unwrap_or_default();
    let parsed = build_stream(index, &strh, &strf)?;
    Ok((Some(parsed.0), Some(parsed.1)))
}

/// Build a StreamInfo from strh + strf payloads.
fn build_stream(index: u32, strh: &[u8], strf: &[u8]) -> Result<(StreamInfo, [u8; 2])> {
    // AVISTREAMHEADER layout (56 bytes):
    //   0  fccType       [4]
    //   4  fccHandler    [4]
    //   8  dwFlags       u32
    //  12  wPriority     u16
    //  14  wLanguage     u16
    //  16  dwInitialFrames u32
    //  20  dwScale       u32
    //  24  dwRate        u32  (rate/scale = samples/sec)
    //  28  dwStart       u32
    //  32  dwLength      u32
    //  36  dwSuggestedBufferSize u32
    //  40  dwQuality     u32
    //  44  dwSampleSize  u32
    //  48  rcFrame       [4 * i16]
    if strh.len() < 48 {
        return Err(Error::invalid("AVI: strh too short"));
    }
    let mut fcc_type = [0u8; 4];
    fcc_type.copy_from_slice(&strh[0..4]);
    let mut fcc_handler = [0u8; 4];
    fcc_handler.copy_from_slice(&strh[4..8]);
    let scale = u32::from_le_bytes([strh[20], strh[21], strh[22], strh[23]]).max(1);
    let rate = u32::from_le_bytes([strh[24], strh[25], strh[26], strh[27]]).max(1);
    let length = u32::from_le_bytes([strh[32], strh[33], strh[34], strh[35]]);
    let sample_size = u32::from_le_bytes([strh[44], strh[45], strh[46], strh[47]]);

    let (media_type, codec_id, params, suffix) = match &fcc_type {
        b"vids" => {
            let bmih = if !strf.is_empty() {
                Some(parse_bitmap_info_header(strf)?)
            } else {
                None
            };
            let compression = bmih.as_ref().map(|b| b.compression).unwrap_or(fcc_handler);
            let codec_id = video_codec_id(&compression);
            let mut p = CodecParameters::video(codec_id.clone());
            if let Some(b) = &bmih {
                p.width = Some(b.width);
                p.height = Some(b.height);
                p.extradata = b.extradata.clone();
            }
            // Frame rate from scale/rate (rate/scale = fps).
            p.frame_rate = Some(Rational::new(rate as i64, scale as i64));
            // MJPEG packets from AVI should be flagged as standalone JPEGs.
            let suffix = if codec_id.as_str() == "rgb24" {
                *b"db"
            } else {
                *b"dc"
            };
            (MediaType::Video, codec_id, p, suffix)
        }
        b"auds" => {
            let wfx = if !strf.is_empty() {
                Some(parse_waveformatex(strf)?)
            } else {
                None
            };
            let format_tag = wfx.as_ref().map(|w| w.format_tag).unwrap_or(0);
            let codec_id = audio_codec_id(format_tag);
            let mut p = CodecParameters::audio(codec_id.clone());
            if let Some(w) = &wfx {
                p.channels = Some(w.channels);
                p.sample_rate = Some(w.samples_per_sec);
                p.extradata = w.extradata.clone();
                p.sample_format = match (codec_id.as_str(), w.bits_per_sample) {
                    ("pcm_s16le", _) => Some(SampleFormat::S16),
                    (_, 16) => Some(SampleFormat::S16),
                    (_, 8) => Some(SampleFormat::U8),
                    _ => None,
                };
                p.bit_rate = if w.avg_bytes_per_sec > 0 {
                    Some(w.avg_bytes_per_sec as u64 * 8)
                } else {
                    None
                };
            }
            (MediaType::Audio, codec_id, p, *b"wb")
        }
        _ => {
            // "txts", "mids", "dats" — represent as data.
            let codec_id = CodecId::new(format!(
                "avi:{}",
                std::str::from_utf8(&fcc_type).unwrap_or("????")
            ));
            let mut p = CodecParameters::audio(codec_id.clone());
            p.media_type = MediaType::Data;
            (MediaType::Data, codec_id, p, *b"xx")
        }
    };

    let _ = codec_id; // absorbed into params

    // Stream time base. For video: scale/rate seconds per frame. For audio
    // at rate/scale samples per second, pick 1/samples_per_sec (standard
    // choice). For anything else, fall back to 1/rate.
    let time_base = match media_type {
        MediaType::Video => TimeBase::new(scale as i64, rate as i64),
        MediaType::Audio => {
            // rate/scale = samples_per_sec for PCM.
            TimeBase::new(scale as i64, rate as i64)
        }
        _ => TimeBase::new(scale as i64, rate as i64),
    };

    let duration = if length > 0 {
        Some(length as i64)
    } else {
        None
    };
    let stream = StreamInfo {
        index,
        time_base,
        duration,
        start_time: Some(0),
        params,
    };
    let _ = sample_size;
    Ok((stream, suffix))
}

fn read_body_bounded<R: std::io::Read + ?Sized>(r: &mut R, size: u32) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; size as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// Parse a raw `idx1` body, decide whether the recorded offsets are
/// file-absolute or `movi`-relative (both are seen in the wild), and
/// populate each entry with a synthesised per-stream pts.
///
/// Offset-base detection: AVI 1.0 is ambiguous about the reference point
/// for idx1 offsets. Some muxers (MS reference, ffmpeg) emit offsets
/// relative to the `movi` FourCC; others emit file-absolute offsets. We
/// probe the first plausible entry by reading the 8-byte chunk header at
/// `file_start + offset` and `movi_start - 4 + offset` (the "- 4" puts us
/// at the `movi` FourCC byte) and picking whichever yields the matching
/// `ckid`. Default to movi-relative if the file is too small to probe.
fn build_idx_table<R: ReadSeek + ?Sized>(
    r: &mut R,
    raw: &[u8],
    movi_start: u64,
    streams: &[StreamInfo],
) -> Result<Vec<IdxEntry>> {
    if raw.len() < 16 {
        return Ok(Vec::new());
    }
    let n = raw.len() / 16;
    // Pick the first entry with a non-zero offset as a probe.
    let mut probe_raw_offset: Option<u32> = None;
    let mut probe_ckid: Option<[u8; 4]> = None;
    for i in 0..n {
        let base = i * 16;
        let off =
            u32::from_le_bytes([raw[base + 8], raw[base + 9], raw[base + 10], raw[base + 11]]);
        if off != 0 {
            let mut ckid = [0u8; 4];
            ckid.copy_from_slice(&raw[base..base + 4]);
            probe_raw_offset = Some(off);
            probe_ckid = Some(ckid);
            break;
        }
    }

    // `movi_start` points at the first chunk header inside movi (i.e. 4
    // bytes *after* the `movi` FourCC). idx1 offsets relative to the
    // `movi` FourCC therefore need an adjustment of `movi_start - 4`.
    let movi_fourcc_pos = movi_start.saturating_sub(4);
    let mut movi_relative = true; // conservative default: most files.
    if let (Some(raw_off), Some(ckid)) = (probe_raw_offset, probe_ckid) {
        let try_movi = movi_fourcc_pos.checked_add(raw_off as u64);
        let try_abs = Some(raw_off as u64);
        let movi_ok = match try_movi {
            Some(p) => probe_offset_has_ckid(r, p, &ckid).unwrap_or(false),
            None => false,
        };
        let abs_ok = match try_abs {
            Some(p) => probe_offset_has_ckid(r, p, &ckid).unwrap_or(false),
            None => false,
        };
        movi_relative = match (movi_ok, abs_ok) {
            (true, false) => true,
            (false, true) => false,
            // If both or neither match, stick with movi-relative (the
            // more common convention). A broken index is tolerable — it
            // just means seek_to lands on wrong data and the player
            // discovers it on next read.
            _ => true,
        };
    }
    let base_off = if movi_relative { movi_fourcc_pos } else { 0 };

    // First pass: build entries with file-absolute offsets. Drop entries
    // for unknown stream indexes (tolerate stray junk).
    let mut entries: Vec<IdxEntry> = Vec::with_capacity(n);
    for i in 0..n {
        let base = i * 16;
        let mut ckid = [0u8; 4];
        ckid.copy_from_slice(&raw[base..base + 4]);
        let flags =
            u32::from_le_bytes([raw[base + 4], raw[base + 5], raw[base + 6], raw[base + 7]]);
        let raw_off =
            u32::from_le_bytes([raw[base + 8], raw[base + 9], raw[base + 10], raw[base + 11]]);
        let size = u32::from_le_bytes([
            raw[base + 12],
            raw[base + 13],
            raw[base + 14],
            raw[base + 15],
        ]);
        let stream = match parse_stream_index(&ckid) {
            Some(s) => s,
            None => continue,
        };
        if (stream as usize) >= streams.len() {
            continue;
        }
        let abs = base_off.saturating_add(raw_off as u64);
        entries.push(IdxEntry {
            stream,
            flags,
            offset: abs,
            size,
            pts: 0,
        });
    }

    // Second pass: assign per-stream pts by walking each stream's entries
    // in idx1 order, mirroring the pts-bump logic in `next_packet`.
    let mut per_stream_pts: Vec<i64> = vec![0; streams.len()];
    for e in entries.iter_mut() {
        let s = e.stream as usize;
        e.pts = per_stream_pts[s];
        let bump = packet_time_delta(&streams[s], e.size as usize) as i64;
        per_stream_pts[s] = per_stream_pts[s].saturating_add(bump);
    }

    Ok(entries)
}

/// Read the 4-byte ckid at `offset` (no seek restore) and check whether
/// it matches `expected`. Returns `Ok(false)` on short read rather than
/// propagating EOF, so the caller can probe both offset bases safely.
fn probe_offset_has_ckid<R: ReadSeek + ?Sized>(
    r: &mut R,
    offset: u64,
    expected: &[u8; 4],
) -> Result<bool> {
    r.seek(SeekFrom::Start(offset))?;
    let mut buf = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut buf[got..]) {
            Ok(0) => return Ok(false),
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Ok(false),
        }
    }
    Ok(&buf == expected)
}

// --- Demuxer runtime ------------------------------------------------------

struct AviDemuxer {
    input: Box<dyn ReadSeek>,
    streams: Vec<StreamInfo>,
    /// For each stream, the expected 2-byte chunk-name suffix in `movi`.
    packet_chunk_suffix: Vec<[u8; 2]>,
    /// Absolute start-of-movi offset (first chunk header after the `movi`
    /// form-type FourCC). Retained so `seek_to` can bound against the
    /// beginning of packet data.
    movi_start: u64,
    /// Absolute end-of-movi offset.
    movi_end: u64,
    /// Running packet counter per stream — used to synthesise PTS.
    per_stream_counter: Vec<u64>,
    metadata: Vec<(String, String)>,
    duration_micros: i64,
    /// Optional idx1-derived seek table (empty = not available).
    idx_table: Vec<IdxEntry>,
}

/// One entry parsed from the `idx1` top-level chunk, normalised to
/// file-absolute offsets and annotated with a stream-local pts.
#[derive(Clone, Copy, Debug)]
struct IdxEntry {
    /// Stream index (0..streams.len()), derived from the first two ASCII
    /// digits of the `ckid` FourCC.
    stream: u32,
    /// Raw flags field; bit 0x10 is `AVIIF_KEYFRAME`.
    flags: u32,
    /// File-absolute offset of the chunk header (8-byte `ckid` + size).
    offset: u64,
    /// Payload size as recorded in idx1.
    #[allow(dead_code)]
    size: u32,
    /// Synthesised PTS at this entry (in the stream's time base). Matches
    /// `per_stream_counter[stream]` right after `next_packet` finishes
    /// returning the packet pointed to by this entry.
    pts: i64,
}

/// `AVIIF_KEYFRAME` bit in an idx1 entry's flags.
const AVIIF_KEYFRAME: u32 = 0x0000_0010;

impl Demuxer for AviDemuxer {
    fn format_name(&self) -> &str {
        "avi"
    }

    fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }

    fn next_packet(&mut self) -> Result<Packet> {
        if self.per_stream_counter.len() != self.streams.len() {
            self.per_stream_counter = vec![0u64; self.streams.len()];
        }
        loop {
            let pos = self.input.stream_position()?;
            if pos >= self.movi_end {
                return Err(Error::Eof);
            }
            let hdr = match read_chunk_header(&mut *self.input)? {
                Some(h) => h,
                None => return Err(Error::Eof),
            };
            // `LIST rec ` is an optional grouping inside movi — some writers
            // cluster chunks this way. Recurse by entering the list body.
            if hdr.id == LIST {
                let _form = read_form_type(&mut *self.input)?; // likely "rec "
                                                               // Continue: next iteration will consume its nested chunks.
                continue;
            }
            // End of movi guard in case sizes disagree.
            let body_end = self.input.stream_position()? + hdr.size as u64;
            if body_end > self.movi_end {
                // Truncated or bad size — stop.
                return Err(Error::Eof);
            }
            if hdr.id == *b"JUNK" || hdr.id == *b"junk" {
                skip_chunk(&mut *self.input, &hdr)?;
                continue;
            }
            // Payload chunk format: "NNsf" where NN is two ASCII digits and
            // sf ∈ {"dc","db","wb","pc","tx"}.
            if let Some(idx) = parse_stream_index(&hdr.id) {
                if (idx as usize) < self.streams.len() {
                    let expected = self.packet_chunk_suffix[idx as usize];
                    let suffix = [hdr.id[2], hdr.id[3]];
                    // Accept expected suffix; skip "pc" (palette change) and others.
                    let accept = suffix == expected
                        || suffix == *b"dc"
                        || suffix == *b"db"
                        || suffix == *b"wb";
                    if accept {
                        let data = read_body_bounded(&mut *self.input, hdr.size)?;
                        skip_pad(&mut *self.input, hdr.size)?;
                        let stream = &self.streams[idx as usize];
                        let counter = self.per_stream_counter[idx as usize];
                        // PTS: for video the counter is a frame index in the
                        // stream's time_base. For audio we advance by the
                        // number of samples in this packet (PCM: block_align
                        // derived from bps*channels; other codecs we just use
                        // the packet counter in units of rate/scale).
                        let pts = counter as i64;
                        let mut pkt = Packet::new(idx, stream.time_base, data);
                        pkt.pts = Some(pts);
                        pkt.dts = Some(pts);
                        pkt.flags.keyframe = true;
                        // Bump counter.
                        let bump = packet_time_delta(stream, pkt.data.len());
                        self.per_stream_counter[idx as usize] = counter + bump;
                        return Ok(pkt);
                    } else {
                        skip_chunk(&mut *self.input, &hdr)?;
                        continue;
                    }
                } else {
                    skip_chunk(&mut *self.input, &hdr)?;
                    continue;
                }
            }
            skip_chunk(&mut *self.input, &hdr)?;
        }
    }

    fn seek_to(&mut self, stream_index: u32, pts: i64) -> Result<i64> {
        if (stream_index as usize) >= self.streams.len() {
            return Err(Error::invalid(format!(
                "AVI: stream index {stream_index} out of range"
            )));
        }
        if self.idx_table.is_empty() {
            return Err(Error::unsupported(
                "AVI: seek requires idx1; OpenDML indx/ix## not implemented",
            ));
        }

        // Find the last keyframe entry for `stream_index` with pts <= target.
        let mut best: Option<&IdxEntry> = None;
        for e in &self.idx_table {
            if e.stream != stream_index || (e.flags & AVIIF_KEYFRAME) == 0 {
                continue;
            }
            if e.pts <= pts {
                best = match best {
                    Some(b) if b.pts >= e.pts => Some(b),
                    _ => Some(e),
                };
            }
        }
        // Fall back to the first keyframe of this stream if nothing matches
        // (e.g. caller asked for a negative pts).
        if best.is_none() {
            for e in &self.idx_table {
                if e.stream == stream_index && (e.flags & AVIIF_KEYFRAME) != 0 {
                    best = Some(e);
                    break;
                }
            }
        }
        let landed = best.ok_or_else(|| {
            Error::unsupported(format!(
                "AVI: no keyframes in idx1 for stream {stream_index}"
            ))
        })?;

        // Seek the input to the landed chunk header. Clamp to movi bounds
        // so a corrupt idx1 can't send us outside the payload region.
        let mut target_off = landed.offset;
        if target_off < self.movi_start {
            target_off = self.movi_start;
        }
        if target_off >= self.movi_end {
            return Err(Error::invalid(
                "AVI: idx1 entry points past end of movi list",
            ));
        }
        self.input.seek(SeekFrom::Start(target_off))?;

        // Reset per-stream pts counters. For streams we have idx entries
        // for, use the stream-local pts at-or-before `target_off`. For
        // streams we don't, reset to zero (the counter will resynchronise
        // once we next see a packet for that stream — this is imperfect
        // but there's no better signal without a dense index).
        if self.per_stream_counter.len() != self.streams.len() {
            self.per_stream_counter = vec![0u64; self.streams.len()];
        } else {
            for c in self.per_stream_counter.iter_mut() {
                *c = 0;
            }
        }
        for e in &self.idx_table {
            if e.offset > target_off {
                break;
            }
            let s = e.stream as usize;
            if s < self.per_stream_counter.len() {
                // Latest idx entry at-or-before target_off for this stream.
                self.per_stream_counter[s] = e.pts.max(0) as u64;
            }
        }

        Ok(landed.pts)
    }

    fn metadata(&self) -> &[(String, String)] {
        &self.metadata
    }

    fn duration_micros(&self) -> Option<i64> {
        if self.duration_micros > 0 {
            Some(self.duration_micros)
        } else {
            None
        }
    }
}

/// Parse "NNsf" where NN is two ASCII digits into the stream index.
fn parse_stream_index(name: &[u8; 4]) -> Option<u32> {
    let h = ascii_hex(name[0])?;
    let l = ascii_hex(name[1])?;
    Some((h as u32) * 16 + l as u32)
}

/// Decode a single ASCII hex digit (0-9, a-f, A-F).
fn ascii_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn packet_time_delta(stream: &StreamInfo, payload_len: usize) -> u64 {
    match stream.params.media_type {
        MediaType::Video => 1,
        MediaType::Audio => {
            // PCM: duration = frames = payload / block_align. Non-PCM: one
            // tick per packet is a reasonable fallback.
            let block_align = stream
                .params
                .channels
                .zip(stream.params.sample_format)
                .map(|(c, f)| (c as usize) * f.bytes_per_sample())
                .filter(|&v| v > 0)
                .unwrap_or(0);
            payload_len.checked_div(block_align).unwrap_or(1) as u64
        }
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_index_parses() {
        assert_eq!(parse_stream_index(b"00dc"), Some(0));
        assert_eq!(parse_stream_index(b"01wb"), Some(1));
        assert_eq!(parse_stream_index(b"0adb"), Some(10));
        assert_eq!(parse_stream_index(b"XXXX"), None);
    }
}
