//! Fresh HEIF / HEIC / AVIF files from the black-box producers on this
//! host, decoded end to end through the framework (round 460).
//!
//! Sources are synthesised in-process and written as PNG through the
//! registry's own `png` encoder + muxer; each producer turns them into
//! HEIF-family files:
//!
//! * Apple ImageIO (`sips`) — 8-bit RGB at 1×1 / 7×5 / 63×61 / 96×80,
//!   RGBA, grayscale, 16-bit RGB (→ 10-bit 4:4:4), and the 512-px
//!   `grid` tiling Apple applies at 1024×768 and 4032×3024.
//! * libheif (`heif-enc`, x265 / aom) — lossy, lossless (`-L`, identity
//!   4:4:4), thumbnail (`-t`), 10-bit (`-b 10`), 4:4:4 / 4:2:2, AVIF
//!   (`-A`, lossy and lossless), grayscale, RGBA, odd sizes.
//! * ImageMagick (`magick`) — HEIC and AVIF, 12-bit, RGBA, grayscale.
//!
//! For every produced file: registry probe → demux → `"heif"` decode;
//! geometry equals the request; the plane layout equals what the
//! black-box probe reports; the visible planes are **byte-exact**
//! against the black-box decoder's raw dump (even sizes); each
//! available third-party reader's PNG render matches our planes within
//! its own rounding (alpha exact); lossless files reproduce the source
//! exactly; lossy ones stay above 30 dB.
//!
//! Every producer / reader leg prints SKIP when its binary is absent —
//! the fixture-backed suites (`image_heif.rs`) carry the unconditional
//! coverage.

use std::path::{Path, PathBuf};
use std::process::Command;

use oxideav_core::{CodecId, CodecParameters, Frame, PixelFormat, RuntimeContext, VideoFrame};
use oxideav_tests::image::{
    decode_still, meta_ctx, oracle_rgb, packed_planes, reference_rgb, rgb_diff, tool, Matrix,
};

/// `((max, mean, alpha_max), (matrix, full_range))` of the best-fitting
/// H.273 render.
type Fit = ((f32, f32, f32), (Matrix, bool));

// ───────────────────────────── sources ─────────────────────────────

/// A synthesised source: smooth gradients plus a few hard edges so
/// chroma siting errors and range mistakes both show up.
struct Source {
    name: &'static str,
    width: u32,
    height: u32,
    format: PixelFormat,
    png: PathBuf,
    /// Normalised `[r, g, b(, a)]` per pixel — the fidelity reference.
    rgb: (usize, Vec<f32>),
}

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

/// Write `frame` as a PNG through the registry (`png` encoder →
/// `png` muxer) — the framework's own writer, no third-party tool.
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

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join("oxideav-r460-heif-producers");
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

fn source(ctx: &RuntimeContext, name: &'static str, w: u32, h: u32, fmt: PixelFormat) -> Source {
    let frame = synth(w, h, fmt);
    let png = scratch().join(format!("src_{name}.png"));
    write_png(ctx, &png, &frame, w, h, fmt);
    // Round-trip through the registry's PNG decoder so the reference is
    // exactly what the producers read.
    let back = decode_still(ctx, &png).expect("re-read source png");
    assert_eq!(
        (back.width(), back.height()),
        (w, h),
        "{name}: png geometry"
    );
    let rgb = oracle_rgb(&back.frame, back.pixel_format(), w, h);
    Source {
        name,
        width: w,
        height: h,
        format: fmt,
        png,
        rgb,
    }
}

// ─────────────────────────── producers ───────────────────────────

