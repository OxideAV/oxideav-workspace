//! End-to-end multi-stream `oxideav transcode` integration test.
//!
//! Synthesises a 2-stream Matroska input (one `pcm_s16le` + one
//! `pcm_f32le` audio track — both have round-trippable Matroska
//! `CodecID` mappings, see `oxideav_mkv::codec_id::to_matroska`), spawns
//! the `oxideav` binary against it with no codec overrides, and
//! confirms that:
//!
//!   1. The binary returns success (the historic "single-stream inputs
//!      only" rejection is gone).
//!   2. The output `.mkv` exists and parses as MKV with two streams.
//!   3. Each output stream's codec id matches the per-MediaType default
//!      the CLI applies (S16 → pcm_s16le, F32 → pcm_f32le).
//!   4. `--codec` and `--codec-audio` apply to *every* audio stream,
//!      forcing an explicit codec across the board.
//!
//! Why MKV + PCM: PCM is registered by `oxideav-basic` (always
//! available), MKV is registered by `oxideav-mkv` (also always
//! available) and is the only multi-stream container the workspace
//! ships with both demux + mux for two arbitrary audio codec ids.

use std::path::PathBuf;
use std::process::Command;

use oxideav_basic::pcm;
use oxideav_core::{
    packet::PacketFlags, ContainerRegistry, Packet, SampleFormat, StreamInfo, TimeBase, WriteSeek,
};
use oxideav_mkv::mux::open as open_mkv_mux;

const SAMPLE_RATE: u32 = 8_000;
const PACKETS_PER_STREAM: u32 = 5;
const SAMPLES_PER_PACKET: u32 = SAMPLE_RATE / 10; // 100 ms each

fn tmp_dir() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("oxideav_cli_multi_stream_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&p);
    p
}

/// Build a 2-stream Matroska file: stream 0 = pcm_s16le mono @ 8 kHz,
/// stream 1 = pcm_f32le mono @ 8 kHz. Returns the absolute path. Both
/// codec ids are registered in MKV's `to_matroska` mapping so the file
/// round-trips through demux without surfacing a synthetic `X_…` id.
fn build_two_stream_mkv(name: &str) -> PathBuf {
    let path = tmp_dir().join(format!("{name}.mkv"));
    let _ = std::fs::remove_file(&path);

    // Per-stream params via the public PCM helper — keeps us aligned
    // with the codec ids the codec registry will surface on the demux
    // side.
    let params_a = pcm::params(SampleFormat::S16, 1, SAMPLE_RATE).expect("pcm s16 params");
    let params_b = pcm::params(SampleFormat::F32, 1, SAMPLE_RATE).expect("pcm f32 params");

    let streams = vec![
        StreamInfo {
            index: 0,
            time_base: TimeBase::new(1, SAMPLE_RATE as i64),
            duration: Some((PACKETS_PER_STREAM * SAMPLES_PER_PACKET) as i64),
            start_time: Some(0),
            params: params_a,
        },
        StreamInfo {
            index: 1,
            time_base: TimeBase::new(1, SAMPLE_RATE as i64),
            duration: Some((PACKETS_PER_STREAM * SAMPLES_PER_PACKET) as i64),
            start_time: Some(0),
            params: params_b,
        },
    ];

    let out: Box<dyn WriteSeek> = Box::new(std::fs::File::create(&path).expect("create mkv"));
    let mut muxer = open_mkv_mux(out, &streams).expect("open mkv muxer");
    muxer.write_header().expect("mkv header");

    // Interleave packets in pts order: stream-0 then stream-1 per
    // 100 ms slice, mirroring how a real recorder would multiplex.
    for i in 0..PACKETS_PER_STREAM {
        let pts = (i * SAMPLES_PER_PACKET) as i64;

        // S16LE: 2 bytes per sample
        let mut s16 = Packet {
            stream_index: 0,
            time_base: TimeBase::new(1, SAMPLE_RATE as i64),
            pts: Some(pts),
            dts: Some(pts),
            duration: Some(SAMPLES_PER_PACKET as i64),
            flags: PacketFlags {
                keyframe: true,
                ..Default::default()
            },
            data: vec![0u8; (SAMPLES_PER_PACKET as usize) * 2],
        };
        // Non-trivial payload so a corrupted demux/mux would surface.
        for (k, slot) in s16.data.chunks_exact_mut(2).enumerate() {
            let v = ((k as i32 + i as i32 * 17) & 0x7FFF) as i16;
            slot.copy_from_slice(&v.to_le_bytes());
        }
        muxer.write_packet(&s16).expect("write s16 pkt");

        // F32LE: 4 bytes per sample
        let mut f32p = Packet {
            stream_index: 1,
            time_base: TimeBase::new(1, SAMPLE_RATE as i64),
            pts: Some(pts),
            dts: Some(pts),
            duration: Some(SAMPLES_PER_PACKET as i64),
            flags: PacketFlags {
                keyframe: true,
                ..Default::default()
            },
            data: vec![0u8; (SAMPLES_PER_PACKET as usize) * 4],
        };
        for (k, slot) in f32p.data.chunks_exact_mut(4).enumerate() {
            // Generate a small ramp so the bytes aren't all zero.
            let v = ((k as f32 + (i as f32) * 0.1) / 1024.0).sin();
            slot.copy_from_slice(&v.to_le_bytes());
        }
        muxer.write_packet(&f32p).expect("write f32 pkt");
    }

    muxer.write_trailer().expect("mkv trailer");
    path
}

