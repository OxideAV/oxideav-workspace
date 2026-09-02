//! Musepack through the framework registry (round 451; `sv=7` leg
//! round 455).
//!
//! `oxideav-musepack` registers a whole-stream `musepack` codec (SV7
//! and SV8 decode through the magic dispatch, from-PCM whole-stream
//! encode) and round 450 added the SV8 seek layer (§9 `SO`/`ST`)
//! plus a from-PCM SV7 encoder; round 454 put the typed encoder
//! options behind `CodecParameters::options`, including the `sv`
//! generation switch. The cross-crate legs here:
//!
//! * a full framework round trip — registry-resolved encoder (SV8,
//!   `MPCK`) → registry-resolved decoder fed the stream split across
//!   arbitrary packet boundaries (the whole-stream accumulation
//!   contract) — gapless-exact sample counts, high fidelity;
//! * the same registry loop with `sv=7` (an `MP+` stream) and with
//!   the SMR-driven `quality` allocation;
//! * the r450 SV7 from-PCM encoder's `MP+` output decoding through
//!   the registry decoder's magic dispatch (cross-generation chain);
//! * §9 random access on an encoder stream rejoining the framework
//!   decoder's linear output;
//! * black-box oracle legs: ffmpeg (mpc7 / mpc8 decoders) decodes
//!   both generations' encoder output, compared against the input.
//!
//! Registration today is id-keyed only — `resolve_payload_magic` has
//! no `MP+`/`MPCK` claims (reported as a followup).

use oxideav_core::{CodecId, CodecParameters, Error, Frame, Packet, RuntimeContext, TimeBase};
use oxideav_musepack::sv7_pcm_encode::{encode_sv7_from_pcm_s16, Sv7EncoderSettings};
use oxideav_musepack::sv8_file_encode::{encode_sv8_from_pcm_s16, Sv8EncoderSettings};
use oxideav_musepack::sv8_seek::{decode_sv8_from_entry, Sv8SeekIndex};
use oxideav_musepack::synthesis::SYNTHESIS_PRIME_SAMPLES;
use oxideav_musepack::SAMPLES_PER_FRAME_PER_CHANNEL;
use oxideav_tests::*;

const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: u16 = 2;

fn mpc_registry() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_musepack::registry::register(&mut ctx);
    ctx
}

fn mpc_params() -> CodecParameters {
    let mut p = CodecParameters::audio(CodecId::new("musepack"));
    p.sample_rate = Some(SAMPLE_RATE);
    p.channels = Some(CHANNELS);
    p.sample_format = Some(oxideav_core::SampleFormat::S16);
    p
}

/// Decode a complete `.mpc` byte stream through the registry decoder,
/// split across `chunks` arbitrary packet boundaries (the whole-stream
/// accumulation contract). Returns interleaved S16 PCM.
fn decode_via_registry(bytes: &[u8], chunks: usize) -> Vec<i16> {
    let ctx = mpc_registry();
    let mut dec = ctx
        .codecs
        .first_decoder(&mpc_params())
        .expect("registry must resolve the musepack decoder");
    let tb = TimeBase::new(1, SAMPLE_RATE as i64);
    let step = bytes.len().div_ceil(chunks.max(1));
    for part in bytes.chunks(step.max(1)) {
        dec.send_packet(&Packet::new(0, tb, part.to_vec()))
            .expect("send packet");
    }
    dec.flush().expect("flush");
    let mut pcm = Vec::new();
    loop {
        match dec.receive_frame() {
            Ok(Frame::Audio(a)) => {
                assert_eq!(a.data.len(), 1, "one interleaved S16 plane");
                pcm.extend(
                    a.data[0]
                        .chunks_exact(2)
                        .map(|b| i16::from_le_bytes([b[0], b[1]])),
                );
            }
            Ok(other) => panic!("unexpected frame {other:?}"),
            Err(Error::Eof) => break,
            Err(Error::NeedMore) => continue,
            Err(e) => panic!("decode error: {e:?}"),
        }
    }
    pcm
}

/// Encode interleaved S16 PCM through the registry encoder; returns
/// the complete `MPCK` stream (one packet at flush).
fn encode_via_registry(pcm: &[i16]) -> Vec<u8> {
    encode_via_registry_with(pcm, &[])
}

/// [`encode_via_registry`] with string-keyed encoder options
/// (`sv`, `quality`, `step`, `ms`, `max_band`, `block_power`, `pns`,
/// `profile`) applied through `CodecParameters::options`.
fn encode_via_registry_with(pcm: &[i16], options: &[(&str, &str)]) -> Vec<u8> {
    let ctx = mpc_registry();
    let mut params = mpc_params();
    for (k, v) in options {
        params.options.insert(*k, *v);
    }
    let mut enc = ctx
        .codecs
        .first_encoder(&params)
        .unwrap_or_else(|e| panic!("registry must resolve the musepack encoder: {e:?}"));
    let nch = CHANNELS as usize;
    for chunk in pcm.chunks(SAMPLES_PER_FRAME_PER_CHANNEL * nch) {
        let mut bytes = Vec::with_capacity(chunk.len() * 2);
        for s in chunk {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let frame = oxideav_core::AudioFrame {
            samples: (chunk.len() / nch) as u32,
            pts: None,
            data: vec![bytes],
        };
        enc.send_frame(&Frame::Audio(frame)).expect("send frame");
    }
    enc.flush().expect("flush");
    let mut out = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(p) => out.extend_from_slice(&p.data),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    }
    out
}

