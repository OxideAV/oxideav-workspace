//! H.264 frame cropping (ITU-T H.264 §7.4.2.1.1) end to end through the
//! built binary, against ffmpeg's decode as a black-box oracle.
//!
//! A picture whose size is not a multiple of the 16x16 macroblock is
//! coded at the next macroblock multiple and the SPS carries a
//! cropping window (`frame_cropping_flag`, `frame_crop_*_offset`
//! scaled by `CropUnitX` / `CropUnitY`). Regression: the decoder
//! ignored the window, so a 1080p stream came out 1088 rows tall and
//! the pipeline then resampled the 112x64 pictures of a 100x60 clip to
//! the 100x60 the container declared; the NVDEC backend stretched the
//! cropped area back to the coded size.
//!
//! Every case writes `.y4m` with the default codec preferences (a
//! hardware backend runs when the host has one) and with
//! `--no-hwaccel`, then compares the frame size and every sample with
//! `ffmpeg -i <input> -f rawvideo -pix_fmt yuv420p`.
//!
//! Fixtures are generated at test time (skipped when `ffmpeg` or its
//! libx264 encoder is absent):
//!
//! ```text
//! ffmpeg -f lavfi -i testsrc2=size=WxH:rate=25 -frames:v N \
//!        -c:v libx264 -pix_fmt yuv420p out.{mp4,mkv,h264}
//! ffmpeg ... -c:v libx264 -flags +ildct+ilme -x264-params tff=1 out.mp4
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;

fn have_x264() -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("libx264"))
        .unwrap_or(false)
}

fn encode(size: &str, frames: u32, extra: &[&str], out: &Path) {
    let frames = frames.to_string();
    let mut args = vec![
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
    ];
    let src = format!("testsrc2=size={size}:rate=25");
    args.push(&src);
    args.extend([
        "-frames:v",
        &frames,
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
    ]);
    args.extend(extra);
    args.push(out.to_str().unwrap());
    let st = Command::new("ffmpeg").args(&args).status().expect("ffmpeg");
    assert!(st.success(), "ffmpeg encode {size}");
}

/// ffmpeg's decode of `input` as packed 4:2:0 8-bit pictures.
fn ffmpeg_decode(input: &Path) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .expect("ffmpeg decode");
    assert!(out.status.success(), "ffmpeg decode {}", input.display());
    out.stdout
}

/// `(width, height, concatenated picture bytes)` of a 4:2:0 Y4M file.
fn y4m(path: &Path) -> (usize, usize, Vec<u8>) {
    let data = std::fs::read(path).expect("read y4m");
    let header_end = data.iter().position(|&b| b == b'\n').expect("header") + 1;
    let header = String::from_utf8_lossy(&data[..header_end]).into_owned();
    let field = |tag: char| -> usize {
        header
            .split_whitespace()
            .find_map(|t| t.strip_prefix(tag))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("{tag} in {header}"))
    };
    let (w, h) = (field('W'), field('H'));
    let size = w * h * 3 / 2;
    let mut pictures = Vec::new();
    let mut pos = header_end;
    while pos < data.len() {
        let nl = pos + data[pos..].iter().position(|&b| b == b'\n').expect("FRAME");
        assert!(data[pos..nl].starts_with(b"FRAME"), "frame marker");
        let start = nl + 1;
        pictures.extend_from_slice(&data[start..start + size]);
        pos = start + size;
    }
    (w, h, pictures)
}

/// Convert `input` to Y4M with and without hardware backends and check
/// the size and samples against ffmpeg.
fn check(input: &Path, w: usize, h: usize, frames: usize) {
    let reference = ffmpeg_decode(input);
    assert_eq!(reference.len(), w * h * 3 / 2 * frames, "ffmpeg reference");
    let probe = oxideav(&["probe", input.to_str().unwrap()]);
    assert!(probe.ok, "probe: {}", probe.stderr);
    assert!(
        probe.stdout.contains(&format!("video {w}x{h}")),
        "{}: declared size: {}",
        input.display(),
        probe.stdout
    );
    let name = input.file_name().unwrap().to_string_lossy().into_owned();
    for prefs in [&[][..], &["--no-hwaccel"][..]] {
        let tag = format!("{name}{}", if prefs.is_empty() { "" } else { "_sw" });
        let out = scratch_file("h264-crop", &format!("{tag}.y4m"));
        let mut args: Vec<&str> = prefs.to_vec();
        args.extend(["convert", input.to_str().unwrap(), out.to_str().unwrap()]);
        let r = oxideav(&args);
        assert!(r.ok, "{tag}: {}", r.stderr);
        let (gw, gh, got) = y4m(&out);
        assert_eq!((gw, gh), (w, h), "{tag}: output size");
        assert_eq!(got.len(), reference.len(), "{tag}: frame count");
        let mismatch = got.iter().zip(&reference).position(|(a, b)| a != b);
        assert!(
            mismatch.is_none(),
            "{tag}: differs from ffmpeg at byte {mismatch:?}"
        );
    }
}

fn dir() -> PathBuf {
    scratch("h264-crop-src")
}

#[test]
fn non_macroblock_aligned_h264_is_cropped_in_every_container() {
    if !have_x264() {
        eprintln!("skipped: ffmpeg with libx264 not available");
        return;
    }
    for ext in ["mp4", "mkv", "h264"] {
        let input = dir().join(format!("crop_100x60.{ext}"));
        encode("100x60", 10, &[], &input);
        check(&input, 100, 60, 10);
    }
}

#[test]
fn h264_1080p_is_1080_rows_not_1088() {
    if !have_x264() {
        eprintln!("skipped: ffmpeg with libx264 not available");
        return;
    }
    let input = dir().join("crop_1920x1080.mp4");
    encode("1920x1080", 2, &["-preset", "ultrafast"], &input);
    check(&input, 1920, 1080, 2);
}

#[test]
fn interlaced_h264_crops_with_the_field_crop_unit() {
    // frame_mbs_only_flag = 0: CropUnitY = 2 * SubHeightC = 4.
    if !have_x264() {
        eprintln!("skipped: ffmpeg with libx264 not available");
        return;
    }
    let input = dir().join("crop_interlaced_100x60.mkv");
    encode(
        "100x60",
        6,
        &["-flags", "+ildct+ilme", "-x264-params", "tff=1"],
        &input,
    );
    check(&input, 100, 60, 6);
}

/// A sample entry that declares the coded size (some writers put the
/// macroblock-aligned 112x64 in `avc1`): the demuxer reports the size
/// the SPS cropping window gives, which is what the decoder emits.
#[test]
fn mp4_declaring_the_coded_size_reports_the_cropped_size() {
    if !have_x264() {
        eprintln!("skipped: ffmpeg with libx264 not available");
        return;
    }
    let input = dir().join("coded_size_src.mp4");
    encode("100x60", 4, &[], &input);
    let mut data = std::fs::read(&input).unwrap();
    let stsd = data.windows(4).position(|w| w == b"stsd").expect("stsd");
    let avc1 = stsd
        + data[stsd..]
            .windows(4)
            .position(|w| w == b"avc1")
            .expect("avc1");
    // VisualSampleEntry: 6 reserved + 2 data_reference_index + 16
    // pre_defined/reserved bytes, then width / height (u16 BE).
    let at = avc1 + 4 + 24;
    assert_eq!(&data[at..at + 4], &[0, 100, 0, 60]);
    data[at..at + 4].copy_from_slice(&[0, 112, 0, 64]);
    let patched = dir().join("coded_size_112x64.mp4");
    std::fs::write(&patched, &data).unwrap();
    check(&patched, 100, 60, 4);
}
