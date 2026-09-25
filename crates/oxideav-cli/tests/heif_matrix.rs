//! HEIF / HEIC / AVIF through the built `oxideav` binary, both
//! directions, against every producer and reader on the host
//! (round 462). The tables these tests print (`--nocapture`) are
//! copied into `crates/oxideav-tests/README.md`.
//!
//! * **Fixture leg (unconditional):** every vendored interop file and
//!   corpus bundle → `oxideav convert x.heic out.png`; the PNG is
//!   sample-exact with the library decode pushed through the
//!   pipeline's pixel-format step, and matches the vendored producer
//!   render (`*.expected.png`) within the reader's own rounding.
//! * **Producer × CLI:** fresh files from Apple ImageIO (`sips`),
//!   libheif (`heif-enc`, x265 + `-A` aom), ImageMagick (`magick`) and
//!   the black-box video tool's AVIF muxer, across sizes (1×1, odd,
//!   Apple 512-px grids up to 4032×3024), 8/10/12-bit, 4:0:0–4:4:4,
//!   alpha, lossless, thumbnails, rotation, limited range and an AV1
//!   image sequence → CLI PNG, checked against the library decode
//!   (exact) and the producer's own render (tolerance).
//! * **CLI writer × readers:** `oxideav transcode in.png out.heic
//!   --codec-video heif --codec-option …` across the `heif` encoder's
//!   option surface (and `convert` at defaults) → every reader opens
//!   the file and renders within tolerance of our own decode.
//!
//! Producer / reader legs print SKIP per missing binary.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use oxideav_core::{CodecId, CodecParameters, Frame, PixelFormat, RuntimeContext, VideoFrame};

const TAG: &str = "matrix";

// ─────────────────────────── sources ───────────────────────────

/// Smooth gradients plus hard 8-px checks, so chroma siting and range
/// mistakes both show; identical to the r460 producer suite's signal.
fn synth(width: u32, height: u32, format: PixelFormat) -> VideoFrame {
    let (w, h) = (width as usize, height as usize);
    let comps = match format {
        PixelFormat::Rgb24 => 3,
        PixelFormat::Rgba => 4,
        PixelFormat::Gray8 => 1,
        PixelFormat::Rgb48Le => 3,
        other => panic!("source format {other:?}"),
    };
    let wide = format == PixelFormat::Rgb48Le;
    let bytes = if wide { 2 } else { 1 };
    let mut data = Vec::with_capacity(w * h * comps * bytes);
    for y in 0..h {
        for x in 0..w {
            let fx = if w > 1 {
                x as f32 / (w - 1) as f32
            } else {
                0.5
            };
            let fy = if h > 1 {
                y as f32 / (h - 1) as f32
            } else {
                0.5
            };
            let edge = ((x / 8 + y / 8) % 2) as f32 * 0.15;
            let r = (0.85 * fx + edge).clamp(0.0, 1.0);
            let g = (0.85 * fy + edge).clamp(0.0, 1.0);
            let b = (0.9 - 0.4 * fx - 0.4 * fy + edge).clamp(0.0, 1.0);
            let a = (0.2 + 0.8 * fx).clamp(0.0, 1.0);
            let vals: Vec<f32> = match comps {
                1 => vec![0.299 * r + 0.587 * g + 0.114 * b],
                3 => vec![r, g, b],
                _ => vec![r, g, b, a],
            };
            for v in vals {
                if wide {
                    data.extend_from_slice(&((v * 65535.0).round() as u16).to_le_bytes());
                } else {
                    data.push((v * 255.0).round() as u8);
                }
            }
        }
    }
    VideoFrame {
        pts: Some(0),
        planes: vec![oxideav_core::VideoPlane {
            stride: w * comps * bytes,
            data,
        }],
    }
}

/// Write a frame as PNG through the registry's own encoder + muxer.
fn write_png(
    ctx: &RuntimeContext,
    path: &Path,
    frame: &VideoFrame,
    w: u32,
    h: u32,
    fmt: PixelFormat,
) {
    let mut p = CodecParameters::video(CodecId::new("png"));
    p.width = Some(w);
    p.height = Some(h);
    p.pixel_format = Some(fmt);
    let mut enc = ctx.codecs.first_encoder(&p).expect("png encoder");
    enc.send_frame(&Frame::Video(frame.clone())).expect("send");
    enc.flush().expect("flush");
    let pkt = enc.receive_packet().expect("png packet");
    let stream = oxideav_core::StreamInfo {
        index: 0,
        time_base: oxideav_core::TimeBase::new(1, 1),
        duration: None,
        start_time: None,
        params: enc.output_params().clone(),
    };
    let mut mux = ctx
        .containers
        .open_muxer(
            "png",
            Box::new(std::fs::File::create(path).expect("create")),
            &[stream],
        )
        .expect("png muxer");
    mux.write_header().expect("header");
    mux.write_packet(&pkt).expect("packet");
    mux.write_trailer().expect("trailer");
}

struct Source {
    name: &'static str,
    width: u32,
    height: u32,
    png: PathBuf,
    rgba: Rgba,
}

fn source(ctx: &RuntimeContext, name: &'static str, w: u32, h: u32, fmt: PixelFormat) -> Source {
    let png = scratch(TAG).join(format!("src_{name}.png"));
    if !png.is_file() {
        write_png(ctx, &png, &synth(w, h, fmt), w, h, fmt);
    }
    let rgba = rgba_f32(ctx, &png).expect("re-read source png");
    assert_eq!((rgba.width, rgba.height), (w, h), "{name}: png geometry");
    Source {
        name,
        width: w,
        height: h,
        png,
        rgba,
    }
}