fn run(bin: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!(
            "{} {:?}: {}\n{}",
            bin.display(),
            args,
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Producer {
    Sips,
    HeifEnc,
    Magick,
}

impl Producer {
    fn bin(self) -> Option<PathBuf> {
        match self {
            Producer::Sips => tool("/usr/bin/sips").or_else(|| tool("sips")),
            Producer::HeifEnc => tool("heif-enc"),
            Producer::Magick => tool("magick"),
        }
    }
}

struct Case {
    label: &'static str,
    producer: Producer,
    source: &'static str,
    /// Producer-specific arguments (before the output for `magick`,
    /// before `-o` for `heif-enc`, `sips` property pairs).
    args: &'static [&'static str],
    ext: &'static str,
    lossless: bool,
    /// Skip the third-party reader renders (large images).
    readers: bool,
}

fn produce(case: &Case, src: &Source) -> Option<PathBuf> {
    let bin = case.producer.bin()?;
    let out = scratch().join(format!("{}_{}.{}", case.label, src.name, case.ext));
    let _ = std::fs::remove_file(&out);
    let src_s = src.png.to_str().expect("utf8");
    let out_s = out.to_str().expect("utf8");
    let result = match case.producer {
        Producer::Sips => {
            let mut a = vec![
                "-s",
                "format",
                if case.ext == "heic" { "heic" } else { "avif" },
            ];
            a.extend_from_slice(case.args);
            a.extend_from_slice(&[src_s, "--out", out_s]);
            run(&bin, &a)
        }
        Producer::HeifEnc => {
            let mut a: Vec<&str> = case.args.to_vec();
            a.extend_from_slice(&[src_s, "-o", out_s]);
            run(&bin, &a)
        }
        Producer::Magick => {
            let mut a = vec![src_s];
            a.extend_from_slice(case.args);
            a.push(out_s);
            run(&bin, &a)
        }
    };
    match result {
        Ok(_) => {
            assert!(out.exists(), "{}: producer wrote nothing", case.label);
            Some(out)
        }
        Err(e) => panic!("{} ({}): {e}", case.label, src.name),
    }
}

// ──────────────────────────── readers ────────────────────────────

/// Black-box probe of the first video stream: `(codec, pix_fmt)`.
fn ffprobe(path: &Path) -> Option<(String, String)> {
    let bin = tool("ffprobe")?;
    let out = run(
        &bin,
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,pix_fmt",
            "-of",
            "csv=p=0",
            path.to_str().expect("utf8"),
        ],
    )
    .ok()?;
    let line = String::from_utf8_lossy(&out);
    let line = line.lines().next()?;
    let mut it = line.trim_end_matches(',').split(',');
    Some((it.next()?.to_owned(), it.next()?.to_owned()))
}

