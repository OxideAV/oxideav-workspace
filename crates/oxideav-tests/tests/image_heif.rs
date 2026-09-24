//! HEIF / HEIC / AVIF decode end to end through the framework (round
//! 460): registry demuxer → `"heif"` decoder → `VideoFrame` for every
//! file the heif crate vendors under `tests/fixtures/` — 39 real-world
//! producer files from Apple ImageIO, libheif and ImageMagick, plus
//! the 13 staged corpus bundles.
//!
//! * Geometry and plane layout against `manifest.tsv`.
//! * **Byte-exact** planes against the black-box video decoder's raw
//!   output wherever the manifest records a fingerprint (26 files).
//! * The PNG oracles (black-box reader renders) within the reader's
//!   own chroma-upsampling / rounding — via a straight H.273 reference
//!   render of our planes, alpha exact.
//! * The `image-sequence-3frame` `pict` track frame by frame.
//! * The `docs/image/heif/` bundles as an extra leg when the docs tree
//!   is staged (SKIP on CI).
//!
//! Every producer file measured here is full-range BT.601 (`colr`
//! `nclx` 1/13/6 full). The registry labels the decoded planes
//! `Yuv420P` / `Yuv444P` (the limited-range names), so the pipeline's
//! pixel-format step renders them narrow-range; the reference render
//! here picks the range itself and reports it. See the round report.
//!
//! Runs unconditionally on CI — no producer binary is needed.

use std::path::PathBuf;

use oxideav_core::{Error, Frame, Packet, PixelFormat};
use oxideav_tests::image::{
    decode_still, fnv1a64, heif_corpus_dirs, heif_docs_fixtures_root, heif_fixtures_root, meta_ctx,
    oracle_rgb, packed_planes, reference_rgb, rgb_diff, DecodedStill, Matrix,
};

/// `((max, mean, alpha_max), (matrix, full_range))` of the best-fitting
/// H.273 render.
type Fit = ((f32, f32, f32), (Matrix, bool));

// ───────────────────────── decode end to end ─────────────────────────

struct ManifestRow {
    width: u32,
    height: u32,
    format: PixelFormat,
    bb_pix_fmt: String,
    bb_raw_bytes: Option<usize>,
    bb_fnv: Option<u64>,
    oracle_png: bool,
}

fn manifest() -> Vec<(String, ManifestRow)> {
    let text = std::fs::read_to_string(heif_fixtures_root().join("interop/manifest.tsv"))
        .expect("manifest.tsv");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            assert!(c.len() >= 11, "manifest row {l:?}");
            let depth: u32 = c[5].parse().expect("depth");
            let alpha = c[6] == "true";
            let format = match (c[4], depth, alpha) {
                ("Yuv420", 8, false) => PixelFormat::Yuv420P,
                ("Yuv420", 8, true) => PixelFormat::Yuva420P,
                ("Yuv420", 10, false) => PixelFormat::Yuv420P10Le,
                ("Yuv420", 12, false) => PixelFormat::Yuv420P12Le,
                ("Yuv422", 8, false) => PixelFormat::Yuv422P,
                ("Yuv444", 8, false) => PixelFormat::Yuv444P,
                ("Yuv444", 8, true) => PixelFormat::Yuva444P,
                ("Yuv444", 10, false) => PixelFormat::Yuv444P10Le,
                ("Mono", 8, false) => PixelFormat::Gray8,
                ("Mono", 10, false) => PixelFormat::Gray10Le,
                ("Mono", 12, false) => PixelFormat::Gray12Le,
                other => panic!("manifest layout {other:?} not mapped"),
            };
            (
                c[0].to_owned(),
                ManifestRow {
                    width: c[2].parse().expect("width"),
                    height: c[3].parse().expect("height"),
                    format,
                    bb_pix_fmt: c[7].to_owned(),
                    bb_raw_bytes: c[8].parse().ok(),
                    bb_fnv: u64::from_str_radix(c[9], 16).ok(),
                    oracle_png: c[10] == "png",
                },
            )
        })
        .collect()
}

