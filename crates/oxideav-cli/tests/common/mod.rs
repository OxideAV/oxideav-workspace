//! Helpers shared by the HEIF suites that drive the built `oxideav`
//! binary (`CARGO_BIN_EXE_oxideav`): fixture paths, scratch dirs,
//! plain and time-budgeted runs, registry decode of a still, and the
//! third-party readers used as black-box oracles.
#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use oxideav_core::{Frame, PixelFormat, RuntimeContext, VideoFrame};

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_oxideav")
}

/// `crates/oxideav-heif/tests/fixtures` — vendored by the codec crate,
/// present on every umbrella checkout.
pub fn heif_fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../oxideav-heif/tests/fixtures")
}

/// The real-world producer files (`interop/`).
pub fn fixtures() -> PathBuf {
    heif_fixtures_root().join("interop")
}

/// The heif crate's fuzz corpus (read-only input for the hostile suite).
pub fn fuzz_corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../oxideav-heif/fuzz/corpus")
}

/// Vendored interop stills / sequences (`.heic` / `.heics` / `.avif`), sorted.
pub fn interop_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .expect("interop dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("heic" | "heics" | "avif")
            )
        })
        .collect();
    v.sort();
    v
}

/// Vendored corpus bundles (`corpus/<name>/input.heic` + `expected.png`), sorted.
pub fn corpus_bundles() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(heif_fixtures_root().join("corpus"))
        .expect("corpus dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.join("input.heic").exists())
        .collect();
    v.sort();
    v
}

/// Per-suite scratch directory under the OS temp dir.
pub fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oxideav-cli-r462-{tag}"));
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// A fresh (unlinked) scratch path.
pub fn scratch_file(tag: &str, name: &str) -> PathBuf {
    let p = scratch(tag).join(name);
    let _ = std::fs::remove_file(&p);
    p
}