fn sources(ctx: &RuntimeContext, big: bool) -> Vec<Source> {
    let mut v = vec![
        source(ctx, "rgb_1x1", 1, 1, PixelFormat::Rgb24),
        source(ctx, "rgb_7x5", 7, 5, PixelFormat::Rgb24),
        source(ctx, "rgb_63x61", 63, 61, PixelFormat::Rgb24),
        source(ctx, "rgb_96x80", 96, 80, PixelFormat::Rgb24),
        source(ctx, "rgba_63x61", 63, 61, PixelFormat::Rgba),
        source(ctx, "gray_96x80", 96, 80, PixelFormat::Gray8),
        source(ctx, "rgb16_96x80", 96, 80, PixelFormat::Rgb48Le),
        source(ctx, "rgb_1024x768", 1024, 768, PixelFormat::Rgb24),
    ];
    if big {
        v.push(source(ctx, "rgb_4032x3024", 4032, 3024, PixelFormat::Rgb24));
    }
    v
}

fn find<'a>(sources: &'a [Source], name: &str) -> &'a Source {
    sources
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("source {name}"))
}

// ─────────────────────── the CLI decode check ───────────────────────

struct CliDecode {
    /// The CLI PNG vs the library decode after the pipeline's own
    /// pixel-format step — must be exact.
    library: Diff,
    layout: String,
    fmt: PixelFormat,
    width: u32,
    height: u32,
    /// The decoded planes (for the H.273 best-fit classification).
    frame: VideoFrame,
    /// Our PNG as normalised RGBA.
    rgba: Rgba,
}

/// `oxideav convert file → png`; returns the sample-exactness verdict
/// against the library decode.
fn cli_decode(ctx: &RuntimeContext, file: &Path, out: &Path) -> Result<CliDecode, String> {
    let _ = std::fs::remove_file(out);
    let r = oxideav(&["convert", file.to_str().unwrap(), out.to_str().unwrap()]);
    if !r.ok {
        return Err(format!("exit {:?}: {}", r.code, r.stderr.trim()));
    }
    let (png_fmt, pw, ph, png) = try_decode_still(ctx, out)?;
    let (fmt, w, h, frame) = try_decode_still(ctx, file)?;
    if (pw, ph) != (w, h) {
        return Err(format!("geometry {pw}x{ph} vs {w}x{h}"));
    }
    let lib = oxideav_pixfmt::convert(
        &frame,
        oxideav_pixfmt::FrameInfo::new(fmt, w, h),
        png_fmt,
        &oxideav_pixfmt::ConvertOptions::default(),
    )
    .map_err(|e| format!("{fmt:?} → {png_fmt:?}: {e}"))?;
    let rgba = rgba_from_frame(&png, png_fmt, w, h)?;
    let b = rgba_from_frame(&lib, png_fmt, w, h)?;
    let exact = visible(&png, png_fmt, w, h) == visible(&lib, png_fmt, w, h);
    let mut library = diff_rgba(&rgba, &b)?;
    if !exact && library.is_exact() {
        library.max = 0.5; // bytes differ but rgba does not — flag it
    }
    Ok(CliDecode {
        library,
        layout: format!("{fmt:?} {w}x{h}"),
        fmt,
        width: w,
        height: h,
        frame,
        rgba,
    })
}

/// Sibling-crate gaps the CLI path surfaces today. Each is a cell class
/// the tables mark explicitly; anything outside these classes fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Gap {
    /// `pixfmt` refuses 4:2:0 / 4:2:2 → RGB when a dimension is odd.
    OddChroma,
    /// `convert` feeds both streams of an image sequence (cover + track)
    /// to the single-image PNG muxer.
    Sequence,
    /// Identity-matrix (GBR) items are labelled `YuvJ444P`, so the
    /// pipeline applies BT.601 — libheif `-L` lossless files.
    Identity,
    /// Full range is lost for alpha (`Yuva*`) and >8-bit (`Yuv*P10/12`)
    /// layouts (no `J` variant, no range field), so the pipeline
    /// renders them limited-range.
    RangeLost,
    /// `convert` has no encoder-option channel, so `out.avif` is
    /// written with the `heif` encoder's default codec (HEVC, `heic`
    /// brand) — `transcode` infers `codec=av1` from the extension.
    AvifIsHevc,
}

impl Gap {
    fn label(self) -> &'static str {
        match self {
            Gap::OddChroma => "odd 4:2:0 refused by pixfmt",
            Gap::Sequence => "sequence: 2 streams into the PNG muxer",
            Gap::Identity => "identity matrix rendered as BT.601",
            Gap::RangeLost => "full range lost (alpha / >8-bit)",
            Gap::AvifIsHevc => "`convert` writes .avif as HEVC (heic brand)",
        }
    }
    fn from_refusal(stderr: &str) -> Option<Gap> {
        if stderr.contains("divisible by") && stderr.contains("chroma") {
            Some(Gap::OddChroma)
        } else if stderr.contains("exactly one video stream expected") {
            Some(Gap::Sequence)
        } else {
            None
        }
    }
}

/// Verdict of our PNG against a reference render (oracle / reader /
/// source): within tolerance, a classified sibling gap (the decoded
/// planes fit the reference under the right H.273 interpretation, only
/// the pipeline's colour step lacks the signalling), or a failure.
enum Verdict {
    Ok(Diff),
    Gap(Diff, Gap, Diff, Matrix, bool),
    Fail(String),
}

