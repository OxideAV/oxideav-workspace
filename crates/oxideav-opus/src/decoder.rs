//! Opus decoder — pragmatic first landing.
//!
//! This crate is on the path to a full RFC 6716 decoder. Landing an entire
//! CELT + SILK + Hybrid decoder is a multi-thousand-line endeavour that
//! spans pyramid vector quantisation, inverse MDCT, adaptive codebooks,
//! linear prediction, long-term prediction, and a small library of
//! precomputed probability tables from Appendix A. This initial cut
//! delivers the surrounding framing layer correctly, plus whatever
//! decoding we can cleanly ship today; everything else returns a
//! descriptive `Unsupported` error.
//!
//! What's handled end-to-end:
//!
//! 1. **TOC parsing** (RFC 6716 §3.1) — mode, bandwidth, frame duration,
//!    stereo flag, framing code.
//! 2. **Framing codes 0/1/2/3** — the packet is split into per-frame byte
//!    slices.
//! 3. **Silence / DTX frames** — an Opus frame of 0 or 1 bytes carries no
//!    coded audio and is treated as silence per RFC 6716. We emit a proper
//!    `AudioFrame` full of zeros for the expected duration.
//! 4. **CELT silence flag** — when a CELT-only frame is ≥ 2 bytes but
//!    its very first range-coded symbol is the silence flag, we emit
//!    silence for that frame's duration.
//! 5. **CELT frame header** — silence + post-filter (octave/period/gain/
//!    tapset) + transient + intra flags are all parsed (RFC 6716 §4.3,
//!    Table 56). The header values are validated by the crate-level
//!    integration test, even though we don't yet act on them.
//! 6. **Mode rejection** — SILK-only and Hybrid frames return
//!    `Error::Unsupported` cleanly. CELT frames whose silence flag is
//!    not set return `Error::Unsupported` with a message that names the
//!    next missing stage by RFC §ref so a follow-up agent knows
//!    exactly where to land work next.

use oxideav_celt::header::decode_header;
use oxideav_celt::quant_bands::unquant_coarse_energy;
use oxideav_celt::range_decoder::RangeDecoder;
use oxideav_celt::tables::{end_band_for_bandwidth_celt, lm_for_frame_samples, NB_EBANDS};
use oxideav_codec::Decoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, Result, SampleFormat, TimeBase,
};

use crate::toc::{OpusMode, Toc};

/// Opus always decodes at 48 kHz regardless of what the original encoder
/// saw at its input.
pub const OPUS_RATE_HZ: u32 = 48_000;

/// Build an Opus decoder from the codec parameters.
///
/// `params.extradata`, if present, should be the `OpusHead` identification
/// packet (19+ bytes starting with `"OpusHead"`). When absent, the decoder
/// defaults to mono unless the `params.channels` field overrides.
pub fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    let channels = params.channels.unwrap_or(1).max(1);
    if channels > 2 {
        // Multi-stream Opus (channel mapping family 1 or 2) multiplexes
        // several stereo/mono Opus streams together — not in scope for
        // this first landing.
        return Err(Error::unsupported(
            "Opus multi-stream (channel mapping family 1/2) not yet supported",
        ));
    }
    Ok(Box::new(OpusDecoder {
        codec_id: params.codec_id.clone(),
        channels,
        time_base: TimeBase::new(1, OPUS_RATE_HZ as i64),
        pending: None,
        eof: false,
        emit_pts: 0,
    }))
}

struct OpusDecoder {
    codec_id: CodecId,
    channels: u16,
    time_base: TimeBase,
    pending: Option<Packet>,
    eof: bool,
    emit_pts: i64,
}

impl Decoder for OpusDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.pending.is_some() {
            return Err(Error::other(
                "Opus decoder: receive_frame must be called before sending another packet",
            ));
        }
        self.pending = Some(packet.clone());
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        let Some(pkt) = self.pending.take() else {
            return if self.eof {
                Err(Error::Eof)
            } else {
                Err(Error::NeedMore)
            };
        };
        decode_packet(self, &pkt)
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

