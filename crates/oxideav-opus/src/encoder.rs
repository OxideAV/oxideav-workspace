//! Opus encoder — CELT-only full-band path, 20 ms single-frame packets.
//!
//! # Mode selection
//!
//! This build only emits **CELT-only** frames at the full-band bandwidth
//! (config 31, 20 ms, 48 kHz). Mode selection is therefore trivial:
//!
//! * `CodecParameters::sample_rate == 48_000` → CELT-only FB, 20 ms.
//! * Any other sample rate → `Error::Unsupported` (resample upstream).
//!
//! SILK-only and Hybrid modes are tracked as follow-up — an in-tree SILK
//! encoder is not yet landed, so even at lower sample rates we prefer
//! the honest "resample to 48 kHz first" route over silently mis-
//! classifying the input.
//!
//! # Packet layout (RFC 6716 §3)
//!
//! ```text
//!   [ TOC byte ] [ CELT bitstream bytes ... ]
//! ```
//!
//! where the TOC byte is `(config << 3) | (stereo << 2) | code` with
//! `config = 31`, `stereo ∈ {0, 1}`, `code = 0` (single frame).
//!
//! # Supported inputs
//!
//! * S16 / S16P / F32 / F32P sample formats.
//! * 48 kHz sample rate only.
//! * Mono (channels = 1) — native path.
//! * Stereo (channels = 2) — **downmixed to mono** before being fed to
//!   the mono-only CELT encoder; the TOC is emitted with `stereo = 0`.
//!   A real CELT stereo path (coupled L/R PVQ with intensity /
//!   dual-stereo) would be needed to honestly advertise `stereo = 1`
//!   in the TOC, and the `oxideav-celt` encoder is mono-only today —
//!   see its module docs. The signal survives and decodes cleanly as
//!   duplicated-mono on both channels; per-channel detail is lost.
//!
//! # Unsupported
//!
//! * Framing codes 1/2/3 (multi-frame packets) — not emitted.
//! * 2.5 / 5 / 10 / 40 / 60 ms frame sizes.
//! * SILK-only / Hybrid modes.
//! * More than 2 channels.

use std::collections::VecDeque;

use oxideav_celt::encoder::{CeltEncoder, FRAME_SAMPLES, SAMPLE_RATE};
use oxideav_codec::Encoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, Result, SampleFormat, TimeBase,
};

/// `config` field value for CELT-only, fullband, 20 ms frames.
const OPUS_CONFIG_CELT_FB_20MS: u8 = 31;

/// Build a TOC byte for config 31 (CELT-only FB 20 ms), code-0 (single
/// frame packet), with the given stereo bit.
///
/// Layout (RFC 6716 §3.1): `config(5) | stereo(1) | code(2)`.
pub fn build_toc_byte(stereo: bool) -> u8 {
    let stereo_bit: u8 = if stereo { 1 } else { 0 };
    (OPUS_CONFIG_CELT_FB_20MS << 3) | (stereo_bit << 2) // code = 0 (single frame)
}

/// Number of PCM samples per 20 ms Opus/CELT frame at 48 kHz.
pub const OPUS_FRAME_SAMPLES: usize = 960;

pub struct OpusEncoder {
    /// Output-stream parameters (after any channel-count adjustments).
    out_params: CodecParameters,
    /// Channel count on the *input* frames (1 or 2). Stereo inputs are
    /// downmixed to mono before hitting the CELT encoder.
    input_channels: u16,
    /// The underlying mono CELT encoder.
    celt: CeltEncoder,
    /// Output packet queue (one Opus packet per 20 ms of input).
    output: VecDeque<Packet>,
    /// PTS counter (in 48 kHz samples).
    pts_counter: i64,
}

