//! `oxideav-image` (Layer 2 of IMAGE_CRATE_API.md) over the real format
//! crates, registered through `oxideav_meta::register_all`.
//!
//! Oracles are independent of the gateway: the umbrella's own
//! registry-path decode (`decode_still`), the `expected*.png` renders
//! vendored next to the HEIF corpus, the `.pgm` luma references next to
//! the JPEG fixtures, and the format specifications' magic bytes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use oxideav_core::{PixelFormat, RuntimeContext};
use oxideav_image::{
    decode_bytes, decode_bytes_with, encode, encode_frames, open, open_with, Image, ImageError,
    OpenOptions, SaveOptions,
};
use oxideav_tests::image::{
    decode_still, heif_corpus_dirs, heif_fixtures_root, meta_ctx, packed_planes,
};

fn crate_fixture(crate_name: &str, rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(crate_name)
        .join(rel)
}

/// Mean / max absolute difference over two equally long byte buffers.
fn diff(a: &[u8], b: &[u8]) -> (f64, u8) {
    assert_eq!(a.len(), b.len(), "buffer lengths");
    let mut sum = 0u64;
    let mut max = 0u8;
    for (x, y) in a.iter().zip(b) {
        let d = x.abs_diff(*y);
        sum += d as u64;
        max = max.max(d);
    }
    (sum as f64 / a.len().max(1) as f64, max)
}

/// A deterministic RGBA picture with every channel varying.
fn gradient_rgba(w: u32, h: u32) -> Vec<u8> {
    (0..h)
        .flat_map(|y| {
            (0..w).flat_map(move |x| {
                [
                    (x * 255 / w.max(1)) as u8,
                    (y * 255 / h.max(1)) as u8,
                    ((x + y) * 7) as u8,
                    if (x + y) % 3 == 0 {
                        255
                    } else {
                        (x * 40 + y * 17) as u8
                    },
                ]
            })
        })
        .collect()
}

/// `img` as RGB under its own colour signal and under the common H.273
/// matrices; the closest to `want` wins.
fn best_matrix_diff(img: &Image, want: &[u8]) -> (f64, u8, &'static str) {
    use oxideav_core::{
        ColorPrimaries, ColorRange, ColorSignal, MatrixCoefficients, TransferCharacteristics,
    };
    let sig = |range, m: u8| {
        ColorSignal::new(
            range,
            ColorPrimaries(1),
            TransferCharacteristics(1),
            MatrixCoefficients(m),
        )
    };
    let mut best = {
        let (mean, max) = diff(&rgb_of(&img.to_rgba8().unwrap()), want);
        (mean, max, "signalled")
    };
    for (name, s) in [
        ("bt601-limited", sig(ColorRange::Limited, 6)),
        ("bt709-limited", sig(ColorRange::Limited, 1)),
        ("bt601-full", sig(ColorRange::Full, 6)),
        ("bt709-full", sig(ColorRange::Full, 1)),
    ] {
        let rgba = img.clone().with_color_signal(s).to_rgba8().unwrap();
        let (mean, max) = diff(&rgb_of(&rgba), want);
        if mean < best.0 {
            best = (mean, max, name);
        }
    }
    best
}

fn rgb_of(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks(4).flat_map(|p| p[..3].to_vec()).collect()
}

fn opaque(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks(4)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
}

/// Every still fixture whose format has a registered demuxer.
fn still_fixtures() -> Vec<(PathBuf, &'static str)> {
    let mut v = vec![
        (
            crate_fixture(
                "oxideav-mjpeg",
                "tests/fixtures/noninterleaved/seqni37_2x1.jpg",
            ),
            "jpeg",
        ),
        (
            crate_fixture(
                "oxideav-mjpeg",
                "tests/fixtures/noninterleaved/prog37_2x2.jpg",
            ),
            "jpeg",
        ),
        (
            heif_fixtures_root().join("corpus/still-yuv444/input.heic"),
            "heif",
        ),
        (
            heif_fixtures_root().join("corpus/still-image-grid-2x2/input.heic"),
            "heif",
        ),
        (
            heif_fixtures_root().join("corpus/still-image-grid-2x2/expected.png"),
            "png",
        ),
        (
            crate_fixture("oxideav-avif", "tests/fixtures/red.avif"),
            "heif",
        ),
        // (bbb_alpha.avif is 17 s of AV1 in a debug build; the alpha path
        // is covered by the heif corpus bundle with alpha.)
        (
            crate_fixture("oxideav-avif", "tests/fixtures/black420.avif"),
            "heif",
        ),
        (
            crate_fixture("oxideav-tiff", "tests/data/icc_xmp/icc_xmp_gray16.tif"),
            "tiff",
        ),
        (
            crate_fixture("oxideav-hdr", "tests/fixtures/gradient_32x16_newrle.hdr"),
            "hdr",
        ),
        (
            crate_fixture("oxideav-dds", "tests/fixtures/grad8.dds"),
            "dds",
        ),
    ];
    v.retain(|(p, _)| p.exists());
    v
}