/// Open the produced output via `oxideav-mkv`'s demuxer and return the
/// list of `(codec_id, sample_rate)` per stream.
fn read_output_streams(path: &std::path::Path) -> Vec<(String, Option<u32>)> {
    let mut containers = ContainerRegistry::default();
    oxideav_mkv::register_containers(&mut containers);
    let mut codecs = oxideav_core::CodecRegistry::default();
    oxideav_basic::register_codecs(&mut codecs);
    // Detect format via probe so the test stays honest about file
    // contents — a truncated / malformed file would fail here.
    let f = std::fs::File::open(path).expect("open output");
    let mut handle: Box<dyn oxideav_core::ReadSeek> = Box::new(f);
    let format = containers
        .probe_input(&mut *handle, Some("mkv"))
        .expect("probe output mkv");
    let demuxer = containers
        .open_demuxer(&format, handle, &codecs)
        .expect("open output demuxer");
    demuxer
        .streams()
        .iter()
        .map(|s| (s.params.codec_id.as_str().to_owned(), s.params.sample_rate))
        .collect()
}

#[test]
fn transcode_two_stream_mkv_default_codecs() {
    // Build the input.
    let in_path = build_two_stream_mkv("default_codecs_in");
    let out_path = in_path.with_file_name("default_codecs_out.mkv");
    let _ = std::fs::remove_file(&out_path);

    // Sanity-check: the input we just wrote really does have two
    // streams (otherwise the test wouldn't be exercising what it
    // claims to).
    let in_streams = read_output_streams(&in_path);
    assert_eq!(
        in_streams.len(),
        2,
        "synthetic input does not have two streams, got {in_streams:?}"
    );

    // Run the CLI.
    let bin = env!("CARGO_BIN_EXE_oxideav");
    let output = Command::new(bin)
        .arg("transcode")
        .arg(&in_path)
        .arg(&out_path)
        .output()
        .expect("spawn oxideav binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "oxideav transcode failed:\n  status: {:?}\n  stdout: {stdout}\n  stderr: {stderr}",
        output.status,
    );
    // The historic single-stream rejection must NOT appear.
    assert!(
        !stderr.contains("single-stream"),
        "transcode_simple still rejects multi-stream inputs: stderr={stderr}",
    );

    // Output exists, has two streams, codecs are the default
    // per-MediaType picks.
    let out_streams = read_output_streams(&out_path);
    assert_eq!(
        out_streams.len(),
        2,
        "expected 2 output streams, got {out_streams:?}",
    );
    assert_eq!(
        out_streams[0].0, "pcm_s16le",
        "stream 0 default did not pick pcm_s16le; full streams: {out_streams:?}",
    );
    assert_eq!(
        out_streams[1].0, "pcm_f32le",
        "stream 1 default did not pick pcm_f32le; full streams: {out_streams:?}",
    );
    assert_eq!(out_streams[0].1, Some(SAMPLE_RATE));
    assert_eq!(out_streams[1].1, Some(SAMPLE_RATE));
}