impl OpusEncoder {
    /// Build a new Opus encoder. Mode selection is purely driven by the
    /// sample rate in `params`: 48 kHz → CELT-only full-band 20 ms. Any
    /// other rate returns `Error::Unsupported`.
    ///
    /// For an explicit, mode-named entry point that keeps the call-site
    /// intent obvious, see [`OpusEncoder::new_celt_only_full_band`].
    pub fn new(params: &CodecParameters) -> Result<Self> {
        let channels = params.channels.unwrap_or(1);
        if channels == 0 || channels > 2 {
            return Err(Error::unsupported(format!(
                "opus encoder: only mono/stereo supported, got {channels}-channel input"
            )));
        }
        let sr = params.sample_rate.unwrap_or(SAMPLE_RATE);
        if sr != SAMPLE_RATE {
            return Err(Error::unsupported(format!(
                "opus encoder: input must be 48 kHz (got {sr}); resample before encoding"
            )));
        }

        // Drive the underlying CELT encoder as mono — stereo input is
        // downmixed on the way in. The CELT-mono path is the only one
        // implemented today.
        let mut celt_params = params.clone();
        celt_params.channels = Some(1);
        celt_params.sample_rate = Some(SAMPLE_RATE);
        // CeltEncoder expects its own codec id; clone the whole parameter
        // block and override the id so the inner encoder doesn't reject
        // us for a mismatch.
        celt_params.codec_id = CodecId::new(oxideav_celt::CODEC_ID_STR);
        let celt = CeltEncoder::new(&celt_params)?;

        // Output params: we report the *input* channel count so that the
        // downstream muxer keeps the packet's implied channel layout in
        // sync with what callers asked for. The bitstream body is always
        // a mono CELT frame though — see module docs.
        let mut out_params = params.clone();
        out_params.sample_rate = Some(SAMPLE_RATE);
        out_params.channels = Some(channels);

        Ok(Self {
            out_params,
            input_channels: channels,
            celt,
            output: VecDeque::new(),
            pts_counter: 0,
        })
    }

    /// Explicit CELT-only full-band (48 kHz, 20 ms) constructor. Equivalent
    /// to [`OpusEncoder::new`] with `params.sample_rate = Some(48_000)`,
    /// but documents the intent at the call site. Returns `Unsupported`
    /// if the caller passed a non-48 kHz rate.
    ///
    /// Channels must be 1 or 2. Stereo input is downmixed to mono — see
    /// the module docs for why.
    pub fn new_celt_only_full_band(params: &CodecParameters) -> Result<Self> {
        let sr = params.sample_rate.unwrap_or(SAMPLE_RATE);
        if sr != SAMPLE_RATE {
            return Err(Error::unsupported(format!(
                "opus encoder (CELT-only FB): input must be 48 kHz, got {sr}"
            )));
        }
        Self::new(params)
    }

    /// Pull all pending CELT packets out of the underlying encoder, wrap
    /// each in an Opus TOC byte, and push the resulting Opus packets to
    /// the output queue.
    fn drain_celt(&mut self) -> Result<()> {
        // CeltEncoder is mono-only so stereo_bit is always 0 here.
        let toc = build_toc_byte(false);
        loop {
            match self.celt.receive_packet() {
                Ok(celt_pkt) => {
                    let mut data = Vec::with_capacity(1 + celt_pkt.data.len());
                    data.push(toc);
                    data.extend_from_slice(&celt_pkt.data);
                    let tb = TimeBase::new(1, SAMPLE_RATE as i64);
                    let pts = self.pts_counter;
                    self.pts_counter += OPUS_FRAME_SAMPLES as i64;
                    let pkt = Packet::new(0, tb, data)
                        .with_pts(pts)
                        .with_duration(OPUS_FRAME_SAMPLES as i64);
                    self.output.push_back(pkt);
                }
                Err(Error::NeedMore) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }
}

impl Encoder for OpusEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.out_params.codec_id
    }

