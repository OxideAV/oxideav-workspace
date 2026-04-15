//! Pure-Rust FLAC encoder (functional, not bit-rate-optimal).
//!
//! This encoder produces valid FLAC streams that decode bit-exactly via any
//! compliant decoder (our own and `ffmpeg` tested). It uses a simple fixed
//! predictor of order 2 with Rice-coded residuals, partition order 0, and
//! picks the best Rice parameter exhaustively for each subframe. Channel
//! decorrelation is **not** applied — channels are stored independently —
//! so files are larger than what `flac --best` produces, but they're valid
//! and losslessly reversible.

use oxideav_codec::Encoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, MediaType, Packet, Result, SampleFormat,
    TimeBase,
};

use crate::bitwriter::BitWriter;
use crate::crc;

const DEFAULT_BLOCK_SIZE: u32 = 4096;

pub fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    let channels = params
        .channels
        .ok_or_else(|| Error::invalid("FLAC encoder: missing channels"))?;
    let sample_rate = params
        .sample_rate
        .ok_or_else(|| Error::invalid("FLAC encoder: missing sample_rate"))?;
    let sample_fmt = params.sample_format.unwrap_or(SampleFormat::S16);
    let bps = match sample_fmt {
        SampleFormat::U8 => 8,
        SampleFormat::S16 => 16,
        SampleFormat::S24 => 24,
        SampleFormat::S32 => 24, // FLAC stores up to 32-bit but WAV→FLAC is typically ≤24
        _ => {
            return Err(Error::unsupported(format!(
                "FLAC encoder: sample format {:?} not supported",
                sample_fmt
            )));
        }
    };
    if !(1..=8).contains(&channels) {
        return Err(Error::invalid("FLAC encoder: channels must be 1..=8"));
    }

    let extradata =
        build_streaminfo_metadata_block(DEFAULT_BLOCK_SIZE, sample_rate, channels as u8, bps);

    let mut output_params = params.clone();
    output_params.media_type = MediaType::Audio;
    output_params.codec_id = CodecId::new("flac");
    output_params.sample_format = Some(sample_fmt);
    output_params.channels = Some(channels);
    output_params.sample_rate = Some(sample_rate);
    output_params.extradata = extradata;

    Ok(Box::new(FlacEncoder {
        output_params,
        sample_format: sample_fmt,
        bps,
        channels,
        sample_rate,
        block_size: DEFAULT_BLOCK_SIZE,
        time_base: TimeBase::new(1, sample_rate as i64),
        interleaved: Vec::new(),
        pending: std::collections::VecDeque::new(),
        frame_number: 0,
        eof: false,
    }))
}

/// Build a full METADATA_BLOCK (header + STREAMINFO payload) marked as LAST.
fn build_streaminfo_metadata_block(
    block_size: u32,
    sample_rate: u32,
    channels: u8,
    bps: u8,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 34);
    // Header: last=1, type=STREAMINFO(0), length=34.
    out.push(0x80);
    out.push(0x00);
    out.push(0x00);
    out.push(0x22);
    // Payload: 2 + 2 + 3 + 3 + 8 + 16 = 34 bytes.
    out.extend_from_slice(&(block_size as u16).to_be_bytes());
    out.extend_from_slice(&(block_size as u16).to_be_bytes());
    out.extend_from_slice(&[0u8; 3]); // min_frame_size (unknown)
    out.extend_from_slice(&[0u8; 3]); // max_frame_size (unknown)
    let packed: u64 = ((sample_rate as u64) << 44)
        | (((channels - 1) as u64 & 0x7) << 41)
        | (((bps - 1) as u64 & 0x1F) << 36);
    out.extend_from_slice(&packed.to_be_bytes());
    out.extend_from_slice(&[0u8; 16]); // md5 (0 = unset, valid per spec)
    out
}

struct FlacEncoder {
    output_params: CodecParameters,
    sample_format: SampleFormat,
    bps: u8,
    channels: u16,
    sample_rate: u32,
    block_size: u32,
    time_base: TimeBase,
    /// Samples queued as interleaved i32. One element per (sample, channel).
    interleaved: Vec<i32>,
    pending: std::collections::VecDeque<Packet>,
    frame_number: u64,
    eof: bool,
}

