//! `oxideav` on HEIF / HEIC / AVIF stills, end to end through the
//! built binary (round 460).
//!
//! Runs the actual `oxideav` executable (`CARGO_BIN_EXE_oxideav`) on
//! the real-world files the heif crate vendors under
//! `crates/oxideav-heif/tests/fixtures/interop/` — Apple ImageIO,
//! libheif and ImageMagick producers — and checks:
//!
//! * `probe` prints the `heif` layout: brand metadata, the still
//!   stream, and every image-sequence track;
//! * `convert photo.heic out.png` is **sample-exact** against the
//!   library decode pushed through the same pixel-format step the
//!   pipeline runs (`oxideav_pixfmt::convert`, default options);
//! * `run` with an explicit JSON job yields the same pixels;
//! * `list` / `info heif` surface the container and codec;
//! * `convert in.png out.heic` — the write path. Today the `heif`
//!   muxer refuses the `heif` codec's whole-file packets, so the test
//!   pins that exact diagnostic; the moment the muxer passes them
//!   through, the same test validates the written file with every
//!   third-party reader on the host and decodes it back.
//!
//! Fixture legs run everywhere (the sibling checkout carries the
//! files); third-party reader legs print SKIP when a binary is absent.

mod common;

use std::path::PathBuf;

use common::*;

// ───────────────────────────── probe ─────────────────────────────

#[test]
fn probe_heic_still_prints_heif_layout() {
    let f = fixtures().join("sips_grid_1024x768.heic");
    let r = oxideav(&["probe", f.to_str().unwrap()]);
    assert!(r.ok, "probe failed: {}", r.stderr);
    for needle in [
        "Format: heif",
        "major_brand       : heic",
        "primary_item_type : grid",
        "Stream #0 [Video]  codec=heif",
        "video 1024x768",
        "[YuvJ420P]",
    ] {
        assert!(
            r.stdout.contains(needle),
            "missing {needle:?} in:\n{}",
            r.stdout
        );
    }
    assert!(
        !r.stdout.contains("Stream #1"),
        "a still has one stream:\n{}",
        r.stdout
    );
}

#[test]
fn probe_heics_sequence_lists_cover_and_track() {
    let f = fixtures().join("sips_seq_rgb_96x80.heics");
    let r = oxideav(&["probe", f.to_str().unwrap()]);
    assert!(r.ok, "probe failed: {}", r.stderr);
    for needle in [
        "Format: heif",
        "major_brand       : msf1",
        "track:1:handler   : pict",
        "Stream #0 [Video]  codec=heif",
        "Stream #1 [Video]  codec=h265",
        "video 96x80",
    ] {
        assert!(
            r.stdout.contains(needle),
            "missing {needle:?} in:\n{}",
            r.stdout
        );
    }
}

#[test]
fn probe_avif_resolves_to_the_heif_container() {
    let f = fixtures().join("henc_avif_rgba_63x61.avif");
    let r = oxideav(&["probe", f.to_str().unwrap()]);
    assert!(r.ok, "probe failed: {}", r.stderr);
    for needle in [
        "Format: heif",
        "major_brand       : avif",
        "Stream #0 [Video]  codec=heif",
        "video 63x61",
        "[Yuva420P]",
    ] {
        assert!(
            r.stdout.contains(needle),
            "missing {needle:?} in:\n{}",
            r.stdout
        );
    }
}

// ───────────────────────── list / info ─────────────────────────

#[test]
fn list_and_info_surface_heif() {
    let r = oxideav(&["list"]);
    assert!(r.ok, "list failed: {}", r.stderr);
    let container = r
        .stdout
        .lines()
        .find(|l| l.trim_start().starts_with("heif ") && l.contains("DM"))
        .unwrap_or_else(|| panic!("no `heif DM` container row in:\n{}", r.stdout));
    assert!(container.contains("DM"), "{container}");
    assert!(
        r.stdout.contains("heif_container"),
        "heif codec backend missing in:\n{}",
        r.stdout
    );

    let r = oxideav(&["info", "heif"]);
    assert!(r.ok, "info failed: {}", r.stderr);
    for needle in [
        "Codec: heif",
        "media_type     : Video",
        "Backend: heif_container",
        "sides          : decode + encode",
        "intra-only",
        "Encoder options (",
    ] {
        assert!(
            r.stdout.contains(needle),
            "missing {needle:?} in:\n{}",
            r.stdout
        );
    }
    // Option NAMES only: the schema grows as the codec adopts producer
    // surface (r460 added quality/speed/rd/tiles); layout and count are
    // not pinned.
    let options: Vec<&str> = r
        .stdout
        .split("Encoder options (")
        .nth(1)
        .expect("encoder options block")
        .lines()
        .skip(1)
        .take_while(|l| l.starts_with("    "))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    for name in [
        "codec",
        "mode",
        "qp",
        "grid",
        "thumbnail",
        "range",
        "quality",
    ] {
        assert!(
            options.contains(&name),
            "option {name:?} missing from {options:?}"
        );
    }
}

