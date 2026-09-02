//! WMA through the framework registry (round 451; encoder legs
//! round 455).
//!
//! `oxideav-wma` (round 450) registers core decoder factories for
//! `wma1` / `wma2` with their `WAVEFORMATEX` wave-tag claims
//! (`0x0160` / `0x0161`), and round 454 added the vendor-wire
//! encoder factories under the same ids. This suite drives that
//! wiring from the consumer side — the flow a container demuxer
//! follows:
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
//! The encoder legs close a registry encode→decode round trip on a
//! synthetic tone: `first_encoder` (no extradata ⇒ the crate's
//! default CBR `flags2`), all `block_align` packets at flush, then
//! `first_decoder` built from the encoder's own `output_params`,
//! judged by lag-fitted correlation (the crate's own black-box bar
//! is corr² > 0.8 on the vendor-decoder leg; the vendor-wire encoder
//! reports corr² .98–.995).
//!
//! No container chain exists in-suite yet: there is no ASF demuxer,
//! and the WAV demuxer does not surface the `fmt ` chunk's `cbSize`
//! extradata tail a WMA stream needs — both reported as followups.

use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, CodecTag, Error, Frame, Packet, ProbeContext,
    RuntimeContext, TimeBase,
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
        assert!(
            ctx.codecs.has_encoder(id),
            "{want} registers its encoder factory (round 454 vendor-wire encoder)"
        );
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

/// Encoder-side parameters for the round-trip legs: no extradata, so
/// the factory picks its default `flags2` and derives `block_align`.
fn encode_params(codec_id: &str, channels: u16, bit_rate: u64) -> CodecParameters {
    let mut p = CodecParameters::audio(CodecId::new(codec_id));
    p.sample_rate = Some(SAMPLE_RATE);
    p.channels = Some(channels);
    p.bit_rate = Some(bit_rate);
    p
}

/// Deterministic per-channel test material in the encoder's ±1.0 F32
/// convention: a 440 Hz tone with a weaker 1234 Hz partial, the right
/// channel attenuated so mid/side has something to do.
fn tone(channels: usize, samples: usize) -> Vec<Vec<f32>> {
    (0..channels)
        .map(|ch| {
            let gain = if ch == 0 { 1.0 } else { 0.8 };
            (0..samples)
                .map(|t| {
                    let t = t as f32 / SAMPLE_RATE as f32;
                    gain * (0.25 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
                        + 0.1 * (2.0 * std::f32::consts::PI * 1234.0 * t).sin())
                })
                .collect()
        })
        .collect()
}

/// Interleave planar F32 into the one-plane frame layout the encoder
/// takes.
fn interleave_f32(planar: &[Vec<f32>]) -> Vec<u8> {
    let n = planar[0].len();
    let mut bytes = Vec::with_capacity(n * planar.len() * 4);
    for t in 0..n {
        for ch in planar {
            bytes.extend_from_slice(&ch[t].to_le_bytes());
        }
    }
    bytes
}

/// Lag-fitted squared correlation of `decoded` against `original`
/// over `0..=max_lag` sample lags (the codec's synthesis lead-in is a
/// fixed but configuration-dependent number of samples): returns
/// `(lag, corr2)` at the best lag.
fn fit_corr2(original: &[f32], decoded: &[f32], max_lag: usize) -> (usize, f64) {
    let mut best = (0usize, 0.0f64);
    for lag in 0..=max_lag {
        let n = original.len().min(decoded.len().saturating_sub(lag));
        if n < original.len() / 2 {
            break;
        }
        let (mut dot, mut ee, mut rr) = (0.0f64, 0.0f64, 0.0f64);
        for t in 0..n {
            let a = f64::from(original[t]);
            let b = f64::from(decoded[t + lag]);
            dot += a * b;
            ee += a * a;
            rr += b * b;
        }
        if ee > 0.0 && rr > 0.0 {
            let corr2 = dot * dot / (ee * rr);
            if corr2 > best.1 {
                best = (lag, corr2);
            }
        }
    }
    best
}

