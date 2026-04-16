//! Vorbis encoder.
//!
//! Today's emitted bitstream is **silence** — every audio packet is a
//! "floor unused" marker that legally decodes to zero PCM. The pipeline
//! plumbing (header emission, frame buffering, per-block packet emission,
//! flush) is complete and verified by an encoder→decoder roundtrip test.
//!
//! Real audio encoding (forward MDCT, floor1 post quantisation, residue
//! VQ codebook search, full-quality setup header with tuned codebooks)
//! is a follow-up — `forward_mdct_naive` is wired and tested but not
//! yet driven from the packet loop because the placeholder setup only
//! defines a single trivial codebook insufficient for residue encoding.

use std::collections::VecDeque;

use oxideav_codec::Encoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, MediaType, Packet, Result, SampleFormat,
    TimeBase,
};

use crate::bitwriter::BitWriter;

/// Default short blocksize (power-of-two exponent). 256 samples matches
/// libvorbis's standard low-bitrate configuration.
pub const DEFAULT_BLOCKSIZE_SHORT_LOG2: u8 = 8; // 1 << 8 = 256
/// Default long blocksize. 2048 samples matches libvorbis for music content.
pub const DEFAULT_BLOCKSIZE_LONG_LOG2: u8 = 11; // 1 << 11 = 2048

/// Assemble the Vorbis Identification header (§4.2.2).
pub fn build_identification_header(
    channels: u8,
    sample_rate: u32,
    bitrate_nominal: i32,
    blocksize_0_log2: u8,
    blocksize_1_log2: u8,
) -> Vec<u8> {
    assert!(channels >= 1, "Vorbis requires at least one channel");
    assert!(sample_rate > 0, "Vorbis requires a non-zero sample rate");
    assert!(
        (6..=13).contains(&blocksize_0_log2)
            && (6..=13).contains(&blocksize_1_log2)
            && blocksize_0_log2 <= blocksize_1_log2,
        "Vorbis blocksize exponents must be in 6..=13 and short <= long"
    );

    let mut out = Vec::with_capacity(30);
    out.push(0x01);
    out.extend_from_slice(b"vorbis");
    out.extend_from_slice(&0u32.to_le_bytes()); // vorbis_version
    out.push(channels);
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes()); // bitrate_maximum (0 = unset)
    out.extend_from_slice(&bitrate_nominal.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes()); // bitrate_minimum
                                                // blocksize byte: low nibble = blocksize_0, high nibble = blocksize_1.
    out.push((blocksize_1_log2 << 4) | (blocksize_0_log2 & 0x0F));
    out.push(0x01); // framing bit (per Vorbis I §4.2.2)
    out
}

