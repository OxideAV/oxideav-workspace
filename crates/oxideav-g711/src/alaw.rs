//! A-law (ITU-T G.711 §2) codec — single-sample conversion helpers plus
//! [`AlawDecoder`] / [`AlawEncoder`] implementing the `oxideav_codec`
//! traits. Each encoded byte carries exactly one S16 PCM sample.

use oxideav_codec::{Decoder, Encoder};
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, MediaType, Packet, Result, SampleFormat,
    TimeBase,
};
use std::collections::VecDeque;

use crate::tables::{ALAW_DECODE, ALAW_XOR};

/// Decode one A-law byte to a linear S16 sample.
#[inline]
pub fn decode_sample(byte: u8) -> i16 {
    ALAW_DECODE[byte as usize]
}

/// Encode one S16 sample as an A-law byte (ITU-T G.711 §2, arithmetic
/// form).
///
/// Algorithm (mirror of [`crate::tables::alaw_decode`]):
///
/// 1. Extract sign; work with absolute magnitude (clamped to the largest
///    representable A-law level = 32256).
/// 2. Find the segment — the position of the topmost set bit of the
///    magnitude. Segments 1..=7 correspond to the magnitude falling into
///    the ranges `[256..512), [512..1024), ..., [16384..32768)`.
/// 3. Extract the 4-bit mantissa from the bits immediately below the
///    segment bit.
/// 4. Compose S|E|M, then XOR with 0x55 for the on-wire alternate-bit
///    inversion.
#[inline]
pub fn encode_sample(sample: i16) -> u8 {
    let mut mag: i32 = sample as i32;
    // A-law sign convention: bit 7 set ⇒ positive, clear ⇒ negative.
    let sign_bit: u8 = if mag < 0 { 0x00 } else { 0x80 };
    if mag < 0 {
        // i16::MIN as i32 is -32768; -(-32768) = 32768 fits in i32.
        mag = -mag;
    }
    // Largest representable A-law linear value is 32256 (segment 7,
    // mantissa 15). Anything above saturates.
    if mag > 32256 {
        mag = 32256;
    }

    let (seg, mant): (u32, u32) = if mag < 256 {
        // Segment 0: mantissa = bits 4..7 of the magnitude.
        (0, (mag >> 4) as u32 & 0x0F)
    } else {
        // Segments 1..=7: find position of topmost set bit. With mag
        // in [256, 32256], the top bit is between position 8 and 14, so
        // segment = top_bit_position − 7. Mantissa = bits just below
        // the top bit.
        let mut seg = 1u32;
        let mut threshold: i32 = 512;
        while seg < 7 && mag >= threshold {
            seg += 1;
            threshold <<= 1;
        }
        let shift = seg + 3; // = top_bit_pos - 4
        let m = ((mag >> shift) & 0x0F) as u32;
        (seg, m)
    };

    let byte = sign_bit | ((seg as u8) << 4) | (mant as u8);
    byte ^ ALAW_XOR
}

// -------------- decoder --------------

pub(crate) fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    let channels = params.channels.unwrap_or(1);
    if channels != 1 {
        return Err(Error::unsupported(format!(
            "G.711 A-law decoder: only mono is supported (got {channels} channels)"
        )));
    }
    let sample_rate = params.sample_rate.unwrap_or(8_000);
    Ok(Box::new(AlawDecoder {
        codec_id: params.codec_id.clone(),
        sample_rate,
        time_base: TimeBase::new(1, sample_rate as i64),
        pending: None,
        eof: false,
    }))
}

pub struct AlawDecoder {
    codec_id: CodecId,
    sample_rate: u32,
    time_base: TimeBase,
    pending: Option<Packet>,
    eof: bool,
}

impl Decoder for AlawDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.pending.is_some() {
            return Err(Error::other(
                "G.711 A-law decoder: call receive_frame before sending another packet",
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
        if pkt.data.is_empty() {
            return Ok(Frame::Audio(AudioFrame {
                format: SampleFormat::S16,
                channels: 1,
                sample_rate: self.sample_rate,
                samples: 0,
                pts: pkt.pts,
                time_base: self.time_base,
                data: vec![Vec::new()],
            }));
        }
        let samples = pkt.data.len();
        let mut out = Vec::with_capacity(samples * 2);
        for &b in &pkt.data {
            let s = decode_sample(b);
            out.extend_from_slice(&s.to_le_bytes());
        }
        Ok(Frame::Audio(AudioFrame {
            format: SampleFormat::S16,
            channels: 1,
            sample_rate: self.sample_rate,
            samples: samples as u32,
            pts: pkt.pts,
            time_base: self.time_base,
            data: vec![out],
        }))
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

// -------------- encoder --------------

pub(crate) fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    let channels = params.channels.unwrap_or(1);
    if channels != 1 {
        return Err(Error::unsupported(format!(
            "G.711 A-law encoder: only mono is supported (got {channels} channels)"
        )));
    }
    let sample_format = params.sample_format.unwrap_or(SampleFormat::S16);
    if sample_format != SampleFormat::S16 {
        return Err(Error::unsupported(format!(
            "G.711 A-law encoder: input sample format {sample_format:?} not supported (need S16)"
        )));
    }
    let sample_rate = params.sample_rate.unwrap_or(8_000);
    let mut output = params.clone();
    output.media_type = MediaType::Audio;
    output.sample_format = Some(SampleFormat::S16);
    output.channels = Some(1);
    output.sample_rate = Some(sample_rate);
    output.codec_id = params.codec_id.clone();
    Ok(Box::new(AlawEncoder {
        output,
        time_base: TimeBase::new(1, sample_rate as i64),
        queue: VecDeque::new(),
    }))
}

pub struct AlawEncoder {
    output: CodecParameters,
    time_base: TimeBase,
    queue: VecDeque<Packet>,
}

impl Encoder for AlawEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.output.codec_id
    }

    fn output_params(&self) -> &CodecParameters {
        &self.output
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let Frame::Audio(a) = frame else {
            return Err(Error::invalid("G.711 A-law encoder: audio frames only"));
        };
        if a.channels != 1 {
            return Err(Error::invalid("G.711 A-law encoder: mono only"));
        }
        if a.format != SampleFormat::S16 {
            return Err(Error::invalid("G.711 A-law encoder: S16 input required"));
        }
        let bytes = a
            .data
            .first()
            .ok_or_else(|| Error::invalid("G.711 A-law encoder: empty frame"))?;
        if bytes.len() % 2 != 0 {
            return Err(Error::invalid("G.711 A-law encoder: odd byte count"));
        }
        let mut out = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.chunks_exact(2) {
            let s = i16::from_le_bytes([chunk[0], chunk[1]]);
            out.push(encode_sample(s));
        }
        let tb = if a.time_base.0.num == 0 {
            self.time_base
        } else {
            a.time_base
        };
        let mut pkt = Packet::new(0, tb, out);
        pkt.pts = a.pts;
        pkt.dts = a.pts;
        pkt.duration = Some(a.samples as i64);
        pkt.flags.keyframe = true;
        self.queue.push_back(pkt);
        Ok(())
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        self.queue.pop_front().ok_or(Error::NeedMore)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