/// Registry encode → registry decode for one codec id; returns the
/// per-channel decoded F32 and the source material.
fn registry_round_trip(codec_id: &str, channels: u16) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let ctx = wma_registry();
    let bit_rate = if channels == 2 { 96_000 } else { 64_000 };
    let params = encode_params(codec_id, channels, bit_rate);
    let mut enc = ctx
        .codecs
        .first_encoder(&params)
        .unwrap_or_else(|e| panic!("{codec_id}: registry encoder: {e:?}"));
    let n = (SAMPLE_RATE as usize * 3) / 2; // 1.5 s
    let source = tone(usize::from(channels), n);
    let bytes = interleave_f32(&source);
    // Ragged chunking: the encoder buffers across frame boundaries.
    let frame_bytes = usize::from(channels) * 4;
    for chunk in bytes.chunks(1500 * frame_bytes) {
        enc.send_frame(&Frame::Audio(AudioFrame {
            samples: (chunk.len() / frame_bytes) as u32,
            pts: None,
            data: vec![chunk.to_vec()],
        }))
        .expect("send frame");
    }
    // All §1 packets materialise at flush.
    assert!(matches!(enc.receive_packet(), Err(Error::NeedMore)));
    enc.flush().expect("flush");
    let mut packets = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(p) => packets.push(p),
            Err(Error::Eof) => break,
            Err(e) => panic!("{codec_id}: receive_packet: {e:?}"),
        }
    }
    assert!(packets.len() >= 2, "{codec_id}: {} packets", packets.len());
    let block_align = packets[0].data.len();
    assert!(
        packets.iter().all(|p| p.data.len() == block_align),
        "{codec_id}: every packet is exactly block_align bytes"
    );

    // The encoder's output parameters are what a muxer would store
    // (wave tag + versioned extradata carrying flags2); the decoder
    // is built from exactly those.
    let out = enc.output_params().clone();
    assert_eq!(out.codec_id.as_str(), codec_id);
    let want_tag = if codec_id == "wma1" { 0x0160 } else { 0x0161 };
    assert_eq!(out.tag, Some(CodecTag::wave_format(want_tag)));
    let id = ctx
        .codecs
        .resolve_tag_ref(&ProbeContext::new(out.tag.as_ref().unwrap()))
        .expect("the emitted tag resolves back to the codec");
    assert_eq!(id.as_str(), codec_id);
    let mut dec = ctx
        .codecs
        .first_decoder(&out)
        .unwrap_or_else(|e| panic!("{codec_id}: registry decoder from output_params: {e:?}"));
    let mut decoded: Vec<Vec<f32>> = vec![Vec::new(); usize::from(channels)];
    let mut drain = |dec: &mut Box<dyn oxideav_core::Decoder>| loop {
        match dec.receive_frame() {
            Ok(Frame::Audio(a)) => {
                assert_eq!(a.data.len(), 1, "one interleaved F32 plane");
                for group in a.data[0].chunks_exact(4 * usize::from(channels)) {
                    for (ch, c) in group.chunks_exact(4).enumerate() {
                        decoded[ch].push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                    }
                }
            }
            Ok(other) => panic!("unexpected frame {other:?}"),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("{codec_id}: decode error: {e:?}"),
        }
    };
    for p in &packets {
        dec.send_packet(p).expect("send packet");
        drain(&mut dec);
    }
    dec.flush().expect("flush");
    drain(&mut dec);
    (decoded, source)
}

/// `first_encoder("wma2")` → `first_decoder(output_params)`: the
/// decoded tone correlates with the source at ≥ 0.9 (corr², lag
/// fitted) on both channels, and the decode covers the input length.
#[test]
fn registry_wma2_encode_decode_round_trip() {
    let (decoded, source) = registry_round_trip("wma2", 2);
    for (ch, (dec, src)) in decoded.iter().zip(&source).enumerate() {
        assert!(
            dec.len() >= src.len(),
            "wma2 ch{ch}: decoded {} < input {}",
            dec.len(),
            src.len()
        );
        assert!(dec.iter().all(|v| v.is_finite() && v.abs() <= 4.0));
        let (lag, corr2) = fit_corr2(src, dec, 4096);
        eprintln!("=== WMA2 registry round trip ch{ch}: lag {lag}, corr2 {corr2:.4} ===");
        assert!(corr2 >= 0.9, "wma2 ch{ch}: corr2 {corr2:.4} at lag {lag}");
    }
}

/// The same loop on `wma1` (v1 framing, `0x0160`): the factory
/// accepts the id and the mono decode correlates with the source.
#[test]
fn registry_wma1_encode_decode_round_trip() {
    let (decoded, source) = registry_round_trip("wma1", 1);
    let (lag, corr2) = fit_corr2(&source[0], &decoded[0], 4096);
    eprintln!("=== WMA1 registry round trip: lag {lag}, corr2 {corr2:.4} ===");
    assert!(decoded[0].len() >= source[0].len());
    assert!(corr2 >= 0.9, "wma1: corr2 {corr2:.4} at lag {lag}");
}

/// Encoder construction errors are typed like the decoder's: missing
/// rate / bit rate, unsupported channel counts, and an extradata
/// `flags2` selecting the (unstaged) LSP envelope path are refused
/// at `first_encoder`, not at encode time.
#[test]
fn registry_encoder_construction_errors_are_typed() {
    let ctx = wma_registry();
    let mut p = encode_params("wma2", 2, 96_000);
    p.sample_rate = None;
    assert!(ctx.codecs.first_encoder(&p).is_err(), "no sample rate");
    let mut p = encode_params("wma2", 2, 96_000);
    p.bit_rate = None;
    assert!(ctx.codecs.first_encoder(&p).is_err(), "no bit rate");
    let p = encode_params("wma2", 6, 96_000);
    assert!(ctx.codecs.first_encoder(&p).is_err(), "6 channels");
    let mut p = encode_params("wma2", 2, 96_000);
    p.extradata = vec![0, 0, 0, 0, 0x26, 0x00]; // flags2 bit 0 clear ⇒ LSP
    assert!(ctx.codecs.first_encoder(&p).is_err(), "LSP envelope path");
}
