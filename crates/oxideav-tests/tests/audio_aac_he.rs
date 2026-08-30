//! aac HE-AAC v1 (SBR) encoder through the framework (round 452).
//!
//! Two routes to the same encoder: the direct `make_he_aac_encoder`
//! factory, and the registry `first_encoder` on codec id `aac` where
//! the profile is selected by an SBR-signalling AudioSpecificConfig in
//! `CodecParameters::extradata` (there is no string "profile" option
//! — the ASC *is* the profile request). Both must produce ADTS at the
//! core (half) rate that the registry decoder — needing no
//! parameters at all — expands back to the full rate with SBR active.

use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, RuntimeContext, SampleFormat,
    TimeBase,
};
use oxideav_tests::*;

const RATE: u32 = 44_100;

fn registry() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_aac::register(&mut ctx);
    ctx
}

fn he_params(channels: u16, extradata: Vec<u8>) -> CodecParameters {
    let mut p = CodecParameters::audio(CodecId::new("aac"));
    p.sample_rate = Some(RATE);
    p.channels = Some(channels);
    p.sample_format = Some(SampleFormat::S16);
    p.bit_rate = Some(40_000 * channels as u64);
    p.extradata = extradata;
    p
}

fn pcm_bytes(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn encode_all(enc: &mut Box<dyn oxideav_core::Encoder>, pcm: &[i16], channels: u16) -> Vec<Packet> {
    let hop = 2048 * channels as usize;
    let mut pts = 0i64;
    for chunk in pcm.chunks(hop) {
        let samples = (chunk.len() / channels as usize) as u32;
        enc.send_frame(&Frame::Audio(AudioFrame {
            samples,
            pts: Some(pts),
            data: vec![pcm_bytes(chunk)],
        }))
        .expect("send frame");
        pts += samples as i64;
    }
    enc.flush().expect("flush");
    let mut out = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(p) => out.push(p),
            Err(Error::Eof) => break,
            Err(e) => panic!("receive_packet: {e:?}"),
        }
    }
    out
}

/// Registry decode with a bare `aac` parameter set: returns
/// (interleaved PCM, per-frame sample counts).
fn decode_all(ctx: &RuntimeContext, packets: &[Packet], channels: u16) -> (Vec<i16>, Vec<u32>) {
    let mut dec = ctx
        .codecs
        .first_decoder(&CodecParameters::audio(CodecId::new("aac")))
        .expect("aac decoder");
    let mut pcm = Vec::new();
    let mut counts = Vec::new();
    for p in packets {
        dec.send_packet(p).expect("send packet");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Audio(a)) => {
                    assert_eq!(a.data.len(), 1, "one interleaved plane");
                    assert_eq!(a.data[0].len(), a.samples as usize * channels as usize * 2);
                    counts.push(a.samples);
                    pcm.extend(
                        a.data[0]
                            .chunks_exact(2)
                            .map(|b| i16::from_le_bytes([b[0], b[1]])),
                    );
                }
                Ok(other) => panic!("expected audio, got {other:?}"),
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("receive_frame: {e:?}"),
            }
        }
    }
    (pcm, counts)
}

fn check_fidelity(label: &str, input: &[i16], output: &[i16], channels: u16) {
    // SBR is a lossy parametric tool above the core band; the
    // lag-aligned RMS on the whole band still has to sit well below the
    // signal level for a tonal test signal.
    let (rms, lag) = audio_rms_diff_aligned(input, output, channels, 8192 * channels as usize);
    let sig = (input.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / input.len() as f64).sqrt();
    eprintln!("{label}: rms {rms:.1} lag {lag} signal {sig:.1}");
    assert!(rms < sig * 0.6, "{label}: rms {rms:.1} vs signal {sig:.1}");
    assert!(
        output.len() + 8192 * channels as usize >= input.len(),
        "{label}: length"
    );
}

#[test]
fn direct_he_factory_emits_half_rate_adts_that_decodes_at_full_rate() {
    for channels in [1u16, 2] {
        let input = generate_audio_signal(RATE, channels, 1.5);
        let params = he_params(channels, Vec::new());
        let mut enc = oxideav_aac::codec_encoder::make_he_aac_encoder(&params).expect("he encoder");
        assert_eq!(enc.output_params().sample_rate, Some(RATE));
        assert!(
            !enc.output_params().extradata.is_empty(),
            "HE ASC advertised"
        );
        let packets = encode_all(&mut enc, &input, channels);
        assert!(packets.len() >= (input.len() / channels as usize) / 2048);
        for p in &packets {
            assert_eq!(p.data[0], 0xFF, "ADTS syncword");
            assert_eq!(p.duration, Some(2048), "2048 output samples per AU");
            assert!(p.flags.keyframe);
        }
        let ctx = registry();
        let (pcm, counts) = decode_all(&ctx, &packets, channels);
        assert!(
            counts.iter().all(|&c| c == 2048),
            "SBR-doubled 2048/frame: {counts:?}"
        );
        assert_eq!(counts.len(), packets.len());
        check_fidelity(&format!("direct ch{channels}"), &input, &pcm, channels);
    }
}

/// The registry's generic `aac` encoder factory dispatches to the HE
/// encoder when `extradata` carries an SBR ASC; a plain LC ASC keeps
/// the LC encoder (1024-sample AUs).
#[test]
fn registry_profile_selection_via_asc_extradata() {
    let channels = 2u16;
    let input = generate_audio_signal(RATE, channels, 1.0);
    let ctx = registry();

    let he_asc = oxideav_aac::asc_writer::he_aac_v1_asc(RATE / 2, RATE, channels as u8, false);
    let mut enc = ctx
        .codecs
        .first_encoder(&he_params(channels, he_asc))
        .expect("registry HE encoder");
    assert_eq!(enc.output_params().sample_rate, Some(RATE));
    let he_packets = encode_all(&mut enc, &input, channels);
    assert!(
        he_packets.iter().all(|p| p.duration == Some(2048)),
        "HE AUs"
    );
    let (pcm, counts) = decode_all(&ctx, &he_packets, channels);
    assert!(counts.iter().all(|&c| c == 2048));
    check_fidelity("registry HE", &input, &pcm, channels);

    // Round-trip the advertised ASC back into the factory: same profile.
    let readvertised = enc.output_params().extradata.clone();
    let enc2 = ctx
        .codecs
        .first_encoder(&he_params(channels, readvertised))
        .expect("re-advertised ASC selects HE again");
    assert_eq!(enc2.output_params().sample_rate, Some(RATE));

    // Control: LC ASC → LC encoder (1024-sample AUs, no SBR).
    let lc_asc = oxideav_aac::asc_writer::aac_lc_asc(RATE, channels as u8);
    let mut lc = ctx
        .codecs
        .first_encoder(&he_params(channels, lc_asc))
        .expect("registry LC encoder");
    let lc_packets = encode_all(&mut lc, &input, channels);
    assert!(
        lc_packets.iter().all(|p| p.duration == Some(1024)),
        "LC AUs"
    );
    let (_, lc_counts) = decode_all(&ctx, &lc_packets, channels);
    assert!(lc_counts.iter().all(|&c| c == 1024), "LC: {lc_counts:?}");

    // A conflicting explicit rate (ASC says 44.1k output, params say
    // 48k) is refused rather than silently re-rated.
    let mut conflict = he_params(
        channels,
        oxideav_aac::asc_writer::he_aac_v1_asc(22_050, 44_100, 2, false),
    );
    conflict.sample_rate = Some(48_000);
    assert!(ctx.codecs.first_encoder(&conflict).is_err());
    let _ = TimeBase::new(1, RATE as i64);
}
