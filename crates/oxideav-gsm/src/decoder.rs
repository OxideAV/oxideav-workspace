//! `oxideav_codec::Decoder` implementation for GSM 06.10 Full Rate.

use oxideav_codec::Decoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, Result, SampleFormat, TimeBase,
};

use crate::frame::{parse_frame, parse_ms_pair, FRAME_SIZE, MS_FRAME_SIZE};
use crate::synthesis::SynthesisState;

/// Codec IDs handled by this decoder.
pub const CODEC_ID_STANDARD: &str = "gsm";
pub const CODEC_ID_MS: &str = "gsm_ms";

pub fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    let sample_rate = params.sample_rate.unwrap_or(8_000);
    if sample_rate != 8_000 {
        return Err(Error::unsupported(format!(
            "GSM 06.10 decoder: only 8000 Hz is supported (got {sample_rate})"
        )));
    }
    let channels = params.channels.unwrap_or(1);
    if channels != 1 {
        return Err(Error::unsupported(format!(
            "GSM 06.10 decoder: only mono is supported (got {channels} channels)"
        )));
    }
    let variant = match params.codec_id.as_str() {
        CODEC_ID_STANDARD => Variant::Standard,
        CODEC_ID_MS => Variant::Microsoft,
        other => {
            return Err(Error::unsupported(format!(
                "GSM decoder: unknown codec id {other:?}"
            )))
        }
    };
    Ok(Box::new(GsmDecoder {
        codec_id: params.codec_id.clone(),
        variant,
        state: SynthesisState::new(),
        pending: None,
        ms_second: None,
        eof: false,
        time_base: TimeBase::new(1, sample_rate as i64),
    }))
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Variant {
    /// 33-byte payloads, each carries one 20 ms frame (160 samples).
    Standard,
    /// 65-byte payloads, each carries two 20 ms frames back-to-back.
    Microsoft,
}

struct GsmDecoder {
    codec_id: CodecId,
    variant: Variant,
    state: SynthesisState,
    /// Buffered input packet awaiting `receive_frame`.
    pending: Option<Packet>,
    /// Carry for MS framing: the second frame of the most-recently-received
    /// 65-byte packet (produced on the second `receive_frame` call).
    ms_second: Option<(crate::frame::GsmFrame, Option<i64>)>,
    eof: bool,
    time_base: TimeBase,
}