/// The black-box decoder's raw planar dump of its default video
/// stream (the composed grid, not a tile; the colour image, not the
/// alpha auxiliary) in its native layout.
fn ffmpeg_raw(path: &Path, pix_fmt: &str) -> Option<Vec<u8>> {
    let bin = tool("ffmpeg")?;
    run(
        &bin,
        &[
            "-v",
            "error",
            "-i",
            path.to_str().expect("utf8"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            pix_fmt,
            "-",
        ],
    )
    .ok()
}

/// Our colour-plane layout for a black-box `pix_fmt` name (alpha, when
/// present, is a separate stream on that side).
fn colour_format_for(pix_fmt: &str) -> Option<PixelFormat> {
    Some(match pix_fmt {
        "yuv420p" | "yuvj420p" => PixelFormat::Yuv420P,
        "yuv422p" | "yuvj422p" => PixelFormat::Yuv422P,
        "yuv444p" | "yuvj444p" | "gbrp" => PixelFormat::Yuv444P,
        "yuv420p10le" => PixelFormat::Yuv420P10Le,
        "yuv422p10le" => PixelFormat::Yuv422P10Le,
        "yuv444p10le" | "gbrp10le" => PixelFormat::Yuv444P10Le,
        "yuv420p12le" => PixelFormat::Yuv420P12Le,
        "yuv444p12le" | "gbrp12le" => PixelFormat::Yuv444P12Le,
        "gray" => PixelFormat::Gray8,
        "gray10le" => PixelFormat::Gray10Le,
        "gray12le" => PixelFormat::Gray12Le,
        _ => return None,
    })
}

/// Strip the alpha plane from a `Yuva*` label.
fn colour_part(fmt: PixelFormat) -> PixelFormat {
    match fmt {
        PixelFormat::Yuva420P => PixelFormat::Yuv420P,
        PixelFormat::Yuva422P => PixelFormat::Yuv422P,
        PixelFormat::Yuva444P => PixelFormat::Yuv444P,
        other => other,
    }
}

#[derive(Clone, Copy, Debug)]
enum Reader {
    HeifConvert,
    Magick,
    Sips,
}

fn render(reader: Reader, path: &Path, alpha: bool) -> Option<PathBuf> {
    let out = path.with_extension(format!("{reader:?}.png"));
    let _ = std::fs::remove_file(&out);
    let p = path.to_str().expect("utf8");
    let o = out.to_str().expect("utf8");
    let ok = match reader {
        Reader::HeifConvert => run(&tool("heif-convert")?, &[p, o]),
        // Pin a true-colour PNG (ImageMagick palettises tiny renders).
        Reader::Magick => run(
            &tool("magick")?,
            &[
                &format!("{p}[0]"),
                &format!("{}:{o}", if alpha { "PNG32" } else { "PNG24" }),
            ],
        ),
        Reader::Sips => run(
            &tool("/usr/bin/sips").or_else(|| tool("sips"))?,
            &["-s", "format", "png", p, "--out", o],
        ),
    };
    match ok {
        Ok(_) if out.exists() => Some(out),
        Ok(_) => None,
        Err(e) => {
            eprintln!(
                "    {reader:?} could not render {}: {}",
                path.display(),
                e.lines().next().unwrap_or("")
            );
            None
        }
    }
}

// ───────────────────────────── checks ─────────────────────────────

/// Best-fitting H.273 render of `d` against a normalised RGB(A)
/// reference — `(max, mean, alpha_max)` in 8-bit units plus the choice.
fn best_diff(d: &oxideav_tests::image::DecodedStill, want: &(usize, Vec<f32>)) -> Fit {
    let (w, h) = (d.width(), d.height());
    let mut best: Option<Fit> = None;
    for matrix in [Matrix::Bt601, Matrix::Bt709, Matrix::Identity] {
        for full in [true, false] {
            let mut got = reference_rgb(&d.frame, d.pixel_format(), w, h, matrix, full);
            if got.0 == 4 && want.0 == 3 {
                got = (
                    3,
                    got.1.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
                );
            }
            if got.0 == 3 && want.0 == 4 {
                got = (
                    4,
                    got.1
                        .chunks(3)
                        .flat_map(|p| [p[0], p[1], p[2], 1.0])
                        .collect(),
                );
            }
            let diff = rgb_diff(&got, want);
            if best.map_or(true, |(b, _)| diff.1 < b.1) {
                best = Some((diff, (matrix, full)));
            }
        }
    }
    best.expect("one render")
}

fn psnr_from_mean_sq(mean_sq: f64) -> f64 {
    if mean_sq <= 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mean_sq).log10()
    }
}

/// PSNR (8-bit units, colour channels) of the best H.273 render of `d`
/// against the source.
fn source_psnr(d: &oxideav_tests::image::DecodedStill, src: &Source) -> (f64, (Matrix, bool)) {
    let (w, h) = (d.width(), d.height());
    let mut best = (f64::NEG_INFINITY, (Matrix::Bt601, true));
    for matrix in [Matrix::Bt601, Matrix::Bt709, Matrix::Identity] {
        for full in [true, false] {
            let got = reference_rgb(&d.frame, d.pixel_format(), w, h, matrix, full);
            let mut sq = 0f64;
            let mut n = 0u64;
            for (a, b) in got.1.chunks(got.0).zip(src.rgb.1.chunks(src.rgb.0)) {
                for c in 0..3 {
                    let e = ((a[c] - b[c]) * 255.0) as f64;
                    sq += e * e;
                    n += 1;
                }
            }
            let p = psnr_from_mean_sq(sq / n.max(1) as f64);
            if p > best.0 {
                best = (p, (matrix, full));
            }
        }
    }
    best
}

