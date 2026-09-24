//! Shared test helpers for cross-codec roundtrip comparison tests.
//!
//! Every codec test in this crate follows the same pattern:
//!
//! 1. Generate a known test signal (ffmpeg lavfi for audio, testsrc for video)
//! 2. **Encoder test**: encode with ours → decode with ffmpeg → compare
//!    against ffmpeg-encode → ffmpeg-decode of the same input
//! 3. **Decoder test**: encode with ffmpeg → decode with ours → compare
//!    against ffmpeg's own decode
//!
//! All tests skip gracefully when ffmpeg is absent.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Default oracle location (the Linux CI runners').
pub const FFMPEG: &str = "/usr/bin/ffmpeg";

/// Resolve the reference-binary path: the `OXIDEAV_FFMPEG` environment
/// override when set (developer machines whose install prefix differs
/// from the CI default), otherwise the historical [`FFMPEG`] location.
/// Deliberately **not** an install-prefix search: auto-detecting other
/// prefixes would silently activate every oracle leg on runners whose
/// preinstalled binary was never part of the suite's baseline.
/// Returns `None` when no oracle is present (tests skip gracefully).
pub fn ffmpeg_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        if let Ok(p) = std::env::var("OXIDEAV_FFMPEG") {
            if Path::new(&p).exists() {
                return Some(p);
            }
        }
        Path::new(FFMPEG).exists().then(|| FFMPEG.to_owned())
    })
    .as_deref()
}

pub fn ffmpeg_available() -> bool {
    ffmpeg_path().is_some()
}

pub fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

/// Run ffmpeg and return true on success.
pub fn ffmpeg(args: &[&str]) -> bool {
    let Some(bin) = ffmpeg_path() else {
        return false;
    };
    let mut cmd_args = vec!["-y", "-hide_banner", "-loglevel", "error"];
    cmd_args.extend_from_slice(args);
    matches!(
        Command::new(bin).args(&cmd_args).status(),
        Ok(s) if s.success()
    )
}

/// Run ffmpeg with PathBuf args.
pub fn ffmpeg_paths(args: &[&std::ffi::OsStr]) -> bool {
    let Some(bin) = ffmpeg_path() else {
        return false;
    };
    let base: Vec<&std::ffi::OsStr> = ["-y", "-hide_banner", "-loglevel", "error"]
        .iter()
        .map(|s| std::ffi::OsStr::new(*s))
        .collect();
    let mut all = base;
    all.extend_from_slice(args);
    matches!(
        Command::new(bin).args(&all).status(),
        Ok(s) if s.success()
    )
}

// ── Audio helpers ─────────────────────────────────────────────────────

/// Write interleaved s16le PCM to a file.
pub fn write_pcm_s16le(path: &Path, pcm: &[i16]) {
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(path, bytes).expect("write pcm");
}

/// Read s16le PCM from a file.
pub fn read_pcm_s16le(path: &Path) -> Vec<i16> {
    let data = std::fs::read(path).expect("read pcm");
    data.chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// Compute RMS difference between two PCM buffers (normalised to [-1, 1]).
pub fn audio_rms_diff(a: &[i16], b: &[i16]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return f64::INFINITY;
    }
    let mut sum = 0.0f64;
    for i in 0..n {
        let da = a[i] as f64 / 32768.0;
        let db = b[i] as f64 / 32768.0;
        let d = da - db;
        sum += d * d;
    }
    (sum / n as f64).sqrt()
}

/// Compute PSNR between two audio PCM buffers.
pub fn audio_psnr(a: &[i16], b: &[i16]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mut mse = 0.0f64;
    for i in 0..n {
        let d = a[i] as f64 - b[i] as f64;
        mse += d * d;
    }
    mse /= n as f64;
    if mse <= 0.0 {
        return f64::INFINITY;
    }
    10.0 * (32767.0f64 * 32767.0f64 / mse).log10()
}