#[test]
fn gateway_agrees_with_the_registry_path_on_every_still_fixture() {
    let ctx = meta_ctx();
    let fixtures = still_fixtures();
    assert!(fixtures.len() >= 8, "fixtures present: {}", fixtures.len());
    for (path, container) in fixtures {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let file = open(&ctx, &path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(file.container(), container, "{name}: container");
        let img = file.primary();
        let oracle = decode_still(&ctx, &path).unwrap_or_else(|e| panic!("{name}: oracle: {e}"));
        assert_eq!(
            (img.width(), img.height(), img.format()),
            (oracle.width(), oracle.height(), oracle.pixel_format()),
            "{name}: geometry / layout"
        );
        let ours = packed_planes(img.frame(), img.format(), img.width(), img.height());
        let theirs = packed_planes(
            &oracle.frame,
            oracle.pixel_format(),
            oracle.width(),
            oracle.height(),
        );
        assert_eq!(ours, theirs, "{name}: planes byte-for-byte");
        assert_eq!(img.palette(), oracle.frame.palette(), "{name}: palette");
        // The raw views are consistent with each other.
        let rgba = img
            .to_rgba8()
            .unwrap_or_else(|e| panic!("{name}: to_rgba8: {e}"));
        let rgb = img.to_rgb8().unwrap();
        assert_eq!(rgba.len(), (img.width() * img.height() * 4) as usize);
        assert_eq!(rgb, rgb_of(&rgba), "{name}: rgb8 is rgba8 minus alpha");
        assert_eq!(img.delay(), None, "{name}: stills have no delay");
        assert_eq!(img.index(), 0);
    }
}

#[test]
fn heif_corpus_matches_expected_png_through_the_gateway() {
    let ctx = meta_ctx();
    for dir in heif_corpus_dirs() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let heic = open(&ctx, dir.join("input.heic")).unwrap_or_else(|e| panic!("{name}: {e}"));
        let oracle = open(&ctx, dir.join("expected.png")).expect("expected.png");
        let (a, b) = (heic.primary(), oracle.primary());
        assert_eq!(
            (a.width(), a.height()),
            (b.width(), b.height()),
            "{name}: geometry"
        );
        let ours = a.to_rgba8().unwrap();
        let theirs = b.to_rgba8().unwrap();
        // Colour within the oracle's rounding (same bound as the heif
        // suite); alpha exact.
        let (mean, max) = diff(&rgb_of(&ours), &rgb_of(&theirs));
        eprintln!("{name}: {:?} mean={mean:.3} max={max}", a.format());
        assert!(
            mean <= 1.5 && max <= 12,
            "{name}: diverges from expected.png: mean={mean:.3} max={max}"
        );
        let alpha_ours: Vec<u8> = ours.chunks(4).map(|p| p[3]).collect();
        let alpha_theirs: Vec<u8> = theirs.chunks(4).map(|p| p[3]).collect();
        assert_eq!(alpha_ours, alpha_theirs, "{name}: alpha exact");
    }
}