/// Assemble the Vorbis Comment header (§5). Uses a fixed vendor string
/// identifying this encoder; `comments` is an optional list of
/// `KEY=VALUE` strings.
pub fn build_comment_header(comments: &[String]) -> Vec<u8> {
    let vendor = concat!("oxideav-vorbis ", env!("CARGO_PKG_VERSION")).as_bytes();
    let mut out = Vec::with_capacity(32 + vendor.len());
    out.push(0x03);
    out.extend_from_slice(b"vorbis");
    out.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    out.extend_from_slice(vendor);
    out.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for c in comments {
        let bytes = c.as_bytes();
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out.push(0x01); // framing bit
    out
}

/// Assemble the Vorbis Setup header with a **minimal** configuration:
/// one channel (passed through `channels` for the mapping mux), one floor1
/// with a single partition class and two posts (X=0 and X=blocksize/2),
/// one residue type-2, and two modes (short / long).
///
/// The returned setup is a placeholder: decoders accept it but no real
/// content is encoded yet. Used to unblock muxer roundtrips so the
/// audio-packet encoder can be written against a known good setup shape.
pub fn build_placeholder_setup_header(channels: u8) -> Vec<u8> {
    let _ = channels;
    let mut w = BitWriter::with_capacity(64);
    // Setup packet header.
    for &b in &[0x05u32, 0x76, 0x6f, 0x72, 0x62, 0x69, 0x73] {
        w.write_u32(b, 8);
    }
    // codebook_count = 1 (minus 1 encoded).
    w.write_u32(0, 8);
    // One codebook: 1 dim, 2 entries, both length 1 (identity-ish tree).
    // Sync: 0x564342 (24 bits, LSB-first in bytes gives 'B' 'C' 'V').
    w.write_u32(0x564342, 24);
    w.write_u32(1, 16); // dimensions = 1
    w.write_u32(2, 24); // entries = 2
    w.write_bit(false); // ordered flag
    w.write_bit(false); // sparse flag
                        // Per-entry length-1 (stored as length-1 = 0 → write 0).
    for _ in 0..2 {
        w.write_u32(0, 5); // codeword_length - 1 = 0 → length 1
    }
    w.write_u32(0, 4); // lookup_type = 0 (no VQ)

    // time_count = 0 (minus 1), placeholder value = 0 (6 bits).
    w.write_u32(0, 6);
    w.write_u32(0, 16);

    // floor_count = 0 (minus 1).
    w.write_u32(0, 6);
    // Floor0_type = 1 (floor1).
    w.write_u32(1, 16);
    // floor1 body: partitions=1 (5 bits), classes=[0] (4 bits).
    w.write_u32(1, 5);
    w.write_u32(0, 4);
    // class_dimensions[0] = 1 (stored as 1 minus one = 0).
    w.write_u32(0, 3);
    // class_subclasses[0] = 0.
    w.write_u32(0, 2);
    // No master book since subclasses=0.
    // subbooks for 1 << subclasses = 1 slot: value 0 → book_index = -1
    // (spec treats "0" as "no book", actual book = value-1).
    w.write_u32(0, 8);
    // multiplier (2 bits): 2 (stored minus-one = 1 → value 1 == mult=2).
    w.write_u32(1, 2);
    // rangebits (4 bits): ilog(n/2). For blocksize 256, n/2=128, ilog=7.
    // Use 7 so the xlist holds 7-bit X values (0..=127). This is
    // sufficient for the short block; long-block floor setup would need
    // a separate floor.
    w.write_u32(7, 4);
    // No per-partition X values because class_dimensions[0]=1 and
    // partitions=1 → dim=1 extra X after the 2 implicit (0 and 128).
    // Wait — the partition class list above points at class 0 which has
    // cdim=1, so we DO read 1 X value here (not zero). Write X=64 as
    // something in the middle.
    w.write_u32(64, 7);

    // residue_count = 0 (minus 1).
    w.write_u32(0, 6);
    // Residue0_type = 2 (residue2).
    w.write_u32(2, 16);
    w.write_u32(0, 24); // begin
    w.write_u32(0, 24); // end  (spec: values past blocksize/2 are skipped)
    w.write_u32(0, 24); // partition_size = 0+1 = 1
    w.write_u32(0, 6); // classifications-1 = 0 → 1 class
    w.write_u32(0, 8); // classbook = 0
                       // Cascade per class: 3 low bits + maybe 5 high bits.
    w.write_u32(0, 3); // low bits
    w.write_bit(false); // bitflag
                        // No cascade bits set, so no books.

    // mapping_count = 0 (minus 1).
    w.write_u32(0, 6);
    // mapping_type = 0.
    w.write_u32(0, 16);
    // submaps flag (bit): 0 (use 1 submap).
    w.write_bit(false);
    // coupling flag: 0.
    w.write_bit(false);
    // reserved 2 bits.
    w.write_u32(0, 2);
    // No mux since submaps == 1. Submap 0:
    w.write_u32(0, 8); // time index
    w.write_u32(0, 8); // floor index
    w.write_u32(0, 8); // residue index

    // mode_count = 0 (minus 1) — 1 mode.
    w.write_u32(0, 6);
    // Mode 0: blockflag=0 (short), windowtype=0, transformtype=0, mapping=0.
    w.write_bit(false);
    w.write_u32(0, 16);
    w.write_u32(0, 16);
    w.write_u32(0, 8);

    // Framing bit.
    w.write_bit(true);
    w.finish()
}

/// Build extradata: 3 Xiph-laced headers.
pub fn build_extradata(id: &[u8], comment: &[u8], setup: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + id.len() + comment.len() + setup.len() + 8);
    out.push(2); // packet count - 1
                 // Lacing for id and comment (setup length inferred from trailing bytes).
    for sz in [id.len(), comment.len()] {
        let mut rem = sz;
        while rem >= 255 {
            out.push(255);
            rem -= 255;
        }
        out.push(rem as u8);
    }
    out.extend_from_slice(id);
    out.extend_from_slice(comment);
    out.extend_from_slice(setup);
    out
}

