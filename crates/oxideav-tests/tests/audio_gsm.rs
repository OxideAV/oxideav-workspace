//! GSM Full Rate (06.10) roundtrip comparison tests against ffmpeg.
//!
//! Round 445 restoration: `oxideav-gsm` re-grew the complete RPE-LTP
//! system (encoder + decoder + DTX/VAD/comfort-noise) against the
//! staged ETSI GSM 06.10 specification, closing the workspace-task
//! #1029 unblock condition for this harness.
//!
//! Both directions use the direct factories
//! (`oxideav_gsm::make_encoder` / `oxideav_gsm::make_decoder`) with
//! `extradata = b"gsm"` selecting the de-facto 33-byte `.gsm`
//! byte-frame packing — the same framing ffmpeg's raw `.gsm` demuxer
//! reads, so the interop legs need no container work at all.

use oxideav_core::{AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, TimeBase};
use oxideav_tests::*;

const SAMPLE_RATE: u32 = 8000;
const CHANNELS: u16 = 1;
const DURATION: f32 = 2.0;

/// §1.5: 20 ms frames of 160 samples at 8 kHz.
const FRAME_SAMPLES: usize = 160;
/// De-facto `.gsm` byte-frame length.
const BYTE_FRAME_LEN: usize = 33;

fn gsm_params() -> CodecParameters {
    let mut params = CodecParameters::audio(CodecId::new("gsm"));
    params.sample_rate = Some(SAMPLE_RATE);
    params.channels = Some(1);
    params.extradata = b"gsm".to_vec(); // .gsm byte-frame packing
    params
}

