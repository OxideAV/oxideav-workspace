//! MP2 (MPEG-1/2 Audio Layer II) comparison tests against ffmpeg.
//!
//! Round 445 restoration: `oxideav-mp2` re-grew a complete Layer II
//! codec against the staged ISO/IEC 11172-3 / 13818-3 specification
//! (both directions, all six sample rates), so the harness suspended
//! during the 2026-05-24 clean-room reset comes back.
//!
//! Like the MP1 harness, we use the direct factories
//! (`oxideav_mp2::codec_decoder::make_decoder` /
//! `oxideav_mp2::codec_encoder::make_encoder`) rather than the shared
//! MP3 demuxer path: the demuxer hard-codes the stream's `CodecId` to
//! `"mp3"` and would route Layer II frames to the wrong decoder.
//! Frame walking uses `oxideav_mp2::header::find_sync` +
//! `FrameHeader::parse` + `FrameHeader::frame_size_bytes` — the same
//! §2.4.3.1 framing the decoder itself uses internally.
//!
//! PCM layout note: the mp2 adapters speak **planar** S16 on both
//! sides (`data.len() == channels`), unlike mp1's interleaved single
//! plane — the helpers below convert to/from the interleaved layout
//! the shared metrics use.

use oxideav_core::{AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, TimeBase};
use oxideav_mp2::header::{find_sync, FrameHeader};
use oxideav_tests::*;

const SAMPLE_RATE: u32 = 44100;
const CHANNELS: u16 = 2;
const DURATION: f32 = 2.0;

/// Samples per channel per Layer II frame (§2.4.2.1: 1152 for Layer II).
const SAMPLES_PER_FRAME: usize = 1152;

/// Walk a raw MPEG audio elementary stream into per-frame byte slices.
/// Skips bytes before the first valid Layer II header and resyncs on
/// any parse failure mid-stream (§2.4.3.1 resynchronisation).
fn walk_frames(es: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut cursor = match find_sync(es) {
        Some(off) => off,
        None => return frames,
    };
    while cursor + 4 <= es.len() {
        let header = match FrameHeader::parse(&es[cursor..cursor + 4]) {
            Ok(h) => h,
            Err(_) => {
                let tail = &es[cursor + 1..];
                match find_sync(tail) {
                    Some(off) => {
                        cursor += 1 + off;
                        continue;
                    }
                    None => break,
                }
            }
        };
        let len = header.frame_size_bytes();
        if len == 0 || cursor + len > es.len() {
            break; // free-format or truncated tail: not produced here
        }
        frames.push(es[cursor..cursor + len].to_vec());
        cursor += len;
    }
    frames
}

/// Interleave the decoder's planar S16 planes into a single buffer.
fn interleave_planes(planes: &[Vec<u8>]) -> Vec<i16> {
    let per_ch: Vec<Vec<i16>> = planes
        .iter()
        .map(|p| {
            p.chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect()
        })
        .collect();
    let n = per_ch.iter().map(Vec::len).min().unwrap_or(0);
    let mut out = Vec::with_capacity(n * per_ch.len());
    for i in 0..n {
        for ch in &per_ch {
            out.push(ch[i]);
        }
    }
    out
}

/// Decode raw Layer II bytes with our decoder, returning interleaved
/// S16 PCM.
fn decode_with_ours(mp2_data: &[u8], sample_rate: u32, channels: u16) -> Vec<i16> {
    let mut params = CodecParameters::audio(CodecId::new("mp2"));
    params.sample_rate = Some(sample_rate);
    params.channels = Some(channels);
    let mut dec = oxideav_mp2::codec_decoder::make_decoder(&params).expect("make_decoder for mp2");
    let tb = TimeBase::new(1, sample_rate as i64);

    let mut out = Vec::new();
    for frame_bytes in walk_frames(mp2_data) {
        let pkt = Packet::new(0, tb, frame_bytes);
        dec.send_packet(&pkt).expect("send_packet");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Audio(a)) => out.extend(interleave_planes(&a.data)),
                Ok(_) => {}
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("decode error: {e:?}"),
            }
        }
    }
    out
}

/// Encode interleaved S16 PCM into a Layer II elementary stream with
/// our encoder (planar input planes, 1152-sample chunks, flush drains
/// the tail).
fn encode_with_ours(pcm: &[i16], sample_rate: u32, channels: u16, bitrate_bps: u64) -> Vec<u8> {
    let mut params = CodecParameters::audio(CodecId::new("mp2"));
    params.sample_rate = Some(sample_rate);
    params.channels = Some(channels);
    params.bit_rate = Some(bitrate_bps);
    let mut enc = oxideav_mp2::codec_encoder::make_encoder(&params).expect("make_encoder for mp2");

    let nch = channels as usize;
    let chunk_len = SAMPLES_PER_FRAME * nch;
    for chunk in pcm.chunks(chunk_len) {
        // De-interleave into per-channel planes.
        let mut planes: Vec<Vec<u8>> = vec![Vec::with_capacity(chunk.len() / nch * 2); nch];
        for (i, s) in chunk.iter().enumerate() {
            planes[i % nch].extend_from_slice(&s.to_le_bytes());
        }
        let frame = AudioFrame {
            samples: (chunk.len() / nch) as u32,
            pts: None,
            data: planes,
        };
        enc.send_frame(&Frame::Audio(frame)).expect("send_frame");
    }
    enc.flush().expect("flush");

    let mut bytes = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(pkt) => bytes.extend_from_slice(&pkt.data),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    }
    bytes
}

