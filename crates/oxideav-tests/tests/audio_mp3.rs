//! MP3 (MPEG-1/2 Audio Layer III) comparison tests against ffmpeg.
//!
//! Round 445 restoration: `oxideav-mp3` re-grew a full Layer III codec
//! (decoder: MPEG-1 + MPEG-2 LSF + MPEG-2.5, mono/stereo/joint;
//! encoder: CBR MPEG-1/2 with joint-stereo variants) against the
//! staged ISO/IEC 11172-3 / 13818-3 specification, so the harness
//! suspended during the 2026-05-24 clean-room reset comes back.
//!
//! Decode goes through the full registry path — the crate's own MP3
//! elementary-stream demuxer (`probe_input` → `open_demuxer` →
//! `first_decoder`) — so this doubles as a cross-crate container
//! integration test. Encode uses the direct
//! `oxideav_mp3::codec_encoder::make_encoder` factory; packets drop
//! out at `flush` time because the cross-frame bit-reservoir schedule
//! needs the whole stream (documented trait adaptation).
//!
//! Fidelity comparisons use a bounded lag search
//! (`audio_rms_diff_aligned`): Layer III's analysis + IMDCT chain
//! imposes an implementation-defined encoder/decoder delay, so a fixed
//! index-0 alignment would measure phase, not fidelity.

use oxideav_core::{AudioFrame, CodecId, CodecParameters, Error, Frame, ReadSeek, SampleFormat};
use oxideav_tests::*;

const SAMPLE_RATE: u32 = 44100;
const CHANNELS: u16 = 2;
const DURATION: f32 = 2.0;

/// Interleave planar S16 planes (the mp3 decoder's output layout:
/// `data[0]` = L, `data[1]` = R; mono keeps the single plane).
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