impl FlacEncoder {
    fn ingest_frame(&mut self, a: &AudioFrame) -> Result<()> {
        if a.channels != self.channels || a.sample_rate != self.sample_rate {
            return Err(Error::invalid(
                "FLAC encoder: frame channels/sample_rate do not match encoder configuration",
            ));
        }
        if a.format != self.sample_format {
            return Err(Error::invalid(format!(
                "FLAC encoder: frame format {:?} does not match encoder format {:?}",
                a.format, self.sample_format
            )));
        }
        if a.format.is_planar() {
            return Err(Error::unsupported("FLAC encoder: planar input unsupported"));
        }
        let data = a
            .data
            .first()
            .ok_or_else(|| Error::invalid("empty frame"))?;
        match self.sample_format {
            SampleFormat::S16 => {
                for chunk in data.chunks_exact(2) {
                    self.interleaved
                        .push(i16::from_le_bytes([chunk[0], chunk[1]]) as i32);
                }
            }
            SampleFormat::S24 => {
                for chunk in data.chunks_exact(3) {
                    let mut v =
                        (chunk[0] as i32) | ((chunk[1] as i32) << 8) | ((chunk[2] as i32) << 16);
                    if v & 0x0080_0000 != 0 {
                        v |= 0xFF00_0000_u32 as i32;
                    }
                    self.interleaved.push(v);
                }
            }
            SampleFormat::S32 => {
                for chunk in data.chunks_exact(4) {
                    self.interleaved
                        .push(i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                }
            }
            SampleFormat::U8 => {
                for &b in data.iter() {
                    self.interleaved.push((b as i32) - 128);
                }
            }
            _ => return Err(Error::unsupported("FLAC encoder: unsupported input format")),
        }
        self.encode_ready_frames(false)
    }

    fn encode_ready_frames(&mut self, drain_all: bool) -> Result<()> {
        let n_ch = self.channels as usize;
        let block = self.block_size as usize;
        loop {
            let frames_avail = self.interleaved.len() / n_ch;
            let take = if drain_all {
                frames_avail
            } else if frames_avail >= block {
                block
            } else {
                return Ok(());
            };
            if take == 0 {
                return Ok(());
            }
            let mut per_channel: Vec<Vec<i32>> =
                (0..n_ch).map(|_| Vec::with_capacity(take)).collect();
            for i in 0..take {
                for c in 0..n_ch {
                    per_channel[c].push(self.interleaved[i * n_ch + c]);
                }
            }
            self.interleaved.drain(..take * n_ch);
            let data = encode_frame(
                self.frame_number,
                take as u32,
                self.sample_rate,
                self.bps,
                &per_channel,
            )?;
            let pts = (self.frame_number as i64) * (self.block_size as i64);
            let mut pkt = Packet::new(0, self.time_base, data);
            pkt.pts = Some(pts);
            pkt.dts = Some(pts);
            pkt.duration = Some(take as i64);
            pkt.flags.keyframe = true;
            self.pending.push_back(pkt);
            self.frame_number += 1;
        }
    }
}

impl Encoder for FlacEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.output_params.codec_id
    }
    fn output_params(&self) -> &CodecParameters {
        &self.output_params
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        match frame {
            Frame::Audio(a) => self.ingest_frame(a),
            _ => Err(Error::invalid("FLAC encoder: audio frames only")),
        }
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        self.pending.pop_front().ok_or(Error::NeedMore)
    }

    fn flush(&mut self) -> Result<()> {
        if !self.eof {
            self.eof = true;
            self.encode_ready_frames(true)?;
        }
        Ok(())
    }
}

// --- Per-frame encoding ---------------------------------------------------

