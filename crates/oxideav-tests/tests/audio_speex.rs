//! Speex cross-crate tests: decoder vs ffmpeg + framework encoder
//! round-trip.
//!
//! Round 449 refresh: `oxideav-speex` ships narrowband and wideband
//! *encoders* alongside the decoder (framework factories under the
//! dual-API `make_decoder` / `make_encoder`), so the old "our crate
//! has no Speex encoder" prose was stale. The decode oracle leg keeps
//! the historical shape (ffmpeg's Ogg/Speex through our Ogg demuxer +
//! registry decoder); the encoder leg round-trips through the crate's
//! own framework factories. (The encoder's raw 20 ms packets are not
//! muxed to Ogg here — the Ogg muxer's Speex header synthesis expects
//! the two-packet header sequence, which the framework encoder's
//! single-blob `extradata` does not split yet.)

use oxideav_core::{Error, Frame};
use oxideav_tests::*;

const SAMPLE_RATE: u32 = 8000;
const CHANNELS: u16 = 1;
const DURATION: f32 = 2.0;

/// Decode an Ogg/Speex file with our decoder via demuxer.
fn decode_with_ours(ogg_data: &[u8]) -> Vec<i16> {
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_meta::register_all(&mut reg);
    let mut file: Box<dyn oxideav::core::ReadSeek> =
        Box::new(std::io::Cursor::new(ogg_data.to_vec()));
    let format = reg
        .containers
        .probe_input(&mut *file, Some("ogg"))
        .expect("probe ogg");
    let mut dmx = reg
        .containers
        .open_demuxer(&format, file, &oxideav_core::NullCodecResolver)
        .expect("open ogg demuxer");
    let params = dmx.streams()[0].params.clone();
    let mut dec = reg
        .codecs
        .first_decoder(&params)
        .expect("make speex decoder");
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
                Ok(Frame::Audio(a)) => {
                    let bytes = &a.data[0];
                    for chunk in bytes.chunks_exact(2) {
                        out.push(i16::from_le_bytes([chunk[0], chunk[1]]));
                    }
                }
                Ok(_) => {}
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("decode error: {e:?}"),
            }
        }
    }
    out
}

/// Decoder test: ffmpeg-encoded Speex, our decode vs ffmpeg decode.
#[test]
fn decoder_vs_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let raw_path = tmp("oxideav-speex-dec-input.raw");
    write_pcm_s16le(&raw_path, &pcm);

    // Encode with ffmpeg (libspeex)
    let ogg_path = tmp("oxideav-speex-dec-test.ogg");
    if !ffmpeg(&[
        "-f",
        "s16le",
        "-ar",
        &SAMPLE_RATE.to_string(),
        "-ac",
        &CHANNELS.to_string(),
        "-i",
        raw_path.to_str().unwrap(),
        "-c:a",
        "libspeex",
        "-b:a",
        "15k",
        ogg_path.to_str().unwrap(),
    ]) {
        eprintln!("skip: ffmpeg libspeex encode failed (libspeex may not be available)");
        return;
    }

    // Decode with ffmpeg
    let ffmpeg_decoded_path = tmp("oxideav-speex-dec-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            ogg_path.to_str().unwrap(),
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
    let ogg_data = std::fs::read(&ogg_path).expect("read ogg");
    let our_decoded = decode_with_ours(&ogg_data);
    let ffmpeg_decoded = read_pcm_s16le(&ffmpeg_decoded_path);

    let rms = audio_rms_diff(&our_decoded, &ffmpeg_decoded);
    let psnr = audio_psnr(&our_decoded, &ffmpeg_decoded);

    eprintln!("=== Speex decoder comparison ===");
    report(
        "decoder",
        rms,
        psnr,
        our_decoded.len(),
        ffmpeg_decoded.len(),
    );

    assert!(rms < 1.0, "Speex decoder RMS {rms:.6} too large (> 1.0)");
}

/// Framework encoder round-trip (no oracle): `make_encoder` → 20 ms
/// packets → `make_decoder` seeded from the encoder's `output_params`
/// (Speex header in `extradata`) → PCM compared against the input.
#[test]
fn encoder_framework_roundtrip() {
    use oxideav_core::{AudioFrame, CodecId, CodecParameters, SampleFormat};

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, 1.0);

    let mut params = CodecParameters::audio(CodecId::new("speex"));
    params.sample_rate = Some(SAMPLE_RATE);
    params.channels = Some(CHANNELS);
    params.sample_format = Some(SampleFormat::S16);
    let mut enc = oxideav_speex::make_encoder(&params).expect("make speex encoder");

    // Feed interleaved S16 in arbitrary hops; the encoder re-blocks
    // into 20 ms frames.
    for chunk in pcm.chunks(700) {
        let bytes: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
        let frame = AudioFrame {
            samples: (chunk.len() / CHANNELS as usize) as u32,
            pts: None,
            data: vec![bytes],
        };
        enc.send_frame(&Frame::Audio(frame)).expect("send frame");
    }
    enc.flush().expect("flush");
    let out_params = enc.output_params().clone();
    assert!(
        !out_params.extradata.is_empty(),
        "encoder must publish the Speex stream header"
    );

    let mut packets = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(p) => packets.push(p),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    }
    assert!(packets.len() > 40, "expected ~50 packets for 1 s of 8 kHz");

    let mut dec = oxideav_speex::make_decoder(&out_params).expect("make speex decoder");
    let mut decoded = Vec::new();
    for p in &packets {
        dec.send_packet(p).expect("send packet");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Audio(a)) => {
                    for chunk in a.data[0].chunks_exact(2) {
                        decoded.push(i16::from_le_bytes([chunk[0], chunk[1]]));
                    }
                }
                Ok(_) => {}
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("decode error: {e:?}"),
            }
        }
    }
    assert_eq!(
        decoded.len(),
        packets.len() * 160,
        "narrowband: 160 samples per 20 ms packet"
    );

    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, CHANNELS, 2048);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== Speex framework encoder roundtrip ===");
    report("encoder", rms, psnr, decoded.len(), pcm.len());
    // Narrowband CELP on a synthetic tonal signal: the gate is loose —
    // the point is a working end-to-end packet contract, the fidelity
    // number is documented by the report line.
    assert!(rms < 0.3, "Speex encoder RMS {rms:.6} too large (> 0.3)");
}
