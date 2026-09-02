//! ape through the `oxideav_meta` aggregator.
//!
//! `audio_ape.rs` drives `oxideav_ape::register` directly on a
//! hand-assembled stream. This suite covers the other half of the
//! release: the `ape` feature is wired into `oxideav_meta::register_all`,
//! so a consumer that only calls the aggregator must (a) resolve the
//! `'MAC '` payload magic, (b) decode the staged docs fixtures
//! byte-exact against their reference PCM, and (c) — since ape 0.0.4
//! (round 453, the 3990 encoder) — find a registered `EncoderFactory`
//! and close a whole-file encode→decode loop through registry-resolved
//! factories with byte-exact PCM.
//!
//! Fixture legs skip when `docs/audio/ape/fixtures/` is not staged.

use std::path::{Path, PathBuf};

use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, RuntimeContext, SampleFormat,
    TimeBase,
};

fn fixtures_root() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/audio/ape/fixtures");
    p.join("noise_stereo/input.ape").exists().then_some(p)
}

/// (name, channels, sample rate, expected total samples per channel).
const CORPUS: &[(&str, u16, u32, usize)] = &[
    ("left_silent_stereo", 2, 44_100, 3_000),
    ("noise_stereo", 2, 44_100, 6_000),
    ("silence_mono8k", 1, 8_000, 1_600),
    ("silence_stereo", 2, 44_100, 22_050),
    ("tone_lr_equal", 2, 44_100, 13_230),
    ("two_frame_mono8k", 1, 8_000, 78_728),
    ("zeros_then_noise_mono", 1, 0, 0),
];

fn meta_ctx() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_meta::register_all(&mut ctx);
    ctx
}

fn decode_all(ctx: &RuntimeContext, file: &[u8], rate: u32) -> (Vec<u8>, Vec<u32>) {
    let id = ctx
        .codecs
        .resolve_payload_magic_ref(file)
        .expect("'MAC ' resolves through register_all")
        .clone();
    assert_eq!(id.as_str(), "ape");
    let mut dec = ctx
        .codecs
        .first_decoder(&CodecParameters::audio(id))
        .expect("meta-registered ape decoder");
    let tb = TimeBase::new(1, rate.max(1) as i64);
    // Split into ~1 KiB packets to exercise the accumulator.
    for chunk in file.chunks(1024) {
        dec.send_packet(&Packet::new(0, tb, chunk.to_vec()))
            .expect("send");
    }
    dec.flush().expect("flush");
    let mut pcm = Vec::new();
    let mut counts = Vec::new();
    loop {
        match dec.receive_frame() {
            Ok(Frame::Audio(a)) => {
                assert_eq!(a.data.len(), 1, "interleaved: one plane");
                counts.push(a.samples);
                pcm.extend_from_slice(&a.data[0]);
            }
            Ok(other) => panic!("expected audio, got {other:?}"),
            Err(Error::Eof) => break,
            Err(e) => panic!("decode error: {e:?}"),
        }
    }
    (pcm, counts)
}

/// `register_all` (with the default `all` bundle) installs the `ape`
/// codec id with decoder + encoder factories and the payload-magic
/// claim.
#[test]
fn register_all_wires_ape_decoder_encoder_and_magic() {
    let ctx = meta_ctx();
    let id = ctx
        .codecs
        .resolve_payload_magic_ref(b"MAC \x96\x0f\x00\x00")
        .expect("magic claim installed");
    assert_eq!(id.as_str(), "ape");
    let params = CodecParameters::audio(id.clone());
    assert!(ctx.codecs.first_decoder(&params).is_ok());
    assert!(
        ctx.codecs.has_encoder(id),
        "ape 0.0.4 registers its encoder factory"
    );
    // The factory needs the stream shape: bare audio params (no
    // sample rate / channels) are declined with a typed error rather
    // than deferred to encode time.
    assert!(ctx.codecs.first_encoder(&params).is_err());
    assert!(ctx.codecs.first_encoder(&encode_params(8_000, 1)).is_ok());
}

/// Encoder-side stream parameters: S16 interleaved at `rate` Hz,
/// `channels` wide, with a small frame so a short input spans several
/// APE frames.
fn encode_params(rate: u32, channels: u16) -> CodecParameters {
    let mut p = CodecParameters::audio(CodecId::new("ape"));
    p.sample_rate = Some(rate);
    p.channels = Some(channels);
    p.sample_format = Some(SampleFormat::S16);
    p.options = p.options.set("blocks_per_frame", "256");
    p
}

