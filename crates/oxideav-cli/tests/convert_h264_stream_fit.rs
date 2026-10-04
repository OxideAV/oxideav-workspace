//! `oxideav convert` / `transcode` on H.264 inputs, end to end through
//! the built binary.
//!
//! * H.264 from MP4, Matroska and a raw Annex-B `.h264` byte stream
//!   decodes every frame and writes the same pictures as the library
//!   decode (`h264_sw`) — with the default codec preferences (hardware
//!   backends allowed, so an NVDEC / VA-API / Vulkan host exercises its
//!   own path) and with `--no-hwaccel`. Regression: the CLI decoded 0
//!   frames (a hardware decoder fed length-prefixed samples, demuxer
//!   metadata rejected as encoder options, no pixel layout declared,
//!   no raw-stream demuxer).
//! * A video + two-audio input written to an audio-only or video-only
//!   output keeps the streams that fit (the best audio stream for
//!   `.wav` / `.flac`, the video for `.y4m`) and notes what it dropped,
//!   instead of failing.
//!
//! Fixtures (`tests/fixtures/`, written by x264 + an external muxer):
//! `h264_testsrc_128x96_25f.{mp4,mkv}` (High profile, B-frames,
//! weighted prediction) and `h264_64x48_two_flac.mkv` (H.264 64×48 +
//! FLAC 8 kHz mono + FLAC 16 kHz mono). The raw byte stream is the
//! oxideav-h264 crate's own fixture of the same pictures.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use oxideav_core::{CodecId, CodecParameters, Frame};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn raw_h264() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oxideav-h264/tests/fixtures/x264_testsrc_128x96_25f.h264")
}

const W: usize = 128;
const H: usize = 96;
const FRAMES: usize = 25;

/// The pictures of a 4:2:0 8-bit Y4M file, one `Vec` per frame.
fn y4m_frames(path: &Path) -> Vec<Vec<u8>> {
    let data = std::fs::read(path).expect("read y4m");
    let header_end = data.iter().position(|&b| b == b'\n').expect("header") + 1;
    let header = String::from_utf8_lossy(&data[..header_end]).into_owned();
    assert!(header.contains(&format!(" W{W} H{H} ")), "{header}");
    let size = W * H * 3 / 2;
    let mut frames = Vec::new();
    let mut pos = header_end;
    while pos < data.len() {
        let nl = pos + data[pos..].iter().position(|&b| b == b'\n').expect("FRAME");
        assert!(data[pos..nl].starts_with(b"FRAME"), "frame marker");
        let start = nl + 1;
        frames.push(data[start..start + size].to_vec());
        pos = start + size;
    }
    frames
}

/// The library decode of the raw byte stream with the software decoder.
fn library_frames() -> Vec<Vec<u8>> {
    let ctx = ctx();
    let mut params = CodecParameters::video(CodecId::new("h264"));
    params.width = Some(W as u32);
    params.height = Some(H as u32);
    let mut dec = ctx.codecs.decoder_by_impl("h264_sw", &params).unwrap();
    let data = std::fs::read(raw_h264()).unwrap();
    dec.send_packet(&oxideav_core::Packet::new(
        0,
        oxideav_core::TimeBase::new(1, 25),
        data,
    ))
    .unwrap();
    dec.flush().unwrap();
    let mut out = Vec::new();
    while let Ok(Frame::Video(v)) = dec.receive_frame() {
        let mut pic = Vec::new();
        for (i, p) in v.planes.iter().take(3).enumerate() {
            let (w, h) = if i == 0 { (W, H) } else { (W / 2, H / 2) };
            for row in 0..h {
                pic.extend_from_slice(&p.data[row * p.stride..row * p.stride + w]);
            }
        }
        out.push(pic);
    }
    out
}

#[test]
fn h264_from_mp4_mkv_and_raw_decodes_every_frame_like_the_library() {
    let reference = library_frames();
    assert_eq!(reference.len(), FRAMES, "library decode");
    let inputs = [
        fixture("h264_testsrc_128x96_25f.mp4"),
        fixture("h264_testsrc_128x96_25f.mkv"),
        raw_h264(),
    ];
    for input in &inputs {
        for prefs in [&[][..], &["--no-hwaccel"][..]] {
            let name = input.file_name().unwrap().to_string_lossy();
            let tag = format!("{name}{}", if prefs.is_empty() { "" } else { "_sw" });
            let out = scratch_file("h264", &format!("{tag}.y4m"));
            let mut args: Vec<&str> = prefs.to_vec();
            args.extend(["convert", input.to_str().unwrap(), out.to_str().unwrap()]);
            let r = oxideav(&args);
            assert!(r.ok, "{tag}: {}", r.stderr);
            assert!(
                r.stderr.contains(&format!("{FRAMES} frame(s) decoded")),
                "{tag}: {}",
                r.stderr
            );
            let frames = y4m_frames(&out);
            assert_eq!(frames.len(), FRAMES, "{tag}: frames written");
            for (i, (got, want)) in frames.iter().zip(&reference).enumerate() {
                assert!(
                    got == want,
                    "{tag}: frame {i} differs from the library decode"
                );
            }
        }
    }
}