    fn output_params(&self) -> &CodecParameters {
        &self.out_params
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let audio = match frame {
            Frame::Audio(a) => a,
            _ => {
                return Err(Error::invalid(
                    "opus encoder: expected audio frame, got video",
                ))
            }
        };
        if audio.sample_rate != SAMPLE_RATE {
            return Err(Error::unsupported(format!(
                "opus encoder: input must be 48 kHz (got {}); resample before encoding",
                audio.sample_rate
            )));
        }
        if audio.channels != self.input_channels {
            return Err(Error::invalid(format!(
                "opus encoder: frame channels ({}) differ from configured input channels ({})",
                audio.channels, self.input_channels
            )));
        }

        // Flatten the input into a mono f32 buffer regardless of whether
        // the container was mono (passthrough) or stereo (downmix).
        let mono = extract_mono_f32(audio)?;

        // Feed the CELT encoder as a single mono F32 frame.
        let mut bytes = Vec::with_capacity(mono.len() * 4);
        for &s in &mono {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let celt_frame = Frame::Audio(AudioFrame {
            format: SampleFormat::F32,
            channels: 1,
            sample_rate: SAMPLE_RATE,
            samples: mono.len() as u32,
            pts: audio.pts,
            time_base: TimeBase::new(1, SAMPLE_RATE as i64),
            data: vec![bytes],
        });
        self.celt.send_frame(&celt_frame)?;
        self.drain_celt()
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        if let Some(p) = self.output.pop_front() {
            Ok(p)
        } else {
            Err(Error::NeedMore)
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.celt.flush()?;
        self.drain_celt()?;
        Ok(())
    }
}

/// Decode the `AudioFrame`'s sample bytes into a mono f32 buffer, applying
/// a stereo → mono downmix (simple mean) when needed. Supports S16 and
/// F32 (interleaved or planar).
fn extract_mono_f32(audio: &AudioFrame) -> Result<Vec<f32>> {
    let n = audio.samples as usize;
    let ch = audio.channels as usize;
    if ch == 0 {
        return Err(Error::invalid("opus encoder: 0-channel audio frame"));
    }
    let mut out = vec![0f32; n];
    match audio.format {
        SampleFormat::S16 => {
            // Interleaved S16.
            let bytes = &audio.data[0];
            let needed = n * ch * 2;
            if bytes.len() < needed {
                return Err(Error::invalid(
                    "opus encoder: S16 input shorter than declared sample count",
                ));
            }
            for i in 0..n {
                let mut acc = 0i32;
                for c in 0..ch {
                    let off = (i * ch + c) * 2;
                    let s = i16::from_le_bytes([bytes[off], bytes[off + 1]]);
                    acc += s as i32;
                }
                out[i] = (acc as f32) / (ch as f32 * 32768.0);
            }
        }
        SampleFormat::S16P => {
            // One plane per channel. Mono = plane 0, stereo = two planes.
            if audio.data.len() < ch {
                return Err(Error::invalid("opus encoder: S16P input missing planes"));
            }
            for i in 0..n {
                let mut acc = 0i32;
                for c in 0..ch {
                    let plane = &audio.data[c];
                    if plane.len() < n * 2 {
                        return Err(Error::invalid(
                            "opus encoder: S16P plane shorter than declared sample count",
                        ));
                    }
                    let off = i * 2;
                    let s = i16::from_le_bytes([plane[off], plane[off + 1]]);
                    acc += s as i32;
                }
                out[i] = (acc as f32) / (ch as f32 * 32768.0);
            }
        }
        SampleFormat::F32 => {
            let bytes = &audio.data[0];
            let needed = n * ch * 4;
            if bytes.len() < needed {
                return Err(Error::invalid(
                    "opus encoder: F32 input shorter than declared sample count",
                ));
            }
            for i in 0..n {
                let mut acc = 0f32;
                for c in 0..ch {
                    let off = (i * ch + c) * 4;
                    acc += f32::from_le_bytes([
                        bytes[off],
                        bytes[off + 1],
                        bytes[off + 2],
                        bytes[off + 3],
                    ]);
                }
                out[i] = acc / ch as f32;
            }
        }
        SampleFormat::F32P => {
            if audio.data.len() < ch {
                return Err(Error::invalid("opus encoder: F32P input missing planes"));
            }
            for i in 0..n {
                let mut acc = 0f32;
                for c in 0..ch {
                    let plane = &audio.data[c];
                    if plane.len() < n * 4 {
                        return Err(Error::invalid(
                            "opus encoder: F32P plane shorter than declared sample count",
                        ));
                    }
                    let off = i * 4;
                    acc += f32::from_le_bytes([
                        plane[off],
                        plane[off + 1],
                        plane[off + 2],
                        plane[off + 3],
                    ]);
                }
                out[i] = acc / ch as f32;
            }
        }
        other => {
            return Err(Error::unsupported(format!(
                "opus encoder: sample format {:?} not supported (use S16 / S16P / F32 / F32P)",
                other
            )));
        }
    }
    // Sanity: the CELT encoder always consumes `FRAME_SAMPLES` (960) per
    // frame. We don't enforce `n == FRAME_SAMPLES` here because the
    // underlying CELT encoder buffers up to a frame boundary internally
    // — but we do surface any non-20-ms chunking downstream as Unsupported
    // there. The caller is free to send any number of samples per frame
    // as long as the aggregate ends on a frame boundary before `flush()`.
    let _ = FRAME_SAMPLES;
    Ok(out)
}

pub fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    Ok(Box::new(OpusEncoder::new(params)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_byte_mono() {
        let b = build_toc_byte(false);
        assert_eq!(b >> 3, 31, "config should be 31");
        assert_eq!((b >> 2) & 1, 0, "stereo bit should be 0");
        assert_eq!(b & 0x3, 0, "framing code should be 0");
    }

    #[test]
    fn toc_byte_stereo() {
        let b = build_toc_byte(true);
        assert_eq!(b >> 3, 31, "config should be 31");
        assert_eq!((b >> 2) & 1, 1, "stereo bit should be 1");
        assert_eq!(b & 0x3, 0, "framing code should be 0");
    }

    #[test]
    fn rejects_non_48k() {
        let mut p = CodecParameters::audio(CodecId::new("opus"));
        p.channels = Some(1);
        p.sample_rate = Some(44_100);
        match OpusEncoder::new(&p) {
            Err(Error::Unsupported(_)) => {}
            Err(e) => panic!("expected Unsupported, got {e:?}"),
            Ok(_) => panic!("expected Unsupported, got Ok"),
        }
    }

    #[test]
    fn rejects_more_than_stereo() {
        let mut p = CodecParameters::audio(CodecId::new("opus"));
        p.channels = Some(6);
        p.sample_rate = Some(SAMPLE_RATE);
        match OpusEncoder::new(&p) {
            Err(Error::Unsupported(_)) => {}
            Err(e) => panic!("expected Unsupported, got {e:?}"),
            Ok(_) => panic!("expected Unsupported, got Ok"),
        }
    }

    #[test]
    fn new_celt_only_fb_accepts_48k_mono() {
        let mut p = CodecParameters::audio(CodecId::new("opus"));
        p.channels = Some(1);
        p.sample_rate = Some(SAMPLE_RATE);
        assert!(OpusEncoder::new_celt_only_full_band(&p).is_ok());
    }

    #[test]
    fn new_celt_only_fb_rejects_non_48k() {
        let mut p = CodecParameters::audio(CodecId::new("opus"));
        p.channels = Some(1);
        p.sample_rate = Some(16_000);
        match OpusEncoder::new_celt_only_full_band(&p) {
            Err(Error::Unsupported(_)) => {}
            Err(e) => panic!("expected Unsupported, got {e:?}"),
            Ok(_) => panic!("expected Unsupported, got Ok"),
        }
    }

    #[test]
    fn mono_encoder_produces_toc_byte() {
        let mut p = CodecParameters::audio(CodecId::new("opus"));
        p.channels = Some(1);
        p.sample_rate = Some(SAMPLE_RATE);
        let mut enc = OpusEncoder::new(&p).unwrap();
        // Feed one frame of silence.
        let bytes = vec![0u8; OPUS_FRAME_SAMPLES * 2];
        let frame = Frame::Audio(AudioFrame {
            format: SampleFormat::S16,
            channels: 1,
            sample_rate: SAMPLE_RATE,
            samples: OPUS_FRAME_SAMPLES as u32,
            pts: None,
            time_base: TimeBase::new(1, SAMPLE_RATE as i64),
            data: vec![bytes],
        });
        enc.send_frame(&frame).unwrap();
        let pkt = enc.receive_packet().unwrap();
        assert!(!pkt.data.is_empty(), "packet must contain TOC + bitstream");
        let toc = pkt.data[0];
        assert_eq!(toc >> 3, 31, "config should be 31");
        assert_eq!((toc >> 2) & 1, 0, "mono → stereo bit = 0");
        assert_eq!(toc & 0x3, 0, "single-frame packet → code 0");
    }
}