fn encode_frame(
    frame_number: u64,
    block_size: u32,
    sample_rate: u32,
    bps: u8,
    channels: &[Vec<i32>],
) -> Result<Vec<u8>> {
    let n_ch = channels.len();
    if !(1..=8).contains(&n_ch) {
        return Err(Error::invalid("FLAC encoder: channel count out of range"));
    }

    let mut w = BitWriter::with_capacity(block_size as usize * 2 * n_ch);

    // Sync (14 bits) + reserved (1) + blocking strategy (1, 0=fixed).
    w.write_u32(0b11111111111110, 14);
    w.write_u32(0, 1);
    w.write_u32(0, 1);

    let (bs_code, bs_extra, bs_extra_bits) = encode_block_size(block_size);
    w.write_u32(bs_code as u32, 4);
    let (sr_code, sr_extra, sr_extra_bits) = encode_sample_rate(sample_rate);
    w.write_u32(sr_code as u32, 4);
    let ch_code = (n_ch - 1) as u32; // Independent channels.
    w.write_u32(ch_code, 4);
    let ss_code = encode_sample_size(bps);
    w.write_u32(ss_code as u32, 3);
    w.write_u32(0, 1); // reserved

    w.write_utf8_u64(frame_number);
    if bs_extra_bits > 0 {
        w.write_u32(bs_extra, bs_extra_bits);
    }
    if sr_extra_bits > 0 {
        w.write_u32(sr_extra, sr_extra_bits);
    }

    // Frame header is byte-aligned at this point.
    debug_assert!(w.is_byte_aligned());
    let hdr_bytes_so_far = w.bytes().to_vec();
    let hdr_crc8 = crc::crc8(&hdr_bytes_so_far);
    w.write_u32(hdr_crc8 as u32, 8);

    // Subframes.
    for ch_samples in channels {
        encode_subframe(&mut w, ch_samples, bps as u32)?;
    }

    // Align frame body to byte boundary for the CRC-16.
    w.align_to_byte();
    let frame_bytes = w.bytes().to_vec();
    let frame_crc16 = crc::crc16(&frame_bytes);
    w.write_u32(((frame_crc16 >> 8) & 0xFF) as u32, 8);
    w.write_u32((frame_crc16 & 0xFF) as u32, 8);

    Ok(w.into_bytes())
}

fn encode_block_size(bs: u32) -> (u8, u32, u32) {
    // (code, extra_value, extra_bits)
    match bs {
        192 => (1, 0, 0),
        576 => (2, 0, 0),
        1152 => (3, 0, 0),
        2304 => (4, 0, 0),
        4608 => (5, 0, 0),
        256 => (8, 0, 0),
        512 => (9, 0, 0),
        1024 => (10, 0, 0),
        2048 => (11, 0, 0),
        4096 => (12, 0, 0),
        8192 => (13, 0, 0),
        16384 => (14, 0, 0),
        32768 => (15, 0, 0),
        _ if bs >= 1 && bs - 1 < 256 => (6, bs - 1, 8),
        _ => (7, bs - 1, 16),
    }
}

fn encode_sample_rate(sr: u32) -> (u8, u32, u32) {
    match sr {
        88_200 => (1, 0, 0),
        176_400 => (2, 0, 0),
        192_000 => (3, 0, 0),
        8_000 => (4, 0, 0),
        16_000 => (5, 0, 0),
        22_050 => (6, 0, 0),
        24_000 => (7, 0, 0),
        32_000 => (8, 0, 0),
        44_100 => (9, 0, 0),
        48_000 => (10, 0, 0),
        96_000 => (11, 0, 0),
        _ => {
            if sr % 1000 == 0 && sr / 1000 <= 255 {
                (12, sr / 1000, 8)
            } else if sr <= 0xFFFF {
                (13, sr, 16)
            } else if sr % 10 == 0 && sr / 10 <= 0xFFFF {
                (14, sr / 10, 16)
            } else {
                (0, 0, 0) // get from STREAMINFO
            }
        }
    }
}

fn encode_sample_size(bps: u8) -> u8 {
    match bps {
        8 => 1,
        12 => 2,
        16 => 4,
        20 => 5,
        24 => 6,
        _ => 0, // get from STREAMINFO (codes 3, 7 are reserved)
    }
}