const CASES: &[Case] = &[
    // Apple ImageIO
    Case {
        label: "sips_q60",
        producer: Producer::Sips,
        source: "rgb_96x80",
        args: &["-s", "formatOptions", "60"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "sips_best",
        producer: Producer::Sips,
        source: "rgb_96x80",
        args: &["-s", "formatOptions", "best"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgb_63x61",
        args: &[],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgb_7x5",
        args: &[],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgb_1x1",
        args: &[],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgba_80x64",
        args: &[],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgba_63x61",
        args: &[],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "gray_96x80",
        args: &[],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "sips",
        producer: Producer::Sips,
        source: "rgb16_96x80",
        args: &["-s", "formatOptions", "best"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "sips_grid",
        producer: Producer::Sips,
        source: "rgb_1024x768",
        args: &[],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "sips_grid",
        producer: Producer::Sips,
        source: "rgb_4032x3024",
        args: &[],
        ext: "heic",
        lossless: false,
        readers: false,
    },
    // libheif
    Case {
        label: "henc_q60",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-q", "60"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "henc_lossless",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-L"],
        ext: "heic",
        lossless: true,
        readers: true,
    },
    Case {
        label: "henc_thumb",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-q", "60", "-t", "32"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "henc_10bit",
        producer: Producer::HeifEnc,
        source: "rgb16_96x80",
        args: &["-q", "60", "-b", "10"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "henc_444",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-q", "60", "-p", "chroma=444"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "henc_422",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-q", "60", "-p", "chroma=422"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "henc_gray",
        producer: Producer::HeifEnc,
        source: "gray_96x80",
        args: &["-q", "60"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "henc",
        producer: Producer::HeifEnc,
        source: "rgba_63x61",
        args: &["-q", "60"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "henc",
        producer: Producer::HeifEnc,
        source: "rgb_7x5",
        args: &["-q", "60"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "henc_avif",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-A", "-q", "60"],
        ext: "avif",
        lossless: false,
        readers: true,
    },
    Case {
        label: "henc_avif_lossless",
        producer: Producer::HeifEnc,
        source: "rgb_96x80",
        args: &["-A", "-L"],
        ext: "avif",
        lossless: true,
        readers: true,
    },
    Case {
        label: "henc_avif",
        producer: Producer::HeifEnc,
        source: "rgba_63x61",
        args: &["-A", "-q", "60"],
        ext: "avif",
        lossless: false,
        readers: true,
    },
    // ImageMagick
    Case {
        label: "magick_q60",
        producer: Producer::Magick,
        source: "rgb_96x80",
        args: &["-quality", "60"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "magick",
        producer: Producer::Magick,
        source: "rgb_63x61",
        args: &["-quality", "60"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "magick",
        producer: Producer::Magick,
        source: "rgba_80x64",
        args: &["-quality", "60"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "magick",
        producer: Producer::Magick,
        source: "gray_96x80",
        args: &["-quality", "60"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "magick_12bit",
        producer: Producer::Magick,
        source: "rgb16_96x80",
        args: &["-quality", "60", "-depth", "12"],
        ext: "heic",
        lossless: false,
        readers: true,
    },
    Case {
        label: "magick_avif",
        producer: Producer::Magick,
        source: "rgb_96x80",
        args: &["-quality", "60"],
        ext: "avif",
        lossless: false,
        readers: true,
    },
    Case {
        label: "magick_avif",
        producer: Producer::Magick,
        source: "rgba_63x61",
        args: &["-quality", "60"],
        ext: "avif",
        lossless: false,
        readers: true,
    },
];

/// The whole producer × reader matrix. One test so the sources are
/// synthesised once; each row prints its own verdict line.
#[test]
fn fresh_producer_files_decode_end_to_end() {
    let ctx = meta_ctx();
    let present: Vec<Producer> = [Producer::Sips, Producer::HeifEnc, Producer::Magick]
        .into_iter()
        .filter(|p| p.bin().is_some())
        .collect();
    if present.is_empty() {
        eprintln!("SKIP: no HEIF producer (sips / heif-enc / magick) on this host");
        return;
    }
    let needed: std::collections::BTreeSet<&str> = CASES
        .iter()
        .filter(|c| present.contains(&c.producer))
        .map(|c| c.source)
        .collect();
    let mut sources: Vec<Source> = Vec::new();
    for name in needed {
        let (w, h, fmt) = match name {
            "rgb_96x80" => (96, 80, PixelFormat::Rgb24),
            "rgb_63x61" => (63, 61, PixelFormat::Rgb24),
            "rgb_7x5" => (7, 5, PixelFormat::Rgb24),
            "rgb_1x1" => (1, 1, PixelFormat::Rgb24),
            "rgba_80x64" => (80, 64, PixelFormat::Rgba),
            "rgba_63x61" => (63, 61, PixelFormat::Rgba),
            "gray_96x80" => (96, 80, PixelFormat::Gray8),
            "rgb16_96x80" => (96, 80, PixelFormat::Rgb48Le),
            "rgb_1024x768" => (1024, 768, PixelFormat::Rgb24),
            "rgb_4032x3024" => (4032, 3024, PixelFormat::Rgb24),
            other => panic!("source {other}"),
        };
        sources.push(source(&ctx, name, w, h, fmt));
    }
    let src_of = |name: &str| sources.iter().find(|s| s.name == name).expect("source");

    let mut produced = 0;
    let mut exact_raw = 0;
    let mut reader_renders = 0;
    for case in CASES {
        let src = src_of(case.source);
        let Some(path) = produce(case, src) else {
            eprintln!(
                "SKIP {} ({}): {:?} not installed",
                case.label, src.name, case.producer
            );
            continue;
        };
        produced += 1;
        let t0 = std::time::Instant::now();
        let d = decode_still(&ctx, &path)
            .unwrap_or_else(|e| panic!("{} ({}): framework decode: {e}", case.label, src.name));
        let decode_ms = t0.elapsed().as_millis();
        assert_eq!(d.container, "heif", "{} ({})", case.label, src.name);
        assert_eq!(d.params().codec_id.as_str(), "heif");
        assert_eq!(
            (d.width(), d.height()),
            (src.width, src.height),
            "{} ({}): geometry",
            case.label,
            src.name
        );
        let has_alpha = d.frame.image_planes().len() == 4;
        assert_eq!(
            has_alpha,
            src.format == PixelFormat::Rgba,
            "{} ({}): alpha plane presence ({:?})",
            case.label,
            src.name,
            d.pixel_format()
        );

        // Layout + byte-exact planes against the black-box decoder.
        let mut raw_note = String::from("raw: no black-box probe");
        if let Some((codec, pix_fmt)) = ffprobe(&path) {
            let expect_codec = if case.ext == "avif" { "av1" } else { "hevc" };
            assert_eq!(
                codec, expect_codec,
                "{} ({}): black-box codec",
                case.label, src.name
            );
            let want = colour_format_for(&pix_fmt)
                .unwrap_or_else(|| panic!("{}: black-box layout {pix_fmt} unmapped", case.label));
            // Odd sizes: the framework composes sub-sampled chroma at
            // 4:4:4 (MIAF §7.3.6.7 promotion), the black-box side crops
            // to even instead.
            let odd = src.width % 2 == 1 || src.height % 2 == 1;
            let ours = colour_part(d.pixel_format());
            assert!(
                ours == want || (odd && ours == PixelFormat::Yuv444P),
                "{} ({}): layout {ours:?} vs black-box {pix_fmt}",
                case.label,
                src.name
            );
            raw_note = format!("raw: {pix_fmt}");
            if src.width % 2 == 0 && src.height % 2 == 0 {
                if let Some(raw) = ffmpeg_raw(&path, &pix_fmt) {
                    let colour = VideoFrame {
                        pts: None,
                        planes: d.frame.image_planes()[..want.plane_count()].to_vec(),
                    };
                    let ours = packed_planes(&colour, want, src.width, src.height);
                    assert_eq!(
                        ours.len(),
                        raw.len(),
                        "{} ({}): raw size",
                        case.label,
                        src.name
                    );
                    assert!(
                        ours == raw,
                        "{} ({}): planes differ from the black-box decoder ({pix_fmt})",
                        case.label,
                        src.name
                    );
                    exact_raw += 1;
                    raw_note = format!("raw: byte-exact {pix_fmt}");
                }
            }
        }

        // Fidelity against the synthesised source.
        let (psnr, choice) = source_psnr(&d, src);
        if case.lossless {
            assert!(
                psnr.is_infinite(),
                "{} ({}): lossless file must reproduce the source exactly (PSNR {psnr:.1} dB, {choice:?})",
                case.label,
                src.name
            );
        } else if src.width.min(src.height) >= 32 {
            // Tiny images (1×1 / 7×5) are dominated by the producer's
            // own coding loss; the reader-agreement leg covers them.
            assert!(
                psnr >= 30.0,
                "{} ({}): PSNR vs source {psnr:.1} dB ({choice:?})",
                case.label,
                src.name
            );
        }
        if has_alpha {
            let got = reference_rgb(
                &d.frame,
                d.pixel_format(),
                d.width(),
                d.height(),
                choice.0,
                choice.1,
            );
            let amax = rgb_diff(&got, &src.rgb).2;
            // The alpha auxiliary is itself coded (lossy unless `-L`).
            let bound = if case.lossless { 0.5 } else { 4.0 };
            assert!(
                amax <= bound,
                "{} ({}): alpha vs source max {amax:.1}",
                case.label,
                src.name
            );
        }

        // Third-party reader renders.
        let mut notes = Vec::new();
        if case.readers {
            for reader in [Reader::HeifConvert, Reader::Magick, Reader::Sips] {
                let Some(png) = render(reader, &path, has_alpha) else {
                    continue;
                };
                let oracle = decode_still(&ctx, &png).expect("reader png");
                assert_eq!(
                    (oracle.width(), oracle.height()),
                    (src.width, src.height),
                    "{} ({}): {reader:?} render geometry",
                    case.label,
                    src.name
                );
                let want = oracle_rgb(&oracle.frame, oracle.pixel_format(), src.width, src.height);
                let ((max, mean, amax), rc) = best_diff(&d, &want);
                assert!(
                    mean <= 3.0 && max <= 32.0,
                    "{} ({}): {reader:?} render diverges: max={max:.1} mean={mean:.3} ({rc:?})",
                    case.label,
                    src.name
                );
                if has_alpha && want.0 == 4 {
                    assert!(
                        amax < 0.5,
                        "{} ({}): {reader:?} alpha differs ({amax:.1})",
                        case.label,
                        src.name
                    );
                }
                notes.push(format!("{reader:?} max={max:.1} mean={mean:.2}"));
                reader_renders += 1;
            }
        }
        eprintln!(
            "OK {:<20} {:<14} {:?} {}x{} {:>6} ms | {} | PSNR {:.1} dB {:?} | {}",
            case.label,
            src.name,
            d.pixel_format(),
            d.width(),
            d.height(),
            decode_ms,
            raw_note,
            psnr,
            choice,
            if notes.is_empty() {
                "-".to_owned()
            } else {
                notes.join(", ")
            }
        );
    }
    eprintln!(
        "produced {produced} files ({present:?}); byte-exact raw {exact_raw}; reader renders {reader_renders}"
    );
    assert!(produced > 0);
}
