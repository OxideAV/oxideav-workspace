//! WMA through the framework registry (round 451).
//!
//! `oxideav-wma` (round 450) registers core decoder factories for
//! `wma1` / `wma2` with their `WAVEFORMATEX` wave-tag claims
//! (`0x0160` / `0x0161`). This suite drives that wiring from the
//! consumer side — the flow a container demuxer follows:
//!
//! 1. a `ProbeContext` carrying nothing but the wave tag resolves to
//!    the codec id through the registry's `CodecResolver` surface;
//! 2. `first_decoder` over the resolved parameters builds the
//!    framework decoder;
//! 3. packets decode to interleaved F32 frames with the crate's
//!    documented cadence (one 2048-sample frame per packet once its
//!    §1 carry successor arrives; flush drains the tail).
//!
//! The packet fixture is crafted at test time through the crate's own
//! public wire layers (`bitio::BitWriter` + `wire_vlc` codebooks +
//! `band_partition`), in the crate-documented craftable-by-hand
//! configuration: fixed-block mono v2, bit reservoir off
//! (`flags2 = 0x0001`), one frame per packet.
//!
//! No container chain exists in-suite yet: there is no ASF demuxer,
//! and the WAV demuxer does not surface the `fmt ` chunk's `cbSize`
//! extradata tail a WMA stream needs — both reported as followups.

use oxideav_core::{
    CodecId, CodecParameters, CodecTag, Error, Frame, Packet, ProbeContext, RuntimeContext,
    TimeBase,
};
use oxideav_wma::bitio::BitWriter;
use oxideav_wma::registration::{make_decoder, register};
use oxideav_wma::wire_vlc::{coef_vlc, scale_vlc};

const SAMPLE_RATE: u32 = 44_100;
const BLOCK_ALIGN: usize = 512;

fn wma_registry() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    register(&mut ctx);
    ctx
}

/// Stream parameters for the craftable fixed-block mono v2
/// configuration (reservoir off), keyed by `codec_id`.
fn v2_params(codec_id: &str) -> CodecParameters {
    let mut p = CodecParameters::audio(CodecId::new(codec_id));
    p.tag = Some(CodecTag::wave_format(0x0161));
    p.sample_rate = Some(SAMPLE_RATE);
    p.channels = Some(1);
    p.bit_rate = Some(64_024);
    // v2 extradata: u32 LE flags1, u16 LE flags2 = 0x0001 (no bit
    // reservoir ⇒ one frame per packet, no packet header).
    p.extradata = vec![0, 0, 0, 0, 0x01, 0x00];
    p
}

/// One crafted packet: a single coded frame (channel-coded flag, a
/// total gain, a flat scale-factor envelope, an immediate coefficient
/// EOB), zero-padded to the block align.
fn crafted_packet() -> Vec<u8> {
    let bands = oxideav_wma::band_partition::exponent_band_count(SAMPLE_RATE, 2048);
    let mut w = BitWriter::new();
    w.write_bit(true); // channel coded
    w.write_bits(50, 7); // total gain
    for _ in 0..bands {
        assert!(scale_vlc().encode_symbol(60, &mut w)); // delta 0
    }
    assert!(coef_vlc(3, false).unwrap().encode_symbol(1, &mut w)); // EOB
    let mut bytes = w.into_bytes();
    assert!(bytes.len() <= BLOCK_ALIGN);
    bytes.resize(BLOCK_ALIGN, 0);
    bytes
}