// ───────────────────────── heic → png ─────────────────────────

/// Producer files across layouts: 4:2:0 / 4:4:4 / alpha / gray /
/// 10-bit / Apple grid. Each `convert` output PNG must equal the
/// library decode after the pipeline's own pixel-format step.
const DECODE_FILES: &[&str] = &[
    "sips_rgb_96x80.heic",
    "sips_rgb_63x61.heic",
    "henc_rgba_80x64.heic",
    "magick_rgba_80x64.heic",
    "henc_gray_96x80.heic",
    "henc_10bit_rgb16_96x80.heic",
    "henc_avif_rgb_96x80.avif",
    "sips_grid_1024x768.heic",
];

fn convert_to_png(tag: &str, name: &str) -> PathBuf {
    let src = fixtures().join(name);
    let out = scratch_file("images", &format!("{tag}_{name}.png"));
    let r = oxideav(&["convert", src.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(r.ok, "convert {name}: {}", r.stderr);
    assert!(out.exists(), "convert {name} wrote nothing");
    out
}

#[test]
fn convert_heic_to_png_is_sample_exact_with_the_library_decode() {
    let ctx = ctx();
    for name in DECODE_FILES {
        let out = convert_to_png("exact", name);
        let (png_fmt, pw, ph, png) = decode_still(&ctx, &out);
        let (fmt, w, h, frame) = decode_still(&ctx, &fixtures().join(name));
        assert_eq!((pw, ph), (w, h), "{name}: geometry");
        let lib = oxideav_pixfmt::convert(
            &frame,
            oxideav_pixfmt::FrameInfo::new(fmt, w, h),
            png_fmt,
            &oxideav_pixfmt::ConvertOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: {fmt:?} → {png_fmt:?}: {e}"));
        assert!(
            visible(&png, png_fmt, w, h) == visible(&lib, png_fmt, w, h),
            "{name}: CLI PNG ({png_fmt:?}) differs from the library decode ({fmt:?} → {png_fmt:?})"
        );
        eprintln!("{name}: {fmt:?} {w}x{h} → {png_fmt:?} sample-exact");
    }
}

#[test]
fn run_json_job_heic_to_png_matches_convert() {
    let ctx = ctx();
    let name = "sips_rgb_96x80.heic";
    let via_convert = convert_to_png("job", name);
    let src = fixtures().join(name);
    let out = scratch_file("images", "run_job.png");
    let job = format!(
        r#"{{"{}": {{"video": [{{"convert": "rgba", "input": {{"from": "{}"}}, "codec": "png"}}]}}}}"#,
        out.to_str().unwrap().replace('\\', "/"),
        src.to_str().unwrap().replace('\\', "/")
    );
    let r = oxideav(&["run", "--inline", &job]);
    assert!(r.ok, "run: {}", r.stderr);
    let (fa, wa, ha, a) = decode_still(&ctx, &via_convert);
    let (fb, wb, hb, b) = decode_still(&ctx, &out);
    assert_eq!((fa, wa, ha), (fb, wb, hb));
    assert!(
        visible(&a, fa, wa, ha) == visible(&b, fb, wb, hb),
        "run vs convert pixels"
    );
}

// ───────────────────────── png → heic ─────────────────────────

/// The write path. Pinned diagnostic while the `heif` muxer refuses
/// the `heif` codec's whole-file packets; full validation once it
/// passes them through (see the module docs).
#[test]
fn convert_png_to_heic_write_path() {
    let ctx = ctx();
    let src = fixtures().join("sips_rgb_96x80.expected.png");
    let out = scratch_file("images", "written.heic");
    let r = oxideav(&["convert", src.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(
        r.ok,
        "png -> heic must write through the heif muxer: {}",
        r.stderr
    );
    // Prove the file (the write path landed in round 460).
    let bytes = std::fs::read(&out).expect("read written heic");
    assert_eq!(&bytes[4..8], b"ftyp", "ISOBMFF signature");
    let (fmt, w, h, frame) = decode_still(&ctx, &out);
    assert_eq!((w, h), (96, 80), "written geometry");
    let (sfmt, _, _, source) = decode_still(&ctx, &src);
    let back = oxideav_pixfmt::convert(
        &frame,
        oxideav_pixfmt::FrameInfo::new(fmt, w, h),
        sfmt,
        &oxideav_pixfmt::ConvertOptions::default(),
    )
    .expect("convert back");
    let (a, b) = (visible(&back, sfmt, w, h), visible(&source, sfmt, w, h));
    let mse: f64 = a
        .iter()
        .zip(&b)
        .map(|(x, y)| {
            let d = *x as f64 - *y as f64;
            d * d
        })
        .sum::<f64>()
        / a.len() as f64;
    let psnr = if mse > 0.0 {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    } else {
        f64::INFINITY
    };
    assert!(psnr >= 30.0, "written HEIC round trip PSNR {psnr:.1} dB");
    let p = out.to_str().unwrap();
    let mut readers = Vec::new();
    if let Some(b) = tool("heif-info") {
        assert!(
            run_tool(&b, &[p]).is_ok(),
            "heif-info refused the written file"
        );
        readers.push("heif-info");
    }
    if let Some(b) = tool("/usr/bin/sips") {
        assert!(
            run_tool(&b, &["-g", "pixelWidth", p]).is_ok(),
            "sips refused"
        );
        readers.push("sips");
    }
    if let Some(b) = tool("magick") {
        assert!(run_tool(&b, &["identify", p]).is_ok(), "magick refused");
        readers.push("magick");
    }
    if let Some(b) = tool("ffprobe") {
        assert!(run_tool(&b, &["-v", "error", p]).is_ok(), "ffprobe refused");
        readers.push("ffprobe");
    }
    eprintln!("png → heic written and accepted by {readers:?}; round trip {psnr:.1} dB");
}

/// `transcode --codec-video h265` into the `heif` muxer writes an
/// HEVC image *sequence* (`msf1`) — the framework reads it back as a
/// cover still plus one `h265` track. Third-party readers' verdicts
/// are reported (the black-box video tool and Apple ImageIO open it;
/// libheif-based readers currently refuse it — see the round report).
#[test]
fn transcode_heic_to_heif_sequence_via_h265() {
    let src = fixtures().join("sips_rgb_96x80.heic");
    let out = scratch_file("images", "sequence.heic");
    let r = oxideav(&[
        "transcode",
        src.to_str().unwrap(),
        out.to_str().unwrap(),
        "--codec-video",
        "h265",
    ]);
    assert!(r.ok, "transcode: {}", r.stderr);
    let p = oxideav(&["probe", out.to_str().unwrap()]);
    assert!(p.ok, "probe of the written sequence: {}", p.stderr);
    for needle in [
        "Format: heif",
        "major_brand       : hevc",
        "track:1:handler   : pict",
        "Stream #0 [Video]  codec=heif",
        "Stream #1 [Video]  codec=h265",
        "video 96x80",
    ] {
        assert!(
            p.stdout.contains(needle),
            "missing {needle:?} in:\n{}",
            p.stdout
        );
    }
    let path = out.to_str().unwrap();
    if let Some(b) = tool("ffprobe") {
        assert!(
            run_tool(&b, &["-v", "error", "-show_streams", path]).is_ok(),
            "black-box video tool refused the written sequence"
        );
    }
    if let Some(b) = tool("/usr/bin/sips") {
        assert!(
            run_tool(&b, &["-g", "pixelWidth", path]).is_ok(),
            "sips refused"
        );
    }
    for reader in ["heif-info", "magick"] {
        if let Some(b) = tool(reader) {
            let args: &[&str] = if reader == "magick" {
                &["identify", path]
            } else {
                &[path]
            };
            match run_tool(&b, args) {
                Ok(_) => eprintln!("{reader}: opens the written sequence"),
                Err(e) => eprintln!(
                    "{reader}: refuses the written sequence — {}",
                    e.lines().last().unwrap_or("").trim()
                ),
            }
        }
    }
}