#[test]
fn heif_sequence_track_frames_carry_delays_and_match_their_oracles() {
    let ctx = meta_ctx();
    let dir = heif_fixtures_root().join("corpus/image-sequence-3frame");
    let file = open(&ctx, dir.join("input.heic")).expect("sequence");
    // Stream 0 is the primary still, stream 1 the `pict` track.
    let track: Vec<&Image> = file.frames().iter().filter(|i| i.stream() == 1).collect();
    assert_eq!(
        track.len(),
        3,
        "three track frames; got {} pictures",
        file.len()
    );
    assert_eq!(file.frames().iter().filter(|i| i.stream() == 0).count(), 1);
    let mut t = Duration::ZERO;
    for (i, img) in track.iter().enumerate() {
        let delay = img.delay().unwrap_or_else(|| panic!("frame {i}: delay"));
        assert!(delay > Duration::ZERO, "frame {i}: positive delay");
        assert_eq!(img.timestamp(), Some(t), "frame {i}: timestamp");
        t += delay;
        let oracle = open(&ctx, dir.join(format!("expected_{i}.png"))).expect("oracle");
        let theirs = rgb_of(&oracle.primary().to_rgba8().unwrap());
        // The oracle renders were made by a black-box reader whose matrix
        // choice is not recorded, so (like the heif suite's oracle_diff)
        // the best of the signalled / BT.601 / BT.709 conversions counts.
        let (mean, max, choice) = best_matrix_diff(img, &theirs);
        eprintln!("frame {i}: mean={mean:.3} max={max} ({choice})");
        assert!(
            mean <= 1.5 && max <= 12,
            "frame {i}: mean={mean:.3} max={max} ({choice})"
        );
    }
    // max_frames stops after the primary.
    let one = open_with(
        &ctx,
        dir.join("input.heic"),
        &OpenOptions::new().with_max_frames(1),
    )
    .unwrap();
    assert_eq!(one.len(), 1);
}

#[test]
fn jpeg_luma_matches_the_pgm_reference() {
    let ctx = meta_ctx();
    let root = crate_fixture("oxideav-mjpeg", "tests/fixtures/noninterleaved");
    for stem in ["seqni37_2x1", "prog37_2x1", "prog37_2x2"] {
        let jpg = open(&ctx, root.join(format!("{stem}.jpg"))).expect("jpg");
        let pgm = open(&ctx, root.join(format!("{stem}_y.pgm"))).expect("pgm");
        let (j, p) = (jpg.primary(), pgm.primary());
        assert_eq!(pgm.container(), "pbm", "{stem}");
        assert_eq!(p.format(), PixelFormat::Gray8, "{stem}");
        assert_eq!((j.width(), j.height()), (p.width(), p.height()), "{stem}");
        // `djpeg -nosmooth -grayscale` is the ±1 IDCT-rounding oracle the
        // mjpeg suite uses for these fixtures.
        let luma = packed_planes(j.frame(), j.format(), j.width(), j.height());
        let luma = &luma[..(j.width() * j.height()) as usize];
        let (mean, max) = diff(luma, p.as_packed().unwrap());
        assert!(
            max <= 1,
            "{stem}: luma max|d| = {max} (mean {mean:.3}) vs djpeg"
        );
    }
}

/// Lossless round trips through real containers: `format`, source
/// layout, whether alpha survives (else the decode is compared against
/// the opaque source), and encoder options. gif is not here (a 9×7
/// gradient has more than 256 colours; `image_gateway_r472` covers it
/// with a palette-sized picture); HEIF `mode=pcm` has its own test.
type RoundTripRow = (
    &'static str,
    PixelFormat,
    bool,
    Vec<(&'static str, &'static str)>,
);

fn round_trip_matrix() -> Vec<RoundTripRow> {
    vec![
        ("png", PixelFormat::Rgba, true, vec![]),
        ("png", PixelFormat::Rgb24, false, vec![]),
        ("png", PixelFormat::Gray8, false, vec![]),
        ("bmp", PixelFormat::Rgba, true, vec![]),
        ("bmp", PixelFormat::Rgb24, false, vec![]),
        ("tga", PixelFormat::Rgba, true, vec![]),
        ("tga", PixelFormat::Rgb24, false, vec![]),
        ("tiff", PixelFormat::Rgba, true, vec![]),
        ("tiff", PixelFormat::Rgb24, false, vec![]),
        ("tiff", PixelFormat::Gray8, false, vec![]),
        ("pcx", PixelFormat::Rgb24, false, vec![]),
        ("ppm", PixelFormat::Rgb24, false, vec![]),
        ("pgm", PixelFormat::Gray8, false, vec![]),
        ("farbfeld", PixelFormat::Rgba, true, vec![]),
        ("ico", PixelFormat::Rgba, true, vec![]),
        ("qoi", PixelFormat::Rgba, true, vec![]),
        ("qoi", PixelFormat::Rgb24, false, vec![]),
        ("jpeg2000", PixelFormat::Rgb24, false, vec![]),
        ("jpegxs", PixelFormat::Rgb24, false, vec![]),
        ("exr", PixelFormat::Rgba, true, vec![]),
        ("icer", PixelFormat::Gray8, false, vec![]),
        // webp / jp2 / jxs / pict: `image_gateway_r472` (webp needs the
        // in-tree 0.3 crate; jp2 / jxs need an explicit codec; pict drops
        // alpha by design).
    ]
}

