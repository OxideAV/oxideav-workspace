//! Pipeline composition.
//!
//! Two pipelines today:
//!
//! - [`remux`] — copy packets from a demuxer into a muxer with no decoding.
//! - [`transcode_simple`] — single-stream demux → decode → encode → mux.
//!
//! A graph-based / multi-stream / filter-aware pipeline will follow as the
//! codec catalog grows. For now `transcode_simple` covers the common "rip
//! audio to PCM" case (FLAC → WAV, etc.).

use oxideav_codec::CodecRegistry;
use oxideav_container::{Demuxer, Muxer};
use oxideav_core::{CodecParameters, Error, Frame, MediaType, Result, StreamInfo, TimeBase};

/// Copy all packets from `demuxer` into `muxer` without re-encoding.
pub fn remux(demuxer: &mut dyn Demuxer, muxer: &mut dyn Muxer) -> Result<u64> {
    muxer.write_header()?;
    let mut packets = 0u64;
    loop {
        match demuxer.next_packet() {
            Ok(pkt) => {
                muxer.write_packet(&pkt)?;
                packets += 1;
            }
            Err(Error::Eof) => break,
            Err(e) => return Err(e),
        }
    }
    muxer.write_trailer()?;
    Ok(packets)
}

/// Plan describing how to derive the output stream from the input stream.
#[derive(Clone, Debug)]
pub enum StreamPlan {
    /// Stream-copy (codec passthrough, no decode).
    Copy,
    /// Decode then re-encode using the named codec. The encoder is constructed
    /// with parameters carried over from the input (sample rate, channels…)
    /// and the chosen codec id.
    Reencode { output_codec: String },
}

/// Single-input single-output transcode (or copy).
///
/// `make_output_streams` lets the caller customise the output StreamInfo
/// (e.g. choose container-specific time bases) before the muxer is opened.
/// For the common case where the output stream layout matches the encoder's
/// declared parameters, just clone & adapt.
pub fn transcode_simple(
    demuxer: &mut dyn Demuxer,
    muxer_open: impl FnOnce(&[StreamInfo]) -> Result<Box<dyn Muxer>>,
    codecs: &CodecRegistry,
    plan: &StreamPlan,
) -> Result<TranscodeStats> {
    let in_streams = demuxer.streams().to_vec();
    if in_streams.len() != 1 {
        return Err(Error::unsupported(
            "transcode_simple only handles single-stream inputs today",
        ));
    }
    let in_stream = &in_streams[0];

    match plan {
        StreamPlan::Copy => {
            let mut muxer = muxer_open(&in_streams)?;
            let n = remux(demuxer, &mut *muxer)?;
            Ok(TranscodeStats {
                packets_in: n,
                packets_out: n,
                frames_decoded: 0,
            })
        }
        StreamPlan::Reencode { output_codec } => {
            // Build a decoder from input parameters.
            let mut decoder = codecs.make_decoder(&in_stream.params)?;

            // Build encoder parameters from the input stream's audio
            // properties + the requested codec id.
            let mut enc_params = CodecParameters::audio(output_codec.as_str().into());
            enc_params.media_type = MediaType::Audio;
            enc_params.sample_rate = in_stream.params.sample_rate;
            enc_params.channels = in_stream.params.channels;
            enc_params.sample_format = in_stream.params.sample_format;
            let mut encoder = codecs.make_encoder(&enc_params)?;
            let out_params = encoder.output_params().clone();

            let out_time_base = match out_params.sample_rate {
                Some(sr) if sr > 0 => TimeBase::new(1, sr as i64),
                _ => in_stream.time_base,
            };
            let out_stream = StreamInfo {
                index: 0,
                time_base: out_time_base,
                duration: in_stream.duration,
                start_time: Some(0),
                params: out_params,
            };
            let mut muxer = muxer_open(std::slice::from_ref(&out_stream))?;
            muxer.write_header()?;

            let mut stats = TranscodeStats::default();

            // Drive the decode→encode loop.
            'outer: loop {
                match demuxer.next_packet() {
                    Ok(pkt) => {
                        stats.packets_in += 1;
                        decoder.send_packet(&pkt)?;
                        loop {
                            match decoder.receive_frame() {
                                Ok(frame) => {
                                    stats.frames_decoded += 1;
                                    let frame = adapt_frame_for_encoder(frame, &out_stream)?;
                                    encoder.send_frame(&frame)?;
                                    drain_encoder(&mut *encoder, &mut *muxer, &mut stats)?;
                                }
                                Err(Error::NeedMore) => break,
                                Err(Error::Eof) => break 'outer,
                                Err(e) => return Err(e),
                            }
                        }
                    }
                    Err(Error::Eof) => {
                        decoder.flush()?;
                        // Drain remaining frames.
                        loop {
                            match decoder.receive_frame() {
                                Ok(frame) => {
                                    stats.frames_decoded += 1;
                                    let frame = adapt_frame_for_encoder(frame, &out_stream)?;
                                    encoder.send_frame(&frame)?;
                                    drain_encoder(&mut *encoder, &mut *muxer, &mut stats)?;
                                }
                                Err(Error::NeedMore) | Err(Error::Eof) => break,
                                Err(e) => return Err(e),
                            }
                        }
                        encoder.flush()?;
                        drain_encoder(&mut *encoder, &mut *muxer, &mut stats)?;
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }

            muxer.write_trailer()?;
            Ok(stats)
        }
    }
}

fn drain_encoder(
    encoder: &mut dyn oxideav_codec::Encoder,
    muxer: &mut dyn Muxer,
    stats: &mut TranscodeStats,
) -> Result<()> {
    loop {
        match encoder.receive_packet() {
            Ok(pkt) => {
                muxer.write_packet(&pkt)?;
                stats.packets_out += 1;
            }
            Err(Error::NeedMore) | Err(Error::Eof) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

/// If the decoder's output frame doesn't quite match the encoder's input
/// expectations (e.g. time_base differs by container choice), adapt minimal
/// fields here. Sample-format conversion is **not** performed yet — the
/// caller must wire compatible decoder and encoder formats.
fn adapt_frame_for_encoder(frame: Frame, out_stream: &StreamInfo) -> Result<Frame> {
    let time_base = out_stream.time_base;
    Ok(match frame {
        Frame::Audio(mut a) => {
            a.time_base = time_base;
            Frame::Audio(a)
        }
        Frame::Video(mut v) => {
            v.time_base = time_base;
            Frame::Video(v)
        }
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TranscodeStats {
    pub packets_in: u64,
    pub packets_out: u64,
    pub frames_decoded: u64,
}

// Compatibility re-export: keep the old `transcode` symbol for now (returns Unsupported).
#[deprecated(note = "use transcode_simple instead")]
pub fn transcode(
    _demuxer: &mut dyn Demuxer,
    _muxer: &mut dyn Muxer,
    _codecs: &CodecRegistry,
    _plans: &[StreamPlan],
) -> Result<()> {
    Err(Error::unsupported("use transcode_simple"))
}