/// Generate a deterministic test signal: 440 Hz sine + chirp + click.
pub fn generate_audio_signal(sample_rate: u32, channels: u16, duration_secs: f32) -> Vec<i16> {
    let n = (sample_rate as f32 * duration_secs) as usize;
    let mut pcm = Vec::with_capacity(n * channels as usize);
    for i in 0..n {
        let t = i as f64 / sample_rate as f64;
        let dur = duration_secs as f64;
        let sine = (2.0 * std::f64::consts::PI * 440.0 * t).sin();
        let chirp_f = 800.0 + 400.0 * (t / dur);
        let chirp = 0.3 * (2.0 * std::f64::consts::PI * chirp_f * t).sin();
        let click_pos = sample_rate as usize;
        let click = if i >= click_pos && i < click_pos + 5 {
            0.8
        } else {
            0.0
        };
        let sample = ((sine * 0.5 + chirp + click).clamp(-1.0, 1.0) * 30000.0) as i16;
        for _ in 0..channels {
            pcm.push(sample);
        }
    }
    pcm
}

/// RMS difference minimised over a bounded interleaved-sample lag
/// search. Codecs with an encoder/decoder delay (MP3's granule +
/// filterbank delay, AAC's overlap warmup, …) shift the decoded signal
/// relative to the input by an implementation-defined number of
/// samples; a fixed index-0 alignment would then measure phase error,
/// not fidelity. Slides `b` forward relative to `a` by `0..=max_lag`
/// whole interleaved samples (multiples of `channels` keep L/R phase)
/// and returns `(best_rms, best_lag)`.
pub fn audio_rms_diff_aligned(a: &[i16], b: &[i16], channels: u16, max_lag: usize) -> (f64, usize) {
    let step = channels.max(1) as usize;
    let mut best = (f64::INFINITY, 0usize);
    let mut lag = 0usize;
    while lag <= max_lag {
        if lag < b.len() {
            let rms = audio_rms_diff(a, &b[lag..]);
            if rms < best.0 {
                best = (rms, lag);
            }
        }
        lag += step;
    }
    best
}

// ── Video helpers ─────────────────────────────────────────────────────

/// Read raw YUV420P from a file. Returns (Y, Cb, Cr) planes.
pub fn read_yuv420p(path: &Path, width: u32, height: u32) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    let frame_size = (width * height * 3 / 2) as usize;
    if data.len() < frame_size {
        return None;
    }
    Some(data)
}

/// Compute Y-plane PSNR between two YUV420P buffers.
pub fn video_y_psnr(a: &[u8], b: &[u8], width: u32, height: u32) -> f64 {
    let n = (width * height) as usize;
    if a.len() < n || b.len() < n {
        return 0.0;
    }
    let mut mse = 0.0f64;
    for i in 0..n {
        let d = a[i] as f64 - b[i] as f64;
        mse += d * d;
    }
    mse /= n as f64;
    if mse <= 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0f64 * 255.0f64 / mse).log10()
}

/// Report line for a comparison result.
pub fn report(label: &str, rms: f64, psnr: f64, ours_len: usize, ref_len: usize) {
    eprintln!("  [{label}] RMS={rms:.6}  PSNR={psnr:.1} dB  ours={ours_len} ref={ref_len} samples");
}

// ───────────────────────── still-image helpers ─────────────────────────
//
// Shared by the HEIF end-to-end suites (`image_heif*.rs`): open any
// registry-resolved still image (HEIF / AVIF / PNG …) down to its
// first decoded `VideoFrame`, re-run the pipeline's pixel-format
// converter, and compare planes.

pub mod image {
    use std::path::{Path, PathBuf};

    use oxideav_core::{Frame, PixelFormat, RuntimeContext, StreamInfo, VideoFrame};

    /// A fully registered runtime (every sibling the umbrella links).
    pub fn meta_ctx() -> RuntimeContext {
        let mut ctx = RuntimeContext::new();
        oxideav_meta::register_all(&mut ctx);
        ctx
    }

