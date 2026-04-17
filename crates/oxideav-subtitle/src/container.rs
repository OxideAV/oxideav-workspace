//! Containers for the three standalone subtitle formats.
//!
//! Each demuxer reads the file into memory, parses it, and queues one
//! [`Packet`] per cue. Each muxer accumulates incoming packets and emits
//! the full file in `write_trailer`.
//!
//! The on-wire payload per packet is the cue's natural textual form
//! (SRT cue body, WebVTT cue body, or `Dialogue:` line). The codec
//! parameters' `extradata` carries the file-level header (empty for
//! SRT, the `WEBVTT ...`/STYLE blocks for WebVTT, and `[Script Info]` +
//! `[V4+ Styles]` + `[Events]` lead-in for ASS/SSA).

use std::collections::VecDeque;
use std::io::{Read, SeekFrom, Write};

use oxideav_container::{
    ContainerRegistry, Demuxer, Muxer, ProbeData, ReadSeek, WriteSeek,
};
use oxideav_core::{
    CodecId, CodecParameters, Error, MediaType, Packet, Result, StreamInfo, TimeBase,
};

use crate::ir::{SourceFormat, SubtitleTrack};
use crate::{ass, srt, webvtt};

/// Codec ids emitted by this crate's containers.
pub const SRT_CODEC_ID: &str = "subrip";
pub const WEBVTT_CODEC_ID: &str = "webvtt";
pub const ASS_CODEC_ID: &str = "ass";

pub fn register(reg: &mut ContainerRegistry) {
    // SRT.
    reg.register_demuxer("srt", open_srt);
    reg.register_muxer("srt", mux_srt);
    reg.register_extension("srt", "srt");
    reg.register_probe("srt", probe_srt);

    // WebVTT.
    reg.register_demuxer("webvtt", open_webvtt);
    reg.register_muxer("webvtt", mux_webvtt);
    reg.register_extension("vtt", "webvtt");
    reg.register_probe("webvtt", probe_webvtt);

    // ASS / SSA.
    reg.register_demuxer("ass", open_ass);
    reg.register_muxer("ass", mux_ass);
    reg.register_extension("ass", "ass");
    reg.register_extension("ssa", "ass");
    reg.register_probe("ass", probe_ass);
}

fn probe_srt(p: &ProbeData) -> u8 {
    if srt::looks_like_srt(p.buf) {
        // A positive integer on the first line followed by a timing line
        // is a pretty distinctive shape, but SRT has no magic bytes, so
        // we cap at 75 to defer to WebVTT / ASS when those match.
        75
    } else {
        0
    }
}

fn probe_webvtt(p: &ProbeData) -> u8 {
    if webvtt::looks_like_webvtt(p.buf) {
        100
    } else {
        0
    }
}

fn probe_ass(p: &ProbeData) -> u8 {
    if ass::looks_like_ass(p.buf) {
        100
    } else {
        0
    }
}

fn read_all(mut input: Box<dyn ReadSeek>) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    input.seek(SeekFrom::Start(0))?;
    input.read_to_end(&mut buf)?;
    drop(input);
    Ok(buf)
}

fn open_srt(input: Box<dyn ReadSeek>) -> Result<Box<dyn Demuxer>> {
    let buf = read_all(input)?;
    let track = srt::parse(&buf)?;
    Ok(Box::new(TextSubtitleDemuxer::new(
        "srt",
        SRT_CODEC_ID,
        track,
        SourceFormat::Srt,
    )))
}

fn open_webvtt(input: Box<dyn ReadSeek>) -> Result<Box<dyn Demuxer>> {
    let buf = read_all(input)?;
    let track = webvtt::parse(&buf)?;
    Ok(Box::new(TextSubtitleDemuxer::new(
        "webvtt",
        WEBVTT_CODEC_ID,
        track,
        SourceFormat::WebVtt,
    )))
}

fn open_ass(input: Box<dyn ReadSeek>) -> Result<Box<dyn Demuxer>> {
    let buf = read_all(input)?;
    let track = ass::parse(&buf)?;
    Ok(Box::new(TextSubtitleDemuxer::new(
        "ass",
        ASS_CODEC_ID,
        track,
        SourceFormat::AssOrSsa,
    )))
}

fn mux_srt(out: Box<dyn WriteSeek>, streams: &[StreamInfo]) -> Result<Box<dyn Muxer>> {
    Ok(Box::new(TextSubtitleMuxer::new(out, streams, "srt")?))
}

fn mux_webvtt(out: Box<dyn WriteSeek>, streams: &[StreamInfo]) -> Result<Box<dyn Muxer>> {
    Ok(Box::new(TextSubtitleMuxer::new(out, streams, "webvtt")?))
}

fn mux_ass(out: Box<dyn WriteSeek>, streams: &[StreamInfo]) -> Result<Box<dyn Muxer>> {
    Ok(Box::new(TextSubtitleMuxer::new(out, streams, "ass")?))
}

// ---------------------------------------------------------------------------
// Generic text subtitle demuxer — one packet per cue.

struct TextSubtitleDemuxer {
    format_name: &'static str,
    streams: [StreamInfo; 1],
    packets: VecDeque<Packet>,
}