/// Build a 2-stream Matroska file where both streams use the same
/// sample format (S16). Used by the override tests where forcing a
/// codec across mismatched sample formats would require a format
/// converter the framework does not yet wire through `transcode_simple`.
fn build_two_stream_mkv_homogeneous(name: &str) -> PathBuf {
    let path = tmp_dir().join(format!("{name}.mkv"));
    let _ = std::fs::remove_file(&path);

    let params_a = pcm::params(SampleFormat::S16, 1, SAMPLE_RATE).expect("pcm s16 params A");
    let params_b = pcm::params(SampleFormat::S16, 1, SAMPLE_RATE).expect("pcm s16 params B");

    let streams = vec![
        StreamInfo {
            index: 0,
            time_base: TimeBase::new(1, SAMPLE_RATE as i64),
            duration: Some((PACKETS_PER_STREAM * SAMPLES_PER_PACKET) as i64),
            start_time: Some(0),
            params: params_a,
        },
        StreamInfo {
            index: 1,
            time_base: TimeBase::new(1, SAMPLE_RATE as i64),
            duration: Some((PACKETS_PER_STREAM * SAMPLES_PER_PACKET) as i64),
            start_time: Some(0),
            params: params_b,
        },
    ];

    let out: Box<dyn WriteSeek> = Box::new(std::fs::File::create(&path).expect("create mkv"));
    let mut muxer = open_mkv_mux(out, &streams).expect("open mkv muxer");
    muxer.write_header().expect("mkv header");

    for i in 0..PACKETS_PER_STREAM {
        let pts = (i * SAMPLES_PER_PACKET) as i64;
        for stream_index in [0u32, 1] {
            let mut pkt = Packet {
                stream_index,
                time_base: TimeBase::new(1, SAMPLE_RATE as i64),
                pts: Some(pts),
                dts: Some(pts),
                duration: Some(SAMPLES_PER_PACKET as i64),
                flags: PacketFlags {
                    keyframe: true,
                    ..Default::default()
                },
                data: vec![0u8; (SAMPLES_PER_PACKET as usize) * 2],
            };
            for (k, slot) in pkt.data.chunks_exact_mut(2).enumerate() {
                let v = ((k as i32 + i as i32 * 17 + stream_index as i32 * 3) & 0x7FFF) as i16;
                slot.copy_from_slice(&v.to_le_bytes());
            }
            muxer.write_packet(&pkt).expect("write s16 pkt");
        }
    }

    muxer.write_trailer().expect("mkv trailer");
    path
}

#[test]
fn transcode_two_stream_mkv_codec_global_override() {
    // `--codec pcm_s16le` forces every audio stream through the s16le
    // encoder. Both source streams are already s16, but the encoder is
    // wired up explicitly via the override (rather than via the
    // per-stream default fallthrough). Two assertions: (a) every output
    // stream is pcm_s16le, (b) the historic single-stream rejection
    // path is not surfaced.
    let in_path = build_two_stream_mkv_homogeneous("global_override_in");
    let out_path = in_path.with_file_name("global_override_out.mkv");
    let _ = std::fs::remove_file(&out_path);

    let bin = env!("CARGO_BIN_EXE_oxideav");
    let output = Command::new(bin)
        .arg("transcode")
        .arg("--codec")
        .arg("pcm_s16le")
        .arg(&in_path)
        .arg(&out_path)
        .output()
        .expect("spawn oxideav binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "oxideav transcode --codec pcm_s16le failed: status={:?} stderr={stderr}",
        output.status,
    );
    assert!(!stderr.contains("single-stream"));

    let out_streams = read_output_streams(&out_path);
    assert_eq!(out_streams.len(), 2);
    for (idx, (codec_id, _)) in out_streams.iter().enumerate() {
        assert_eq!(
            codec_id, "pcm_s16le",
            "stream {idx} did not pick up the --codec override",
        );
    }
}

#[test]
fn transcode_two_stream_mkv_per_type_override_audio() {
    // `--codec-audio pcm_s16le` should reach every audio stream in the
    // input. Same input as `_codec_global_override` but routed through
    // the typed `--codec-audio` flag instead — this proves the
    // per-MediaType dispatch table is wired correctly.
    let in_path = build_two_stream_mkv_homogeneous("per_type_override_in");
    let out_path = in_path.with_file_name("per_type_override_out.mkv");
    let _ = std::fs::remove_file(&out_path);

    let bin = env!("CARGO_BIN_EXE_oxideav");
    let output = Command::new(bin)
        .arg("transcode")
        .arg("--codec-audio")
        .arg("pcm_s16le")
        .arg(&in_path)
        .arg(&out_path)
        .output()
        .expect("spawn oxideav binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "oxideav transcode --codec-audio pcm_s16le failed: status={:?} stderr={stderr}",
        output.status,
    );

    let out_streams = read_output_streams(&out_path);
    assert_eq!(out_streams.len(), 2);
    for (idx, (codec_id, _)) in out_streams.iter().enumerate() {
        assert_eq!(
            codec_id, "pcm_s16le",
            "stream {idx} did not pick up the --codec-audio override",
        );
    }
}