fn encode_subframe(w: &mut BitWriter, samples: &[i32], bps: u32) -> Result<()> {
    let n = samples.len();
    if n == 0 {
        return Err(Error::invalid("FLAC encoder: empty subframe"));
    }

    // Constant subframe if all samples are equal.
    if samples.iter().all(|&s| s == samples[0]) {
        w.write_u32(0, 1); // pad
        w.write_u32(0b000000, 6); // CONSTANT
        w.write_u32(0, 1); // no wasted bits
        w.write_i32(samples[0], bps);
        return Ok(());
    }

    // Fall back to VERBATIM for blocks too short to fit the predictor order.
    if n < 3 {
        return encode_verbatim(w, samples, bps);
    }

    // Fixed predictor order 2: residual[i] = s[i] - 2*s[i-1] + s[i-2].
    let order = 2usize;
    let mut residuals = Vec::with_capacity(n - order);
    for i in order..n {
        let pred = 2i64 * (samples[i - 1] as i64) - (samples[i - 2] as i64);
        let r = (samples[i] as i64) - pred;
        if !(i32::MIN as i64..=i32::MAX as i64).contains(&r) {
            return encode_verbatim(w, samples, bps);
        }
        residuals.push(r as i32);
    }

    w.write_u32(0, 1); // pad
    w.write_u32(0b001010, 6); // FIXED order 2 (0b001000 | 2)
    w.write_u32(0, 1); // no wasted bits
    for i in 0..order {
        w.write_i32(samples[i], bps);
    }
    encode_rice_residual(w, &residuals);
    Ok(())
}

fn encode_verbatim(w: &mut BitWriter, samples: &[i32], bps: u32) -> Result<()> {
    w.write_u32(0, 1); // pad
    w.write_u32(0b000001, 6); // VERBATIM
    w.write_u32(0, 1); // no wasted bits
    for &s in samples {
        w.write_i32(s, bps);
    }
    Ok(())
}

fn encode_rice_residual(w: &mut BitWriter, residuals: &[i32]) {
    // Method 0 (4-bit Rice parameter), partition_order = 0 (one partition).
    let k = choose_rice_k(residuals);
    w.write_u32(0, 2); // method
    w.write_u32(0, 4); // partition_order
    w.write_u32(k, 4); // Rice parameter
    for &r in residuals {
        let u = zigzag_encode(r);
        let q = u >> k;
        w.write_unary(q);
        if k > 0 {
            let rem = u & ((1u32 << k) - 1);
            w.write_u32(rem, k);
        }
    }
}

fn zigzag_encode(s: i32) -> u32 {
    ((s << 1) ^ (s >> 31)) as u32
}