impl TextSubtitleDemuxer {
    fn new(
        format_name: &'static str,
        codec_id: &'static str,
        track: SubtitleTrack,
        source: SourceFormat,
    ) -> Self {
        let time_base = TimeBase::new(1, 1_000_000); // microseconds
        let mut params = CodecParameters::audio(CodecId::new(codec_id)); // fields reset below
        params.media_type = MediaType::Subtitle;
        params.sample_rate = None;
        params.channels = None;
        params.sample_format = None;
        params.extradata = track.extradata.clone();

        let total_us: i64 = track.cues.last().map(|c| c.end_us).unwrap_or(0);

        let mut packets: VecDeque<Packet> = VecDeque::with_capacity(track.cues.len());
        for cue in &track.cues {
            let payload = match source {
                SourceFormat::Srt => srt::cue_to_bytes(cue),
                SourceFormat::WebVtt => webvtt::cue_to_bytes(cue),
                SourceFormat::AssOrSsa => ass::cue_to_bytes(cue),
            };
            let mut pkt = Packet::new(0, time_base, payload);
            pkt.pts = Some(cue.start_us);
            pkt.dts = Some(cue.start_us);
            pkt.duration = Some((cue.end_us - cue.start_us).max(0));
            pkt.flags.keyframe = true;
            packets.push_back(pkt);
        }

        let stream = StreamInfo {
            index: 0,
            time_base,
            duration: Some(total_us),
            start_time: Some(0),
            params,
        };

        Self {
            format_name,
            streams: [stream],
            packets,
        }
    }
}

impl Demuxer for TextSubtitleDemuxer {
    fn format_name(&self) -> &str {
        self.format_name
    }

    fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }

    fn next_packet(&mut self) -> Result<Packet> {
        self.packets.pop_front().ok_or(Error::Eof)
    }

    fn duration_micros(&self) -> Option<i64> {
        self.streams[0].duration
    }
}

// ---------------------------------------------------------------------------
// Generic text subtitle muxer — buffer cues, reassemble file in write_trailer.

struct TextSubtitleMuxer {
    out: Box<dyn WriteSeek>,
    format: &'static str,
    extradata: Vec<u8>,
    buffered: Vec<Packet>,
    time_base: TimeBase,
    header_written: bool,
}

impl TextSubtitleMuxer {
    fn new(
        output: Box<dyn WriteSeek>,
        streams: &[StreamInfo],
        format: &'static str,
    ) -> Result<Self> {
        if streams.len() != 1 {
            return Err(Error::invalid(format!(
                "{format} muxer: exactly one subtitle stream required"
            )));
        }
        let s = &streams[0];
        let id = s.params.codec_id.as_str();
        let expected = match format {
            "srt" => SRT_CODEC_ID,
            "webvtt" => WEBVTT_CODEC_ID,
            "ass" => ASS_CODEC_ID,
            _ => "",
        };
        if id != expected {
            return Err(Error::invalid(format!(
                "{format} muxer: expected codec `{}`, got `{}`",
                expected, id
            )));
        }
        Ok(Self {
            out: output,
            format,
            extradata: s.params.extradata.clone(),
            buffered: Vec::new(),
            time_base: s.time_base,
            header_written: false,
        })
    }
}

impl Muxer for TextSubtitleMuxer {
    fn format_name(&self) -> &str {
        self.format
    }

    fn write_header(&mut self) -> Result<()> {
        self.header_written = true;
        Ok(())
    }

    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        if !self.header_written {
            return Err(Error::invalid("subtitle muxer: write_header not called"));
        }
        self.buffered.push(packet.clone());
        Ok(())
    }

    fn write_trailer(&mut self) -> Result<()> {
        let track = rebuild_track(self.format, &self.extradata, &self.buffered, self.time_base)?;
        let bytes = match self.format {
            "srt" => srt::write(&track),
            "webvtt" => webvtt::write(&track),
            "ass" => ass::write(&track),
            other => {
                return Err(Error::invalid(format!(
                    "subtitle muxer: unknown format {other}"
                )));
            }
        };
        self.out.write_all(&bytes)?;
        Ok(())
    }
}

fn rebuild_track(
    format: &str,
    extradata: &[u8],
    packets: &[Packet],
    _time_base: TimeBase,
) -> Result<SubtitleTrack> {
    let mut track = SubtitleTrack {
        extradata: extradata.to_vec(),
        ..SubtitleTrack::default()
    };
    // Seed styles / script-info from the extradata so the writer can
    // re-emit them. We do this by parsing the extradata as a truncated
    // file of this format — works because extradata carries the exact
    // header the demuxer saw.
    if !extradata.is_empty() {
        let parsed = match format {
            "srt" => None, // SRT has no header
            "webvtt" => Some(webvtt::parse(extradata).ok()),
            "ass" => Some(ass::parse(extradata).ok()),
            _ => None,
        };
        if let Some(Some(p)) = parsed {
            track.styles = p.styles;
            track.metadata = p.metadata;
        }
    }
    for pkt in packets {
        let cue = match format {
            "srt" => srt::bytes_to_cue(&pkt.data)?,
            "webvtt" => webvtt::bytes_to_cue(&pkt.data)?,
            "ass" => ass::bytes_to_cue(&pkt.data)?,
            other => return Err(Error::invalid(format!("unknown subtitle format {other}"))),
        };
        track.cues.push(cue);
    }
    Ok(track)
}