/// Registry SV8 encode → registry decode across split packets:
/// gapless-exact counts, input-aligned, high fidelity.
#[test]
fn registry_sv8_encode_decode_round_trip() {
    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, 1.5);
    let stream = encode_via_registry(&pcm);
    assert_eq!(&stream[..4], b"MPCK", "framework encoder emits SV8");

    let decoded = decode_via_registry(&stream, 5);
    assert_eq!(
        decoded.len(),
        pcm.len(),
        "gapless window must return exactly the input sample count"
    );
    let rms = audio_rms_diff(&pcm, &decoded);
    let psnr = audio_psnr(&pcm, &decoded);
    eprintln!("=== Musepack SV8 framework round trip ===");
    report("sv8", rms, psnr, decoded.len(), pcm.len());
    assert!(rms < 0.05, "SV8 round-trip RMS {rms:.6} too large (> 0.05)");
}

/// The registry encoder's `sv=7` option (round 454 typed options)
/// emits an `MP+` stream from the same S16 frames, and `quality`
/// switches both generations to the SMR-driven allocation; each
/// closes the registry decode loop gapless-exact and within the SV8
/// leg's fidelity bar. `sv=7` is stereo-only and `sv` outside
/// `{7, 8}` is refused at `first_encoder`.
#[test]
fn registry_sv7_option_encode_decode_round_trip() {
    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, 1.5);
    for (tag, options) in [
        ("sv7", &[("sv", "7")][..]),
        ("sv7-q5", &[("sv", "7"), ("quality", "5")][..]),
        ("sv8-q5", &[("sv", "8"), ("quality", "5")][..]),
    ] {
        let stream = encode_via_registry_with(&pcm, options);
        let magic: &[u8] = if tag.starts_with("sv7") {
            b"MP+"
        } else {
            b"MPCK"
        };
        assert!(stream.starts_with(magic), "{tag}: stream magic");
        let decoded = decode_via_registry(&stream, 4);
        assert_eq!(decoded.len(), pcm.len(), "{tag}: gapless-exact count");
        let rms = audio_rms_diff(&pcm, &decoded);
        let psnr = audio_psnr(&pcm, &decoded);
        eprintln!("=== Musepack registry {tag} round trip ===");
        report(tag, rms, psnr, decoded.len(), pcm.len());
        assert!(rms < 0.05, "{tag}: RMS {rms:.6} too large (> 0.05)");
    }

    let ctx = mpc_registry();
    let mut mono = mpc_params();
    mono.channels = Some(1);
    mono.options.insert("sv", "7");
    assert!(
        ctx.codecs.first_encoder(&mono).is_err(),
        "sv=7 output is stereo-only"
    );
    let mut bad = mpc_params();
    bad.options.insert("sv", "9");
    assert!(ctx.codecs.first_encoder(&bad).is_err(), "sv=9 refused");
}

/// The r450 SV7 from-PCM encoder's `MP+` stream decodes through the
/// registry decoder's magic dispatch — the cross-generation chain.
#[test]
fn sv7_encoder_output_decodes_through_registry() {
    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, 1.0);
    let enc =
        encode_sv7_from_pcm_s16(&pcm, 2, 0, &Sv7EncoderSettings::default()).expect("SV7 encode");
    assert_eq!(&enc.bytes[..3], b"MP+", "SV7 magic");

    let decoded = decode_via_registry(&enc.bytes, 3);
    assert_eq!(
        decoded.len(),
        pcm.len(),
        "SV7 decode is declared-count exact and time-aligned"
    );
    let rms = audio_rms_diff(&pcm, &decoded);
    let psnr = audio_psnr(&pcm, &decoded);
    eprintln!("=== Musepack SV7 from-PCM → registry decode ===");
    report("sv7", rms, psnr, decoded.len(), pcm.len());
    assert!(rms < 0.05, "SV7 chain RMS {rms:.6} too large (> 0.05)");
}

