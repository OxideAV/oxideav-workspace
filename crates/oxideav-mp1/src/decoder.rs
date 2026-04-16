//! MPEG-1 Audio Layer I → PCM decoder.
//!
//! Layer I organises each frame as follows (ISO/IEC 11172-3 §2.4.1):
//!
//! 1. 32-bit header (parsed in [`crate::header`]).
//! 2. Optional 16-bit CRC when `protection == true`.
//! 3. Bit allocation — 4 bits per subband per channel (see
//!    [`crate::bitalloc`]).
//! 4. Scalefactors — 6 bits per subband per channel (only when the
//!    subband's allocation is non-zero).
//! 5. 12 samples per subband per channel, interleaved
//!    (`s[0][0..32], s[1][0..32], ..., s[11][0..32]` for mono — per-block
//!    inner loop touches every subband once before moving to the next
//!    block).
//!
//! For each 12-sample block we run the 32-band polyphase synthesis
//! filter ([`crate::synthesis`]) 12 times — each step consumes one
//! subband sample per subband and emits 32 PCM samples per channel.
//! One Layer I frame therefore emits `32 * 12 = 384` PCM samples per
//! channel.
//!
//! Joint-stereo is handled by `mode_extension` selecting a bound: for
//! subbands `[bound..32)` there is a single allocation / scalefactor
//! pair shared between channels, and one sample per subband per block
//! is read and duplicated to both output channels (M/S-style shared
//! stream; intensity-stereo scaling is not part of Layer I).

use oxideav_codec::Decoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, Result, SampleFormat, TimeBase,
};

use crate::bitalloc::{bits_per_sample, dequant_table, SAMPLES_PER_SUBBAND, SBLIMIT};
use crate::bitreader::BitReader;
use crate::header::FrameHeader;
use crate::synthesis::SynthesisState;

/// Maximum PCM channels (Layer I is stereo at most).
const MAX_CH: usize = 2;

pub fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    Ok(Box::new(Mp1Decoder {
        codec_id: params.codec_id.clone(),
        time_base: TimeBase::new(1, 48_000),
        pending: None,
        synth_state: [SynthesisState::new(), SynthesisState::new()],
        eof: false,
    }))
}

struct Mp1Decoder {
    codec_id: CodecId,
    time_base: TimeBase,
    pending: Option<Packet>,
    synth_state: [SynthesisState; MAX_CH],
    eof: bool,
}