pub struct Run {
    pub ok: bool,
    /// Exit status; `None` when the process died from a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub fn oxideav(args: &[&str]) -> Run {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn oxideav");
    Run {
        ok: out.status.success(),
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Exited(i32),
    Signalled,
    TimedOut,
}

pub struct TimedRun {
    pub outcome: Outcome,
    pub elapsed: Duration,
    pub stderr: String,
}

/// Run the binary with a wall-clock budget; the child is killed when
/// the budget is exceeded. stdout is discarded, stderr collected.
pub fn oxideav_timed(args: &[&str], budget: Duration) -> TimedRun {
    let start = Instant::now();
    let mut child = Command::new(bin())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn oxideav");
    let mut pipe = child.stderr.take().expect("stderr pipe");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let outcome = loop {
        match child.try_wait().expect("wait") {
            Some(status) => {
                break match status.code() {
                    Some(c) => Outcome::Exited(c),
                    None => Outcome::Signalled,
                }
            }
            None if start.elapsed() > budget => {
                let _ = child.kill();
                let _ = child.wait();
                break Outcome::TimedOut;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let stderr = reader.join().unwrap_or_default();
    TimedRun {
        outcome,
        elapsed: start.elapsed(),
        stderr,
    }
}

pub fn ctx() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_meta::register_all(&mut ctx);
    ctx
}

/// Library path: probe → demux → first packet of stream 0 → `first_decoder`.
pub fn try_decode_still(
    ctx: &RuntimeContext,
    path: &Path,
) -> Result<(PixelFormat, u32, u32, VideoFrame), String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    let name = ctx
        .containers
        .probe_input(&mut f, ext.as_deref())
        .map_err(|e| format!("probe: {e}"))?;
    let mut dm = ctx
        .containers
        .open_demuxer(&name, Box::new(f), &ctx.codecs)
        .map_err(|e| format!("demuxer: {e}"))?;
    let params = dm.streams()[0].params.clone();
    let pkt = loop {
        let p = dm.next_packet().map_err(|e| format!("packet: {e}"))?;
        if p.stream_index == 0 {
            break p;
        }
    };
    let mut dec = ctx
        .codecs
        .first_decoder(&params)
        .map_err(|e| format!("decoder: {e}"))?;
    dec.send_packet(&pkt).map_err(|e| format!("send: {e}"))?;
    let Frame::Video(v) = dec.receive_frame().map_err(|e| format!("frame: {e}"))? else {
        return Err("video frame expected".into());
    };
    Ok((
        params.pixel_format.ok_or("pixel format")?,
        params.width.ok_or("width")?,
        params.height.ok_or("height")?,
        v,
    ))
}

pub fn decode_still(ctx: &RuntimeContext, path: &Path) -> (PixelFormat, u32, u32, VideoFrame) {
    try_decode_still(ctx, path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Visible bytes of every image plane, stride padding stripped.
pub fn visible(frame: &VideoFrame, fmt: PixelFormat, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, p) in frame.image_planes().iter().enumerate() {
        let row = fmt.plane_row_bytes(i, w).expect("row bytes");
        let (_, rows) = fmt.plane_dimensions(i, w, h).expect("plane dims");
        for r in 0..rows as usize {
            out.extend_from_slice(&p.data[r * p.stride..r * p.stride + row]);
        }
    }
    out
}

/// A still as normalised straight RGBA (`0.0..=1.0`, row-major).
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

/// Decode any still (PNG, HEIF, …) to normalised RGBA. Packed RGB /
/// grey layouts are expanded directly (bit-exact); anything else goes
/// through the pipeline's pixel-format step to `Rgba64Le` first.
pub fn rgba_f32(ctx: &RuntimeContext, path: &Path) -> Result<Rgba, String> {
    let (fmt, w, h, frame) = try_decode_still(ctx, path)?;
    rgba_from_frame(&frame, fmt, w, h)
}

pub fn rgba_from_frame(
    frame: &VideoFrame,
    fmt: PixelFormat,
    w: u32,
    h: u32,
) -> Result<Rgba, String> {
    let n = (w as usize) * (h as usize);
    let mut data = Vec::with_capacity(n * 4);
    let packed = visible(frame, fmt, w, h);
    let b8 = |v: u8| v as f32 / 255.0;
    let b16 = |c: &[u8]| u16::from_le_bytes([c[0], c[1]]) as f32 / 65535.0;
    match fmt {
        PixelFormat::Gray8 => {
            for px in packed.chunks_exact(1) {
                let g = b8(px[0]);
                data.extend_from_slice(&[g, g, g, 1.0]);
            }
        }
        PixelFormat::Gray16Le => {
            for px in packed.chunks_exact(2) {
                let g = b16(px);
                data.extend_from_slice(&[g, g, g, 1.0]);
            }
        }
        PixelFormat::Rgb24 => {
            for px in packed.chunks_exact(3) {
                data.extend_from_slice(&[b8(px[0]), b8(px[1]), b8(px[2]), 1.0]);
            }
        }
        PixelFormat::Rgba => {
            for px in packed.chunks_exact(4) {
                data.extend_from_slice(&[b8(px[0]), b8(px[1]), b8(px[2]), b8(px[3])]);
            }
        }
        PixelFormat::Rgb48Le => {
            for px in packed.chunks_exact(6) {
                data.extend_from_slice(&[b16(&px[0..2]), b16(&px[2..4]), b16(&px[4..6]), 1.0]);
            }
        }
        PixelFormat::Rgba64Le => {
            for px in packed.chunks_exact(8) {
                data.extend_from_slice(&[
                    b16(&px[0..2]),
                    b16(&px[2..4]),
                    b16(&px[4..6]),
                    b16(&px[6..8]),
                ]);
            }
        }
        PixelFormat::Ya16Le => {
            for px in packed.chunks_exact(4) {
                let g = b16(&px[0..2]);
                data.extend_from_slice(&[g, g, g, b16(&px[2..4])]);
            }
        }
        other => {
            let conv = oxideav_pixfmt::convert(
                frame,
                oxideav_pixfmt::FrameInfo::new(other, w, h),
                PixelFormat::Rgba64Le,
                &oxideav_pixfmt::ConvertOptions::default(),
            )
            .map_err(|e| format!("{other:?} → Rgba64Le: {e}"))?;
            return rgba_from_frame(&conv, PixelFormat::Rgba64Le, w, h);
        }
    }
    if data.len() != n * 4 {
        return Err(format!(
            "{fmt:?} {w}x{h}: expanded {} values, expected {}",
            data.len(),
            n * 4
        ));
    }
    Ok(Rgba {
        width: w,
        height: h,
        data,
    })
}

/// Per-channel differences in 8-bit units: colour max / mean, alpha max.
#[derive(Clone, Copy, Debug, Default)]
pub struct Diff {
    pub max: f32,
    pub mean: f32,
    pub alpha_max: f32,
}

impl Diff {
    pub fn is_exact(&self) -> bool {
        self.max == 0.0 && self.alpha_max == 0.0
    }
    /// `exact` or `max/mean` (alpha appended when it differs).
    pub fn cell(&self) -> String {
        if self.is_exact() {
            "exact".to_owned()
        } else if self.alpha_max > 0.0 {
            format!("{:.0}/{:.2} α{:.0}", self.max, self.mean, self.alpha_max)
        } else {
            format!("{:.0}/{:.2}", self.max, self.mean)
        }
    }
}

pub fn diff_rgba(a: &Rgba, b: &Rgba) -> Result<Diff, String> {
    if (a.width, a.height) != (b.width, b.height) {
        return Err(format!(
            "geometry {}x{} vs {}x{}",
            a.width, a.height, b.width, b.height
        ));
    }
    let (mut max, mut sum, mut amax, mut n) = (0f32, 0f64, 0f32, 0usize);
    for (pa, pb) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
        for c in 0..3 {
            let d = (pa[c] - pb[c]).abs() * 255.0;
            max = max.max(d);
            sum += d as f64;
            n += 1;
        }
        amax = amax.max((pa[3] - pb[3]).abs() * 255.0);
    }
    Ok(Diff {
        max,
        mean: (sum / n.max(1) as f64) as f32,
        alpha_max: amax,
    })
}

/// First binary on PATH (or an absolute path that exists).
pub fn tool(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_absolute() {
        return p.exists().then(|| p.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(name))
            .find(|c| c.is_file())
    })
}

/// Apple ImageIO's `sips` (absolute on macOS; PATH lookup elsewhere).
pub fn sips() -> Option<PathBuf> {
    tool("/usr/bin/sips").or_else(|| tool("sips"))
}

pub fn run_tool(bin: &Path, args: &[&str]) -> Result<String, String> {
    run_tool_bytes(bin, args).map(|b| String::from_utf8_lossy(&b).into_owned())
}

pub fn run_tool_bytes(bin: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "{} {:?}: {} — {}",
            bin.file_name().and_then(|n| n.to_str()).unwrap_or("tool"),
            args,
            out.status,
            err.lines().last().unwrap_or("").trim()
        ))
    }
}