impl Decoder for GsmDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.pending.is_some() || self.ms_second.is_some() {
            return Err(Error::other(
                "GSM decoder: receive_frame must drain previous packet first",
            ));
        }
        self.pending = Some(packet.clone());
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        // Drain a buffered MS-second frame first.
        if let Some((gf, pts)) = self.ms_second.take() {
            let pcm = self.state.decode_frame(&gf);
            return Ok(pcm_to_audio_frame(&pcm, pts, self.time_base));
        }
        let Some(pkt) = self.pending.take() else {
            return if self.eof {
                Err(Error::Eof)
            } else {
                Err(Error::NeedMore)
            };
        };
        match self.variant {
            Variant::Standard => {
                if pkt.data.len() != FRAME_SIZE {
                    return Err(Error::invalid(format!(
                        "GSM: expected {FRAME_SIZE}-byte packet, got {}",
                        pkt.data.len()
                    )));
                }
                let gf = parse_frame(&pkt.data)?;
                let pcm = self.state.decode_frame(&gf);
                Ok(pcm_to_audio_frame(&pcm, pkt.pts, self.time_base))
            }
            Variant::Microsoft => {
                if pkt.data.len() != MS_FRAME_SIZE {
                    return Err(Error::invalid(format!(
                        "GSM-MS: expected {MS_FRAME_SIZE}-byte packet, got {}",
                        pkt.data.len()
                    )));
                }
                let [g0, g1] = parse_ms_pair(&pkt.data)?;
                let pcm = self.state.decode_frame(&g0);
                // The second half gets the pts + 160 samples if both pts and
                // time_base were in 1/sr units. Otherwise we carry unknown.
                let pts2 = pkt.pts.map(|p| p + 160);
                self.ms_second = Some((g1, pts2));
                Ok(pcm_to_audio_frame(&pcm, pkt.pts, self.time_base))
            }
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

fn pcm_to_audio_frame(samples: &[i16; 160], pts: Option<i64>, time_base: TimeBase) -> Frame {
    let mut bytes = Vec::with_capacity(160 * 2);
    for &s in samples.iter() {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    Frame::Audio(AudioFrame {
        format: SampleFormat::S16,
        channels: 1,
        sample_rate: 8_000,
        samples: 160,
        pts,
        time_base,
        data: vec![bytes],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitreader::BitWriter;
    use crate::frame::{GsmFrame, SubFrame};

    fn pack_standard(frame: &GsmFrame) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write(0xD, 4);
        const LAR_BITS: [u32; 8] = [6, 6, 5, 5, 4, 4, 3, 3];
        for i in 0..8 {
            w.write(frame.larc[i] as u16, LAR_BITS[i]);
        }
        for s in &frame.sub {
            w.write(s.nc as u16, 7);
            w.write(s.bc as u16, 2);
            w.write(s.mc as u16, 2);
            w.write(s.xmaxc as u16, 6);
            for p in &s.xmc {
                w.write(*p as u16, 3);
            }
        }
        while w.data.len() < FRAME_SIZE {
            w.data.push(0);
        }
        w.data
    }

    fn excited_frame() -> GsmFrame {
        let sub = SubFrame {
            nc: 60,
            bc: 1,
            mc: 0,
            xmaxc: 40,
            xmc: [7, 0, 7, 0, 7, 0, 7, 0, 7, 0, 7, 0, 7],
        };
        GsmFrame {
            larc: [0; 8],
            sub: [sub; 4],
        }
    }

    #[test]
    fn decoder_emits_nonsilent_audio_frames() {
        let mut params = CodecParameters::audio(CodecId::new(CODEC_ID_STANDARD));
        params.sample_rate = Some(8_000);
        params.channels = Some(1);
        let mut dec = make_decoder(&params).expect("make_decoder");

        // Drive a handful of identical excited frames and verify the decoder
        // emits 160 samples per call with at least one non-zero PCM value.
        let bytes = pack_standard(&excited_frame());
        let mut saw_nonzero = false;
        for i in 0..4 {
            let pkt = Packet::new(0, TimeBase::new(1, 8_000), bytes.clone()).with_pts(i * 160);
            dec.send_packet(&pkt).expect("send_packet");
            let Frame::Audio(a) = dec.receive_frame().expect("receive_frame") else {
                panic!("expected audio frame");
            };
            assert_eq!(a.samples, 160);
            assert_eq!(a.channels, 1);
            assert_eq!(a.sample_rate, 8_000);
            assert_eq!(a.data.len(), 1);
            assert_eq!(a.data[0].len(), 320);
            let max = a.data[0]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs() as u32)
                .max()
                .unwrap();
            if max > 0 {
                saw_nonzero = true;
            }
        }
        assert!(saw_nonzero, "decoder produced all-silent output");
    }

    #[test]
    fn decoder_rejects_wrong_sample_rate() {
        let mut params = CodecParameters::audio(CodecId::new(CODEC_ID_STANDARD));
        params.sample_rate = Some(16_000);
        assert!(make_decoder(&params).is_err());
    }

    #[test]
    fn decoder_rejects_wrong_frame_size() {
        let mut params = CodecParameters::audio(CodecId::new(CODEC_ID_STANDARD));
        params.sample_rate = Some(8_000);
        let mut dec = make_decoder(&params).unwrap();
        let bad = vec![0xD0; 32]; // one byte short
        let pkt = Packet::new(0, TimeBase::new(1, 8_000), bad);
        dec.send_packet(&pkt).unwrap();
        assert!(dec.receive_frame().is_err());
    }
}
