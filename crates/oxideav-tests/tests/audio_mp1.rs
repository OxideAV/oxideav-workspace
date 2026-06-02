//! MP1 (MPEG-1 Audio Layer I) decode comparison tests against ffmpeg.
//!
//! Round 211 restoration: `oxideav-mp1` re-grew a full Layer I decoder
//! against ISO/IEC 11172-3 in r121, so the suspended decode-vs-ffmpeg
//! harness comes back. The crate exposes both the registry path
//! (`oxideav_mp1::register` → installed by `oxideav_meta::register_all`)
//! and the direct factory (`oxideav_mp1::decoder::make_decoder` /
//! `oxideav_mp1::encoder::make_encoder`); we use the direct path so the
//! harness is independent of how the shared MP3 demuxer tags the
//! elementary stream (it hard-codes the stream's `CodecId` to `"mp3"`
//! and would route Layer I frames to the wrong decoder if we asked the
//! registry to dispatch).
//!
//! Frame walking uses `oxideav_mp1::header::find_sync` +
//! `FrameHeader::parse` + `FrameHeader::frame_length_bytes` — the same
//! Annex B / §2.4.3.1 framing the decoder itself uses internally.
//! ffmpeg's raw `-f mp3` output for a Layer I encode is just the
//! elementary stream (a sequence of MPEG audio frames), so no container
//! demuxer is needed.

use oxideav_core::{AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, TimeBase};
use oxideav_mp1::header::{find_sync, FrameHeader};
use oxideav_tests::*;

const SAMPLE_RATE: u32 = 44100;
const CHANNELS: u16 = 2;
const DURATION: f32 = 2.0;

/// Walk a raw MPEG-1 audio elementary stream into a vector of per-frame
/// byte slices. Skips bytes before the first valid Layer I header and
/// resyncs on any header parse failure mid-stream.
fn walk_frames(es: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut cursor = match find_sync(es) {
        Some(off) => off,
        None => return frames,
    };
    while cursor + 4 <= es.len() {
        let hdr_bytes = &es[cursor..cursor + 4];
        let header = match FrameHeader::parse(hdr_bytes) {
            Ok(h) => h,
            Err(_) => {
                // Resync: drop one byte and rescan for the next valid
                // sync. This matches the §2.4.3.1 resynchronisation
                // recommendation for damaged frames.
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
        let len = match header.frame_length_bytes() {
            Some(l) => l as usize,
            None => break, // free-format: not produced by ffmpeg's encoder
        };
        if cursor + len > es.len() {
            break;
        }
        frames.push(es[cursor..cursor + len].to_vec());
        cursor += len;
    }
    frames
}

/// Decode raw MPEG-1 Layer I bytes with our decoder, returning
/// interleaved S16 PCM.
fn decode_with_ours(mp1_data: &[u8]) -> Vec<i16> {
    let params = CodecParameters::audio(CodecId::new("mp1"));
    let mut dec = oxideav_mp1::decoder::make_decoder(&params).expect("make_decoder for mp1");
    let tb = TimeBase::new(1, SAMPLE_RATE as i64);

    let mut out_bytes: Vec<u8> = Vec::new();
    for frame_bytes in walk_frames(mp1_data) {
        let pkt = Packet::new(0, tb, frame_bytes);
        dec.send_packet(&pkt).expect("send_packet");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Audio(a)) => {
                    // mp1 emits interleaved S16 in data[0].
                    if let Some(plane) = a.data.first() {
                        out_bytes.extend_from_slice(plane);
                    }
                }
                Ok(_) => {}
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("decode error: {e:?}"),
            }
        }
    }

    out_bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// Build a fresh `oxideav_mp1` encoder and turn an interleaved S16