#[test]
fn encode_round_trips_through_real_containers() {
    let ctx = meta_ctx();
    let (w, h) = (9u32, 7u32);
    let rgba = gradient_rgba(w, h);
    let src = Image::from_rgba8(w, h, rgba.clone()).unwrap();
    let mut failures = Vec::new();
    for (format, layout, alpha, options) in round_trip_matrix() {
        let input = src.to_format(layout).unwrap();
        let mut opts = SaveOptions::default();
        for (k, v) in options {
            opts = opts.with_option(k, v);
        }
        let want = match layout {
            PixelFormat::Rgba if alpha => rgba.clone(),
            PixelFormat::Rgba => opaque(&rgba),
            PixelFormat::Rgb24 => opaque(&rgba),
            PixelFormat::Gray8 => input.to_rgba8().unwrap(),
            other => panic!("matrix layout {other:?}"),
        };
        let bytes = match encode(&ctx, &input, format, &opts) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{format}/{layout:?}: encode: {e}"));
                continue;
            }
        };
        let back = match decode_bytes_with(&ctx, &bytes, &OpenOptions::new().with_ext_hint(format))
        {
            Ok(f) => f,
            Err(e) => {
                failures.push(format!("{format}/{layout:?}: decode: {e}"));
                continue;
            }
        };
        let img = back.primary();
        if (img.width(), img.height()) != (w, h) {
            failures.push(format!(
                "{format}/{layout:?}: geometry {}x{}",
                img.width(),
                img.height()
            ));
            continue;
        }
        let got = img.to_rgba8().unwrap();
        if got != want {
            let (mean, max) = diff(&got, &want);
            failures.push(format!(
                "{format}/{layout:?}: pixels differ (decoded {:?}, mean={mean:.3} max={max})",
                img.format()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "round-trip failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn former_codec_only_formats_write_their_magic_and_open_again() {
    let ctx = meta_ctx();
    let img = Image::from_rgba8(4, 4, gradient_rgba(4, 4)).unwrap();
    let qoi = encode(&ctx, &img, "qoi", &SaveOptions::default()).expect("qoi");
    assert_eq!(&qoi[..4], b"qoif", "QOI magic");
    let webp = encode(&ctx, &img, "webp", &SaveOptions::default()).expect("webp");
    assert_eq!(&webp[..4], b"RIFF");
    assert_eq!(&webp[8..12], b"WEBP");
    let gif = encode(&ctx, &img, "gif", &SaveOptions::default()).expect("gif");
    assert_eq!(&gif[..4], b"GIF8");
    // Since round 472 every image crate registers a container, so the
    // files open again by magic alone (no extension hint).
    // (webp opens in `image_gateway_r472` with the in-tree 0.3 crate;
    // `register_all` still links webp 0.2.3, which has no container.)
    for (name, bytes, container) in [("qoi", &qoi, "qoi"), ("gif", &gif, "gif")] {
        let back = decode_bytes(&ctx, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(back.container(), container, "{name}");
        assert_eq!(
            (back.primary().width(), back.primary().height()),
            (4, 4),
            "{name}"
        );
    }
    // A codec-only name that is NOT a container still says so.
    assert!(matches!(
        decode_bytes_with(
            &ctx,
            b"\0\0\0\0garbage",
            &OpenOptions::new().with_ext_hint("nosuch")
        ),
        Err(ImageError::UnknownFormat(_) | ImageError::NoImage(_) | ImageError::Core(_))
    ));
}

#[test]
fn apng_encode_frames_round_trips_frame_count_and_delays() {
    let ctx = meta_ctx();
    let delays = [100u64, 200, 50];
    let frames: Vec<Image> = delays
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let mut px = gradient_rgba(6, 4);
            for p in px.chunks_mut(4) {
                p[0] = p[0].wrapping_add(i as u8 * 60);
            }
            Image::from_rgba8(6, 4, px)
                .unwrap()
                .with_delay(Some(Duration::from_millis(*d)))
        })
        .collect();
    let bytes = encode_frames(&ctx, &frames, "png", &SaveOptions::default()).expect("apng");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert!(bytes.windows(4).any(|w| w == b"acTL"), "acTL present");
    let back = decode_bytes(&ctx, &bytes).expect("decode apng");
    assert_eq!(back.container(), "png");
    assert_eq!(back.len(), 3);
    for (i, (img, want)) in back.frames().iter().zip(&frames).enumerate() {
        assert_eq!(
            img.delay(),
            Some(Duration::from_millis(delays[i])),
            "frame {i}: delay"
        );
        assert_eq!(
            img.to_rgba8().unwrap(),
            want.to_rgba8().unwrap(),
            "frame {i}: pixels"
        );
    }
}

#[test]
fn save_picks_the_format_from_the_extension() {
    let ctx = meta_ctx();
    let dir = oxideav_tests::tmp("image_gateway_save");
    std::fs::create_dir_all(&dir).unwrap();
    let img = Image::from_rgba8(5, 4, gradient_rgba(5, 4)).unwrap();
    // GIF has 1-bit transparency and the framework palette record is RGB
    // only, so its picture is the opaque gradient (20 colours → Pal8).
    let img_opaque = Image::from_rgba8(5, 4, opaque(&gradient_rgba(5, 4))).unwrap();
    // (webp: `image_gateway_r472`, register_all still links webp 0.2.)
    for ext in ["png", "bmp", "tga", "tif", "qoi", "gif", "exr"] {
        let img = if ext == "gif" { &img_opaque } else { &img };
        let path = dir.join(format!("picture.{ext}"));
        oxideav_image::save(&ctx, img, &path, &SaveOptions::default())
            .unwrap_or_else(|e| panic!("{ext}: save: {e}"));
        assert!(
            path.metadata().map(|m| m.len() > 0).unwrap_or(false),
            "{ext}: written"
        );
        let back = open(&ctx, &path).unwrap_or_else(|e| panic!("{ext}: open: {e}"));
        if ext == "gif" {
            // 20 colours fit a palette: lossless too.
            assert_eq!(back.primary().format(), PixelFormat::Pal8, "{ext}");
        }
        if ext == "exr" {
            // 8-bit → linear float → 8-bit is exact (b / 255 round-trips).
            assert_eq!(back.primary().format(), PixelFormat::RgbaF32Le, "{ext}");
        }
        assert_eq!(
            back.primary().to_rgba8().unwrap(),
            img.to_rgba8().unwrap(),
            "{ext}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quality_is_dropped_for_lossless_encoders_and_reaches_lossy_ones() {
    let ctx = meta_ctx();
    let img = Image::from_rgba8(32, 32, gradient_rgba(32, 32)).unwrap();
    // png declares no "quality": the option is not forwarded, encode works.
    let png = encode(&ctx, &img, "png", &SaveOptions::new().with_quality(50)).expect("png");
    assert_eq!(
        decode_bytes(&ctx, &png)
            .unwrap()
            .primary()
            .to_rgba8()
            .unwrap(),
        img.to_rgba8().unwrap()
    );
    let schema = oxideav_image::encoder_options(&ctx, "png", &SaveOptions::default()).unwrap();
    assert!(schema.iter().all(|f| f.name != "quality"));
    // jpeg's encoder declares "quality" (round 472), so
    // SaveOptions::quality reaches it: two qualities give two different
    // files that both decode. The RGBA source goes through the ladder to
    // `Rgba` → refused → `Rgb24`, an Adobe transform-0 JPEG the demuxer
    // labels `Rgb24`.
    let schema = oxideav_image::encoder_options(&ctx, "jpg", &SaveOptions::default()).unwrap();
    assert!(
        schema.iter().any(|f| f.name == "quality"),
        "mjpeg declares quality"
    );
    let lo = encode(&ctx, &img, "jpg", &SaveOptions::new().with_quality(10)).expect("jpg q10");
    let hi = encode(&ctx, &img, "jpg", &SaveOptions::new().with_quality(95)).expect("jpg q95");
    assert_ne!(lo, hi, "quality changes the jpeg output");
    let (mean_lo, _) = diff(
        &rgb_of(
            &decode_bytes(&ctx, &lo)
                .unwrap()
                .primary()
                .to_rgba8()
                .unwrap(),
        ),
        &rgb_of(&img.to_rgba8().unwrap()),
    );
    let (mean_hi, _) = diff(
        &rgb_of(
            &decode_bytes(&ctx, &hi)
                .unwrap()
                .primary()
                .to_rgba8()
                .unwrap(),
        ),
        &rgb_of(&img.to_rgba8().unwrap()),
    );
    assert!(
        mean_hi < mean_lo,
        "q95 ({mean_hi:.3}) closer than q10 ({mean_lo:.3})"
    );
    // heif's HEVC path takes `qp` through with_option (its `quality` is
    // the AV1 knob, per its schema help text).
    let schema = oxideav_image::encoder_options(&ctx, "heic", &SaveOptions::default()).unwrap();
    assert!(schema.iter().any(|f| f.name == "qp"), "heif declares qp");
    let lo = encode(
        &ctx,
        &img,
        "heic",
        &SaveOptions::new().with_option("qp", "45"),
    )
    .expect("qp45");
    let hi = encode(
        &ctx,
        &img,
        "heic",
        &SaveOptions::new().with_option("qp", "8"),
    )
    .expect("qp8");
    assert_ne!(lo, hi, "qp changes the heif output");
    assert_eq!(decode_bytes(&ctx, &hi).unwrap().container(), "heif");
}

#[test]
fn max_pixels_refuses_before_decoding() {
    let ctx = meta_ctx();
    let path = heif_fixtures_root().join("corpus/still-yuv444/input.heic");
    let probe = open(&ctx, &path).unwrap();
    let px = u64::from(probe.primary().width()) * u64::from(probe.primary().height());
    assert!(open_with(&ctx, &path, &OpenOptions::new().with_max_pixels(px)).is_ok());
    assert!(matches!(
        open_with(&ctx, &path, &OpenOptions::new().with_max_pixels(px - 1)),
        Err(ImageError::LimitExceeded(_))
    ));
}

#[test]
fn crop_through_the_gateway_matches_the_registry_decode() {
    let ctx = meta_ctx();
    let path = heif_fixtures_root().join("corpus/still-yuv444/input.heic");
    let file = open(&ctx, &path).unwrap();
    let img = file.primary();
    let (w, h) = (img.width(), img.height());
    assert!(w >= 4 && h >= 4);
    let c = img.crop(2, 2, w - 2, h - 2).unwrap();
    let full = img.to_rgba8().unwrap();
    let part = c.to_rgba8().unwrap();
    // Row 0 of the crop is row 2 of the picture from column 2.
    let row0 = &full[(2 * w as usize + 2) * 4..(3 * w as usize) * 4];
    assert_eq!(&part[..row0.len()], row0);
}

#[test]
fn unknown_and_unsupported_requests_have_their_errors() {
    let ctx = meta_ctx();
    let img = Image::from_rgb8(2, 2, vec![0; 12]).unwrap();
    assert!(matches!(
        encode(&ctx, &img, "nosuchformat", &SaveOptions::default()),
        Err(ImageError::UnknownFormat(_))
    ));
    // Text-ish bytes may be claimed by a text container (subtitles) that
    // then holds no picture: either way it is not an image.
    assert!(matches!(
        decode_bytes(&ctx, b"definitely not an image file at all"),
        Err(ImageError::UnknownFormat(_) | ImageError::NoImage(_) | ImageError::Core(_))
    ));
    assert!(matches!(
        open(&ctx, "/nonexistent/dir/picture.png"),
        Err(ImageError::Io(_))
    ));
    let _: &RuntimeContext = &ctx;
}

#[test]
fn heif_pcm_round_trip_is_exact_with_alpha() {
    let ctx = meta_ctx();
    let (w, h) = (9u32, 7u32);
    let rgba = gradient_rgba(w, h);
    let src = Image::from_rgba8(w, h, rgba.clone()).unwrap();
    let bytes = encode(
        &ctx,
        &src,
        "heic",
        &SaveOptions::new().with_option("mode", "pcm"),
    )
    .expect("heic pcm");
    let back = decode_bytes(&ctx, &bytes).expect("decode heic");
    let img = back.primary();
    assert_eq!(back.container(), "heif");
    assert_eq!((img.width(), img.height()), (w, h));
    assert!(
        img.format().has_alpha(),
        "alpha carried: {:?}",
        img.format()
    );
    let got = img.to_rgba8().unwrap();
    let alpha: Vec<u8> = got.chunks(4).map(|p| p[3]).collect();
    let want_alpha: Vec<u8> = rgba.chunks(4).map(|p| p[3]).collect();
    assert_eq!(alpha, want_alpha, "alpha plane exact through PCM");
    // Round 472 ruling: lossless RGB into the YCbCr codec is coded as an
    // identity-matrix 4:4:4 item (`Gbrap8` here), so the round trip is
    // exact under the signal the file carries.
    assert_eq!(img.format(), PixelFormat::Gbrap8, "identity planar RGBA");
    assert_eq!(got, rgba, "pcm RGBA round trip exact");
}
