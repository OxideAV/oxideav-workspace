//! g729 through the registry (round 452).
//!
//! What holds today: the registry codec `g729` (installed by
//! `oxideav_meta::register_all`) is the fixed-rate 8 kbit/s wire codec
//! — 10-octet packed frames of 80 samples. The Annex B DTX/CNG work
//! landed **decoder-side only** and lives in the crate's ITU-serial
//! `annex_b` module; there is no Annex B encoder (no VAD, no SID
//! emission) and the registry decoder has no SID / no-transmission
//! packet path: a 2-octet SID or an empty no-transmission packet is
//! rejected as invalid rather than synthesised as comfort noise.
//! Those pins are recorded here as the current behaviour so the
//! divergence from a full Annex B registry surface is visible; when a
//! DTX encoder + registry SID routing land, flip the two negative
//! assertions in `annex_b_sid_packets_are_not_yet_routed`.

use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, RuntimeContext, SampleFormat,
    TimeBase,
};
use oxideav_tests::*;

const RATE: u32 = 8_000;
const FRAME: usize = 80;
const WIRE: usize = 10;

fn registry() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_meta::register_all(&mut ctx);
    ctx
}

fn params() -> CodecParameters {
    let mut p = CodecParameters::audio(CodecId::new("g729"));
    p.sample_rate = Some(RATE);
    p.channels = Some(1);
    p.sample_format = Some(SampleFormat::S16);
    p
}

fn bytes(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn samples(b: &[u8]) -> Vec<i16> {
    b.chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

#[test]
fn registry_active_frames_round_trip() {
    let ctx = registry();
    let input = generate_audio_signal(RATE, 1, 1.0);
    let mut enc = ctx.codecs.first_encoder(&params()).expect("g729 encoder");
    assert_eq!(enc.output_params().sample_rate, Some(RATE));
    assert_eq!(enc.output_params().bit_rate, Some(8_000));
    for (i, chunk) in input.chunks(FRAME * 4).enumerate() {
        enc.send_frame(&Frame::Audio(AudioFrame {
            samples: chunk.len() as u32,
            pts: Some((i * FRAME * 4) as i64),
            data: vec![bytes(chunk)],
        }))
        .expect("send");
    }
    enc.flush().expect("flush");
    let mut packets = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(p) => packets.push(p),
            Err(Error::Eof | Error::NeedMore) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
    assert_eq!(
        packets.len(),
        input.len().div_ceil(FRAME),
        "one packet per 80-sample frame"
    );
    for p in &packets {
        assert_eq!(p.data.len(), WIRE, "10-octet packed frame");
        assert_eq!(p.duration, Some(FRAME as i64));
    }

    let mut dec = ctx.codecs.first_decoder(&params()).expect("g729 decoder");
    let mut out = Vec::new();
    for p in &packets {
        dec.send_packet(p).expect("send packet");
        let Frame::Audio(a) = dec.receive_frame().expect("frame") else {
            panic!("audio expected");
        };
        assert_eq!(a.samples as usize, FRAME);
        out.extend(samples(&a.data[0]));
    }
    assert_eq!(out.len(), packets.len() * FRAME);
    let (rms, lag) = audio_rms_diff_aligned(&input, &out, 1, 160);
    let sig = (input.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / input.len() as f64).sqrt();
    eprintln!("g729: rms {rms:.1} lag {lag} signal {sig:.1}");
    assert!(
        rms < sig * 0.5,
        "g729 registry round trip: rms {rms:.1} vs signal {sig:.1}"
    );

    // Multi-frame packets decode as one frame carrying all of them.
    let mut dec = ctx.codecs.first_decoder(&params()).expect("g729 decoder");
    let joined: Vec<u8> = packets[..3].iter().flat_map(|p| p.data.clone()).collect();
    dec.send_packet(&Packet::new(0, TimeBase::new(1, RATE as i64), joined))
        .expect("send joined");
    let Frame::Audio(a) = dec.receive_frame().expect("joined frame") else {
        panic!("audio expected");
    };
    assert_eq!(a.samples as usize, 3 * FRAME);
}

/// Annex B DTX/CNG on the registry path — current behaviour pinned.
#[test]
fn annex_b_sid_packets_are_not_yet_routed() {
    let ctx = registry();
    // Parameter validation: G.729 is 8 kHz mono only.
    let mut wrong = params();
    wrong.sample_rate = Some(16_000);
    assert!(ctx.codecs.first_decoder(&wrong).is_err());
    assert!(ctx.codecs.first_encoder(&wrong).is_err());

    let mut dec = ctx.codecs.first_decoder(&params()).expect("g729 decoder");
    let tb = TimeBase::new(1, RATE as i64);
    // 2-octet SID frame (Annex B Ftyp 2 on the octet-aligned wire).
    let sid = Packet::new(0, tb, vec![0x00, 0x00]);
    // 0-octet no-transmission frame (Ftyp 0).
    let untransmitted = Packet::new(0, tb, Vec::new());
    // DIVERGENCE (oxideav-g729, codec.rs `send_packet`): Annex B SID /
    // untransmitted packets are rejected, not synthesised as CNG.
    assert!(
        dec.send_packet(&sid).is_err(),
        "SID packets are refused today"
    );
    assert!(
        dec.send_packet(&untransmitted).is_err(),
        "empty packets are refused today"
    );
    // The decoder is still usable afterwards.
    dec.send_packet(&Packet::new(0, tb, vec![0u8; WIRE]))
        .expect("active frame");
    let Frame::Audio(a) = dec.receive_frame().expect("frame") else {
        panic!("audio expected");
    };
    assert_eq!(a.samples as usize, FRAME);
}
