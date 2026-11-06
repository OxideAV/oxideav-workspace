//! Mid-stream output-shape changes (`Frame::StreamParams`) through the
//! CLI: `probe --decode` lists them, `transcode` follows them — the
//! pipeline route and the `--codec-option` route alike — and an encoder
//! that cannot follow fails the run with a typed error naming both
//! shapes. A change is never counted as a decoded frame.
//!
//! Fixtures: the spliced streams `oxideav-tests` keeps under
//! `tests/fixtures/stream_params/` (same umbrella repository; see the
//! `generate.sh` next to them).

mod common;

use common::*;

fn spliced(name: &str) -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oxideav-tests/tests/fixtures/stream_params")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

fn probe_decode(path: &str) -> String {
    let r = oxideav(&["--no-hwaccel", "probe", "--decode", path]);
    assert!(r.ok, "probe --decode {path}: {}", r.stderr);
    r.stdout
}

#[test]
fn probe_decode_lists_every_mid_stream_change() {
    for (name, summary, changes) in [
        (
            "h264_resize_64x48_96x64.h264",
            "Stream #0: 12 frames, 64x48 Yuv420P, 1 mid-stream shape change",
            &["from frame 6: 96x64 Yuv420P"][..],
        ),
        (
            "mp3_rate_44100_32000.mka",
            "Stream #0: 19 frames, 44100 Hz, 2 ch, S16P, 1 mid-stream shape change",
            &["from frame 11 (pts 287, 0.287s): 32000 Hz, 2 ch, S16P"][..],
        ),
        (
            "vp8_resize_64x48_96x64.webm",
            "Stream #0: 10 frames, 64x48 Yuv420P, 1 mid-stream shape change",
            &["from frame 5 (pts 500, 0.500s): 96x64 Yuv420P"][..],
        ),
        (
            "vp9_resize_64x48_96x64.webm",
            "Stream #0: 10 frames, 64x48 Yuv420P, 1 mid-stream shape change",
            &["from frame 5 (pts 500, 0.500s): 96x64 Yuv420P"][..],
        ),
        (
            "mjpeg_mixed_64x48_48x32_64x48.avi",
            "Stream #0: 6 frames, 64x48 YuvJ420P, 2 mid-stream shape changes",
            &[
                "from frame 2 (pts 2, 0.200s): 48x32 YuvJ420P",
                "from frame 4 (pts 4, 0.400s): 64x48 YuvJ420P",
            ][..],
        ),
    ] {
        let out = probe_decode(&spliced(name));
        let decoded = out
            .split_once("Decoded:\n")
            .unwrap_or_else(|| panic!("{name}: no Decoded block:\n{out}"))
            .1;
        let lines: Vec<&str> = decoded.lines().map(str::trim).collect();
        assert_eq!(lines[0], summary, "{name}:\n{out}");
        assert_eq!(&lines[1..], changes, "{name}:\n{out}");
    }
}

#[test]
fn probe_without_decode_does_not_decode() {
    let out = oxideav(&["probe", &spliced("h264_resize_64x48_96x64.h264")]);
    assert!(out.ok, "{}", out.stderr);
    assert!(!out.stdout.contains("Decoded:"), "{}", out.stdout);
}

/// The `--codec-option` route (the CLI's own decode → encode loop) and
/// the pipeline route both hand the h264 change to the h264 encoder,
/// which re-configures: 12 frames decoded (the event is not one), and
/// the output decodes back with the same switch at frame 6.
#[test]
fn h264_switch_is_followed_by_a_reconfiguring_encoder() {
    let input = spliced("h264_resize_64x48_96x64.h264");
    for (tag, extra) in [("options", &["-o", "qp=30"][..]), ("pipeline", &[][..])] {
        let out = scratch_file("stream_params", &format!("h264_{tag}.mkv"));
        let out_s = out.to_str().unwrap();
        let mut args = vec![
            "--no-hwaccel",
            "transcode",
            &input,
            out_s,
            "--codec-video",
            "h264",
        ];
        args.extend_from_slice(extra);
        let r = oxideav(&args);
        assert!(r.ok, "{tag}: {}", r.stderr);
        assert!(
            r.stdout
                .contains("12 pkts in, 12 frames decoded, 12 pkts out"),
            "{tag}: {}",
            r.stdout
        );
        if tag == "options" {
            assert!(
                r.stdout.contains("1 mid-stream shape change followed"),
                "{tag}: {}",
                r.stdout
            );
        }
        let back = probe_decode(out_s);
        assert!(
            back.contains("Stream #0: 12 frames, 64x48 Yuv420P, 1 mid-stream shape change")
                && back.contains("from frame 6 (pts 600, 0.600s): 96x64 Yuv420P"),
            "{tag}: output decode:\n{back}"
        );
    }
}

#[test]
fn mjpeg_size_changes_are_followed_with_options() {
    let input = spliced("mjpeg_mixed_64x48_48x32_64x48.avi");
    let out = scratch_file("stream_params", "mjpeg_options.mkv");
    let r = oxideav(&[
        "--no-hwaccel",
        "transcode",
        &input,
        out.to_str().unwrap(),
        "--codec-video",
        "mjpeg",
        "-o",
        "quality=80",
    ]);
    assert!(r.ok, "{}", r.stderr);
    assert!(
        r.stdout
            .contains("6 frames decoded, 6 pkts out, 2 mid-stream shape changes followed"),
        "{}",
        r.stdout
    );
}

/// An encoder that cannot re-configure fails the run with
/// `Error::Unsupported` (exit code 4) naming both shapes, on either
/// route.
#[test]
fn an_encoder_that_cannot_follow_fails_with_both_shapes() {
    for (name, codec_flag, codec, opts, out_name, old, new) in [
        (
            "vp9_resize_64x48_96x64.webm",
            "--codec-video",
            "ffv1",
            &["-o", "level=3"][..],
            "vp9_ffv1_options.mkv",
            "64x48 Yuv420P",
            "96x64 Yuv420P",
        ),
        (
            "vp9_resize_64x48_96x64.webm",
            "--codec-video",
            "ffv1",
            &[][..],
            "vp9_ffv1_pipeline.mkv",
            "64x48 Yuv420P",
            "96x64 Yuv420P",
        ),
        (
            "mp3_rate_44100_32000.mka",
            "--codec-audio",
            "flac",
            &["-o", "level=5"][..],
            "mp3_flac_options.flac",
            "44100 Hz",
            "32000 Hz",
        ),
        (
            "mp3_rate_44100_32000.mka",
            "--codec-audio",
            "pcm_s16le",
            &[][..],
            "mp3_pcm_pipeline.wav",
            "44100 Hz",
            "32000 Hz",
        ),
    ] {
        let input = spliced(name);
        let out = scratch_file("stream_params", out_name);
        let _ = std::fs::remove_file(&out);
        let mut args = vec![
            "--no-hwaccel",
            "transcode",
            &input,
            out.to_str().unwrap(),
            codec_flag,
            codec,
        ];
        args.extend_from_slice(opts);
        let r = oxideav(&args);
        assert!(!r.ok, "{name} → {codec} {opts:?} succeeded: {}", r.stdout);
        assert_eq!(r.code, Some(4), "{name} → {codec}: exit code: {}", r.stderr);
        assert!(
            r.stderr.contains("unsupported")
                && r.stderr.contains("mid-stream")
                && r.stderr.contains(old)
                && r.stderr.contains(new),
            "{name} → {codec} {opts:?}: {}",
            r.stderr
        );
        assert!(
            !out.exists(),
            "{name} → {codec}: a failed run leaves no output"
        );
    }
}