/// Third-party readers that render a HEIF-family file to PNG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reader {
    Sips,
    HeifConvert,
    Magick,
    Ffmpeg,
}

impl Reader {
    pub const ALL: [Reader; 4] = [
        Reader::Sips,
        Reader::HeifConvert,
        Reader::Magick,
        Reader::Ffmpeg,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Reader::Sips => "sips",
            Reader::HeifConvert => "heif-convert",
            Reader::Magick => "magick",
            Reader::Ffmpeg => "ffmpeg",
        }
    }

    /// The video tool decodes the colour item only (alpha is a separate
    /// auxiliary it does not merge), so its render is compared on RGB.
    pub fn keeps_alpha(self) -> bool {
        self != Reader::Ffmpeg
    }

    pub fn bin(self) -> Option<PathBuf> {
        match self {
            Reader::Sips => sips(),
            Reader::HeifConvert => tool("heif-convert"),
            Reader::Magick => tool("magick"),
            Reader::Ffmpeg => tool("ffmpeg"),
        }
    }

    /// Render `path` to a PNG next to it (`<stem>.<reader>.png`).
    /// `None` = reader not installed; `Err` = the reader refused.
    pub fn render(self, path: &Path) -> Option<Result<PathBuf, String>> {
        let bin = self.bin()?;
        let out = path.with_extension(format!("{}.png", self.label()));
        let _ = std::fs::remove_file(&out);
        let p = path.to_str().expect("utf8 path");
        let o = out.to_str().expect("utf8 path");
        let r = match self {
            Reader::Sips => run_tool(&bin, &["-s", "format", "png", p, "--out", o]),
            Reader::HeifConvert => run_tool(&bin, &[p, o]),
            // Pin a 16-bit true-colour PNG with alpha so tiny renders are
            // never palettised and >8-bit sources keep their precision.
            Reader::Magick => run_tool(&bin, &[&format!("{p}[0]"), &format!("PNG64:{o}")]),
            Reader::Ffmpeg => run_tool(&bin, &["-v", "error", "-y", "-i", p, "-frames:v", "1", o]),
        };
        Some(match r {
            Ok(_) if out.is_file() => Ok(out),
            Ok(_) => Err(format!("{} wrote nothing", self.label())),
            Err(e) => Err(e),
        })
    }
}