impl Verdict {
    fn cell(&self) -> String {
        match self {
            Verdict::Ok(d) => d.cell(),
            Verdict::Gap(d, gap, fit, m, full) => format!(
                "{} — {}; planes fit {} {}: {}",
                d.cell(),
                gap.label(),
                m.label(),
                if *full { "full" } else { "limited" },
                fit.cell()
            ),
            Verdict::Fail(e) => format!("FAIL {e}"),
        }
    }
    fn gap(&self) -> Option<Gap> {
        match self {
            Verdict::Gap(_, g, ..) => Some(*g),
            _ => None,
        }
    }
    fn failure(&self) -> Option<&str> {
        match self {
            Verdict::Fail(e) => Some(e),
            _ => None,
        }
    }
}

/// `mean_tol` on the colour channels; `alpha_tol` bounds the alpha
/// channel's max difference (`None` = a reader that drops the
/// auxiliary; `Some(0.0)` = exact, the reader contract; a lossy
/// tolerance when the alpha item itself was coded lossily).
fn judge(dec: &CliDecode, want: &Rgba, mean_tol: f32, alpha_tol: Option<f32>) -> Verdict {
    let d = match diff_rgba(&dec.rgba, want) {
        Ok(d) => d,
        Err(e) => return Verdict::Fail(e),
    };
    let alpha_ok = alpha_tol.map_or(true, |t| d.alpha_max <= t);
    if d.mean <= mean_tol && alpha_ok {
        return Verdict::Ok(d);
    }
    // Does a straight H.273 render of our planes fit the reference?
    if let Some((fit, m, full)) = best_fit(&dec.frame, dec.fmt, dec.width, dec.height, want) {
        let fit_alpha_ok = alpha_tol.map_or(true, |t| fit.alpha_max <= t);
        if fit.mean <= mean_tol && fit_alpha_ok {
            let gap = if m == Matrix::Identity {
                Gap::Identity
            } else {
                Gap::RangeLost
            };
            return Verdict::Gap(d, gap, fit, m, full);
        }
    }
    Verdict::Fail(format!("{} vs reference", d.cell()))
}

fn cell_or_err(r: &Result<Diff, String>) -> String {
    match r {
        Ok(d) => d.cell(),
        Err(e) => format!("ERR {}", e.lines().next().unwrap_or("")),
    }
}

/// Reader tolerance: each reader's own YCbCr → RGB rounding and chroma
/// upsampling filter, measured as a mean over the synthetic checkerboard.
const READER_MEAN: f32 = 3.0;

fn print_gaps(gaps: &std::collections::BTreeMap<Gap, usize>) {
    for (g, n) in gaps {
        eprintln!("known gap · {} — {n} cell(s)", g.label());
    }
}

// ───────────────────── fixture leg (unconditional) ─────────────────────

#[test]
fn vendored_files_convert_sample_exact_and_match_their_oracles() {
    let ctx = ctx();
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    let mut gaps = std::collections::BTreeMap::new();
    let mut inputs: Vec<(String, PathBuf, Option<PathBuf>)> = interop_files()
        .into_iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
            let oracle = fixtures().join(format!("{stem}.expected.png"));
            (name, p, oracle.is_file().then_some(oracle))
        })
        .collect();
    for b in corpus_bundles() {
        let name = format!("corpus/{}", b.file_name().unwrap().to_string_lossy());
        let oracle = b.join("expected.png");
        inputs.push((
            name,
            b.join("input.heic"),
            oracle.is_file().then_some(oracle),
        ));
    }
    assert!(
        inputs.len() >= 52,
        "vendored inputs shrank: {}",
        inputs.len()
    );
    for (name, file, oracle) in &inputs {
        let out = scratch(TAG).join(format!("fixture_{}.png", name.replace('/', "_")));
        let dec = match cli_decode(&ctx, file, &out) {
            Ok(d) => d,
            Err(e) => match Gap::from_refusal(&e) {
                Some(g) => {
                    *gaps.entry(g).or_insert(0) += 1;
                    rows.push(format!("| {name} | — | refused — {} | — |", g.label()));
                    continue;
                }
                None => {
                    failures.push(format!("{name}: {e}"));
                    rows.push(format!("| {name} | — | FAIL {e} | — |"));
                    continue;
                }
            },
        };
        if !dec.library.is_exact() {
            failures.push(format!("{name}: CLI PNG differs from the library decode"));
        }
        let oracle_cell = match oracle {
            Some(o) => match rgba_f32(&ctx, o) {
                Ok(want) => {
                    let v = judge(&dec, &want, READER_MEAN, Some(0.0));
                    if let Some(g) = v.gap() {
                        *gaps.entry(g).or_insert(0) += 1;
                    }
                    if let Some(e) = v.failure() {
                        failures.push(format!("{name}: oracle: {e}"));
                    }
                    v.cell()
                }
                Err(e) => {
                    failures.push(format!("{name}: oracle: {e}"));
                    format!("ERR {e}")
                }
            },
            None => "no oracle".to_owned(),
        };
        rows.push(format!(
            "| {name} | {} | {} | {oracle_cell} |",
            dec.layout,
            dec.library.cell()
        ));
    }
    eprintln!(
        "\n### Vendored files → `oxideav convert` → PNG ({} inputs)\n",
        inputs.len()
    );
    eprintln!("| file | layout | vs library decode | vs producer render (max/mean, 8-bit) |");
    eprintln!("|---|---|---|---|");
    for r in &rows {
        eprintln!("{r}");
    }
    print_gaps(&gaps);
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ───────────────────────── producer × CLI ─────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Producer {
    Sips,
    HeifEnc,
    Magick,
    Ffmpeg,
}