impl Decoder for Mp1Decoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.pending.is_some() {
            return Err(Error::other(
                "MP1 decoder: receive_frame must be called before sending another packet",
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
        self.decode_packet(&pkt)
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

impl Mp1Decoder {
    fn decode_packet(&mut self, pkt: &Packet) -> Result<Frame> {
        let data = &pkt.data;
        if data.len() < 4 {
            return Err(Error::NeedMore);
        }
        let hdr = FrameHeader::parse(data)?;
        let channels = hdr.mode.channel_count() as usize;
        let bound = hdr.bound() as usize;

        // Header + optional CRC (we don't verify; we just skip past it).
        let crc_bytes = if hdr.protection { 2 } else { 0 };
        let payload_start = 4 + crc_bytes;
        if data.len() < payload_start {
            return Err(Error::NeedMore);
        }

        self.time_base = TimeBase::new(1, hdr.sample_rate as i64);

        // Bitstream body.
        let mut br = BitReader::new(&data[payload_start..]);

        // Step 1: allocation. Per channel for subbands [0..bound), then
        // a single allocation per subband for [bound..SBLIMIT).
        let mut alloc = [[0u8; SBLIMIT]; MAX_CH];
        for sb in 0..bound {
            for ch in 0..channels {
                let a = br.read_u32(4)? as u8;
                if a == 15 {
                    return Err(Error::invalid("MP1: forbidden allocation code 15"));
                }
                alloc[ch][sb] = a;
            }
        }
        for sb in bound..SBLIMIT {
            let a = br.read_u32(4)? as u8;
            if a == 15 {
                return Err(Error::invalid("MP1: forbidden allocation code 15"));
            }
            alloc[0][sb] = a;
            if channels == 2 {
                alloc[1][sb] = a;
            }
        }

        // Step 2: scalefactor indices (6 bits) for every subband whose
        // allocation is non-zero. In joint-stereo-shared subbands both
        // channels carry their own scalefactor even though samples are
        // shared — §2.4.2.3 scale_factor[2][sb] is always per channel.
        let mut scf = [[0u8; SBLIMIT]; MAX_CH];
        for sb in 0..SBLIMIT {
            for ch in 0..channels {
                let a = if sb < bound {
                    alloc[ch][sb]
                } else {
                    alloc[0][sb]
                };
                if a != 0 {
                    scf[ch][sb] = br.read_u32(6)? as u8;
                }
            }
        }

        // Step 3: samples. Layer I reads 12 blocks of 32 subband values.
        // For each block and each subband, read one sample per channel
        // (channel-paired) for sb < bound, or one shared sample for
        // sb >= bound (stored into both channels).
        //
        // Layout per block: sb=0 ch=0, sb=0 ch=1, sb=1 ch=0, sb=1 ch=1,
        // ... up to sb=bound-1; then sb=bound..SBLIMIT with a single
        // sample per subband (shared).
        //
        // `subband_samples[ch][block][sb]` holds the dequantised float.
        let deq = dequant_table();
        let mut subband_samples = vec![[[0.0f32; SBLIMIT]; SAMPLES_PER_SUBBAND]; channels];

        for block in 0..SAMPLES_PER_SUBBAND {
            for sb in 0..bound {
                for ch in 0..channels {
                    let a = alloc[ch][sb];
                    if a == 0 {
                        continue;
                    }
                    let nb =
                        bits_per_sample(a).ok_or_else(|| Error::invalid("MP1: bad allocation"))?;
                    let raw = br.read_u32(nb as u32)?;
                    let level = raw as i32 - (1i32 << (nb - 1)) + 1;
                    let dq = deq[nb as usize][scf[ch][sb] as usize];
                    subband_samples[ch][block][sb] = dq * level as f32;
                }
            }
            for sb in bound..SBLIMIT {
                let a = alloc[0][sb];
                if a == 0 {
                    continue;
                }
                let nb = bits_per_sample(a).ok_or_else(|| Error::invalid("MP1: bad allocation"))?;
                let raw = br.read_u32(nb as u32)?;
                let level = raw as i32 - (1i32 << (nb - 1)) + 1;
                // Shared sample, per-channel scalefactor.
                let d0 = deq[nb as usize][scf[0][sb] as usize];
                subband_samples[0][block][sb] = d0 * level as f32;
                if channels == 2 {
                    let d1 = deq[nb as usize][scf[1][sb] as usize];
                    subband_samples[1][block][sb] = d1 * level as f32;
                }
            }
        }

        // Step 4: polyphase synthesis. 384 PCM samples per channel.
        let samples_per_frame = (SAMPLES_PER_SUBBAND * SBLIMIT) as u32;
        let bytes_per_sample = SampleFormat::S16.bytes_per_sample();
        let mut interleaved: Vec<f32> = vec![0.0f32; samples_per_frame as usize * channels];
        // Per channel, fill the interleaved buffer: pcm[i*channels + ch].
        for ch in 0..channels {
            for block in 0..SAMPLES_PER_SUBBAND {
                let mut pcm32 = [0.0f32; SBLIMIT];
                self.synth_state[ch].synthesize(&subband_samples[ch][block], &mut pcm32);
                for j in 0..SBLIMIT {
                    let idx = (block * SBLIMIT + j) * channels + ch;
                    interleaved[idx] = pcm32[j];
                }
            }
        }

        // Pack to S16 LE.
        let mut out_bytes =
            Vec::with_capacity(samples_per_frame as usize * channels * bytes_per_sample);
        for &v in interleaved.iter() {
            let s = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
            out_bytes.extend_from_slice(&s.to_le_bytes());
        }

        Ok(Frame::Audio(AudioFrame {
            format: SampleFormat::S16,
            channels: channels as u16,
            sample_rate: hdr.sample_rate,
            samples: samples_per_frame,
            pts: pkt.pts,
            time_base: self.time_base,
            data: vec![out_bytes],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: build the decoder (should succeed) and feed a trivial
    /// frame with all-zero subband data (everything nb=0) — output must
    /// be silence, and the synthesis state must have advanced.
    #[test]
    fn decode_all_silence_frame() {
        // Header: MPEG-1 L1, no CRC, 32 kbps, 32 kHz, no padding, stereo.
        // bitrate_index=1, sample_rate_idx=2 (32 kHz), mode=00 (stereo).
        //
        // Bit layout:
        // 12 sync | 1 id=1 | 2 layer=11 (L1) | 1 prot=1 (no CRC) |
        // 4 br=0001 | 2 sfreq=10 | 1 pad=0 | 1 priv=0 | 2 mode=00 |
        // 2 mext=00 | 1 cr=0 | 1 orig=0 | 2 emph=00
        //
        // As bytes (MSB first):
        // 11111111 11111 1 11 1 0001 10 0 0 00 00 0 0 00
        // = 11111111 11111111 00011000 00000000 = 0xFF 0xFF 0x18 0x00
        //
        // Frame size: (12*32000/32000 + 0)*4 = 48 bytes. The header eats
        // 4 of those; the remaining 44 bytes hold allocation (32 subbands
        // * 4 bits * 2 channels = 256 bits = 32 bytes), plus samples.
        //
        // With all allocations = 0, scalefactors and samples are skipped
        // entirely. We pad to 48 bytes with zeros.
        let mut frame = vec![0xFFu8, 0xFF, 0x18, 0x00];
        frame.resize(48, 0);

        let params = CodecParameters::audio(CodecId::new(crate::CODEC_ID_STR));
        let mut dec = make_decoder(&params).unwrap();
        let pkt = Packet::new(0, TimeBase::new(1, 32_000), frame);
        dec.send_packet(&pkt).unwrap();
        let f = dec.receive_frame().unwrap();
        match f {
            Frame::Audio(a) => {
                assert_eq!(a.sample_rate, 32_000);
                assert_eq!(a.samples, 384);
                assert_eq!(a.channels, 2);
                // Silence in → silence out (synthesis state was already
                // zero; the filter just settles).
                for chunk in a.data[0].chunks_exact(2) {
                    let s = i16::from_le_bytes([chunk[0], chunk[1]]);
                    assert_eq!(s, 0, "expected silence got {s}");
                }
            }
            _ => panic!("expected audio frame"),
        }
    }
}