/// The wave-tag claims resolve through the registry exactly as a
/// container demuxer would resolve them: `0x0160` → `wma1`,
/// `0x0161` → `wma2`, and an unclaimed tag stays unresolved.
#[test]
fn wave_tags_resolve_like_a_container_would() {
    let ctx = wma_registry();
    for (raw, want) in [(0x0160u16, "wma1"), (0x0161, "wma2")] {
        let tag = CodecTag::wave_format(raw);
        let mut probe = ProbeContext::new(&tag);
        probe.channels = Some(1);
        probe.sample_rate = Some(SAMPLE_RATE);
        let id = ctx
            .codecs
            .resolve_tag_ref(&probe)
            .unwrap_or_else(|| panic!("wave tag {raw:#06x} must resolve"));
        assert_eq!(id.as_str(), want, "wave tag {raw:#06x}");
        assert!(ctx.codecs.has_decoder(id), "{want} must have a decoder");
        assert!(!ctx.codecs.has_encoder(id), "{want} is decode-only today");
    }
    // 0x0162 (WMA Pro) is NOT claimed — resolution must decline, not
    // mis-route onto the v1/v2 decoder.
    let tag = CodecTag::wave_format(0x0162);
    assert!(ctx
        .codecs
        .resolve_tag_ref(&ProbeContext::new(&tag))
        .is_none());
}

/// End-to-end container-shaped flow: resolve the tag, build the
/// decoder through `first_decoder`, decode crafted packets, and pin
/// the crate's documented frame cadence (2048 samples per packet, a
/// half-frame synthesis tail at flush).
#[test]
fn registry_resolved_decoder_decodes_crafted_v2_packets() {
    let ctx = wma_registry();
    let tag = CodecTag::wave_format(0x0161);
    let id = ctx
        .codecs
        .resolve_tag_ref(&ProbeContext::new(&tag))
        .expect("tag resolves")
        .clone();
    let params = v2_params(id.as_str());
    let mut dec = ctx
        .codecs
        .first_decoder(&params)
        .expect("registry-resolved decoder");

    let tb = TimeBase::new(1, SAMPLE_RATE as i64);
    let mut frames: Vec<oxideav_core::AudioFrame> = Vec::new();
    let drain = |dec: &mut Box<dyn oxideav_core::Decoder>,
                 out: &mut Vec<oxideav_core::AudioFrame>| loop {
        match dec.receive_frame() {
            Ok(Frame::Audio(a)) => out.push(a),
            Ok(other) => panic!("unexpected frame {other:?}"),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("decode error: {e:?}"),
        }
    };
    const N: usize = 3;
    for _ in 0..N {
        dec.send_packet(&Packet::new(0, tb, crafted_packet()))
            .expect("send packet");
        drain(&mut dec, &mut frames);
    }
    // Reservoir-off decode is one packet deep: N packets in, N−1
    // frames out before flush.
    assert_eq!(frames.len(), N - 1, "decode trails input by one packet");
    dec.flush().expect("flush");
    drain(&mut dec, &mut frames);
    assert_eq!(frames.len(), N + 1, "flush drains the last frame + tail");

    let mut total = 0usize;
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(f.data.len(), 1, "frame {i}: one interleaved plane");
        assert_eq!(f.data[0].len(), f.samples as usize * 4, "frame {i}: F32");
        for chunk in f.data[0].chunks_exact(4) {
            let v = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            assert!(v.is_finite() && v.abs() <= 4.0, "frame {i}: sample {v}");
        }
        total += f.samples as usize;
    }
    assert_eq!(
        total,
        N * 2048 + 1024,
        "N full frames + the half-frame synthesis tail"
    );
}

/// The registry factory and the direct dual-API factory reject broken
/// parameters with typed errors instead of deferring to decode time.
#[test]
fn construction_errors_are_typed() {
    let ctx = wma_registry();
    // Missing extradata: the §0 flags words are required.
    let mut p = v2_params("wma2");
    p.extradata.clear();
    assert!(ctx.codecs.first_decoder(&p).is_err(), "no extradata");
    // Channel counts outside 1..=2 are unsupported.
    let mut p = v2_params("wma2");
    p.channels = Some(6);
    assert!(ctx.codecs.first_decoder(&p).is_err(), "6 channels");
    // bit_rate is required (the §0 configuration derives from it).
    let mut p = v2_params("wma2");
    p.bit_rate = None;
    assert!(make_decoder(&p).is_err(), "missing bit_rate");
    // Direct factory falls back to the wave tag for foreign ids —
    // the dual-API contract a container resolver relies on.
    let mut p = v2_params("not-a-registry-id");
    p.tag = Some(CodecTag::wave_format(0x0161));
    assert!(make_decoder(&p).is_ok(), "tag fallback");
}
