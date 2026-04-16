//! MOD as a container format.
//!
//! MOD files are self-contained and don't have a natural packetisation,
//! so the container here is a thin shim: it reads the whole file into
//! memory, parses the header to populate the stream's `CodecParameters`
//! (channel count, sample rate, sample format), then delivers the entire
//! file as a single packet to the codec.

use std::io::Read;

use oxideav_container::{ContainerRegistry, Demuxer, ReadSeek};
use oxideav_core::{
    CodecId, CodecParameters, Error, MediaType, Packet, Result, SampleFormat, StreamInfo, TimeBase,
};

use crate::header::parse_header;

/// Output sample rate used by the decoder. 44.1 kHz is a common choice
/// that matches most "modern" MOD players; the Amiga Paula chip ran at
/// 7093789.2 Hz / divider so there's no "native" rate.
pub const OUTPUT_SAMPLE_RATE: u32 = 44_100;

pub fn register(reg: &mut ContainerRegistry) {
    reg.register_demuxer("mod", open);
    reg.register_extension("mod", "mod");
}

fn open(mut input: Box<dyn ReadSeek>) -> Result<Box<dyn Demuxer>> {
    let mut blob = Vec::new();
    input.read_to_end(&mut blob)?;
    if blob.len() < crate::header::HEADER_FIXED_SIZE {
        return Err(Error::invalid("MOD: file shorter than 1084-byte header"));
    }
    let header = parse_header(&blob)?;

    let mut params = CodecParameters::audio(CodecId::new(crate::CODEC_ID_STR));
    params.media_type = MediaType::Audio;
    params.channels = Some(2); // mixed stereo output
    params.sample_rate = Some(OUTPUT_SAMPLE_RATE);
    params.sample_format = Some(SampleFormat::S16);
    params.extradata = blob.clone();

    let stream = StreamInfo {
        index: 0,
        time_base: TimeBase::new(1, OUTPUT_SAMPLE_RATE as i64),
        duration: None, // computed lazily by the decoder
        start_time: Some(0),
        params,
    };

    let metadata = build_metadata(&header);
    // Upper-bound duration estimate at the ProTracker default tempo
    // (speed=6 ticks/row, BPM=125 → 50 ticks/sec). Real songs commonly
    // change tempo via Fxx effects so this is typically a loose upper
    // bound; a true value needs a full playback simulation. Formula:
    //   song_length * 64 rows * 6 ticks / 50 tps.
    let duration_micros: i64 = (header.song_length as i64).saturating_mul(64 * 6 * 1_000_000) / 50;

    Ok(Box::new(ModDemuxer {
        streams: vec![stream],
        blob,
        consumed: false,
        metadata,
        duration_micros,
        _header: header,
    }))
}

fn build_metadata(h: &crate::header::ModHeader) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if !h.title.is_empty() {
        out.push(("title".into(), h.title.clone()));
    }
    // Emit the same key for every sample name so CLI continuation
    // formatting collapses them into one block (matching ffprobe).
    for s in h.samples.iter() {
        if !s.name.is_empty() {
            out.push(("sample".into(), s.name.clone()));
        }
    }
    let n_nonempty_samples = h.samples.iter().filter(|s| s.length > 0).count();
    out.push((
        "extra_info".into(),
        format!(
            "{} patterns, {} channels, {}/{} samples",
            h.n_patterns,
            h.channels,
            n_nonempty_samples,
            h.samples.len()
        ),
    ));
    out
}

struct ModDemuxer {
    streams: Vec<StreamInfo>,
    blob: Vec<u8>,
    consumed: bool,
    metadata: Vec<(String, String)>,
    duration_micros: i64,
    _header: crate::header::ModHeader,
}

impl Demuxer for ModDemuxer {
    fn format_name(&self) -> &str {
        "mod"
    }

    fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }

    fn next_packet(&mut self) -> Result<Packet> {
        if self.consumed {
            return Err(Error::Eof);
        }
        self.consumed = true;
        let data = std::mem::take(&mut self.blob);
        let stream = &self.streams[0];
        let mut pkt = Packet::new(0, stream.time_base, data);
        pkt.pts = Some(0);
        pkt.dts = Some(0);
        pkt.flags.keyframe = true;
        Ok(pkt)
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