/// Decode an MP3 elementary stream via the registry: probe → demux
/// (one packet per MP3 frame) → `first_decoder`. Returns interleaved
/// S16 PCM.
fn decode_with_ours(mp3_data: &[u8]) -> Vec<i16> {
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_mp3::register(&mut reg);
    let mut file: Box<dyn ReadSeek> = Box::new(std::io::Cursor::new(mp3_data.to_vec()));
    let format = reg
        .containers
        .probe_input(&mut *file, Some("mp3"))
        .expect("probe mp3");
    let mut dmx = reg
        .containers
        .open_demuxer(&format, file, &oxideav_core::NullCodecResolver)
        .expect("open mp3 demuxer");
    let params = dmx.streams()[0].params.clone();
    let mut dec = reg.codecs.first_decoder(&params).expect("make mp3 decoder");
    let mut out = Vec::new();
    loop {
        let pkt = match dmx.next_packet() {
            Ok(p) => p,
            Err(Error::Eof) => break,
            Err(e) => panic!("demux error: {e:?}"),
        };
        dec.send_packet(&pkt).expect("send");
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

/// Encode interleaved S16 PCM to an MP3 elementary stream with our
/// encoder. The Layer III trait adaptation is flush-driven: packets
/// only drop out after `flush` (bit-reservoir scheduling).
fn encode_with_ours(pcm: &[i16], sample_rate: u32, channels: u16, bitrate_bps: u64) -> Vec<u8> {
    let mut params = CodecParameters::audio(CodecId::new("mp3"));
    params.sample_rate = Some(sample_rate);
    params.channels = Some(channels);
    params.bit_rate = Some(bitrate_bps);
    params.sample_format = Some(SampleFormat::S16);
    let mut enc = oxideav_mp3::codec_encoder::make_encoder(&params).expect("make_encoder for mp3");

    let nch = channels as usize;
    for chunk in pcm.chunks(1152 * nch) {
        let bytes: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
        let frame = AudioFrame {
            samples: (chunk.len() / nch) as u32,
            pts: None,
            data: vec![bytes],
        };
        enc.send_frame(&Frame::Audio(frame)).expect("send_frame");
        // NeedMore is the documented pre-flush answer (reservoir
        // scheduling needs the whole stream before any frame exists).
        match enc.receive_packet() {
            Err(Error::NeedMore | Error::Eof) => {}
            Ok(_) => panic!("mp3 encoder documented as flush-driven; got a packet before flush"),
            Err(e) => panic!("encode error: {e:?}"),
        }
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

/// Self-roundtrip that runs without ffmpeg: our encode → registry
/// demux → our decode, with a lag-tolerant fidelity budget.
#[test]
fn self_roundtrip_encodes_and_decodes() {
    let sample_rate: u32 = 44_100;
    let channels: u16 = 1;
    let n_samples = (sample_rate / 2) as usize; // 0.5 s

    let mut pcm: Vec<i16> = Vec::with_capacity(n_samples);
    for n in 0..n_samples {
        let t = n as f64 / sample_rate as f64;
        pcm.push(((2.0 * std::f64::consts::PI * 1_000.0 * t).sin() * 16_000.0) as i16);
    }

    let es = encode_with_ours(&pcm, sample_rate, channels, 128_000);
    assert!(!es.is_empty(), "encoder produced no output");

    let decoded = decode_with_ours(&es);
    assert!(
        decoded.len() >= 1152,
        "decoded under one Layer III frame: {} samples",
        decoded.len()
    );

    // Encoder delay + decoder synthesis delay: search a generous but
    // bounded lag window (a few frames).
    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, channels, 4 * 1152);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== MP3 self-roundtrip ===");
    report("self", rms, psnr, decoded.len(), pcm.len());
    assert!(rms < 0.1, "self-roundtrip RMS {rms:.6} too large (> 0.1)");
}

/// Decoder test: ffmpeg-encoded MP3, our decode vs ffmpeg decode.
/// ffmpeg encodes Layer III only through an external-library codec
/// (`libmp3lame` / `libshine`), so probe and skip when absent.
#[test]
fn decoder_vs_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let raw_path = tmp("oxideav-mp3-dec-input.raw");
    write_pcm_s16le(&raw_path, &pcm);

    let mp3_path = tmp("oxideav-mp3-dec-test.mp3");
    let encoded = ["libmp3lame", "libshine"].iter().any(|codec| {
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
            codec,
            "-b:a",
            "192k",
            "-f",
            "mp3",
            mp3_path.to_str().unwrap(),
        ])
    });
    if !encoded {
        eprintln!("skip: ffmpeg lacks an MP3 encoder (libmp3lame/libshine)");
        return;
    }

    let ffmpeg_decoded_path = tmp("oxideav-mp3-dec-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            mp3_path.to_str().unwrap(),
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

    let mp3_data = std::fs::read(&mp3_path).expect("read mp3");
    let our_decoded = decode_with_ours(&mp3_data);
    let ffmpeg_decoded = read_pcm_s16le(&ffmpeg_decoded_path);
    assert!(!our_decoded.is_empty(), "our decoder produced no samples");

    // ffmpeg trims the LAME-tag encoder delay/padding from its own
    // decode while our registry path emits every decoded granule, so
    // the two decodes are offset by the delay: align within a bounded
    // window before comparing.
    let (rms, lag) = audio_rms_diff_aligned(&ffmpeg_decoded, &our_decoded, CHANNELS, 4 * 1152);
    let psnr = audio_psnr(&ffmpeg_decoded, &our_decoded[lag..]);
    eprintln!("=== MP3 decoder comparison ===");
    report(
        "decoder",
        rms,
        psnr,
        our_decoded.len(),
        ffmpeg_decoded.len(),
    );
    assert!(rms < 0.1, "MP3 decoder RMS {rms:.6} too large (> 0.1)");
}

/// Encoder test: our encode, ffmpeg decode (native Layer III decoder,
/// always present), compare against the input signal.
#[test]
fn encoder_vs_ffmpeg_decode() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let es = encode_with_ours(&pcm, SAMPLE_RATE, CHANNELS, 192_000);
    assert!(!es.is_empty(), "our encoder produced no output");

    let mp3_path = tmp("oxideav-mp3-enc-test.mp3");
    std::fs::write(&mp3_path, &es).expect("write mp3");

    let decoded_path = tmp("oxideav-mp3-enc-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            mp3_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our Layer III elementary stream"
    );

    let decoded = read_pcm_s16le(&decoded_path);
    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, CHANNELS, 8 * 1152);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== MP3 encoder comparison ===");
    report("encoder", rms, psnr, decoded.len(), pcm.len());

    // 192 kbit/s stereo CBR on the busy test signal — lossy budget wide
    // enough for the fixed-gain rate loop, tight enough to catch a
    // structurally-wrong encode.
    assert!(rms < 0.2, "MP3 encoder RMS {rms:.6} too large (> 0.2)");
}