/// Encode mono S16 PCM into concatenated 33-byte `.gsm` frames.
fn encode_with_ours(pcm: &[i16]) -> Vec<u8> {
    let mut enc = oxideav_gsm::make_encoder(&gsm_params()).expect("make_encoder for gsm");
    let mut bytes = Vec::new();
    for chunk in pcm.chunks(FRAME_SAMPLES) {
        let plane: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
        let frame = AudioFrame {
            samples: chunk.len() as u32,
            pts: None,
            data: vec![plane],
        };
        enc.send_frame(&Frame::Audio(frame)).expect("send_frame");
        loop {
            match enc.receive_packet() {
                Ok(pkt) => bytes.extend_from_slice(&pkt.data),
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("encode error: {e:?}"),
            }
        }
    }
    enc.flush().expect("flush");
    loop {
        match enc.receive_packet() {
            Ok(pkt) => bytes.extend_from_slice(&pkt.data),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    }
    bytes
}

/// Decode concatenated 33-byte `.gsm` frames with our decoder,
/// returning mono S16 PCM.
fn decode_with_ours(gsm_data: &[u8]) -> Vec<i16> {
    let mut dec = oxideav_gsm::make_decoder(&gsm_params()).expect("make_decoder for gsm");
    let tb = TimeBase::new(1, SAMPLE_RATE as i64);
    // Feed whole byte-frames; the adapter walks the payload in 33-byte
    // units, so the entire stream can ride one packet.
    let whole = gsm_data.len() - gsm_data.len() % BYTE_FRAME_LEN;
    let pkt = Packet::new(0, tb, gsm_data[..whole].to_vec());
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

/// Self-roundtrip that runs without ffmpeg: our encode → our decode,
/// framing shape + fidelity assertions.
#[test]
fn self_roundtrip_encodes_and_decodes() {
    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let n_frames = pcm.len() / FRAME_SAMPLES;

    let es = encode_with_ours(&pcm[..n_frames * FRAME_SAMPLES]);
    assert_eq!(
        es.len(),
        n_frames * BYTE_FRAME_LEN,
        "encoder must emit one 33-byte frame per 160 input samples"
    );
    // Every byte-frame leads with the 0xD marker nibble.
    for frame in es.chunks_exact(BYTE_FRAME_LEN) {
        assert_eq!(frame[0] >> 4, 0xD, "byte-frame missing 0xD marker nibble");
    }

    let decoded = decode_with_ours(&es);
    assert_eq!(
        decoded.len(),
        n_frames * FRAME_SAMPLES,
        "decoded sample count != frames * 160"
    );

    let rms = audio_rms_diff(&pcm, &decoded);
    let psnr = audio_psnr(&pcm, &decoded);
    eprintln!("=== GSM self-roundtrip ===");
    report("self", rms, psnr, decoded.len(), pcm.len());
    // 13 kbit/s RPE-LTP on a loud multi-tone signal is decidedly lossy;
    // the budget catches structural breakage, not perceptual polish.
    assert!(rms < 0.3, "self-roundtrip RMS {rms:.6} too large (> 0.3)");
}

/// Encoder test: our `.gsm` stream, decoded by ffmpeg's native GSM
/// decoder via the raw `.gsm` demuxer, compared against the input.
#[test]
fn encoder_vs_ffmpeg_decode() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let n_frames = pcm.len() / FRAME_SAMPLES;
    let clipped = &pcm[..n_frames * FRAME_SAMPLES];
    let es = encode_with_ours(clipped);

    let gsm_path = tmp("oxideav-gsm-enc-test.gsm");
    std::fs::write(&gsm_path, &es).expect("write gsm");

    let decoded_path = tmp("oxideav-gsm-enc-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-f",
            "gsm",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-i",
            gsm_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            "1",
            decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our .gsm byte-frame stream"
    );

    let decoded = read_pcm_s16le(&decoded_path);
    let rms = audio_rms_diff(clipped, &decoded);
    let psnr = audio_psnr(clipped, &decoded);
    eprintln!("=== GSM encoder comparison ===");
    report("encoder", rms, psnr, decoded.len(), clipped.len());
    assert!(rms < 0.3, "GSM encoder RMS {rms:.6} too large (> 0.3)");
}

/// Decoder test: ffmpeg-encoded `.gsm` (external-library encoder —
/// probe and skip when absent), our decode vs ffmpeg decode. GSM 06.10
/// decoding is a fixed-point bit-exact pipeline, so two conforming
/// decoders agree very closely.
#[test]
fn decoder_vs_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let raw_path = tmp("oxideav-gsm-dec-input.raw");
    write_pcm_s16le(&raw_path, &pcm);

    let gsm_path = tmp("oxideav-gsm-dec-test.gsm");
    if !ffmpeg(&[
        "-f",
        "s16le",
        "-ar",
        &SAMPLE_RATE.to_string(),
        "-ac",
        "1",
        "-i",
        raw_path.to_str().unwrap(),
        "-c:a",
        "libgsm",
        "-f",
        "gsm",
        gsm_path.to_str().unwrap(),
    ]) {
        eprintln!("skip: ffmpeg lacks a GSM encoder (libgsm)");
        return;
    }

    let ffmpeg_decoded_path = tmp("oxideav-gsm-dec-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-f",
            "gsm",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-i",
            gsm_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            "1",
            ffmpeg_decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg decode failed"
    );

    let gsm_data = std::fs::read(&gsm_path).expect("read gsm");
    let our_decoded = decode_with_ours(&gsm_data);
    let ffmpeg_decoded = read_pcm_s16le(&ffmpeg_decoded_path);
    assert!(!our_decoded.is_empty(), "our decoder produced no samples");

    let rms = audio_rms_diff(&our_decoded, &ffmpeg_decoded);
    let psnr = audio_psnr(&our_decoded, &ffmpeg_decoded);
    eprintln!("=== GSM decoder comparison ===");
    report(
        "decoder",
        rms,
        psnr,
        our_decoded.len(),
        ffmpeg_decoded.len(),
    );
    assert!(rms < 0.05, "GSM decoder RMS {rms:.6} too large (> 0.05)");
}