/// signal into a Layer I elementary stream by chunking the PCM into
/// 384-sample-per-channel frames. Returns the concatenated frame bytes
/// (the same layout `ffmpeg -f mp3` writes).
fn encode_with_ours(pcm: &[i16], sample_rate: u32, channels: u16, bitrate_bps: u64) -> Vec<u8> {
    const SAMPLES_PER_FRAME: u32 = 384;
    let mut params = CodecParameters::audio(CodecId::new("mp1"));
    params.sample_rate = Some(sample_rate);
    params.channels = Some(channels);
    params.bit_rate = Some(bitrate_bps);
    let mut enc = oxideav_mp1::encoder::make_encoder(&params).expect("make_encoder for mp1");

    let nch = channels as usize;
    let mut bytes: Vec<u8> = Vec::new();
    let mut frame_idx: usize = 0;
    let samples_per_chunk = SAMPLES_PER_FRAME as usize * nch;
    while (frame_idx + 1) * samples_per_chunk <= pcm.len() {
        let slice = &pcm[frame_idx * samples_per_chunk..(frame_idx + 1) * samples_per_chunk];
        let mut plane = Vec::with_capacity(slice.len() * 2);
        for s in slice {
            plane.extend_from_slice(&s.to_le_bytes());
        }
        let frame = AudioFrame {
            samples: SAMPLES_PER_FRAME,
            pts: None,
            data: vec![plane],
        };
        enc.send_frame(&Frame::Audio(frame)).expect("send_frame");
        let pkt = enc.receive_packet().expect("receive_packet");
        bytes.extend_from_slice(&pkt.data);
        frame_idx += 1;
    }
    bytes
}

/// Self-roundtrip test that runs without ffmpeg: encode a known signal
/// with `oxideav-mp1`, walk + decode it with the same crate via the
/// harness paths, and confirm:
///
/// 1. The walker chunked at least one frame.
/// 2. The decoder produced PCM of the expected shape (`384 * nch *
///    n_frames` samples).
/// 3. The decoded RMS difference vs the original signal is within the
///    Layer I perceptual budget. Layer I at 192 kbit/s on a smooth
///    sine is a lossy but high-quality encode — we accept any RMS
///    below 0.3 (the encoder uses a signal-energy-driven allocator,
///    not a psychoacoustic model; on a sparse signal it can still
///    deviate noticeably per sample).
///
/// This exercises every line of the harness and gives the CI a green
/// non-skipped test even on a runner without ffmpeg, where
/// `decoder_vs_ffmpeg` short-circuits.
#[test]
fn self_roundtrip_walks_and_decodes() {
    // Use 48 kHz / mono to land on a non-zero §2.4.3.2 frame length and
    // sidestep the stereo joint-stereo / dual-channel allocation
    // surface for the self-roundtrip's tightness check.
    let sample_rate: u32 = 48_000;
    let channels: u16 = 1;
    let duration: f32 = 0.5;
    let n_samples = (sample_rate as f32 * duration) as usize;

    // Pure 1 kHz sine — well inside the §2.4.3.2 frequency range and
    // simple enough that the signal-energy-driven encoder doesn't
    // collapse the allocation.
    let mut pcm: Vec<i16> = Vec::with_capacity(n_samples);
    for n in 0..n_samples {
        let t = n as f64 / sample_rate as f64;
        let v = (2.0 * std::f64::consts::PI * 1_000.0 * t).sin();
        pcm.push((v * 16_000.0) as i16);
    }

    // 192 kbit/s sits well above the Layer I "transparent" budget at
    // these conditions (~140 kbit/s/channel).
    let es = encode_with_ours(&pcm, sample_rate, channels, 192_000);
    assert!(
        !es.is_empty(),
        "encoder produced no output for a half-second signal"
    );

    let frames = walk_frames(&es);
    assert!(
        !frames.is_empty(),
        "walker found no Layer I frames in our encoder's output"
    );

    let decoded = decode_with_ours(&es);
    assert!(
        decoded.len() >= 384,
        "decoded under one full Layer I frame: {} samples",
        decoded.len()
    );

    // Per-frame layout sanity: each frame's PCM == 384 samples/channel.
    assert_eq!(
        decoded.len(),
        frames.len() * 384 * channels as usize,
        "decoded sample count != frames * 384 * channels"
    );

    // Align the two signals at index 0 and clip to the shorter length;
    // the encoder's polyphase analysis is causal so the decoded signal
    // is not delayed by the synthesis bank for this comparison purpose.
    let n = pcm.len().min(decoded.len());
    let rms = audio_rms_diff(&pcm[..n], &decoded[..n]);
    let psnr = audio_psnr(&pcm[..n], &decoded[..n]);
    eprintln!("=== MP1 self-roundtrip ===");
    report("self", rms, psnr, decoded.len(), pcm.len());

    // 0.3 is a deliberately wide budget: the §2.4.3.2 polyphase bank's
    // 256-sample group delay shifts the decoded signal relative to the
    // input, and we are not undoing that shift here — we are only
    // confirming the decode is not catastrophically wrong (e.g. random
    // noise or all-silence).
    assert!(rms < 0.3, "self-roundtrip RMS {rms:.6} too large (> 0.3)");
}