/// `heif-info` verdict on a file: `None` when absent, else `Ok(report)` / `Err`.
pub fn heif_info(path: &Path) -> Option<Result<String, String>> {
    let bin = tool("heif-info")?;
    Some(run_tool(&bin, &[path.to_str().expect("utf8 path")]))
}

// ───────────────── H.273 reference render (best fit) ─────────────────

/// Colour matrix of a straight reference render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
    /// GBR: Y = G, Cb = B, Cr = R (libheif `-L` lossless files).
    Identity,
}

impl Matrix {
    pub const ALL: [Matrix; 3] = [Matrix::Bt601, Matrix::Bt709, Matrix::Identity];
    pub fn label(self) -> &'static str {
        match self {
            Matrix::Bt601 => "BT.601",
            Matrix::Bt709 => "BT.709",
            Matrix::Identity => "identity",
        }
    }
}

/// Chroma subsampling (log2 of the horizontal / vertical factors),
/// plane bit depth and alpha presence of a planar layout.
fn planar_layout(fmt: PixelFormat) -> Option<(u32, u32, u32, bool)> {
    use PixelFormat::*;
    Some(match fmt {
        Yuv420P | YuvJ420P => (1, 1, 8, false),
        Yuva420P => (1, 1, 8, true),
        Yuv422P | YuvJ422P => (1, 0, 8, false),
        Yuva422P => (1, 0, 8, true),
        Yuv444P | YuvJ444P => (0, 0, 8, false),
        Yuva444P => (0, 0, 8, true),
        Yuv420P10Le => (1, 1, 10, false),
        Yuv422P10Le => (1, 0, 10, false),
        Yuv444P10Le => (0, 0, 10, false),
        Yuv420P12Le => (1, 1, 12, false),
        Yuv444P12Le => (0, 0, 12, false),
        Gray8 => (0, 0, 8, false),
        Gray10Le => (0, 0, 10, false),
        Gray12Le => (0, 0, 12, false),
        Gray16Le => (0, 0, 16, false),
        _ => return None,
    })
}

fn is_gray(fmt: PixelFormat) -> bool {
    matches!(
        fmt,
        PixelFormat::Gray8 | PixelFormat::Gray10Le | PixelFormat::Gray12Le | PixelFormat::Gray16Le
    )
}

