//! MP2 packet → AudioFrame decoder, wired into [`oxideav_codec::Decoder`].
//!
//! # Layout of one Layer II frame (ISO/IEC 11172-3 §2.4.1 + §2.4.2):
//! ```text
//!   32-bit header  [ + 16-bit CRC ]
//!   bit-allocation (variable)
//!   SCFSI          (2 bits per transmitted subband/channel)
//!   scalefactors   (0..3 × 6 bits per transmitted subband/channel)
//!   samples        (3 × 4 × per-triple codewords — sbgroup / triple / sb/ch)
//!   ancillary data (padding to frame end)
//! ```
//!
//! The synthesis filter bank produces 32 PCM samples per input 32-subband
//! granule; Layer II has 36 subband samples per subband per frame, split
//! into 3 × 12 "sbgroups" each sharing one scalefactor. 36 matrix pulls →
//! 36 × 32 = 1152 PCM samples per channel per frame.
//!
//! # Limitations
//! - MPEG-1 only (32/44.1/48 kHz). MPEG-2 LSF and MPEG-2.5 are rejected
//!   with `Error::Unsupported`.
//! - CRC-16 is accepted (bits advanced) but not verified.
//! - Free-format and reserved bitrate/sample-rate indices are rejected at
//!   header parse time.

use oxideav_codec::Decoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, Result, SampleFormat, TimeBase,
};

use crate::bitalloc::{read_layer2_side, validate_allocations};
use crate::bitreader::BitReader;
use crate::header::{parse_header, Mode};
use crate::requant::{read_samples, ReadState};
use crate::synth::SynthesisState;
use crate::tables::select_alloc_table;

/// Build a Layer II decoder. The codec parameters are consulted for the
/// canonical `codec_id` only — everything else is derived from the
/// incoming frame headers.
pub fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    Ok(Box::new(Mp2Decoder {
        codec_id: params.codec_id.clone(),
        time_base: TimeBase::new(1, 48_000),
        pending: None,
        synth: [SynthesisState::new(), SynthesisState::new()],
        eof: false,
    }))
}

struct Mp2Decoder {
    codec_id: CodecId,
    time_base: TimeBase,
    pending: Option<Packet>,
    synth: [SynthesisState; 2],
    eof: bool,
}

impl Decoder for Mp2Decoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.pending.is_some() {
            return Err(Error::other(
                "MP2 decoder: receive_frame must be called before sending another packet",
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

impl Mp2Decoder {
    fn decode_packet(&mut self, pkt: &Packet) -> Result<Frame> {
        let data = &pkt.data;
        let hdr = parse_header(data)?;
        let channels = hdr.channels() as usize;

        // Skip past the header and optional CRC-16.
        let mut offset = 4usize;
        if hdr.protection {
            if data.len() < offset + 2 {
                return Err(Error::invalid("mp2: truncated frame (missing CRC)"));
            }
            // CRC not verified.
            offset += 2;
        }
        if data.len() < hdr.frame_length() {
            return Err(Error::invalid(format!(
                "mp2: short frame: need {} bytes, got {}",
                hdr.frame_length(),
                data.len()
            )));
        }

        // Bitrate-index derivation for allocation-table lookup.
        // The header parser discarded the raw index, so reverse-map
        // from bitrate_kbps.
        let bri = bitrate_to_index(hdr.bitrate_kbps).ok_or_else(|| {
            Error::invalid(format!(
                "mp2: unexpected bitrate {} kbps for table lookup",
                hdr.bitrate_kbps
            ))
        })?;
        let stereo = !matches!(hdr.mode, Mode::Mono);
        let table = select_alloc_table(hdr.sample_rate, stereo, bri);

        // The joint-stereo bound must be clamped to sblimit for the
        // allocation reader.
        let bound = (hdr.bound as usize).min(table.sblimit);

        let mut br = BitReader::new(&data[offset..hdr.frame_length()]);

        // --- 1. Bit allocation, SCFSI, scalefactors ---
        let side = read_layer2_side(&mut br, table, hdr.mode, bound)?;
        validate_allocations(&side, table)?;

        // --- 2. Sample payload (36 samples × sblimit subbands × channels) ---
        let rs = ReadState {
            table,
            allocation: &side.allocation,
            scalefactor: &side.scalefactor,
            channels: side.channels,
            sblimit: table.sblimit,
            bound,
        };
        let subband_samples = read_samples(&mut br, &rs)?;

        // --- 3. 36 synthesis passes per channel → 1152 PCM samples/channel ---
        self.time_base = TimeBase::new(1, hdr.sample_rate as i64);
        let mut pcm = vec![[0.0f32; 1152]; channels];
        for step in 0..36 {
            for ch in 0..channels {
                let mut sb = [0.0f32; 32];
                for (sb_idx, item) in sb.iter_mut().enumerate().take(table.sblimit) {
                    *item = subband_samples[ch][sb_idx][step];
                }
                let mut out = [0.0f32; 32];
                self.synth[ch].synthesize(&sb, &mut out);
                pcm[ch][step * 32..(step + 1) * 32].copy_from_slice(&out);
            }
        }

        // Interleave & quantise to s16.
        let total_samples = 1152u32;
        let mut out_bytes = Vec::with_capacity(total_samples as usize * channels * 2);
        for i in 0..total_samples as usize {
            for ch_samples in pcm.iter().take(channels) {
                let f = ch_samples[i].clamp(-1.0, 1.0);
                let s = (f * 32767.0) as i16;
                out_bytes.extend_from_slice(&s.to_le_bytes());
            }
        }

        Ok(Frame::Audio(AudioFrame {
            format: SampleFormat::S16,
            channels: channels as u16,
            sample_rate: hdr.sample_rate,
            samples: total_samples,
            pts: pkt.pts,
            time_base: self.time_base,
            data: vec![out_bytes],
        }))
    }
}

/// Reverse-map a bitrate in kbps to its header-field index (1..=14 for
/// MPEG-1 Layer II).
fn bitrate_to_index(bitrate_kbps: u32) -> Option<u32> {
    const LUT: [u32; 15] = [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
    ];
    LUT.iter()
        .position(|&v| v == bitrate_kbps)
        .map(|idx| idx as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_index_lookup() {
        assert_eq!(bitrate_to_index(128), Some(8));
        assert_eq!(bitrate_to_index(192), Some(10));
        assert_eq!(bitrate_to_index(32), Some(1));
        assert_eq!(bitrate_to_index(384), Some(14));
        assert_eq!(bitrate_to_index(999), None);
    }
}