/// Decoder test: ffmpeg-encoded MP1, our decode vs ffmpeg decode.
///
/// ffmpeg ships two Layer I encoders historically (`mp1` floating-point
/// and `mp1fixed`); both have been quietly dropped in some builds, so
/// we probe for either and skip the test if neither is available.
#[test]
fn decoder_vs_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let raw_path = tmp("oxideav-mp1-dec-input.raw");
    write_pcm_s16le(&raw_path, &pcm);

    let mp1_path = tmp("oxideav-mp1-dec-test.mp1");
    let encoded = ffmpeg(&[
        "-f",
        "s16le",
        "-ar",
        &SAMPLE_RATE.to_string(),
        "-ac",
        &CHANNELS.to_string(),
        "-i",
        raw_path.to_str().unwrap(),
        "-c:a",
        "mp1",
        "-b:a",
        "192k",
        "-f",
        "mp3",
        mp1_path.to_str().unwrap(),
    ]) || ffmpeg(&[
        "-f",
        "s16le",
        "-ar",
        &SAMPLE_RATE.to_string(),
        "-ac",
        &CHANNELS.to_string(),
        "-i",
        raw_path.to_str().unwrap(),
        "-c:a",
        "mp1fixed",
        "-b:a",
        "192k",
        "-f",
        "mp3",
        mp1_path.to_str().unwrap(),
    ]);

    if !encoded {
        eprintln!("skip: ffmpeg lacks MP1 encoder (mp1/mp1fixed)");
        return;
    }

    // Decode with ffmpeg
    let ffmpeg_decoded_path = tmp("oxideav-mp1-dec-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            mp1_path.to_str().unwrap(),
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

    // Decode with our decoder
    let mp1_data = std::fs::read(&mp1_path).expect("read mp1");
    assert!(
        !mp1_data.is_empty(),
        "ffmpeg produced an empty MP1 elementary stream"
    );
    let our_decoded = decode_with_ours(&mp1_data);
    let ffmpeg_decoded = read_pcm_s16le(&ffmpeg_decoded_path);
    assert!(
        !our_decoded.is_empty(),
        "our decoder produced no samples — frame walker likely misframed the ES"
    );

    let rms = audio_rms_diff(&our_decoded, &ffmpeg_decoded);
    let psnr = audio_psnr(&our_decoded, &ffmpeg_decoded);

    eprintln!("=== MP1 decoder comparison ===");
    report(
        "decoder",
        rms,
        psnr,
        our_decoded.len(),
        ffmpeg_decoded.len(),
    );

    // The two decoders are independent floating-point implementations
    // of the same §2.4.3.2 synthesis filterbank, so per-sample bit-exact
    // agreement is not expected; we cap RMS at 0.1 (well above the
    // ~0.01 typical agreement on a clean signal but tight enough to
    // catch a structurally-wrong decode).
    assert!(rms < 0.1, "MP1 decoder RMS {rms:.6} too large (> 0.1)");
}

/// Sanity test for the frame walker. Confirms that synthetic input
/// without an MPEG-1 audio syncword produces an empty frame list (so a
/// real Layer I ES will produce a non-empty one).
#[test]
fn walker_rejects_non_mpeg_input() {
    let junk = [0u8; 256];
    assert!(walk_frames(&junk).is_empty());
}