/// Straight H.273 render of decoded planes (nearest chroma, no
/// filtering) to normalised RGBA; `None` for layouts this helper does
/// not model (packed RGB etc. — use [`rgba_from_frame`]).
pub fn reference_rgba(
    frame: &VideoFrame,
    fmt: PixelFormat,
    w: u32,
    h: u32,
    matrix: Matrix,
    full: bool,
) -> Option<Rgba> {
    let (sx, sy, depth, alpha) = planar_layout(fmt)?;
    let planes = frame.image_planes();
    let gray = is_gray(fmt);
    if planes.len() < if gray { 1 } else { 3 } {
        return None;
    }
    let bytes = if depth > 8 { 2 } else { 1 };
    let maxv = ((1u32 << depth) - 1) as f32;
    let sample = |pi: usize, x: usize, y: usize| -> f32 {
        let p = &planes[pi];
        let i = y * p.stride + x * bytes;
        let v = if bytes == 2 {
            u16::from_le_bytes([p.data[i], p.data[i + 1]]) as f32
        } else {
            p.data[i] as f32
        };
        v / maxv
    };
    let (kr, kb) = match matrix {
        Matrix::Bt601 => (0.299, 0.114),
        Matrix::Bt709 => (0.2126, 0.0722),
        Matrix::Identity => (0.0, 0.0),
    };
    let scale = 1u32 << (depth - 8);
    let (ymin, yrange, cmid, crange) = if full {
        (0.0, 1.0, 0.5, 1.0)
    } else {
        (
            16.0 * scale as f32 / maxv,
            219.0 * scale as f32 / maxv,
            128.0 * scale as f32 / maxv,
            224.0 * scale as f32 / maxv,
        )
    };
    let mut data = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let yy = sample(0, x, y);
            let a = if alpha {
                let ai = if gray { 1 } else { 3 };
                if planes.len() > ai {
                    sample(ai, x, y)
                } else {
                    1.0
                }
            } else {
                1.0
            };
            if gray {
                let g = ((yy - ymin) / yrange).clamp(0.0, 1.0);
                data.extend_from_slice(&[g, g, g, a]);
                continue;
            }
            let cb = sample(1, x >> sx, y >> sy);
            let cr = sample(2, x >> sx, y >> sy);
            let (r, g, b) = match matrix {
                Matrix::Identity => (
                    (cr - ymin) / yrange,
                    (yy - ymin) / yrange,
                    (cb - ymin) / yrange,
                ),
                _ => {
                    let yl = (yy - ymin) / yrange;
                    let pb = (cb - cmid) / crange;
                    let pr = (cr - cmid) / crange;
                    let kg = 1.0 - kr - kb;
                    let r = yl + 2.0 * (1.0 - kr) * pr;
                    let b = yl + 2.0 * (1.0 - kb) * pb;
                    let g = (yl - kr * r - kb * b) / kg;
                    (r, g, b)
                }
            };
            data.extend_from_slice(&[r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a]);
        }
    }
    Some(Rgba {
        width: w,
        height: h,
        data,
    })
}

/// Best-fitting `(matrix, full_range)` interpretation of decoded
/// planes against a reference RGBA, with its difference.
pub fn best_fit(
    frame: &VideoFrame,
    fmt: PixelFormat,
    w: u32,
    h: u32,
    want: &Rgba,
) -> Option<(Diff, Matrix, bool)> {
    let mut best: Option<(Diff, Matrix, bool)> = None;
    for matrix in Matrix::ALL {
        for full in [true, false] {
            let got = reference_rgba(frame, fmt, w, h, matrix, full)?;
            let d = diff_rgba(&got, want).ok()?;
            if best.as_ref().map_or(true, |(b, _, _)| d.mean < b.mean) {
                best = Some((d, matrix, full));
            }
        }
    }
    best
}

/// Crop an RGBA image to its top-left `w × h` window.
pub fn crop_rgba(a: &Rgba, w: u32, h: u32) -> Rgba {
    let mut data = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for y in 0..h as usize {
        let row = y * a.width as usize * 4;
        data.extend_from_slice(&a.data[row..row + (w as usize) * 4]);
    }
    Rgba {
        width: w,
        height: h,
        data,
    }
}