/// §9 random access rejoins the framework decoder's linear output:
/// a small-packet SV8 encode carries a multi-entry seek layer, and a
/// mid-stream entry decode matches the registry decoder's PCM once
/// the synthesis priming transient has passed.
#[test]
fn sv8_seek_entry_rejoins_framework_decode() {
    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, 3.0);
    let settings = Sv8EncoderSettings {
        block_power: 1, // 4 frames per AP ⇒ many seek entries
        ..Sv8EncoderSettings::default()
    };
    let enc = encode_sv8_from_pcm_s16(&pcm, 2, 0, &settings).expect("SV8 encode");
    let linear = decode_via_registry(&enc.bytes, 1);
    assert_eq!(linear.len(), pcm.len());

    let index = Sv8SeekIndex::from_seek_packets(&enc.bytes)
        .expect("seek packets parse")
        .expect("encoder writes a §9 seek layer");
    assert!(index.positions.len() >= 8, "want a multi-entry table");

    let nch = CHANNELS as usize;
    // The framework decoder's gapless window starts at
    // `SYNTHESIS_PRIME_SAMPLES + beginning_silence` on the untrimmed
    // decoded timeline; read the declared silence from the SH header.
    let header = oxideav_musepack::sv8_decode::decode_sv8_stream(&enc.bytes)
        .expect("linear SV8 decode")
        .header;
    let window = (SYNTHESIS_PRIME_SAMPLES + header.beginning_silence as usize) * nch;
    let entry = index.positions.len() / 2;
    let seek = decode_sv8_from_entry(&enc.bytes, &index, entry).expect("entry decode");
    assert_eq!(
        seek.first_frame,
        entry as u64 * index.frames_per_entry(),
        "entry → frame arithmetic"
    );
    let seek_start = seek.first_frame as usize * SAMPLES_PER_FRAME_PER_CHANNEL * nch;
    let transient = (SYNTHESIS_PRIME_SAMPLES + 1) * nch;
    let mut compared = 0usize;
    let mut max_delta = 0i32;
    for (t, &s) in seek.pcm.iter().enumerate().skip(transient) {
        let Some(out_idx) = (seek_start + t).checked_sub(window) else {
            continue;
        };
        let Some(&lin) = linear.get(out_idx) else {
            break;
        };
        let s16 = s.round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i32;
        max_delta = max_delta.max((s16 - i32::from(lin)).abs());
        compared += 1;
    }
    assert!(
        compared > SAMPLES_PER_FRAME_PER_CHANNEL,
        "compared only {compared} samples"
    );
    eprintln!("=== Musepack seek rejoin: entry {entry}, {compared} samples, max Δ {max_delta} ===");
    assert!(
        max_delta <= 1,
        "mid-stream entry must rejoin within ±1 LSB, got Δ {max_delta}"
    );
}

/// `true` when the oracle binary advertises the named decoder.
fn ffmpeg_has_decoder(name: &str) -> bool {
    let Some(bin) = ffmpeg_path() else {
        return false;
    };
    std::process::Command::new(bin)
        .args(["-hide_banner", "-decoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(name))
        .unwrap_or(false)
}

/// Oracle leg shared shape: our encoder's `.mpc` file decoded by
/// ffmpeg (black-box), compared against the input signal.
fn ffmpeg_oracle_leg(tag: &str, decoder: &str, bytes: &[u8], pcm: &[i16]) {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }
    if !ffmpeg_has_decoder(decoder) {
        eprintln!("skip: oracle build lacks the {decoder} decoder");
        return;
    }
    let mpc_path = tmp(&format!("oxideav-musepack-{tag}.mpc"));
    std::fs::write(&mpc_path, bytes).expect("write mpc");
    let raw_path = tmp(&format!("oxideav-musepack-{tag}-ffmpeg.raw"));
    assert!(
        ffmpeg(&[
            "-i",
            mpc_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            raw_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our {tag} stream"
    );
    let decoded = read_pcm_s16le(&raw_path);
    assert!(!decoded.is_empty(), "ffmpeg produced no samples");
    let (rms, lag) = audio_rms_diff_aligned(pcm, &decoded, CHANNELS, 4096);
    let psnr = audio_psnr(pcm, &decoded[lag..]);
    eprintln!("=== Musepack {tag} vs ffmpeg decode ===");
    report(tag, rms, psnr, decoded.len(), pcm.len());
    assert!(rms < 0.05, "{tag} oracle RMS {rms:.6} too large (> 0.05)");
}

/// The framework encoder's SV8 stream decodes in ffmpeg (mpc8).
#[test]
fn sv8_registry_encode_decodes_in_ffmpeg() {
    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, 2.0);
    let stream = encode_via_registry(&pcm);
    ffmpeg_oracle_leg("sv8", "mpc8", &stream, &pcm);
}

/// The r450 SV7 from-PCM encoder's stream decodes in ffmpeg (mpc7).
#[test]
fn sv7_from_pcm_encode_decodes_in_ffmpeg() {
    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, 2.0);
    let enc =
        encode_sv7_from_pcm_s16(&pcm, 2, 0, &Sv7EncoderSettings::default()).expect("SV7 encode");
    ffmpeg_oracle_leg("sv7", "mpc7", &enc.bytes, &pcm);
}