/// Every interop file: registry demux → `"heif"` decoder → one
/// `VideoFrame` whose geometry / pixel format match the manifest, and
/// whose visible planes are **byte-exact** against the black-box
/// video decoder's raw dump wherever the manifest fingerprints one
/// (25+ files across all three producers; 8 / 10 / 12-bit, 4:0:0 /
/// 4:2:0 / 4:2:2 / 4:4:4, Apple's 512-px grid, the sequence cover).
#[test]
fn interop_files_decode_to_manifest_geometry_and_black_box_fingerprint() {
    let ctx = meta_ctx();
    let rows = manifest();
    assert!(rows.len() >= 39);
    let mut exact = 0;
    for (name, row) in &rows {
        let path = heif_fixtures_root().join("interop").join(name);
        let d = decode_still(&ctx, &path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(d.container, "heif", "{name}");
        assert_eq!(
            d.params().codec_id.as_str(),
            "heif",
            "{name}: stream 0 codec"
        );
        assert_eq!((d.width(), d.height()), (row.width, row.height), "{name}");
        assert_eq!(d.pixel_format(), row.format, "{name}: layout");
        assert_eq!(
            d.frame.image_planes().len(),
            row.format.plane_count(),
            "{name}: plane count"
        );
        let packed = packed_planes(&d.frame, row.format, row.width, row.height);
        if let (Some(bytes), Some(fnv)) = (row.bb_raw_bytes, row.bb_fnv) {
            assert_eq!(packed.len(), bytes, "{name}: raw size ({})", row.bb_pix_fmt);
            let planes: Vec<oxideav_core::VideoPlane> = vec![oxideav_core::VideoPlane {
                stride: packed.len(),
                data: packed,
            }];
            assert_eq!(
                fnv1a64(&planes),
                fnv,
                "{name}: planes differ from the black-box decoder ({})",
                row.bb_pix_fmt
            );
            exact += 1;
        }
        if name.ends_with(".heics") {
            assert_eq!(d.streams.len(), 2, "{name}: cover + one sequence track");
            assert_eq!(d.streams[1].params.codec_id.as_str(), "h265");
        } else {
            assert_eq!(d.streams.len(), 1, "{name}: one still stream");
        }
    }
    eprintln!(
        "byte-exact against the black-box decoder: {exact}/{}",
        rows.len()
    );
    assert!(exact >= 25, "byte-exact count regressed: {exact}");
}

/// Spec-formula render of the decoded still against the PNG oracle:
/// the smallest `(max, mean, alpha_max)` difference (8-bit code units)
/// over the H.273 matrix / range choices, plus the winning choice. The
/// registry does not carry the file's `colr` into `CodecParameters`,
/// so the test picks the best-fitting one and reports it. Alpha is
/// compared exactly.
fn oracle_diff(d: &DecodedStill, oracle: &DecodedStill) -> Fit {
    let (w, h) = (d.width(), d.height());
    let want = oracle_rgb(&oracle.frame, oracle.pixel_format(), w, h);
    let mut best: Option<Fit> = None;
    for matrix in [
        Matrix::Bt601,
        Matrix::Bt709,
        Matrix::Bt2020,
        Matrix::Identity,
    ] {
        for full in [true, false] {
            let mut got = reference_rgb(&d.frame, d.pixel_format(), w, h, matrix, full);
            if got.0 == 4 && want.0 == 3 {
                // Oracle dropped alpha: compare colour only.
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
            let diff = rgb_diff(&got, &want);
            if best.map_or(true, |(b, _)| diff.1 < b.1) {
                best = Some((diff, (matrix, full)));
            }
        }
    }
    best.expect("one conversion")
}

/// The `expected.png` oracles (black-box reader renders): our decode,
/// pushed through the pipeline's pixel-format step, lands within the
/// oracle's own chroma-upsampling / rounding — a small mean and a
/// bounded max — with alpha exact where present.
#[test]
fn interop_files_match_their_png_oracles() {
    let ctx = meta_ctx();
    let mut checked = 0;
    for (name, row) in manifest() {
        if !row.oracle_png {
            continue;
        }
        let dir = heif_fixtures_root().join("interop");
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name);
        let oracle_path = dir.join(format!("{stem}.expected.png"));
        let d = decode_still(&ctx, &dir.join(&name)).expect("decode");
        let oracle = decode_still(&ctx, &oracle_path).expect("png oracle");
        assert_eq!(oracle.container, "png");
        assert_eq!((oracle.width(), oracle.height()), (row.width, row.height));
        let ((max, mean, amax), (matrix, full)) = oracle_diff(&d, &oracle);
        eprintln!(
            "{name}: vs {stem}.expected.png max={max:.1} mean={mean:.3} alpha={amax:.1} ({matrix:?} full={full})"
        );
        assert!(
            mean <= 1.5 && max <= 12.0,
            "{name}: diverges from its PNG oracle: max={max:.1} mean={mean:.3}"
        );
        assert!(
            amax < 0.5,
            "{name}: alpha plane must be exact (max {amax:.1})"
        );
        checked += 1;
    }
    assert!(checked >= 20, "oracle rows: {checked}");
}

/// The 13 staged corpus bundles (grid / overlay / alpha / 10-bit /
/// 4:4:4 / monochrome / thumbnail / Exif / XMP / ICC / burst /
/// sequence / 1×1): decode through the registry and match
/// `expected.png` within the oracle's rounding.
#[test]
fn corpus_bundles_decode_and_match_expected_png() {
    let ctx = meta_ctx();
    for dir in heif_corpus_dirs() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let d =
            decode_still(&ctx, &dir.join("input.heic")).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(d.container, "heif", "{name}");
        let oracle = decode_still(&ctx, &dir.join("expected.png")).expect("expected.png");
        assert_eq!(
            (d.width(), d.height()),
            (oracle.width(), oracle.height()),
            "{name}: geometry"
        );
        let ((max, mean, amax), choice) = oracle_diff(&d, &oracle);
        eprintln!(
            "{name}: {:?} max={max:.1} mean={mean:.3} alpha={amax:.1} ({choice:?})",
            d.pixel_format()
        );
        assert!(
            mean <= 1.5 && max <= 12.0,
            "{name}: diverges from expected.png: max={max:.1} mean={mean:.3}"
        );
        assert!(amax < 0.5, "{name}: alpha plane must be exact");
    }
}

