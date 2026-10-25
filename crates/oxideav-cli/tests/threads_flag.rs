//! The global `--threads N` budget (0 / absent = auto, 1 = serial)
//! parses before or after every subcommand that builds codecs, and the
//! output does not depend on it.
//!
//! Fixture: `tests/fixtures/h264_testsrc_128x96_25f.mp4` (x264 High
//! profile with B-frames).

mod common;

use common::*;

fn fixture_mp4() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/h264_testsrc_128x96_25f.mp4")
        .to_str()
        .unwrap()
        .to_owned()
}

#[test]
fn transcode_output_is_identical_at_every_budget() {
    let input = fixture_mp4();
    let mut outputs = Vec::new();
    for (i, threads) in [None, Some("1"), Some("4"), Some("0")]
        .into_iter()
        .enumerate()
    {
        let out = scratch_file("threads", &format!("transcode_{i}.y4m"));
        let out_s = out.to_str().unwrap();
        let mut args = vec!["--no-hwaccel"];
        // Before and after the subcommand: the flag is global.
        if i % 2 == 0 {
            if let Some(t) = threads {
                args.extend(["--threads", t]);
            }
            args.extend(["transcode", &input, out_s]);
        } else {
            args.extend(["transcode", &input, out_s]);
            if let Some(t) = threads {
                args.extend(["--threads", t]);
            }
        }
        let r = oxideav(&args);
        assert!(r.ok, "{args:?}: {}", r.stderr);
        outputs.push(std::fs::read(&out).unwrap());
    }
    assert!(outputs[0].len() > 128 * 96, "y4m too small");
    for o in &outputs[1..] {
        assert!(o == &outputs[0], "output differs between thread budgets");
    }
}

#[test]
fn run_takes_the_global_threads_flag() {
    let input = fixture_mp4();
    let mut outputs = Vec::new();
    for threads in ["1", "3"] {
        let out = scratch_file("threads", &format!("run_{threads}.y4m"));
        let job = format!(
            r#"{{"{}": {{"video": [{{"from": "{}", "codec": "rawvideo"}}]}}}}"#,
            out.to_str().unwrap().replace('\\', "/"),
            input.replace('\\', "/")
        );
        // `oxideav run --threads N` keeps working now that the flag
        // is global.
        let r = oxideav(&[
            "--no-hwaccel",
            "run",
            "--inline",
            &job,
            "--threads",
            threads,
        ]);
        assert!(r.ok, "run --threads {threads}: {}", r.stderr);
        outputs.push(std::fs::read(&out).unwrap());
    }
    assert!(
        outputs[0] == outputs[1],
        "run output differs between budgets"
    );
}

#[test]
fn threads_rejects_a_non_number() {
    let r = oxideav(&["--threads", "many", "list"]);
    assert_eq!(r.code, Some(2), "{}", r.stderr);
    assert!(r.stderr.contains("--threads"), "{}", r.stderr);
}

/// `oxideav bench h264` at a size that is not a whole number of
/// macroblocks: the encoder codes the next 16-aligned size, and the
/// decode side benches that stream instead of failing to synthesise
/// it (it used to report "can't synth decode stream" at 1920x1080).
#[test]
fn bench_h264_at_an_unaligned_size_decodes() {
    let r = oxideav(&[
        "--threads",
        "2",
        "bench",
        "h264",
        "--width",
        "120",
        "--height",
        "68",
        "--duration",
        "0.05",
        "--prep-frames",
        "3",
    ]);
    assert!(r.ok, "{}", r.stderr);
    assert!(r.stdout.contains("codec threads   : 2"), "{}", r.stdout);
    assert!(
        !r.stdout.contains("can't synth decode stream"),
        "{}",
        r.stdout
    );
    let sw_rows: Vec<&str> = r.stdout.lines().filter(|l| l.contains("h264_sw")).collect();
    assert!(
        sw_rows.iter().any(|l| l.contains("decode")),
        "no h264_sw decode row: {}",
        r.stdout
    );
    for l in &sw_rows {
        assert!(!l.contains("init:"), "h264_sw row failed: {l}");
    }
}

/// The `--codec-option` transcode path (its own decode → encode loop)
/// takes the budget too, and drains the decoder at end of stream: the
/// fixture's B-frame reorder tail (9 of its 25 pictures) used to be
/// lost there.
#[test]
fn transcode_with_codec_options_keeps_every_picture() {
    let input = fixture_mp4();
    let out = scratch_file("threads", "options.mkv");
    let r = oxideav(&[
        "--no-hwaccel",
        "--threads",
        "2",
        "transcode",
        &input,
        out.to_str().unwrap(),
        "--codec-video",
        "h264",
        "--codec-option",
        "qp=20",
    ]);
    assert!(r.ok, "{}", r.stderr);
    assert!(
        r.stdout.contains("25 frames decoded, 25 pkts out"),
        "{}",
        r.stdout
    );
}
