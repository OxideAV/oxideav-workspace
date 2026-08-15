//! AAC (ADTS-framed AAC-LC) comparison tests against ffmpeg.
//!
//! Round 445 restoration: `oxideav-aac` re-grew an AAC codec against
//! the staged ISO/IEC 13818-7 / 14496-3 specifications (LC/HE/LD/ER
//! decode plus an LC encoder), so the harness suspended during the
//! 2026-05-24 clean-room reset comes back.
//!
//! Both directions ride the ADTS elementary-stream framing:
//!
//! * decode — `oxideav_aac::codec_decoder::make_decoder`; a packet may
//!   carry one or more complete ADTS frames, so the whole stream goes
//!   in as a single packet and frames drain per access unit;
//! * encode — `oxideav_aac::codec_encoder::make_encoder`; interleaved
//!   S16 in, one ADTS frame per packet out (1024-sample hops), flush
//!   pads the tail and appends the overlap-drain frame.
//!
//! Fidelity comparisons use a bounded lag search: the §4.6.11
//! overlap-add filterbank imposes a one-frame warmup on the encode
//! side, so index-0 alignment would measure phase, not fidelity.

use oxideav_core::{AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, TimeBase};
use oxideav_tests::*;

const SAMPLE_RATE: u32 = 44100;
const CHANNELS: u16 = 2;
const DURATION: f32 = 2.0;

/// AAC access-unit size for the default frame family.
const FRAME_LEN: usize = 1024;

/// Decode an ADTS elementary stream with our decoder, returning
/// interleaved S16 PCM (the aac adapter's output layout: one plane,
/// interleaved little-endian `i16`).
fn decode_with_ours(adts_data: &[u8], sample_rate: u32, channels: u16) -> Vec<i16> {
    let mut params = CodecParameters::audio(CodecId::new("aac"));
    params.sample_rate = Some(sample_rate);
    params.channels = Some(channels);
    let mut dec = oxideav_aac::codec_decoder::make_decoder(&params).expect("make_decoder for aac");

    let tb = TimeBase::new(1, sample_rate as i64);
    let pkt = Packet::new(0, tb, adts_data.to_vec());
    dec.send_packet(&pkt).expect("send_packet");
    dec.flush().expect("flush");

    let mut out = Vec::new();
    loop {
        match dec.receive_frame() {
            Ok(Frame::Audio(a)) => {
                for chunk in a.data[0].chunks_exact(2) {
                    out.push(i16::from_le_bytes([chunk[0], chunk[1]]));
                }
            }
            Ok(_) => {}
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("decode error: {e:?}"),
        }
    }
    out
}

/// Encode interleaved S16 PCM into an ADTS elementary stream with our
/// AAC-LC encoder.
fn encode_with_ours(pcm: &[i16], sample_rate: u32, channels: u16, bitrate_bps: u64) -> Vec<u8> {
    let mut params = CodecParameters::audio(CodecId::new("aac"));
    params.sample_rate = Some(sample_rate);
    params.channels = Some(channels);
    params.bit_rate = Some(bitrate_bps);
    let mut enc = oxideav_aac::codec_encoder::make_encoder(&params).expect("make_encoder for aac");

    let nch = channels as usize;
    let mut bytes = Vec::new();
    let drain = |enc: &mut Box<dyn oxideav_core::Encoder>, bytes: &mut Vec<u8>| loop {
        match enc.receive_packet() {
            Ok(pkt) => bytes.extend_from_slice(&pkt.data),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    };
    for chunk in pcm.chunks(FRAME_LEN * nch) {
        let plane: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
        let frame = AudioFrame {
            samples: (chunk.len() / nch) as u32,
            pts: None,
            data: vec![plane],
        };
        enc.send_frame(&Frame::Audio(frame)).expect("send_frame");
        drain(&mut enc, &mut bytes);
    }
    enc.flush().expect("flush");
    drain(&mut enc, &mut bytes);
    bytes
}

/// Self-roundtrip that runs without ffmpeg: our encode → our decode,
/// ADTS shape + fidelity assertions.
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

    let es = encode_with_ours(&pcm, sample_rate, channels, 96_000);
    assert!(!es.is_empty(), "encoder produced no output");

    // The stream must start on an ADTS syncword our own parser accepts.
    let (hdr, _consumed) = oxideav_aac::adts::AdtsHeader::parse(&es).expect("leading ADTS header");
    assert_eq!(hdr.sample_rate(), sample_rate);

    let decoded = decode_with_ours(&es, sample_rate, channels);
    assert!(
        decoded.len() >= FRAME_LEN,
        "decoded under one AAC frame: {} samples",
        decoded.len()
    );

    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, channels, 4 * FRAME_LEN);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== AAC self-roundtrip ===");
    report("self", rms, psnr, decoded.len(), pcm.len());
    assert!(rms < 0.1, "self-roundtrip RMS {rms:.6} too large (> 0.1)");
}

/// Decoder test: ffmpeg-encoded ADTS AAC (native encoder, always
/// built), our decode vs ffmpeg decode.
#[test]
fn decoder_vs_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let raw_path = tmp("oxideav-aac-dec-input.raw");
    write_pcm_s16le(&raw_path, &pcm);

    let adts_path = tmp("oxideav-aac-dec-test.aac");
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
            "aac",
            "-b:a",
            "128k",
            "-f",
            "adts",
            adts_path.to_str().unwrap(),
        ]),
        "ffmpeg aac encode failed"
    );

    let ffmpeg_decoded_path = tmp("oxideav-aac-dec-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            adts_path.to_str().unwrap(),
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

    let adts_data = std::fs::read(&adts_path).expect("read adts");
    let our_decoded = decode_with_ours(&adts_data, SAMPLE_RATE, CHANNELS);
    let ffmpeg_decoded = read_pcm_s16le(&ffmpeg_decoded_path);
    assert!(!our_decoded.is_empty(), "our decoder produced no samples");

    // Raw ADTS carries no priming metadata, but ffmpeg's own decode may
    // still trim the encoder warmup — align within a bounded window.
    let (rms, lag) = audio_rms_diff_aligned(&ffmpeg_decoded, &our_decoded, CHANNELS, 4 * FRAME_LEN);
    let psnr = audio_psnr(&ffmpeg_decoded, &our_decoded[lag..]);
    eprintln!("=== AAC decoder comparison ===");
    report(
        "decoder",
        rms,
        psnr,
        our_decoded.len(),
        ffmpeg_decoded.len(),
    );
    assert!(rms < 0.1, "AAC decoder RMS {rms:.6} too large (> 0.1)");
}

/// Encoder test: our encode, ffmpeg decode, compare against the input
/// signal within the AAC-LC perceptual budget.
#[test]
fn encoder_vs_ffmpeg_decode() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let es = encode_with_ours(&pcm, SAMPLE_RATE, CHANNELS, 128_000);
    assert!(!es.is_empty(), "our encoder produced no output");

    let adts_path = tmp("oxideav-aac-enc-test.aac");
    std::fs::write(&adts_path, &es).expect("write adts");

    let decoded_path = tmp("oxideav-aac-enc-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            adts_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our ADTS stream"
    );

    let decoded = read_pcm_s16le(&decoded_path);
    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, CHANNELS, 8 * FRAME_LEN);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== AAC encoder comparison ===");
    report("encoder", rms, psnr, decoded.len(), pcm.len());

    // 128 kbit/s stereo AAC-LC on the busy test signal.
    assert!(rms < 0.2, "AAC encoder RMS {rms:.6} too large (> 0.2)");
}