/// `image-sequence-3frame`: stream 1 is the `pict` track — three
/// `h265` packets with `hvcC` extradata — and each decoded sample
/// matches its per-frame oracle.
#[test]
fn sequence_bundle_track_frames_match_per_frame_oracles() {
    let ctx = meta_ctx();
    let dir = heif_fixtures_root().join("corpus/image-sequence-3frame");
    let mut f = std::fs::File::open(dir.join("input.heic")).expect("open");
    let name = ctx
        .containers
        .probe_input(&mut f, Some("heic"))
        .expect("probe");
    assert_eq!(name, "heif");
    let mut dm = ctx
        .containers
        .open_demuxer(&name, Box::new(f), &ctx.codecs)
        .expect("demuxer");
    let streams = dm.streams().to_vec();
    assert_eq!(streams.len(), 2);
    let track = &streams[1];
    assert_eq!(track.params.codec_id.as_str(), "h265");
    assert!(!track.params.extradata.is_empty(), "hvcC extradata");
    let mut pkts: Vec<Packet> = Vec::new();
    loop {
        match dm.next_packet() {
            Ok(p) if p.stream_index == 1 => pkts.push(p),
            Ok(_) => {}
            Err(Error::Eof) => break,
            Err(e) => panic!("next_packet: {e}"),
        }
    }
    assert_eq!(pkts.len(), 3, "three samples");
    assert!(pkts[0].flags.keyframe, "first sample is a sync sample");
    let mut dec = ctx
        .codecs
        .first_decoder(&track.params)
        .expect("h265 decoder");
    let (w, h) = (
        track.params.width.expect("w"),
        track.params.height.expect("h"),
    );
    let fmt = track.params.pixel_format.expect("fmt");
    let mut frames = Vec::new();
    for p in &pkts {
        dec.send_packet(p).expect("send");
        while let Ok(Frame::Video(v)) = dec.receive_frame() {
            frames.push(v);
        }
    }
    dec.flush().expect("flush");
    while let Ok(Frame::Video(v)) = dec.receive_frame() {
        frames.push(v);
    }
    assert_eq!(frames.len(), 3, "three decoded frames");
    for (i, frame) in frames.iter().enumerate() {
        let oracle = decode_still(&ctx, &dir.join(format!("expected_{i}.png"))).expect("oracle");
        let d = DecodedStill {
            container: "heif".into(),
            streams: vec![oxideav_core::StreamInfo {
                index: 0,
                time_base: track.time_base,
                duration: None,
                start_time: None,
                params: {
                    let mut p = track.params.clone();
                    p.width = Some(w);
                    p.height = Some(h);
                    p.pixel_format = Some(fmt);
                    p
                },
            }],
            frame: frame.clone(),
        };
        let ((max, mean, _), choice) = oracle_diff(&d, &oracle);
        eprintln!("frame {i}: max={max:.1} mean={mean:.3} ({choice:?})");
        assert!(
            mean <= 1.5 && max <= 12.0,
            "frame {i}: diverges from expected_{i}.png: max={max:.1} mean={mean:.3}"
        );
    }
}

/// Extra leg when `docs/image/heif/fixtures/` is staged: every bundle
/// there (the vendored 13 plus `single-image-512x512-q60`) decodes and
/// matches its oracle. Prints SKIP on CI.
#[test]
fn docs_bundles_decode_when_staged() {
    let Some(root) = heif_docs_fixtures_root() else {
        eprintln!("SKIP: docs/image/heif/fixtures not staged");
        return;
    };
    let ctx = meta_ctx();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("docs fixtures")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.join("input.heic").exists())
        .collect();
    dirs.sort();
    assert!(dirs.len() >= 14, "docs bundles: {}", dirs.len());
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let d =
            decode_still(&ctx, &dir.join("input.heic")).unwrap_or_else(|e| panic!("{name}: {e}"));
        let oracle = decode_still(&ctx, &dir.join("expected.png")).expect("expected.png");
        assert_eq!((d.width(), d.height()), (oracle.width(), oracle.height()));
        let ((max, mean, _), choice) = oracle_diff(&d, &oracle);
        eprintln!("docs {name}: max={max:.1} mean={mean:.3} ({choice:?})");
        assert!(
            mean <= 1.5 && max <= 12.0,
            "{name}: max={max:.1} mean={mean:.3}"
        );
    }
}
