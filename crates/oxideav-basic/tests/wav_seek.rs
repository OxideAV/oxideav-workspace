//! Integration test for `WavDemuxer::seek_to`.
//!
//! WAV is keyframe-only PCM, so `seek_to(pts)` must land exactly on
//! `pts` (no keyframe quantisation) and the next packet it emits must
//! start at that sample boundary with matching payload bytes.

use oxideav_basic::register_containers;
use oxideav_container::{ContainerRegistry, ReadSeek, WriteSeek};
use oxideav_core::{
    CodecId, CodecParameters, MediaType, Packet, SampleFormat, StreamInfo, TimeBase,
};

fn mono_s16_stream(sample_rate: u32) -> StreamInfo {
    let mut params = CodecParameters::audio(CodecId::new("pcm_s16le"));
    params.media_type = MediaType::Audio;
    params.channels = Some(1);
    params.sample_rate = Some(sample_rate);
    params.sample_format = Some(SampleFormat::S16);
    StreamInfo {
        index: 0,
        time_base: TimeBase::new(1, sample_rate as i64),
        duration: None,
        start_time: Some(0),
        params,
    }
}

/// 1 s of S16LE sawtooth at `sample_rate` Hz. Each sample is a unique
/// 16-bit LE value derived from its index, so we can verify the exact
/// position we landed on after a seek.
fn synth_1s_s16(sample_rate: u32) -> Vec<u8> {
    let n = sample_rate as usize;
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        // ((i * 31) mod 65536) - 32768 — deterministic and distinct modulo
        // 65536, good enough for a 1-second buffer at 48 kHz.
        let s = (((i as i64) * 31).rem_euclid(65536) - 32768) as i16;
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[test]
fn seek_to_half_second_lands_at_exact_sample() {
    let sample_rate = 48_000u32;
    let payload = synth_1s_s16(sample_rate);
    assert_eq!(payload.len(), sample_rate as usize * 2);

    // Mux a WAV to a temp file.
    let mut reg = ContainerRegistry::new();
    register_containers(&mut reg);

    let tmp = std::env::temp_dir().join("oxideav-basic-wav-seek.wav");
    let _ = std::fs::remove_file(&tmp);
    let stream = mono_s16_stream(sample_rate);
    {
        let f = std::fs::File::create(&tmp).unwrap();
        let ws: Box<dyn WriteSeek> = Box::new(f);
        let mut mux = reg
            .open_muxer("wav", ws, std::slice::from_ref(&stream))
            .expect("open wav muxer");
        mux.write_header().unwrap();
        let pkt = Packet::new(0, stream.time_base, payload.clone());
        mux.write_packet(&pkt).unwrap();
        mux.write_trailer().unwrap();
    }

    // Open the demuxer and seek to 0.5 s.
    let f = std::fs::File::open(&tmp).unwrap();
    let rs: Box<dyn ReadSeek> = Box::new(f);
    let mut dmx = reg.open_demuxer("wav", rs).expect("open wav demuxer");
    let target = (sample_rate / 2) as i64; // 24000
    let landed = dmx.seek_to(0, target).expect("seek_to");
    assert_eq!(
        landed, target,
        "WAV is keyframe-only PCM — landed pts must equal target pts"
    );

    // First packet after the seek must start at `target` and its payload
    // must match the synthetic source at that byte offset.
    let pkt = dmx.next_packet().expect("next_packet after seek");
    assert_eq!(pkt.pts, Some(target), "next packet pts must equal target");
    assert_eq!(pkt.dts, Some(target));
    assert!(pkt.flags.keyframe);

    let want_start = (target as usize) * 2; // S16 mono → 2 bytes/frame
    let want_end = want_start + pkt.data.len();
    assert_eq!(
        pkt.data,
        &payload[want_start..want_end],
        "packet bytes must match the source at the seek offset"
    );

    // Seeking past EOF clamps to total_samples (returns `sample_rate`).
    let clamped = dmx.seek_to(0, i64::MAX).expect("seek past EOF clamps");
    assert_eq!(clamped, sample_rate as i64);
    assert!(matches!(dmx.next_packet(), Err(oxideav_core::Error::Eof)));

    // Seeking back to 0 must restore byte-for-byte streaming from the top.
    let zero = dmx.seek_to(0, 0).expect("seek to 0");
    assert_eq!(zero, 0);
    let mut out = Vec::new();
    loop {
        match dmx.next_packet() {
            Ok(p) => out.extend_from_slice(&p.data),
            Err(oxideav_core::Error::Eof) => break,
            Err(e) => panic!("demux error: {e}"),
        }
    }
    assert_eq!(
        out, payload,
        "full re-stream after seek(0) must match input"
    );

    // Non-zero stream index must fail.
    assert!(dmx.seek_to(1, 0).is_err());

    let _ = std::fs::remove_file(&tmp);
}