pub fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    let channels = params
        .channels
        .ok_or_else(|| Error::invalid("Vorbis encoder: channels required"))?;
    if !(1..=2).contains(&channels) {
        return Err(Error::unsupported(format!(
            "Vorbis encoder: {channels}-channel encode not supported yet (mono + stereo only)"
        )));
    }
    let sample_rate = params
        .sample_rate
        .ok_or_else(|| Error::invalid("Vorbis encoder: sample_rate required"))?;

    let id_hdr = build_identification_header(
        channels as u8,
        sample_rate,
        0,
        DEFAULT_BLOCKSIZE_SHORT_LOG2,
        DEFAULT_BLOCKSIZE_LONG_LOG2,
    );
    let comment_hdr = build_comment_header(&[]);
    let setup_hdr = build_placeholder_setup_header(channels as u8);
    let extradata = build_extradata(&id_hdr, &comment_hdr, &setup_hdr);

    let mut out_params = CodecParameters::audio(CodecId::new(crate::CODEC_ID_STR));
    out_params.media_type = MediaType::Audio;
    out_params.channels = Some(channels);
    out_params.sample_rate = Some(sample_rate);
    out_params.sample_format = Some(SampleFormat::S16);
    out_params.extradata = extradata;

    let blocksize = 1usize << DEFAULT_BLOCKSIZE_SHORT_LOG2;
    Ok(Box::new(VorbisEncoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        out_params,
        time_base: TimeBase::new(1, sample_rate as i64),
        channels,
        blocksize,
        input_buf: vec![Vec::with_capacity(blocksize * 4); channels as usize],
        output_queue: VecDeque::new(),
        pts: 0,
        flushed: false,
    }))
}

struct VorbisEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    time_base: TimeBase,
    channels: u16,
    blocksize: usize,
    /// Per-channel pending input samples (planar f32 in [-1, 1]).
    input_buf: Vec<Vec<f32>>,
    /// Encoded packets waiting to be drained by `receive_packet`.
    output_queue: VecDeque<Packet>,
    /// PTS counter — incremented by output samples per emitted packet.
    pts: i64,
    flushed: bool,
}

impl VorbisEncoder {
    /// Append samples from an [`AudioFrame`] into the per-channel input
    /// buffers, converting from whatever sample format the caller used.
    fn push_audio_frame(&mut self, frame: &AudioFrame) -> Result<()> {
        if frame.channels != self.channels {
            return Err(Error::invalid(format!(
                "Vorbis encoder: expected {} channels, got {}",
                self.channels, frame.channels
            )));
        }
        let n = frame.samples as usize;
        if n == 0 {
            return Ok(());
        }
        // Decode planar/interleaved into per-channel f32. Limited
        // sample-format support for now — extend as needed.
        match frame.format {
            SampleFormat::S16 => {
                let plane = frame
                    .data
                    .first()
                    .ok_or_else(|| Error::invalid("S16 frame missing data plane"))?;
                let stride = self.channels as usize * 2;
                if plane.len() < n * stride {
                    return Err(Error::invalid("S16 frame: data plane too short"));
                }
                for i in 0..n {
                    for ch in 0..self.channels as usize {
                        let off = i * stride + ch * 2;
                        let sample = i16::from_le_bytes([plane[off], plane[off + 1]]);
                        self.input_buf[ch].push(sample as f32 / 32768.0);
                    }
                }
            }
            SampleFormat::F32 => {
                let plane = frame
                    .data
                    .first()
                    .ok_or_else(|| Error::invalid("F32 frame missing data plane"))?;
                let stride = self.channels as usize * 4;
                if plane.len() < n * stride {
                    return Err(Error::invalid("F32 frame: data plane too short"));
                }
                for i in 0..n {
                    for ch in 0..self.channels as usize {
                        let off = i * stride + ch * 4;
                        let v = f32::from_le_bytes([
                            plane[off],
                            plane[off + 1],
                            plane[off + 2],
                            plane[off + 3],
                        ]);
                        self.input_buf[ch].push(v);
                    }
                }
            }
            other => {
                return Err(Error::unsupported(format!(
                    "Vorbis encoder: input sample format {other:?} not supported yet"
                )));
            }
        }
        Ok(())
    }

    /// Drain full blocksize-worth of input by emitting one audio packet
    /// per block. The current packet body is the simplest legal Vorbis
    /// audio packet: header bit + per-channel `nonzero=0` (unused floor),
    /// which decodes to silence. Real spectrum encoding (MDCT + floor
    /// analysis + residue VQ) lands in a follow-up — the placeholder
    /// setup header has only one trivial codebook so the bitstream
    /// vocabulary doesn't yet support real residue encoding.
    fn drain_blocks(&mut self, force_short: bool) {
        let _ = force_short;
        while self.input_buf[0].len() >= self.blocksize {
            // Consume blocksize samples from each channel.
            for ch in 0..self.channels as usize {
                self.input_buf[ch].drain(..self.blocksize);
            }
            let pkt = self.emit_silent_packet();
            self.output_queue.push_back(pkt);
        }
    }