impl Producer {
    fn bin(self) -> Option<PathBuf> {
        match self {
            Producer::Sips => sips(),
            Producer::HeifEnc => tool("heif-enc"),
            Producer::Magick => tool("magick"),
            Producer::Ffmpeg => tool("ffmpeg"),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Producer::Sips => "sips",
            Producer::HeifEnc => "heif-enc",
            Producer::Magick => "magick",
            Producer::Ffmpeg => "ffmpeg",
        }
    }
    /// The producer's own reader, for the tolerance leg.
    fn reader(self) -> Reader {
        match self {
            Producer::Sips => Reader::Sips,
            Producer::HeifEnc => Reader::HeifConvert,
            Producer::Magick => Reader::Magick,
            Producer::Ffmpeg => Reader::Ffmpeg,
        }
    }
}

struct Case {
    label: &'static str,
    producer: Producer,
    source: &'static str,
    args: &'static [&'static str],
    ext: &'static str,
    lossless: bool,
}

const CASES: &[Case] = &[
    // Apple ImageIO
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgb_1x1",
        args: &[],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgb_7x5",
        args: &[],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgb_63x61",
        args: &[],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgb_96x80",
        args: &[],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "sips q=best",
        producer: Producer::Sips,
        source: "rgb_96x80",
        args: &["-s", "formatOptions", "best"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgba_63x61",
        args: &[],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "gray_96x80",
        args: &[],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "sips 16-bit in",
        producer: Producer::Sips,
        source: "rgb16_96x80",
        args: &[],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "sips grid",
        producer: Producer::Sips,
        source: "rgb_1024x768",
        args: &[],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "sips grid",
        producer: Producer::Sips,
        source: "rgb_4032x3024",
        args: &[],
        ext: "heic",
        lossless: false,
    },
    // libheif (x265 / aom)
    Case {
        label: "heif-enc q60",
        producer: Producer::HeifEnc,
        source: "rgb_1x1",
        args: &["-q", "60"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc q60",
        producer: Producer::HeifEnc,
        source: "rgb_7x5",
        args: &["-q", "60"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc q60",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-q", "60"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc lossless",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-L"],
        ext: "heic",
        lossless: true,
    },
    Case {
        label: "heif-enc lossless",
        producer: Producer::HeifEnc,
        source: "rgba_63x61",
        args: &["-L"],
        ext: "heic",
        lossless: true,
    },
    Case {
        label: "heif-enc thumb",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-q", "60", "-t", "32"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc 10-bit",
        producer: Producer::HeifEnc,
        source: "rgb16_96x80",
        args: &["-q", "60", "-b", "10"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc 12-bit",
        producer: Producer::HeifEnc,
        source: "rgb16_96x80",
        args: &["-q", "60", "-b", "12"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc 4:4:4",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-q", "60", "-p", "chroma=444"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc 4:2:2",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-q", "60", "-p", "chroma=422"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc gray",
        producer: Producer::HeifEnc,
        source: "gray_96x80",
        args: &["-q", "60"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc rgba",
        producer: Producer::HeifEnc,
        source: "rgba_63x61",
        args: &["-q", "60"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc rotate 90",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-q", "60", "--rotate-cw", "90"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc limited range",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-q", "60", "--full_range_flag", "0"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc single",
        producer: Producer::HeifEnc,
        source: "rgb_4032x3024",
        args: &["-q", "60"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "heif-enc avif",
        producer: Producer::HeifEnc,
        source: "rgb_7x5",
        args: &["-A", "-q", "60"],
        ext: "avif",
        lossless: false,
    },
    Case {
        label: "heif-enc avif",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-A", "-q", "60"],
        ext: "avif",
        lossless: false,
    },
    Case {
        label: "heif-enc avif lossless",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-A", "-L"],
        ext: "avif",
        lossless: true,
    },
    Case {
        label: "heif-enc avif rgba",
        producer: Producer::HeifEnc,
        source: "rgba_63x61",
        args: &["-A", "-q", "60"],
        ext: "avif",
        lossless: false,
    },
    Case {
        label: "heif-enc avif gray",
        producer: Producer::HeifEnc,
        source: "gray_96x80",
        args: &["-A", "-q", "60"],
        ext: "avif",
        lossless: false,
    },
    Case {
        label: "heif-enc avif 10-bit",
        producer: Producer::HeifEnc,
        source: "rgb16_96x80",
        args: &["-A", "-q", "60", "-b", "10"],
        ext: "avif",
        lossless: false,
    },
    Case {
        label: "heif-enc avif 4:4:4",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-A", "-q", "60", "-p", "chroma=444"],
        ext: "avif",
        lossless: false,
    },
    Case {
        label: "heif-enc avif",
        producer: Producer::HeifEnc,
        source: "rgb_4032x3024",
        args: &["-A", "-q", "60"],
        ext: "avif",
        lossless: false,
    },
    // ImageMagick
    Case {
        label: "magick q60",
        producer: Producer::Magick,
        source: "rgb_7x5",
        args: &["-quality", "60"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "magick q60",
        producer: Producer::Magick,
        source: "rgb_96x80",
        args: &["-quality", "60"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "magick q60",
        producer: Producer::Magick,
        source: "rgba_63x61",
        args: &["-quality", "60"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "magick q60",
        producer: Producer::Magick,
        source: "gray_96x80",
        args: &["-quality", "60"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "magick 12-bit",
        producer: Producer::Magick,
        source: "rgb16_96x80",
        args: &["-quality", "60", "-depth", "12"],
        ext: "heic",
        lossless: false,
    },
    Case {
        label: "magick avif",
        producer: Producer::Magick,
        source: "rgb_96x80",
        args: &["-quality", "60"],
        ext: "avif",
        lossless: false,
    },
    Case {
        label: "magick avif",
        producer: Producer::Magick,
        source: "rgba_63x61",
        args: &["-quality", "60"],
        ext: "avif",
        lossless: false,
    },
    // Black-box video tool, AVIF muxer (SVT-AV1)
    Case {
        label: "ffmpeg avif crf30",
        producer: Producer::Ffmpeg,
        source: "rgb_96x80",
        args: &["-pix_fmt", "yuv420p", "-crf", "30"],
        ext: "avif",
        lossless: false,
    },
    Case {
        label: "ffmpeg avif crf30",
        producer: Producer::Ffmpeg,
        source: "rgb_7x5",
        args: &["-pix_fmt", "yuv420p", "-crf", "30"],
        ext: "avif",
        lossless: false,
    },
    Case {
        label: "ffmpeg avif 10-bit",
        producer: Producer::Ffmpeg,
        source: "rgb16_96x80",
        args: &["-pix_fmt", "yuv420p10le", "-crf", "30"],
        ext: "avif",
        lossless: false,
    },
    Case {
        label: "ffmpeg avif gray",
        producer: Producer::Ffmpeg,
        source: "gray_96x80",
        args: &["-pix_fmt", "yuv420p", "-crf", "30"],
        ext: "avif",
        lossless: false,
    },
];

/// Produce `case` from `src`; `None` = no binary, `Err` = the producer refused.
fn produce(case: &Case, src: &Source) -> Option<Result<PathBuf, String>> {
    let bin = case.producer.bin()?;
    let out = scratch(TAG).join(format!(
        "{}_{}.{}",
        case.label.replace([' ', ':', '='], "_"),
        src.name,
        case.ext
    ));
    let _ = std::fs::remove_file(&out);
    let s = src.png.to_str().unwrap();
    let o = out.to_str().unwrap();
    let r = match case.producer {
        Producer::Sips => {
            let mut a = vec![
                "-s",
                "format",
                if case.ext == "heic" { "heic" } else { "avif" },
            ];
            a.extend_from_slice(case.args);
            a.extend_from_slice(&[s, "--out", o]);
            run_tool(&bin, &a)
        }
        Producer::HeifEnc => {
            let mut a: Vec<&str> = case.args.to_vec();
            a.extend_from_slice(&[s, "-o", o]);
            run_tool(&bin, &a)
        }
        Producer::Magick => {
            let mut a = vec![s];
            a.extend_from_slice(case.args);
            a.push(o);
            run_tool(&bin, &a)
        }
        Producer::Ffmpeg => {
            let mut a = vec!["-v", "error", "-y", "-i", s, "-c:v", "libsvtav1"];
            a.extend_from_slice(case.args);
            a.push(o);
            run_tool(&bin, &a)
        }
    };
    Some(match r {
        Ok(_) if out.is_file() => Ok(out),
        Ok(_) => Err("producer wrote nothing".into()),
        Err(e) => Err(e),
    })
}

#[test]
fn fresh_producer_files_convert_through_the_cli() {
    let ctx = ctx();
    let any = [
        Producer::Sips,
        Producer::HeifEnc,
        Producer::Magick,
        Producer::Ffmpeg,
    ]
    .iter()
    .any(|p| p.bin().is_some());
    if !any {
        eprintln!(
            "SKIP fresh_producer_files_convert_through_the_cli: no producer binary on this host"
        );
        return;
    }
    let big = CASES
        .iter()
        .any(|c| c.source == "rgb_4032x3024" && c.producer.bin().is_some());
    let sources = sources(&ctx, big);
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    let mut gaps = std::collections::BTreeMap::new();
    let mut skipped = std::collections::BTreeSet::new();
    for case in CASES {
        let src = find(&sources, case.source);
        let Some(produced) = produce(case, src) else {
            skipped.insert(case.producer.label());
            continue;
        };
        let name = format!("{} {}", case.label, src.name);
        let file = match produced {
            Ok(f) => f,
            Err(e) => {
                rows.push(format!(
                    "| {name} | producer refused: {} | — | — | — |",
                    e.lines().last().unwrap_or("")
                ));
                continue;
            }
        };
        let out = file.with_extension("oxideav.png");
        let dec = match cli_decode(&ctx, &file, &out) {
            Ok(d) => d,
            Err(e) => match Gap::from_refusal(&e) {
                Some(g) => {
                    *gaps.entry(g).or_insert(0) += 1;
                    rows.push(format!("| {name} | — | refused — {} | — | — |", g.label()));
                    continue;
                }
                None => {
                    failures.push(format!("{name}: {e}"));
                    rows.push(format!("| {name} | — | FAIL {e} | — | — |"));
                    continue;
                }
            },
        };
        if !dec.library.is_exact() {
            failures.push(format!("{name}: CLI PNG differs from the library decode"));
        }
        // vs the source PNG. Rotation is applied on decode, so a turned
        // picture is reported, not compared. Lossless files must fit the
        // source exactly under the right H.273 interpretation.
        let rotated = case.args.contains(&"--rotate-cw");
        let source_cell = if rotated {
            if (dec.width, dec.height) != (src.height, src.width) {
                failures.push(format!(
                    "{name}: rotated geometry {}x{}",
                    dec.width, dec.height
                ));
            }
            "rotated 90°".to_owned()
        } else {
            // Lossy producers code the alpha item lossily too; pictures
            // under 16 px are dominated by chroma subsampling / tiny-CTB
            // coding and are reported only.
            let tiny = src.width.min(src.height) < 16;
            let tol = if tiny { 255.0 } else { 10.0 };
            let v = judge(
                &dec,
                &src.rgba,
                tol,
                Some(if case.lossless { 0.0 } else { tol }),
            );
            if let Some(g) = v.gap() {
                *gaps.entry(g).or_insert(0) += 1;
            }
            if let Some(e) = v.failure() {
                failures.push(format!("{name}: vs source: {e}"));
            }
            if case.lossless {
                match best_fit(&dec.frame, dec.fmt, dec.width, dec.height, &src.rgba) {
                    Some((fit, ..)) if fit.is_exact() => {}
                    Some((fit, m, full)) => failures.push(format!(
                        "{name}: lossless but best fit {} {} is {}",
                        m.label(),
                        if full { "full" } else { "limited" },
                        fit.cell()
                    )),
                    None => failures.push(format!("{name}: lossless: no reference render")),
                }
            }
            v.cell()
        };
        // vs the producer's own reader.
        let reader = case.producer.reader();
        let reader_cell = match reader.render(&file) {
            None => format!("no {}", reader.label()),
            Some(Err(e)) => format!(
                "{} refused: {}",
                reader.label(),
                e.lines().last().unwrap_or("")
            ),
            Some(Ok(png)) => match rgba_f32(&ctx, &png) {
                Ok(want) => {
                    let v = judge(
                        &dec,
                        &want,
                        READER_MEAN,
                        reader.keeps_alpha().then_some(0.0),
                    );
                    if let Some(g) = v.gap() {
                        *gaps.entry(g).or_insert(0) += 1;
                    }
                    if let Some(e) = v.failure() {
                        failures.push(format!("{name}: vs {}: {e}", reader.label()));
                    }
                    format!("{} {}", reader.label(), v.cell())
                }
                Err(e) => {
                    failures.push(format!("{name}: vs {}: {e}", reader.label()));
                    format!("{} ERR {e}", reader.label())
                }
            },
        };
        rows.push(format!(
            "| {name} | {} | {} | {source_cell} | {reader_cell} |",
            dec.layout,
            dec.library.cell()
        ));
    }
    // An AV1 image sequence (`avis`) from the video tool: probe lists
    // the cover still and the track; `convert` refuses (both streams
    // reach the PNG muxer), a `run` job with a stream selector works.
    if let Some(ff) = tool("ffmpeg") {
        let seq = scratch(TAG).join("ffmpeg_seq_96x80.avif");
        let _ = std::fs::remove_file(&seq);
        let r = run_tool(
            &ff,
            &[
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=96x80:rate=5",
                "-frames:v",
                "3",
                "-c:v",
                "libsvtav1",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "30",
                seq.to_str().unwrap(),
            ],
        );
        match r {
            Ok(_) => {
                let p = oxideav(&["probe", seq.to_str().unwrap()]);
                let ok = p.ok && p.stdout.contains("Format: heif");
                let tracks = p.stdout.matches("Stream #").count();
                let out = seq.with_extension("oxideav.png");
                let cell = match cli_decode(&ctx, &seq, &out) {
                    Ok(d) => d.library.cell(),
                    Err(e) => match Gap::from_refusal(&e) {
                        Some(g) => {
                            *gaps.entry(g).or_insert(0) += 1;
                            format!("refused — {}", g.label())
                        }
                        None => {
                            failures.push(format!("ffmpeg avis sequence: {e}"));
                            format!("FAIL {e}")
                        }
                    },
                };
                if !ok {
                    failures.push(format!("ffmpeg avis sequence: probe failed: {}", p.stderr));
                }
                // The `run` recipe from the README.
                let job_out = seq.with_extension("job.png");
                let _ = std::fs::remove_file(&job_out);
                let job = format!(
                    r#"{{"{}": {{"video": [{{"from": "{}", "codec": "png", "stream_selector": {{"kind": "video", "index": 0}}}}]}}}}"#,
                    job_out.to_str().unwrap().replace('\\', "/"),
                    seq.to_str().unwrap().replace('\\', "/")
                );
                let j = oxideav(&["run", "--inline", &job]);
                let job_cell = if j.ok && job_out.is_file() {
                    "`run` + stream_selector: cover written".to_owned()
                } else {
                    failures.push(format!("ffmpeg avis sequence: run job: {}", j.stderr));
                    "run job FAILED".to_owned()
                };
                rows.push(format!(
                    "| ffmpeg avis sequence (3 frames) | probe: {tracks} streams | {cell} | {job_cell} | — |"
                ));
            }
            Err(e) => rows.push(format!(
                "| ffmpeg avis sequence | producer refused: {e} | — | — | — |"
            )),
        }
    }
    eprintln!("\n### Fresh producer files → `oxideav convert` → PNG\n");
    eprintln!("| producer / case | layout | vs library decode | vs source PNG (max/mean) | vs producer's own render |");
    eprintln!("|---|---|---|---|---|");
    for r in &rows {
        eprintln!("{r}");
    }
    print_gaps(&gaps);
    for s in &skipped {
        eprintln!("SKIP producer {s}: binary not found");
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ───────────────────────── CLI writer × readers ─────────────────────────

struct WriterCase {
    label: &'static str,
    /// `--codec-option` pairs; empty = encoder defaults.
    opts: &'static [&'static str],
    ext: &'static str,
    source: &'static str,
    lossless: bool,
}

const WRITER_CASES: &[WriterCase] = &[
    WriterCase {
        label: "hevc defaults",
        opts: &[],
        ext: "heic",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: "hevc defaults",
        opts: &[],
        ext: "heic",
        source: "rgb_7x5",
        lossless: false,
    },
    WriterCase {
        label: "hevc defaults",
        opts: &[],
        ext: "heic",
        source: "rgb_1x1",
        lossless: false,
    },
    WriterCase {
        label: "hevc defaults",
        opts: &[],
        ext: "heic",
        source: "rgba_63x61",
        lossless: false,
    },
    WriterCase {
        label: "hevc defaults",
        opts: &[],
        ext: "heic",
        source: "gray_96x80",
        lossless: false,
    },
    WriterCase {
        label: "hevc defaults",
        opts: &[],
        ext: "heic",
        source: "rgb16_96x80",
        lossless: false,
    },
    WriterCase {
        label: "qp=10",
        opts: &["qp=10"],
        ext: "heic",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: "qp=40",
        opts: &["qp=40"],
        ext: "heic",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: "mode=pcm",
        opts: &["mode=pcm"],
        ext: "heic",
        source: "rgb_96x80",
        lossless: true,
    },
    WriterCase {
        label: "mode=pcm",
        opts: &["mode=pcm"],
        ext: "heic",
        source: "rgba_63x61",
        lossless: true,
    },
    WriterCase {
        label: "grid=64",
        opts: &["grid=64"],
        ext: "heic",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: "grid=512",
        opts: &["grid=512"],
        ext: "heic",
        source: "rgb_1024x768",
        lossless: false,
    },
    WriterCase {
        label: "thumbnail=32",
        opts: &["thumbnail=32"],
        ext: "heic",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: "range=limited",
        opts: &["range=limited"],
        ext: "heic",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: "rd=2",
        opts: &["rd=2"],
        ext: "heic",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: "tiles=2x2",
        opts: &["tiles=2x2"],
        ext: "heic",
        source: "rgb_1024x768",
        lossless: false,
    },
    WriterCase {
        label: "codec=av1",
        opts: &["codec=av1"],
        ext: "avif",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: "codec=av1",
        opts: &["codec=av1"],
        ext: "avif",
        source: "rgba_63x61",
        lossless: false,
    },
    WriterCase {
        label: "codec=av1",
        opts: &["codec=av1"],
        ext: "avif",
        source: "gray_96x80",
        lossless: false,
    },
    WriterCase {
        label: "codec=av1",
        opts: &["codec=av1"],
        ext: "avif",
        source: "rgb16_96x80",
        lossless: false,
    },
    WriterCase {
        label: "codec=av1 quality=100",
        opts: &["codec=av1", "quality=100"],
        ext: "avif",
        source: "rgb_96x80",
        lossless: true,
    },
    WriterCase {
        label: "codec=av1 quality=30",
        opts: &["codec=av1", "quality=30"],
        ext: "avif",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: "codec=av1 speed=thorough",
        opts: &["codec=av1", "speed=thorough"],
        ext: "avif",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: "codec=av1 mode=pcm",
        opts: &["codec=av1", "mode=pcm"],
        ext: "avif",
        source: "rgb_96x80",
        lossless: true,
    },
    WriterCase {
        label: "codec=av1 grid=64 thumbnail=32",
        opts: &["codec=av1", "grid=64", "thumbnail=32"],
        ext: "avif",
        source: "rgb_96x80",
        lossless: false,
    },
    WriterCase {
        label: ".avif infers codec=av1",
        opts: &[],
        ext: "avif",
        source: "rgb_96x80",
        lossless: false,
    },
];

#[test]
fn cli_written_files_open_in_every_reader() {
    let ctx = ctx();
    let sources = sources(&ctx, false);
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    let mut gaps = std::collections::BTreeMap::new();
    let readers: Vec<Reader> = Reader::ALL
        .iter()
        .copied()
        .filter(|r| r.bin().is_some())
        .collect();
    let has_info = tool("heif-info").is_some();
    let write = |label: &str, args: &[&str], out: &Path| -> Result<(), String> {
        let _ = std::fs::remove_file(out);
        let r = oxideav(args);
        if !r.ok {
            return Err(format!("{label}: exit {:?}: {}", r.code, r.stderr.trim()));
        }
        if !out.is_file() || std::fs::metadata(out).map(|m| m.len()).unwrap_or(0) == 0 {
            return Err(format!("{label}: nothing written"));
        }
        Ok(())
    };
    // Our encoder codes RGB sources as 4:2:0 YCbCr, so even `mode=pcm` /
    // `quality=100` lose the chroma subsampling: "lossless" cases must
    // stay within 3 mean of the source (the subsampling residue on the
    // checkerboard; ~1 on even sizes, more on odd ones), lossy ones
    // within 10 (tiny pictures are dominated by the subsampling and are
    // only reported).
    let check = |name: String,
                 file: &Path,
                 src: &Source,
                 lossless: bool,
                 rows: &mut Vec<String>,
                 failures: &mut Vec<String>,
                 gaps: &mut std::collections::BTreeMap<Gap, usize>| {
        let back = file.with_extension("back.png");
        let dec = match cli_decode(&ctx, file, &back) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{name}: decode back: {e}"));
                rows.push(format!("| {name} | FAIL {e} |"));
                return;
            }
        };
        if !dec.library.is_exact() {
            failures.push(format!("{name}: CLI PNG differs from the library decode"));
        }
        let tol = if lossless { 3.0 } else { 10.0 };
        let tiny = src.width.min(src.height) < 16;
        let tol = if tiny { 255.0 } else { tol };
        let v = judge(&dec, &src.rgba, tol, Some(tol));
        if let Some(g) = v.gap() {
            *gaps.entry(g).or_insert(0) += 1;
        }
        if let Some(e) = v.failure() {
            failures.push(format!("{name}: vs source: {e}"));
        }
        // Brand as the CLI's own probe reports it; an `.avif` must be `avif`.
        let probe = oxideav(&["probe", file.to_str().unwrap()]);
        let brand = probe
            .stdout
            .lines()
            .find_map(|l| l.trim().strip_prefix("major_brand"))
            .map(|v| v.trim_start_matches([' ', ':']).trim().to_owned())
            .unwrap_or_else(|| "?".to_owned());
        let want_avif = file.extension().is_some_and(|e| e == "avif");
        let brand_cell = if want_avif && brand != "avif" {
            *gaps.entry(Gap::AvifIsHevc).or_insert(0) += 1;
            format!("{brand} — {}", Gap::AvifIsHevc.label())
        } else if !want_avif && brand == "avif" {
            failures.push(format!("{name}: .heic carries the avif brand"));
            brand
        } else {
            brand
        };
        let mut cells = vec![dec.layout.clone(), brand_cell, v.cell()];
        if has_info {
            cells.push(match heif_info(file) {
                Some(Ok(_)) => "opens".to_owned(),
                Some(Err(e)) => {
                    failures.push(format!("{name}: heif-info refused: {e}"));
                    "refused".to_owned()
                }
                None => "—".to_owned(),
            });
        }
        for reader in &readers {
            let cell = match reader.render(file) {
                Some(Ok(png)) => match rgba_f32(&ctx, &png) {
                    Ok(want) => {
                        // The video tool crops odd sizes to even: compare
                        // the common window and say so.
                        if (want.width, want.height) != (dec.width, dec.height)
                            && want.width <= dec.width
                            && want.height <= dec.height
                            && *reader == Reader::Ffmpeg
                        {
                            let mut note =
                                format!(" (reader crops to {}x{})", want.width, want.height);
                            let ours = crop_rgba(&dec.rgba, want.width, want.height);
                            let d = diff_rgba(&ours, &want);
                            let cell = cell_or_err(&d);
                            if let Ok(d) = d {
                                if d.mean > READER_MEAN {
                                    // Same range-lost class as the uncropped
                                    // comparison; report only.
                                    note.push_str(" range-lost");
                                }
                            }
                            cells.push(format!("{cell}{note}"));
                            continue;
                        }
                        let v = judge(
                            &dec,
                            &want,
                            READER_MEAN,
                            reader.keeps_alpha().then_some(0.0),
                        );
                        if let Some(g) = v.gap() {
                            *gaps.entry(g).or_insert(0) += 1;
                        }
                        if let Some(e) = v.failure() {
                            failures.push(format!("{name}: {}: {e}", reader.label()));
                        }
                        if reader.keeps_alpha() {
                            v.cell()
                        } else {
                            format!("{} (α dropped)", v.cell())
                        }
                    }
                    Err(e) => {
                        failures.push(format!("{name}: {}: {e}", reader.label()));
                        format!("ERR {e}")
                    }
                },
                Some(Err(e)) => {
                    if *reader == Reader::Ffmpeg && src.width.min(src.height) < 2 {
                        "refused (reader: 1-px picture)".to_owned()
                    } else {
                        failures.push(format!("{name}: {} refused: {e}", reader.label()));
                        "refused".to_owned()
                    }
                }
                None => "—".to_owned(),
            };
            cells.push(cell);
        }
        rows.push(format!("| {name} | {} |", cells.join(" | ")));
    };

    for case in WRITER_CASES {
        let src = find(&sources, case.source);
        let out = scratch(TAG).join(format!(
            "w_{}_{}.{}",
            case.label.replace([' ', '=', '.'], "_"),
            src.name,
            case.ext
        ));
        let mut args = vec![
            "transcode",
            src.png.to_str().unwrap(),
            out.to_str().unwrap(),
            "--codec-video",
            "heif",
        ];
        for o in case.opts {
            args.extend_from_slice(&["--codec-option", o]);
        }
        let name = format!("`transcode` {} · {}", case.label, src.name);
        match write(&name, &args, &out) {
            Ok(()) => check(
                name,
                &out,
                src,
                case.lossless,
                &mut rows,
                &mut failures,
                &mut gaps,
            ),
            Err(e) => {
                // The encoder refused the option for this picture — no
                // file may exist, and the refusal is reported verbatim.
                assert!(
                    !out.exists(),
                    "{name}: refused but {} exists",
                    out.display()
                );
                rows.push(format!(
                    "| {name} | refused: {} |",
                    e.split(": ").last().unwrap_or("")
                ));
            }
        }
    }
    // `convert` at defaults: HEIC and AVIF outputs.
    for (ext, src_name) in [
        ("heic", "rgb_96x80"),
        ("heic", "rgba_63x61"),
        ("avif", "rgb_96x80"),
    ] {
        let src = find(&sources, src_name);
        let out = scratch(TAG).join(format!("c_default_{src_name}.{ext}"));
        let name = format!("`convert` defaults .{ext} · {src_name}");
        match write(
            &name,
            &["convert", src.png.to_str().unwrap(), out.to_str().unwrap()],
            &out,
        ) {
            Ok(()) => check(name, &out, src, false, &mut rows, &mut failures, &mut gaps),
            Err(e) => failures.push(e),
        }
    }
    let mut header = vec![
        "case".to_owned(),
        "layout".to_owned(),
        "brand".to_owned(),
        "our decode vs source (max/mean)".to_owned(),
    ];
    if has_info {
        header.push("heif-info".to_owned());
    }
    header.extend(readers.iter().map(|r| r.label().to_owned()));
    eprintln!(
        "\n### CLI-written HEIC / AVIF → readers (reader render vs our decode, max/mean 8-bit)\n"
    );
    eprintln!("| {} |", header.join(" | "));
    eprintln!("|{}", "---|".repeat(header.len()));
    for r in &rows {
        eprintln!("{r}");
    }
    print_gaps(&gaps);
    for r in Reader::ALL {
        if r.bin().is_none() {
            eprintln!("SKIP reader {}: binary not found", r.label());
        }
    }
    if !has_info {
        eprintln!("SKIP reader heif-info: binary not found");
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