/// Hand-assembled interleaved S16 LE input: a deterministic ramp on
/// the left, a slowly varying square on the right — not a fixture.
fn synthetic_s16(samples: usize, channels: u16) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples * usize::from(channels) * 2);
    for i in 0..samples {
        for ch in 0..channels {
            let v: i16 = if ch == 0 {
                ((i as i32 * 37) % 20_000 - 10_000) as i16
            } else if (i / 64) % 2 == 0 {
                7_000
            } else {
                -7_000
            };
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    bytes
}

/// Registry encode → registry decode, both factories resolved through
/// `register_all`: the encoder emits one complete `.ape` file at
/// flush, the file self-identifies through the payload-magic index,
/// and the decoded PCM is byte-exact against the input.
#[test]
fn register_all_encode_decode_round_trip_is_byte_exact() {
    let ctx = meta_ctx();
    for (rate, channels, samples) in [(8_000u32, 1u16, 1_000usize), (44_100, 2, 700)] {
        let params = encode_params(rate, channels);
        let input = synthetic_s16(samples, channels);
        let mut enc = ctx
            .codecs
            .first_encoder(&params)
            .expect("meta-registered ape encoder");
        // Ragged chunking across frame boundaries (whole-file contract:
        // any chunking is fine, the file finalises at flush).
        let frame_bytes = usize::from(channels) * 2;
        for chunk in input.chunks(154 * frame_bytes) {
            enc.send_frame(&Frame::Audio(AudioFrame {
                samples: (chunk.len() / frame_bytes) as u32,
                pts: None,
                data: vec![chunk.to_vec()],
            }))
            .expect("send frame");
        }
        assert!(matches!(enc.receive_packet(), Err(Error::NeedMore)));
        enc.flush().expect("flush");
        let file = enc.receive_packet().expect("the complete .ape file");
        assert_eq!(file.duration, Some(samples as i64), "{rate}/{channels}");
        assert!(matches!(enc.receive_packet(), Err(Error::Eof)));

        // The emitted file resolves through the aggregator's magic
        // index and decodes through the registry decoder.
        let (pcm, counts) = decode_all(&ctx, &file.data, rate);
        assert_eq!(
            counts.iter().sum::<u32>() as usize,
            samples,
            "{rate}/{channels}: samples per channel"
        );
        assert!(counts.len() >= 2, "{rate}/{channels}: multi-frame file");
        assert_eq!(pcm, input, "{rate}/{channels}: PCM must be byte-exact");
    }
}

/// Every staged docs fixture decodes byte-exact against its
/// `expected.pcm` through the aggregator-registered decoder.
#[test]
fn docs_fixtures_decode_byte_exact_via_register_all() {
    let Some(root) = fixtures_root() else {
        eprintln!("skipping: docs/audio/ape/fixtures not staged");
        return;
    };
    let ctx = meta_ctx();
    let mut checked = 0;
    for &(name, ch, rate, total) in CORPUS {
        let dir = root.join(name);
        let (Ok(file), Ok(expected)) = (
            std::fs::read(dir.join("input.ape")),
            std::fs::read(dir.join("expected.pcm")),
        ) else {
            eprintln!("skipping {name}: fixture pair missing");
            continue;
        };
        let (pcm, counts) = decode_all(&ctx, &file, rate);
        assert_eq!(pcm.len(), expected.len(), "{name}: PCM byte length");
        assert_eq!(pcm, expected, "{name}: PCM must be byte-exact");
        let samples: u32 = counts.iter().sum();
        if total != 0 {
            assert_eq!(samples as usize, total, "{name}: samples per channel");
            assert_eq!(pcm.len(), total * ch as usize * 2, "{name}: s16le layout");
        } else {
            assert_eq!(pcm.len() % 2, 0, "{name}: s16le layout");
        }
        checked += 1;
    }
    assert!(
        checked >= 6,
        "expected the staged corpus, checked {checked}"
    );
}

/// Container probing does not claim `.ape` (native carriage is the
/// codec's own payload magic, not a container): the aggregator must
/// leave the file to the codec path rather than mis-probing it.
#[test]
fn register_all_leaves_ape_to_the_payload_magic_path() {
    let Some(root) = fixtures_root() else {
        eprintln!("skipping: docs/audio/ape/fixtures not staged");
        return;
    };
    let file = std::fs::read(root.join("noise_stereo/input.ape")).unwrap();
    let ctx = meta_ctx();
    let mut rs: Box<dyn oxideav_core::ReadSeek> = Box::new(std::io::Cursor::new(file.clone()));
    let probed = ctx.containers.probe_input(&mut *rs, Some("ape")).ok();
    assert!(
        probed.is_none(),
        "no container may claim an APE stream, got {probed:?}"
    );
    assert!(ctx.codecs.resolve_payload_magic_ref(&file).is_some());
}