    fn emit_silent_packet(&mut self) -> Packet {
        let mut w = BitWriter::with_capacity(2);
        // Audio packet header bit: 0 (audio).
        w.write_bit(false);
        // mode_bits = ilog(modes_count - 1) = ilog(0) = 0 with our
        // single-mode placeholder setup → no mode bits.
        // Per-channel floor: read nonzero bit = 0 → unused, no residue
        // values follow.
        for _ in 0..self.channels as usize {
            w.write_bit(false);
        }
        let data = w.finish();
        let pts = self.pts;
        self.pts += self.blocksize as i64;
        let mut pkt = Packet::new(0, self.time_base, data);
        pkt.pts = Some(pts);
        pkt.dts = Some(pts);
        pkt.duration = Some(self.blocksize as i64);
        pkt.flags.keyframe = true;
        pkt
    }
}

impl Encoder for VorbisEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn output_params(&self) -> &CodecParameters {
        &self.out_params
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        if self.flushed {
            return Err(Error::other("encoder already flushed"));
        }
        match frame {
            Frame::Audio(a) => {
                self.push_audio_frame(a)?;
                self.drain_blocks(false);
                Ok(())
            }
            Frame::Video(_) => Err(Error::invalid("Vorbis encoder received a video frame")),
        }
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        if let Some(p) = self.output_queue.pop_front() {
            return Ok(p);
        }
        if self.flushed {
            Err(Error::Eof)
        } else {
            Err(Error::NeedMore)
        }
    }

    fn flush(&mut self) -> Result<()> {
        if self.flushed {
            return Ok(());
        }
        // Pad final partial block with zeros and emit one last packet
        // so total emitted samples cover the input.
        let pending = self.input_buf[0].len();
        if pending > 0 {
            for ch in 0..self.channels as usize {
                self.input_buf[ch].resize(self.blocksize, 0.0);
            }
            self.drain_blocks(true);
        }
        self.flushed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identification::parse_identification_header;
    use crate::setup::parse_setup;

    #[test]
    fn identification_header_roundtrip() {
        let bytes = build_identification_header(2, 48_000, 128_000, 8, 11);
        let id = parse_identification_header(&bytes).expect("parse");
        assert_eq!(id.audio_channels, 2);
        assert_eq!(id.audio_sample_rate, 48_000);
        assert_eq!(id.bitrate_nominal, 128_000);
        assert_eq!(id.blocksize_0, 8);
        assert_eq!(id.blocksize_1, 11);
    }

    #[test]
    fn comment_header_signature() {
        let bytes = build_comment_header(&["TITLE=Test".to_string()]);
        assert_eq!(bytes[0], 0x03);
        assert_eq!(&bytes[1..7], b"vorbis");
        // Last byte is framing bit.
        assert_eq!(*bytes.last().unwrap() & 0x01, 0x01);
    }

    #[test]
    fn placeholder_setup_parses() {
        let bytes = build_placeholder_setup_header(1);
        // Feed through our own parser to verify it's syntactically valid.
        let setup = parse_setup(&bytes, 1).expect("our placeholder setup must parse");
        assert_eq!(setup.codebooks.len(), 1);
        assert_eq!(setup.floors.len(), 1);
        assert_eq!(setup.residues.len(), 1);
        assert_eq!(setup.mappings.len(), 1);
        assert_eq!(setup.modes.len(), 1);
    }

    #[test]
    fn extradata_lacing_splits_back() {
        let id = build_identification_header(1, 48_000, 0, 8, 11);
        let comm = build_comment_header(&[]);
        let setup = build_placeholder_setup_header(1);
        let blob = build_extradata(&id, &comm, &setup);
        assert_eq!(blob[0], 2); // packet count - 1

        // Decode via the same Xiph lacing the decoder uses.
        let n_packets = blob[0] as usize + 1;
        let mut sizes = Vec::new();
        let mut i = 1usize;
        for _ in 0..n_packets - 1 {
            let mut s = 0usize;
            loop {
                let b = blob[i];
                i += 1;
                s += b as usize;
                if b < 255 {
                    break;
                }
            }
            sizes.push(s);
        }
        sizes.push(blob.len() - i - sizes.iter().sum::<usize>());
        assert_eq!(sizes[0], id.len());
        assert_eq!(sizes[1], comm.len());
        assert_eq!(sizes[2], setup.len());
    }

    #[test]
    fn make_encoder_emits_headers() {
        let mut params = CodecParameters::audio(CodecId::new(crate::CODEC_ID_STR));
        params.channels = Some(1);
        params.sample_rate = Some(48_000);
        let enc = make_encoder(&params).expect("make_encoder");
        assert!(!enc.output_params().extradata.is_empty());
    }

    #[test]
    fn send_frame_emits_silent_packet_per_block() {
        let mut params = CodecParameters::audio(CodecId::new(crate::CODEC_ID_STR));
        params.channels = Some(1);
        params.sample_rate = Some(48_000);
        let mut enc = make_encoder(&params).unwrap();
        // Send exactly one block of S16 silence.
        let block = 1usize << DEFAULT_BLOCKSIZE_SHORT_LOG2;
        let frame = Frame::Audio(AudioFrame {
            format: SampleFormat::S16,
            channels: 1,
            sample_rate: 48_000,
            samples: block as u32,
            pts: Some(0),
            time_base: TimeBase::new(1, 48_000),
            data: vec![vec![0u8; block * 2]],
        });
        enc.send_frame(&frame).expect("send_frame");
        let pkt = enc.receive_packet().expect("packet");
        assert_eq!(pkt.pts, Some(0));
        assert_eq!(pkt.duration, Some(block as i64));
        // Packet body: 1 header bit + 1 floor-nonzero bit = 2 bits → 1 byte.
        assert_eq!(pkt.data.len(), 1);
        // Both bits zero.
        assert_eq!(pkt.data[0], 0);
        // No more packets pending until next send_frame.
        assert!(matches!(enc.receive_packet(), Err(Error::NeedMore)));
    }

    #[test]
    fn flush_emits_final_padded_packet() {
        let mut params = CodecParameters::audio(CodecId::new(crate::CODEC_ID_STR));
        params.channels = Some(2);
        params.sample_rate = Some(48_000);
        let mut enc = make_encoder(&params).unwrap();
        // Send less than one block — encoder buffers but emits nothing.
        let frame = Frame::Audio(AudioFrame {
            format: SampleFormat::S16,
            channels: 2,
            sample_rate: 48_000,
            samples: 64,
            pts: Some(0),
            time_base: TimeBase::new(1, 48_000),
            data: vec![vec![0u8; 64 * 4]],
        });
        enc.send_frame(&frame).unwrap();
        assert!(matches!(enc.receive_packet(), Err(Error::NeedMore)));
        enc.flush().unwrap();
        // After flush, a final padded packet appears.
        let pkt = enc.receive_packet().expect("flush emits packet");
        assert_eq!(pkt.pts, Some(0));
        // Then EOF.
        assert!(matches!(enc.receive_packet(), Err(Error::Eof)));
    }

    #[test]
    fn roundtrip_silence_via_our_decoder() {
        // Encode N blocks of silence, then decode the resulting packets
        // through our own VorbisDecoder. Output should be silent PCM
        // matching the input sample count (modulo Vorbis's first-packet
        // discard).
        use crate::decoder::make_decoder as make_dec;
        let mut params = CodecParameters::audio(CodecId::new(crate::CODEC_ID_STR));
        params.channels = Some(1);
        params.sample_rate = Some(48_000);
        let mut enc = make_encoder(&params).unwrap();

        let block = 1usize << DEFAULT_BLOCKSIZE_SHORT_LOG2;
        let n_blocks = 4;
        let frame = Frame::Audio(AudioFrame {
            format: SampleFormat::S16,
            channels: 1,
            sample_rate: 48_000,
            samples: (block * n_blocks) as u32,
            pts: Some(0),
            time_base: TimeBase::new(1, 48_000),
            data: vec![vec![0u8; block * n_blocks * 2]],
        });
        enc.send_frame(&frame).unwrap();
        enc.flush().unwrap();

        let mut packets = Vec::new();
        while let Ok(p) = enc.receive_packet() {
            packets.push(p);
        }
        assert_eq!(packets.len(), n_blocks);

        // Round-trip via our decoder using the encoder's emitted extradata.
        let mut dec_params = enc.output_params().clone();
        dec_params.extradata = enc.output_params().extradata.clone();
        let mut dec = make_dec(&dec_params).expect("decoder accepts our extradata");

        let mut emitted = 0usize;
        for pkt in &packets {
            dec.send_packet(pkt).expect("send_packet");
            if let Ok(Frame::Audio(a)) = dec.receive_frame() {
                emitted += a.samples as usize;
                // All bytes should be zero (silence).
                for plane in &a.data {
                    assert!(plane.iter().all(|&b| b == 0), "expected silence");
                }
            }
        }
        // First packet emits 0 samples (warm-up); remaining n-1 emit
        // n_half samples each.
        assert!(emitted > 0, "expected some samples decoded");
    }
}