/// Choose the Rice parameter that minimises total encoded bits. The 4-bit
/// field range is 0..15 with 15 reserved as an escape — pick 0..=14.
fn choose_rice_k(residuals: &[i32]) -> u32 {
    if residuals.is_empty() {
        return 0;
    }
    let mut best_k = 0u32;
    let mut best_bits = u64::MAX;
    for k in 0..=14u32 {
        let mut total: u64 = 0;
        for &r in residuals {
            let u = zigzag_encode(r) as u64;
            total += (u >> k) + 1 + k as u64;
            if total >= best_bits {
                break;
            }
        }
        if total < best_bits {
            best_bits = total;
            best_k = k;
        }
    }
    best_k
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder;

    fn roundtrip(channels_pcm: Vec<Vec<i32>>, sample_rate: u32, bps: u8) {
        // Build a multi-frame encoded bitstream.
        let n_ch = channels_pcm.len();
        let block_size = 1024u32;
        let total = channels_pcm[0].len();
        let mut all_frames: Vec<Vec<u8>> = Vec::new();
        let mut frame_num = 0u64;
        let mut i = 0;
        while i < total {
            let take = ((total - i) as u32).min(block_size) as usize;
            let per_ch: Vec<Vec<i32>> = (0..n_ch)
                .map(|c| channels_pcm[c][i..i + take].to_vec())
                .collect();
            let data = encode_frame(frame_num, take as u32, sample_rate, bps, &per_ch).unwrap();
            all_frames.push(data);
            frame_num += 1;
            i += take;
        }
        // Build a full FLAC stream: fLaC magic + STREAMINFO + frames.
        let mut stream = Vec::new();
        stream.extend_from_slice(&crate::metadata::FLAC_MAGIC);
        stream.extend_from_slice(&build_streaminfo_metadata_block(
            block_size,
            sample_rate,
            n_ch as u8,
            bps,
        ));
        for f in &all_frames {
            stream.extend_from_slice(f);
        }

        // Decode with our own decoder and check all samples round-trip.
        use oxideav_container::Demuxer as _;
        let mut params = oxideav_core::CodecParameters::audio(oxideav_core::CodecId::new("flac"));
        params.channels = Some(n_ch as u16);
        params.sample_rate = Some(sample_rate);
        // Extradata is the metadata-block portion (without fLaC magic).
        params.extradata =
            build_streaminfo_metadata_block(block_size, sample_rate, n_ch as u8, bps);
        let mut dec = decoder::make_decoder(&params).unwrap();

        // Walk frames and feed one at a time.
        let mut out_interleaved: Vec<i32> = Vec::new();
        for f in all_frames {
            let mut pkt =
                oxideav_core::Packet::new(0, oxideav_core::TimeBase::new(1, sample_rate as i64), f);
            pkt.pts = Some(0);
            dec.send_packet(&pkt).unwrap();
            let frame = dec.receive_frame().unwrap();
            let oxideav_core::Frame::Audio(a) = frame else {
                panic!("expected audio frame");
            };
            for chunk in a.data[0].chunks_exact(a.format.bytes_per_sample() * n_ch) {
                for c in 0..n_ch {
                    let off = c * a.format.bytes_per_sample();
                    let s = match a.format {
                        oxideav_core::SampleFormat::S16 => {
                            i16::from_le_bytes([chunk[off], chunk[off + 1]]) as i32
                        }
                        oxideav_core::SampleFormat::S24 => {
                            let mut v = (chunk[off] as i32)
                                | ((chunk[off + 1] as i32) << 8)
                                | ((chunk[off + 2] as i32) << 16);
                            if v & 0x0080_0000 != 0 {
                                v |= 0xFF00_0000_u32 as i32;
                            }
                            v
                        }
                        _ => panic!("unexpected format"),
                    };
                    out_interleaved.push(s);
                }
            }
        }

        // Compare sample-by-sample.
        for i in 0..total {
            for c in 0..n_ch {
                assert_eq!(
                    channels_pcm[c][i],
                    out_interleaved[i * n_ch + c],
                    "mismatch at sample {i} ch {c}"
                );
            }
        }
        let _ = stream;
    }

    #[test]
    fn encode_decode_mono_s16_sine() {
        let sr = 48_000u32;
        let n = 4096usize;
        let mut ch: Vec<i32> = Vec::with_capacity(n);
        for i in 0..n {
            let v = ((i as f64 / sr as f64 * 440.0 * 2.0 * std::f64::consts::PI).sin() * 20_000.0)
                as i32;
            ch.push(v);
        }
        roundtrip(vec![ch], sr, 16);
    }

    #[test]
    fn encode_decode_stereo_s16() {
        let sr = 44_100u32;
        let n = 2000usize;
        let mut l: Vec<i32> = Vec::with_capacity(n);
        let mut r: Vec<i32> = Vec::with_capacity(n);
        for i in 0..n {
            let base = (i as f64 / sr as f64 * 330.0 * 2.0 * std::f64::consts::PI).sin() * 15_000.0;
            l.push(base as i32);
            r.push((base * 0.8) as i32);
        }
        roundtrip(vec![l, r], sr, 16);
    }

    #[test]
    fn encode_decode_constant_block() {
        // All samples the same → CONSTANT subframe.
        let samples = vec![12345i32; 1024];
        roundtrip(vec![samples], 48_000, 16);
    }
}