/// Self-roundtrip that runs without ffmpeg: our encode → walk → our
/// decode, shape and fidelity assertions.
#[test]
fn self_roundtrip_walks_and_decodes() {
    let sample_rate: u32 = 48_000;
    let channels: u16 = 1;
    let n_samples = (sample_rate / 2) as usize; // 0.5 s

    // Pure 1 kHz sine — simple enough that the psychoacoustic
    // allocation keeps the encode transparent at 192 kbit/s mono.
    let mut pcm: Vec<i16> = Vec::with_capacity(n_samples);
    for n in 0..n_samples {
        let t = n as f64 / sample_rate as f64;
        pcm.push(((2.0 * std::f64::consts::PI * 1_000.0 * t).sin() * 16_000.0) as i16);
    }

    let es = encode_with_ours(&pcm, sample_rate, channels, 192_000);
    assert!(!es.is_empty(), "encoder produced no output");

    let frames = walk_frames(&es);
    assert!(
        !frames.is_empty(),
        "walker found no Layer II frames in our encoder's output"
    );

    let decoded = decode_with_ours(&es, sample_rate, channels);
    assert_eq!(
        decoded.len(),
        frames.len() * SAMPLES_PER_FRAME * channels as usize,
        "decoded sample count != frames * 1152 * channels"
    );

    // The §2.4.3.2 analysis/synthesis chain delays the signal by the
    // polyphase bank's group delay; search a bounded lag instead of
    // pinning index 0.
    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, channels, 2048);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== MP2 self-roundtrip ===");
    report("self", rms, psnr, decoded.len(), pcm.len());
    assert!(rms < 0.1, "self-roundtrip RMS {rms:.6} too large (> 0.1)");
}

/// Decoder test: ffmpeg-encoded MP2 (native `mp2` encoder), our decode
/// vs ffmpeg decode.
#[test]
fn decoder_vs_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let raw_path = tmp("oxideav-mp2-dec-input.raw");
    write_pcm_s16le(&raw_path, &pcm);

    let mp2_path = tmp("oxideav-mp2-dec-test.mp2");
    assert!(
        ffmpeg(&[
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            "-i",
            raw_path.to_str().unwrap(),
            "-c:a",
            "mp2",
            "-b:a",
            "192k",
            "-f",
            "mp2",
            mp2_path.to_str().unwrap(),
        ]),
        "ffmpeg mp2 encode failed"
    );

    let ffmpeg_decoded_path = tmp("oxideav-mp2-dec-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            mp2_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            ffmpeg_decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg decode failed"
    );

    let mp2_data = std::fs::read(&mp2_path).expect("read mp2");
    let our_decoded = decode_with_ours(&mp2_data, SAMPLE_RATE, CHANNELS);
    let ffmpeg_decoded = read_pcm_s16le(&ffmpeg_decoded_path);
    assert!(
        !our_decoded.is_empty(),
        "our decoder produced no samples — frame walker likely misframed the ES"
    );

    let rms = audio_rms_diff(&our_decoded, &ffmpeg_decoded);
    let psnr = audio_psnr(&our_decoded, &ffmpeg_decoded);
    eprintln!("=== MP2 decoder comparison ===");
    report(
        "decoder",
        rms,
        psnr,
        our_decoded.len(),
        ffmpeg_decoded.len(),
    );

    // Two independent implementations of the §2.4.3.2 synthesis bank:
    // not bit-exact, but structurally-equal decodes agree closely.
    assert!(rms < 0.1, "MP2 decoder RMS {rms:.6} too large (> 0.1)");
}

/// Encoder test: our encode, ffmpeg decode, compare against the input
/// signal within the Layer II perceptual budget.
#[test]
fn encoder_vs_ffmpeg_decode() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let es = encode_with_ours(&pcm, SAMPLE_RATE, CHANNELS, 192_000);
    assert!(!es.is_empty(), "our encoder produced no output");

    let mp2_path = tmp("oxideav-mp2-enc-test.mp2");
    std::fs::write(&mp2_path, &es).expect("write mp2");

    let decoded_path = tmp("oxideav-mp2-enc-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            mp2_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our Layer II elementary stream"
    );

    let decoded = read_pcm_s16le(&decoded_path);
    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, CHANNELS, 4096);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== MP2 encoder comparison ===");
    report("encoder", rms, psnr, decoded.len(), pcm.len());

    // 192 kbit/s stereo is a mid-quality Layer II operating point on a
    // busy signal (sine + chirp + click): allow a lossy-but-sane budget.
    assert!(rms < 0.15, "MP2 encoder RMS {rms:.6} too large (> 0.15)");
}

/// Walker sanity: input without a §2.4.3.1 syncword yields no frames.
#[test]
fn walker_rejects_non_mpeg_input() {
    assert!(walk_frames(&[0u8; 256]).is_empty());
}