#[test]
fn h264_mp4_to_png_sequence_writes_every_frame() {
    let dir = scratch("h264-png");
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let _ = std::fs::remove_file(e.path());
    }
    let pattern = dir.join("f_%02d.png");
    let input = fixture("h264_testsrc_128x96_25f.mp4");
    let r = oxideav(&[
        "--no-hwaccel",
        "convert",
        input.to_str().unwrap(),
        pattern.to_str().unwrap(),
    ]);
    assert!(r.ok, "{}", r.stderr);
    let pngs = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
        .count();
    assert_eq!(pngs, FRAMES, "{}", r.stderr);
}

/// `oxideav probe` stream lines of `path`.
fn probed_streams(path: &Path) -> Vec<String> {
    let r = oxideav(&["probe", path.to_str().unwrap()]);
    assert!(r.ok, "probe {}: {}", path.display(), r.stderr);
    r.stdout
        .lines()
        .filter(|l| l.trim_start().starts_with("Stream #"))
        .map(str::to_string)
        .collect()
}

#[test]
fn audio_only_outputs_keep_the_best_audio_stream_and_note_the_rest() {
    let input = fixture("h264_64x48_two_flac.mkv");
    for ext in ["wav", "flac"] {
        let out = scratch_file("stream-fit", &format!("audio.{ext}"));
        let r = oxideav(&["convert", input.to_str().unwrap(), out.to_str().unwrap()]);
        assert!(r.ok, ".{ext}: {}", r.stderr);
        assert!(
            r.stderr
                .contains("note: dropping video stream #0 (h264): the output cannot hold video"),
            ".{ext}: {}",
            r.stderr
        );
        assert!(
            r.stderr.contains(
                "dropping audio stream #1 (flac): the output holds one audio stream; keeping #2"
            ),
            ".{ext}: {}",
            r.stderr
        );
        let streams = probed_streams(&out);
        assert_eq!(streams.len(), 1, ".{ext}: {streams:?}");
        assert!(streams[0].contains("[Audio]"), ".{ext}: {streams:?}");
        assert!(
            streams[0].contains("16000 Hz"),
            ".{ext}: best = 16 kHz: {streams:?}"
        );
    }

    // `transcode` makes the same selection.
    let out = scratch_file("stream-fit", "transcoded.wav");
    let r = oxideav(&["transcode", input.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(r.ok, "transcode: {}", r.stderr);
    assert!(
        r.stderr.contains("dropping video stream #0"),
        "{}",
        r.stderr
    );
    let streams = probed_streams(&out);
    assert_eq!(streams.len(), 1, "{streams:?}");
    assert!(streams[0].contains("16000 Hz"), "{streams:?}");
}

#[test]
fn video_only_output_drops_the_audio_streams() {
    let input = fixture("h264_64x48_two_flac.mkv");
    let out = scratch_file("stream-fit", "video.y4m");
    let r = oxideav(&["convert", input.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(r.ok, "{}", r.stderr);
    for i in [1, 2] {
        assert!(
            r.stderr.contains(&format!(
                "dropping audio stream #{i} (flac): the output cannot hold audio"
            )),
            "{}",
            r.stderr
        );
    }
    let streams = probed_streams(&out);
    assert_eq!(streams.len(), 1, "{streams:?}");
    assert!(
        streams[0].contains("[Video]") && streams[0].contains("64x48"),
        "{streams:?}"
    );
}

#[test]
fn a_container_that_holds_everything_keeps_every_stream() {
    let input = fixture("h264_64x48_two_flac.mkv");
    let out = scratch_file("stream-fit", "all.mkv");
    let r = oxideav(&["transcode", input.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(r.ok, "{}", r.stderr);
    assert!(!r.stderr.contains("dropping"), "{}", r.stderr);
    assert_eq!(probed_streams(&out).len(), 3);
}