fn decode_packet(dec: &mut OpusDecoder, packet: &Packet) -> Result<Frame> {
    let parsed = crate::toc::parse_packet(&packet.data)?;
    // For now we coalesce all frames within a packet into a single AudioFrame
    // so callers see one frame per packet (matching the way Vorbis and other
    // audio decoders in this workspace behave). Total sample count is
    // `n_frames * frame_samples_48k`.
    let n_frames = parsed.frames.len();
    let per_frame = parsed.toc.frame_samples_48k as usize;
    let total_samples = per_frame * n_frames;

    // Validate stereo consistency against header when known.
    let toc_ch = parsed.toc.channels();
    if toc_ch > dec.channels {
        // The container said mono but the TOC says stereo — be permissive:
        // follow the TOC since that's what the bitstream actually contains.
    }
    let out_channels = dec.channels.max(toc_ch);

    // Each frame is decoded (or stubbed to silence) independently. We
    // then concatenate per-channel PCM.
    let mut per_ch: Vec<Vec<f32>> = (0..out_channels)
        .map(|_| Vec::with_capacity(total_samples))
        .collect();

    for frame_bytes in parsed.frames.iter() {
        let mut ch_buf = decode_frame(&parsed.toc, frame_bytes, out_channels as usize)?;
        for (dst, src) in per_ch.iter_mut().zip(ch_buf.drain(..)) {
            dst.extend_from_slice(&src);
        }
    }

    // Pack interleaved S16.
    let mut interleaved = Vec::with_capacity(total_samples * out_channels as usize * 2);
    for i in 0..total_samples {
        for ch_buf in per_ch.iter().take(out_channels as usize) {
            let s = ch_buf.get(i).copied().unwrap_or(0.0);
            let clamped = (s * 32768.0).clamp(-32768.0, 32767.0) as i16;
            interleaved.extend_from_slice(&clamped.to_le_bytes());
        }
    }

    let pts = packet.pts.unwrap_or(dec.emit_pts);
    dec.emit_pts = pts + total_samples as i64;

    Ok(Frame::Audio(AudioFrame {
        format: SampleFormat::S16,
        channels: out_channels,
        sample_rate: OPUS_RATE_HZ,
        samples: total_samples as u32,
        pts: Some(pts),
        time_base: dec.time_base,
        data: vec![interleaved],
    }))
}

/// Decode one Opus frame. Returns per-channel f32 samples in the range
/// `[-1.0, 1.0]`. Errors with `Unsupported` when we hit a mode we can't
/// produce yet, which the caller propagates without crashing.
fn decode_frame(toc: &Toc, bytes: &[u8], channels: usize) -> Result<Vec<Vec<f32>>> {
    let n_samples = toc.frame_samples_48k as usize;

    // RFC 6716 §3: "Any frame whose size is 1 byte or less is considered to
    // be a packet loss concealment / DTX frame and is treated as silence."
    // The concrete rule here is "zero coded audio" — a legitimate bitstream
    // short-hand the encoder uses to mark discontinuous transmission.
    if bytes.len() <= 1 {
        return Ok(silence(channels, n_samples));
    }

    match toc.mode {
        OpusMode::CeltOnly => decode_celt_frame(toc, bytes, channels, n_samples),
        OpusMode::SilkOnly => Err(Error::unsupported(
            "Opus SILK-only frames not yet: CELT-only + silence frames supported",
        )),
        OpusMode::Hybrid => Err(Error::unsupported(
            "Opus Hybrid frames not yet: CELT-only + silence frames supported",
        )),
    }
}

/// CELT frame decoder — partial implementation.
///
/// Currently lands the front-of-frame range-coded header symbols
/// (silence / post-filter / transient / intra) per RFC 6716 §4.3, Table 56.
/// The remaining stages (coarse + fine band energy, bit allocation, PVQ
/// shape decode, anti-collapse, inverse MDCT, post-filter convolution)
/// return `Unsupported` with a message identifying the next missing
/// stage by its specific RFC §ref.
fn decode_celt_frame(
    toc: &Toc,
    bytes: &[u8],
    channels: usize,
    n_samples: usize,
) -> Result<Vec<Vec<f32>>> {
    let mut rc = RangeDecoder::new(bytes);
    // Parse silence + post-filter + transient + intra. `None` means the
    // silence flag was set: emit a frame of zeros.
    let header = match decode_header(&mut rc) {
        Some(h) => h,
        None => return Ok(silence_inner(channels, n_samples)),
    };

    // RFC 6716 §4.3.2.1: coarse band energy decode.
    let lm = lm_for_frame_samples(toc.frame_samples_48k) as usize;
    let end_band = end_band_for_bandwidth_celt(toc.bandwidth.cutoff_hz());
    let mut old_e_bands = vec![0.0f32; NB_EBANDS * channels];
    unquant_coarse_energy(
        &mut rc,
        &mut old_e_bands,
        0,
        end_band,
        header.intra,
        channels,
        lm,
    );
    let _ = (header, old_e_bands);

    Err(Error::unsupported(
        "Opus CELT decode incomplete: §4.3.2.1 coarse energy decoded, but \
         §4.3.3 bit allocation + §4.3.2.2 fine energy + §4.3.4 PVQ shape + \
         §4.3.5 anti-collapse + §4.3.7 IMDCT + §4.3.8 post-filter not yet implemented",
    ))
}

