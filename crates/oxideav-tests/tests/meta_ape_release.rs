//! ape 0.0.3 through the `oxideav_meta` aggregator.
//!
//! `audio_ape.rs` drives `oxideav_ape::register` directly on a
//! hand-assembled stream. This suite covers the other half of the
//! release: the `ape` feature is wired into `oxideav_meta::register_all`,
//! so a consumer that only calls the aggregator must (a) resolve the
//! `'MAC '` payload magic, (b) decode the staged docs fixtures
//! byte-exact against their reference PCM, and (c) find no encoder —
//! ape 0.0.3 registers a decoder factory only (the encoder-side
//! primitives exist in-crate but have no registry `EncoderFactory`
//! yet), so the encode→decode round trip through the registry is
//! pinned as "not resolvable" here until that factory lands.
//!
//! Fixture legs skip when `docs/audio/ape/fixtures/` is not staged.

use std::path::{Path, PathBuf};

use oxideav_core::{CodecParameters, Error, Frame, Packet, RuntimeContext, TimeBase};

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
/// codec id with a decoder factory and the payload-magic claim.
#[test]
fn register_all_wires_ape_decoder_and_magic() {
    let ctx = meta_ctx();
    let id = ctx
        .codecs
        .resolve_payload_magic_ref(b"MAC \x96\x0f\x00\x00")
        .expect("magic claim installed");
    assert_eq!(id.as_str(), "ape");
    let params = CodecParameters::audio(id.clone());
    assert!(ctx.codecs.first_decoder(&params).is_ok());
    // ape 0.0.3: decoder-only registration. Once an EncoderFactory is
    // registered this assertion flips and the round-trip leg below
    // should be promoted from "pinned absent" to a real encode.
    assert!(
        ctx.codecs.first_encoder(&params).is_err(),
        "ape 0.0.3 registers no encoder factory"
    );
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
