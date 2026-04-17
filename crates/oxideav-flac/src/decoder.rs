//! Top-level FLAC frame decoder, wired into the [`oxideav_codec::Decoder`] trait.

use oxideav_codec::Decoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, Result, SampleFormat, TimeBase,
};

use crate::bitreader::BitReader;
use crate::crc;
use crate::frame::{parse_frame_header, ChannelAssignment};
use crate::metadata::{BlockHeader, BlockType, StreamInfo};
use crate::subframe::decode_subframe;

pub fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    let streaminfo = find_streaminfo(&params.extradata)?;
    let bps = streaminfo.bits_per_sample;
    let output_format = match bps {
        1..=16 => SampleFormat::S16,
        17..=24 => SampleFormat::S24,
        25..=32 => SampleFormat::S32,
        _ => return Err(Error::unsupported(format!("FLAC bps {bps}"))),
    };
    let time_base = TimeBase::new(1, streaminfo.sample_rate as i64);
    Ok(Box::new(FlacDecoder {
        codec_id: params.codec_id.clone(),
        streaminfo,
        output_format,
        time_base,
        pending: None,
        eof: false,
    }))
}

struct FlacDecoder {
    codec_id: CodecId,
    streaminfo: StreamInfo,
    output_format: SampleFormat,
    time_base: TimeBase,
    pending: Option<Packet>,
    eof: bool,
}

impl Decoder for FlacDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.pending.is_some() {
            return Err(Error::other(
                "FLAC decoder: receive_frame must be called before sending another packet",
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
        decode_one_frame(
            &pkt.data,
            &self.streaminfo,
            self.output_format,
            self.time_base,
            pkt.pts,
        )
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }

    // FLAC has no inter-packet DSP state (subframe LPC predictors are
    // initialised from the frame header), so the default drain-then-
    // forget `reset` is sufficient.
}

fn find_streaminfo(extradata: &[u8]) -> Result<StreamInfo> {
    let mut i = 0;
    while i + 4 <= extradata.len() {
        let hdr = BlockHeader::parse(&extradata[i..i + 4])?;
        let payload_start = i + 4;
        let payload_end = payload_start + hdr.length as usize;
        if payload_end > extradata.len() {
            return Err(Error::invalid(
                "FLAC extradata: metadata block exceeds buffer",
            ));
        }
        if hdr.block_type == BlockType::StreamInfo {
            return StreamInfo::parse(&extradata[payload_start..payload_end]);
        }
        if hdr.last {
            break;
        }
        i = payload_end;
    }
    Err(Error::invalid("FLAC decoder: no STREAMINFO in extradata"))
}

fn decode_one_frame(
    data: &[u8],
    streaminfo: &StreamInfo,
    output_format: SampleFormat,
    time_base: TimeBase,
    pts: Option<i64>,
) -> Result<Frame> {
    let header = parse_frame_header(data)?;
    let body_offset = header.header_byte_len;

    let bps = if header.bits_per_sample != 0 {
        header.bits_per_sample as u32
    } else {
        streaminfo.bits_per_sample as u32
    };

    let mut br = BitReader::new(&data[body_offset..]);

    let n_channels = header.channels.channel_count() as usize;
    let mut channels: Vec<Vec<i32>> = Vec::with_capacity(n_channels);
    for ch in 0..n_channels {
        let bps_for_channel = match header.channels {
            ChannelAssignment::LeftSide if ch == 1 => bps + 1,
            ChannelAssignment::RightSide if ch == 0 => bps + 1,
            ChannelAssignment::MidSide if ch == 1 => bps + 1,
            _ => bps,
        };
        let samples = decode_subframe(&mut br, header.block_size, bps_for_channel)?;
        if samples.len() != header.block_size as usize {
            return Err(Error::invalid("subframe sample count mismatch"));
        }
        channels.push(samples);
    }

    apply_decorrelation(&mut channels, header.channels);

    // Skip frame-body padding to reach the byte-aligned 16-bit CRC at end.
    br.align_to_byte();
    let body_used = br.byte_position();
    let frame_byte_len = body_offset + body_used;
    if frame_byte_len + 2 > data.len() {
        return Err(Error::invalid(
            "FLAC frame: not enough bytes for trailing CRC-16",
        ));
    }
    let claimed_crc = u16::from_be_bytes([data[frame_byte_len], data[frame_byte_len + 1]]);
    let computed = crc::crc16(&data[..frame_byte_len]);
    if computed != claimed_crc {
        return Err(Error::invalid(format!(
            "FLAC frame CRC-16 mismatch (got {:#06x}, want {:#06x})",
            computed, claimed_crc
        )));
    }

    let total_samples = header.block_size as usize;
    let n_out = channels.len();
    let bytes_per_sample = output_format.bytes_per_sample();
    let mut out = Vec::with_capacity(total_samples * n_out * bytes_per_sample);
    for i in 0..total_samples {
        for c in 0..n_out {
            let s = channels[c][i];
            match output_format {
                SampleFormat::S16 => out.extend_from_slice(&(s as i16).to_le_bytes()),
                SampleFormat::S24 => {
                    out.push((s & 0xFF) as u8);
                    out.push(((s >> 8) & 0xFF) as u8);
                    out.push(((s >> 16) & 0xFF) as u8);
                }
                SampleFormat::S32 => out.extend_from_slice(&s.to_le_bytes()),
                _ => {
                    return Err(Error::unsupported(
                        "FLAC decoder output format not supported",
                    ))
                }
            }
        }
    }

    Ok(Frame::Audio(AudioFrame {
        format: output_format,
        channels: n_out as u16,
        sample_rate: streaminfo.sample_rate,
        samples: total_samples as u32,
        pts,
        time_base,
        data: vec![out],
    }))
}

fn apply_decorrelation(channels: &mut [Vec<i32>], assign: ChannelAssignment) {
    match assign {
        ChannelAssignment::Independent(_) => {}
        ChannelAssignment::LeftSide => {
            // ch[0] = left, ch[1] = side = left - right → right = left - side.
            let n = channels[0].len();
            for i in 0..n {
                channels[1][i] = channels[0][i].wrapping_sub(channels[1][i]);
            }
        }
        ChannelAssignment::RightSide => {
            // ch[0] = side = left - right, ch[1] = right → left = right + side.
            let n = channels[0].len();
            for i in 0..n {
                channels[0][i] = channels[1][i].wrapping_add(channels[0][i]);
            }
        }
        ChannelAssignment::MidSide => {
            // ch[0] = mid (with absorbed LSB), ch[1] = side.
            let n = channels[0].len();
            for i in 0..n {
                let mid = channels[0][i] as i64;
                let side = channels[1][i] as i64;
                let m = (mid << 1) | (side & 1);
                channels[0][i] = ((m + side) >> 1) as i32;
                channels[1][i] = ((m - side) >> 1) as i32;
            }
        }
    }
}