fn silence(channels: usize, n_samples: usize) -> Vec<Vec<f32>> {
    silence_inner(channels, n_samples)
}

fn silence_inner(channels: usize, n_samples: usize) -> Vec<Vec<f32>> {
    (0..channels).map(|_| vec![0.0; n_samples]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toc::{OpusBandwidth, OpusMode};
    use oxideav_core::{CodecId, MediaType};

    fn celt_toc() -> Toc {
        // CELT 20 ms stereo fullband.
        Toc {
            config: 31,
            mode: OpusMode::CeltOnly,
            bandwidth: OpusBandwidth::Fullband,
            frame_samples_48k: 960,
            stereo: true,
            code: 0,
        }
    }

    #[test]
    fn short_frame_returns_silence() {
        let toc = celt_toc();
        let out = decode_frame(&toc, &[], 2).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].len(), 960);
        assert!(out[0].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn silk_frame_is_unsupported_not_panic() {
        let toc = Toc {
            config: 0,
            mode: OpusMode::SilkOnly,
            bandwidth: OpusBandwidth::Narrowband,
            frame_samples_48k: 480,
            stereo: false,
            code: 0,
        };
        // A 3-byte non-trivial frame should return Unsupported, not crash.
        let err = decode_frame(&toc, &[0xAA, 0xBB, 0xCC], 1).unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)));
    }

    #[test]
    fn hybrid_frame_is_unsupported_not_panic() {
        let toc = Toc {
            config: 15,
            mode: OpusMode::Hybrid,
            bandwidth: OpusBandwidth::Fullband,
            frame_samples_48k: 960,
            stereo: true,
            code: 0,
        };
        let err = decode_frame(&toc, &[0xAA, 0xBB, 0xCC], 2).unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)));
    }

    #[test]
    fn make_decoder_mono() {
        let mut p = CodecParameters::audio(CodecId::new("opus"));
        p.channels = Some(1);
        let d = make_decoder(&p).unwrap();
        assert_eq!(d.codec_id().as_str(), "opus");
    }

    #[test]
    fn make_decoder_rejects_multistream() {
        let mut p = CodecParameters::audio(CodecId::new("opus"));
        p.channels = Some(6);
        match make_decoder(&p) {
            Err(Error::Unsupported(_)) => {}
            _ => panic!("expected Unsupported"),
        }
    }

    #[test]
    fn receive_frame_silence_packet() {
        // A TOC byte pointing at CELT 20 ms stereo, code-0, with a 0-byte
        // (silence) body.
        let mut p = CodecParameters::audio(CodecId::new("opus"));
        p.channels = Some(2);
        let mut dec = make_decoder(&p).unwrap();
        let pkt = Packet::new(
            0,
            TimeBase::new(1, 48_000),
            vec![(31u8 << 3) | (1 << 2)], // CELT FB 20ms stereo, code=0
        );
        dec.send_packet(&pkt).unwrap();
        let f = dec.receive_frame().unwrap();
        match f {
            Frame::Audio(a) => {
                assert_eq!(a.samples, 960);
                assert_eq!(a.channels, 2);
                assert_eq!(a.sample_rate, 48_000);
                assert_eq!(a.format, SampleFormat::S16);
                // All samples silent.
                let s16_bytes = &a.data[0];
                assert!(s16_bytes.chunks(2).all(|c| c[0] == 0 && c[1] == 0));
            }
            _ => panic!("expected AudioFrame"),
        }
        // Silence parameters shouldn't vary by container.
        let _ = MediaType::Audio;
    }
}