    /// `crates/oxideav-heif/tests/fixtures` — vendored by the codec
    /// crate itself, so present on every CI checkout (the umbrella
    /// clones every sibling into `crates/`).
    pub fn heif_fixtures_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../oxideav-heif/tests/fixtures")
    }

    /// `docs/image/heif/fixtures` when the private docs tree is
    /// staged next to the umbrella; `None` on CI.
    pub fn heif_docs_fixtures_root() -> Option<PathBuf> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/image/heif/fixtures");
        p.join("single-image-1x1/input.heic").exists().then_some(p)
    }

    /// The 39 vendored real-world producer files (`.heic` / `.heics` /
    /// `.avif`), sorted.
    pub fn heif_interop_files() -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(heif_fixtures_root().join("interop"))
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
        assert!(v.len() >= 39, "vendored interop set shrank: {}", v.len());
        v
    }

    /// The 13 vendored corpus bundle directories (each with
    /// `input.heic` + `expected.png`), sorted.
    pub fn heif_corpus_dirs() -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(heif_fixtures_root().join("corpus"))
            .expect("corpus dir")
            .map(|e| e.expect("entry").path())
            .filter(|p| p.join("input.heic").exists())
            .collect();
        v.sort();
        assert_eq!(v.len(), 13, "vendored corpus bundle count");
        v
    }

    /// FNV-1a 64 over the concatenated plane bytes — the fingerprint
    /// the heif crate's `manifest.tsv` records for the black-box
    /// decoder's raw output.
    pub fn fnv1a64(planes: &[oxideav_core::VideoPlane]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for p in planes {
            for &b in &p.data {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }

    /// One decoded still: the container the registry resolved, every
    /// stream the demuxer exposed, and stream 0's first frame.
    pub struct DecodedStill {
        pub container: String,
        pub streams: Vec<StreamInfo>,
        pub frame: VideoFrame,
    }

    impl DecodedStill {
        pub fn params(&self) -> &oxideav_core::CodecParameters {
            &self.streams[0].params
        }
        pub fn width(&self) -> u32 {
            self.params().width.expect("width")
        }
        pub fn height(&self) -> u32 {
            self.params().height.expect("height")
        }
        pub fn pixel_format(&self) -> PixelFormat {
            self.params().pixel_format.expect("pixel format")
        }
    }

    /// Registry path end to end: probe (with the file's extension as
    /// hint) → demuxer → first packet of stream 0 → `first_decoder`
    /// → `VideoFrame`.
    pub fn decode_still(ctx: &RuntimeContext, path: &Path) -> oxideav_core::Result<DecodedStill> {
        let mut f = std::fs::File::open(path)?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase());
        let container = ctx.containers.probe_input(&mut f, ext.as_deref())?;
        let mut dm = ctx
            .containers
            .open_demuxer(&container, Box::new(f), &ctx.codecs)?;
        let streams = dm.streams().to_vec();
        let pkt = loop {
            let p = dm.next_packet()?;
            if p.stream_index == 0 {
                break p;
            }
        };
        let mut dec = ctx.codecs.first_decoder(&streams[0].params)?;
        dec.send_packet(&pkt)?;
        let frame = match dec.receive_frame()? {
            Frame::Video(v) => v,
            other => {
                return Err(oxideav_core::Error::invalid(format!(
                    "expected a video frame, got {other:?}"
                )))
            }
        };
        Ok(DecodedStill {
            container,
            streams,
            frame,
        })
    }

    /// The pipeline's own pixel-format step (`oxideav_pixfmt::convert`
    /// with default options) — what `oxideav convert` / `oxideav run`
    /// apply between a planar decode and a packed-RGB encoder.
    pub fn convert_pixfmt(
        frame: &VideoFrame,
        src: PixelFormat,
        width: u32,
        height: u32,
        dst: PixelFormat,
    ) -> VideoFrame {
        oxideav_pixfmt::convert(
            frame,
            oxideav_pixfmt::FrameInfo::new(src, width, height),
            dst,
            &oxideav_pixfmt::ConvertOptions::default(),
        )
        .expect("pixfmt convert")
    }

    /// Per-plane comparison restricted to the visible `width × height`
    /// of `fmt` (strides may pad). Returns `(max_abs_diff, mean_abs_diff)`
    /// over every visible byte.
    pub fn plane_diff(
        a: &VideoFrame,
        b: &VideoFrame,
        fmt: PixelFormat,
        width: u32,
        height: u32,
    ) -> (u32, f64) {
        let pa = a.image_planes();
        let pb = b.image_planes();
        assert_eq!(pa.len(), pb.len(), "plane count");
        let mut max = 0u32;
        let mut sum = 0u64;
        let mut n = 0u64;
        for (i, (x, y)) in pa.iter().zip(pb).enumerate() {
            let row = fmt.plane_row_bytes(i, width).expect("row bytes");
            let (_, rows) = fmt.plane_dimensions(i, width, height).expect("plane dims");
            for r in 0..rows as usize {
                let ra = &x.data[r * x.stride..r * x.stride + row];
                let rb = &y.data[r * y.stride..r * y.stride + row];
                for (p, q) in ra.iter().zip(rb) {
                    let d = p.abs_diff(*q) as u32;
                    max = max.max(d);
                    sum += d as u64;
                    n += 1;
                }
            }
        }
        (max, if n == 0 { 0.0 } else { sum as f64 / n as f64 })
    }

    /// Visible bytes of every image plane, padding stripped, in plane
    /// order — the layout a black-box `rawvideo` dump uses.
    pub fn packed_planes(frame: &VideoFrame, fmt: PixelFormat, width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, p) in frame.image_planes().iter().enumerate() {
            let row = fmt.plane_row_bytes(i, width).expect("row bytes");
            let (_, rows) = fmt.plane_dimensions(i, width, height).expect("plane dims");
            for r in 0..rows as usize {
                out.extend_from_slice(&p.data[r * p.stride..r * p.stride + row]);
            }
        }
        out
    }

    /// H.273 matrix coefficients for the reference conversion below.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Matrix {
        /// `MatrixCoefficients = 0` — planes are G, B, R.
        Identity,
        Bt601,
        Bt709,
        Bt2020,
    }

    /// Bit depth of a planar / gray / packed integer format, from its
    /// canonical name (`…10Le` / `…12Le` / `…16Le` / `Rgb48Le` /
    /// `Rgba64Le`; everything else is 8-bit).
    pub fn bit_depth(fmt: PixelFormat) -> u32 {
        let n = format!("{fmt:?}");
        if n.ends_with("10Le") {
            10
        } else if n.ends_with("12Le") {
            12
        } else if n.ends_with("16Le") || n == "Rgb48Le" || n == "Rgba64Le" {
            16
        } else {
            8
        }
    }

    /// Straight spec-formula Y'CbCr → R'G'B' (ITU-T H.273 Eq. 8-1 … 8-3
    /// with the full- / narrow-range code mappings of §8.3), nearest-
    /// neighbour chroma siting, no rounding: every pixel comes back as
    /// `[r, g, b]` (+ `a` when the frame has an alpha plane) in `0.0..=1.0`.
    /// Monochrome frames broadcast Y' to all three channels. This is the
    /// oracle-side reference for "within the reader's own rounding"
    /// comparisons — independent of the pipeline's converter.
    pub fn reference_rgb(
        frame: &VideoFrame,
        fmt: PixelFormat,
        width: u32,
        height: u32,
        matrix: Matrix,
        full_range: bool,
    ) -> (usize, Vec<f32>) {
        let planes = frame.image_planes();
        let depth = bit_depth(fmt);
        let bytes = if depth > 8 { 2 } else { 1 };
        let maxv = ((1u32 << depth) - 1) as f32;
        let half = (1u32 << (depth - 1)) as f32;
        let sample = |plane: usize, x: usize, y: usize| -> f32 {
            let p = &planes[plane];
            let i = y * p.stride + x * bytes;
            if bytes == 2 {
                u16::from_le_bytes([p.data[i], p.data[i + 1]]) as f32
            } else {
                p.data[i] as f32
            }
        };
        let mono = planes.len() == 1;
        let has_alpha = planes.len() == 4;
        let (sx, sy) = fmt.chroma_subsampling().unwrap_or((0, 0));
        let (kr, kb) = match matrix {
            Matrix::Bt601 => (0.299f32, 0.114f32),
            Matrix::Bt709 => (0.2126, 0.0722),
            Matrix::Bt2020 => (0.2627, 0.0593),
            Matrix::Identity => (0.0, 0.0),
        };
        let kg = 1.0 - kr - kb;
        let scale = (1u32 << (depth - 8)) as f32;
        let norm_y = |v: f32| {
            if full_range {
                v / maxv
            } else {
                (v - 16.0 * scale) / (219.0 * scale)
            }
        };
        let norm_c = |v: f32| {
            if full_range {
                (v - half) / maxv
            } else {
                (v - 128.0 * scale) / (224.0 * scale)
            }
        };
        let channels = if has_alpha { 4 } else { 3 };
        let mut out = Vec::with_capacity(width as usize * height as usize * channels);
        for y in 0..height as usize {
            for x in 0..width as usize {
                let (r, g, b) = if mono {
                    let v = norm_y(sample(0, x, y));
                    (v, v, v)
                } else if matrix == Matrix::Identity {
                    // GBR plane order (H.273 §8.3, MatrixCoefficients 0).
                    let g = sample(0, x, y) / maxv;
                    let b = sample(1, x >> sx, y >> sy) / maxv;
                    let r = sample(2, x >> sx, y >> sy) / maxv;
                    (r, g, b)
                } else {
                    let yy = norm_y(sample(0, x, y));
                    let cb = norm_c(sample(1, x >> sx, y >> sy));
                    let cr = norm_c(sample(2, x >> sx, y >> sy));
                    let r = yy + 2.0 * (1.0 - kr) * cr;
                    let b = yy + 2.0 * (1.0 - kb) * cb;
                    let g = (yy - kr * r - kb * b) / kg;
                    (r, g, b)
                };
                out.push(r.clamp(0.0, 1.0));
                out.push(g.clamp(0.0, 1.0));
                out.push(b.clamp(0.0, 1.0));
                if has_alpha {
                    out.push(sample(3, x, y) / maxv);
                }
            }
        }
        (channels, out)
    }

    /// A packed RGB / gray oracle frame (what the registry's PNG decoder
    /// yields) as `[r, g, b(, a)]` per pixel in `0.0..=1.0`, gray
    /// broadcast to three channels.
    pub fn oracle_rgb(
        frame: &VideoFrame,
        fmt: PixelFormat,
        width: u32,
        height: u32,
    ) -> (usize, Vec<f32>) {
        if fmt == PixelFormat::Pal8 {
            // Palette PNGs (some readers quantise tiny renders): expand
            // through the pipeline converter, which reads the frame's
            // palette side-channel.
            let rgba = convert_pixfmt(frame, PixelFormat::Pal8, width, height, PixelFormat::Rgba);
            return oracle_rgb(&rgba, PixelFormat::Rgba, width, height);
        }
        let p = &frame.image_planes()[0];
        let depth = bit_depth(fmt);
        let bytes = if depth > 8 { 2 } else { 1 };
        let maxv = ((1u32 << depth) - 1) as f32;
        let comps = match fmt {
            PixelFormat::Gray8 | PixelFormat::Gray16Le => 1,
            PixelFormat::Rgb24 | PixelFormat::Rgb48Le => 3,
            PixelFormat::Rgba | PixelFormat::Rgba64Le => 4,
            other => panic!("oracle format {other:?} not handled"),
        };
        let channels = if comps == 4 { 4 } else { 3 };
        let mut out = Vec::with_capacity(width as usize * height as usize * channels);
        for y in 0..height as usize {
            for x in 0..width as usize {
                let base = y * p.stride + x * comps * bytes;
                let get = |c: usize| -> f32 {
                    let i = base + c * bytes;
                    let v = if bytes == 2 {
                        u16::from_le_bytes([p.data[i], p.data[i + 1]]) as f32
                    } else {
                        p.data[i] as f32
                    };
                    v / maxv
                };
                if comps == 1 {
                    let v = get(0);
                    out.extend_from_slice(&[v, v, v]);
                } else {
                    out.push(get(0));
                    out.push(get(1));
                    out.push(get(2));
                    if comps == 4 {
                        out.push(get(3));
                    }
                }
            }
        }
        (channels, out)
    }

    /// `(max, mean)` absolute difference in 8-bit code units between two
    /// normalised pixel buffers of the same channel count, over the
    /// colour channels only; the third field is the max alpha
    /// difference (0 when neither side carries alpha).
    pub fn rgb_diff(a: &(usize, Vec<f32>), b: &(usize, Vec<f32>)) -> (f32, f32, f32) {
        assert_eq!(a.0, b.0, "channel count");
        assert_eq!(a.1.len(), b.1.len(), "pixel count");
        let ch = a.0;
        let mut max = 0f32;
        let mut sum = 0f64;
        let mut n = 0u64;
        let mut amax = 0f32;
        for (pa, pb) in a.1.chunks(ch).zip(b.1.chunks(ch)) {
            for c in 0..3 {
                let d = (pa[c] - pb[c]).abs() * 255.0;
                max = max.max(d);
                sum += d as f64;
                n += 1;
            }
            if ch == 4 {
                amax = amax.max((pa[3] - pb[3]).abs() * 255.0);
            }
        }
        (max, (sum / n.max(1) as f64) as f32, amax)
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
}
