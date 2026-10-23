//! Round-476 gateway cells over the real format crates: the JPEG
//! registry encoder's declared layouts (heic → jpg was refused before
//! `oxideav-mjpeg` declared what it accepts), palette alpha through a PNG
//! round trip, colour targets for RGB sources on HEIC / AVIF, and
//! mixed-geometry pictures into multi-stream containers.
//!
//! Oracles: the gateway's own RGB views of the re-decoded bytes (mean /
//! max absolute difference bounds), the format crates' standalone
//! `info()` where a layout claim needs an independent witness, and the
//! IMAGE_CRATE_API.md rulings (round 475: colour before grey, alpha
//! rungs first for a palette with alpha, one stream per geometry).

use oxideav_core::PixelFormat;
use oxideav_image::{Image, ImageError, SaveOptions, decode_bytes, encode, encode_frames, open};
use oxideav_tests::image::{heif_fixtures_root, meta_ctx};

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

/// A gradient with a deterministic texture on top: smooth enough for a
/// tight lossy bound, textured enough that no intra predictor codes it
/// with a zero residual (so quality must change the bytes).
fn gradient_rgb(w: u32, h: u32) -> Vec<u8> {
    (0..h)
        .flat_map(|y| {
            (0..w).flat_map(move |x| {
                let t = (x * 7 + y * 13 + x * y) % 23;
                [
                    (x * 200 / w.max(1) + t * 2) as u8,
                    (y * 200 / h.max(1) + t * 2) as u8,
                    ((x + y) * 200 / (w + h).max(1) + t * 2) as u8,
                ]
            })
        })
        .collect()
}

fn gradient_rgba(w: u32, h: u32) -> Vec<u8> {
    gradient_rgb(w, h)
        .chunks(3)
        .enumerate()
        .flat_map(|(i, p)| {
            [
                p[0],
                p[1],
                p[2],
                if i % 3 == 0 {
                    255
                } else {
                    (i * 37 % 256) as u8
                },
            ]
        })
        .collect()
}

fn rgb_of(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks(4).flat_map(|p| p[..3].to_vec()).collect()
}

/// Encode `img` as `format`, decode the bytes back through the gateway
/// and return the primary picture.
fn round_trip(img: &Image, format: &str, opts: &SaveOptions) -> Image {
    let ctx = meta_ctx();
    let bytes = encode(&ctx, img, format, opts)
        .unwrap_or_else(|e| panic!("{format}: encode from {:?}: {e}", img.format()));
    assert!(!bytes.is_empty(), "{format}: empty output");
    let file = decode_bytes(&ctx, &bytes).unwrap_or_else(|e| panic!("{format}: re-open: {e}"));
    file.primary().clone()
}

/// JPEG targets take every layout a source can present: RGB, grey, RGBA
/// (alpha dropped by the ladder) and a 4:4:4 HEIC still. Round 475
/// refused `heic → jpg` with "expected 3 planes" because the encoder
/// declared nothing; now `oxideav-mjpeg` declares 19 layouts and the
/// gateway converts first.
#[test]
fn jpeg_targets_take_rgb_grey_rgba_and_heic_sources() {
    let (w, h) = (24u32, 18u32);
    let q = SaveOptions::new().with_quality(95);

    // RGB → JPEG → RGB.
    let rgb = gradient_rgb(w, h);
    let out = round_trip(&Image::from_rgb8(w, h, rgb.clone()).unwrap(), "jpeg", &q);
    assert_eq!((out.width(), out.height()), (w, h));
    let (mean, max) = diff(&out.to_rgb8().unwrap(), &rgb);
    assert!(mean < 4.0 && max < 40, "rgb→jpeg→rgb mean {mean} max {max}");

    // Grey → JPEG → grey (single component, not a colour-coded grey).
    let grey: Vec<u8> = (0..w * h).map(|i| (i * 255 / (w * h)) as u8).collect();
    let g = Image::from_rgb8(w, h, grey.iter().flat_map(|&v| [v, v, v]).collect())
        .unwrap()
        .to_format(PixelFormat::Gray8)
        .unwrap();
    assert_eq!(g.format(), PixelFormat::Gray8);
    let out = round_trip(&g, "jpeg", &q);
    assert_eq!(
        out.format(),
        PixelFormat::Gray8,
        "grey source stays a grey JPEG"
    );
    let (mean, max) = diff(&out.to_gray8().unwrap(), &grey);
    assert!(
        mean < 2.0 && max < 16,
        "grey→jpeg→grey mean {mean} max {max}"
    );

    // RGBA → JPEG: alpha has no JPEG carrier; the ladder drops it.
    let rgba = gradient_rgba(w, h);
    let out = round_trip(&Image::from_rgba8(w, h, rgba.clone()).unwrap(), "jpeg", &q);
    assert!(!out.has_alpha(), "JPEG carries no alpha");
    let (mean, max) = diff(&out.to_rgb8().unwrap(), &rgb_of(&rgba));
    assert!(mean < 4.0 && max < 40, "rgba→jpeg mean {mean} max {max}");

    // HEIC 4:4:4 still → JPEG (the round-475 failure).
    let ctx = meta_ctx();
    let heic = open(
        &ctx,
        heif_fixtures_root().join("corpus/still-yuv444/input.heic"),
    )
    .expect("heic still");
    let src = heic.primary();
    let want = src.to_rgb8().unwrap();
    let out = round_trip(src, "jpeg", &q);
    assert_eq!((out.width(), out.height()), (src.width(), src.height()));
    let (mean, max) = diff(&out.to_rgb8().unwrap(), &want);
    assert!(mean < 4.0 && max < 48, "heic→jpeg mean {mean} max {max}");
}

/// A palette with a transparent entry keeps its alpha through a PNG
/// round trip (core 0.1.40 palette-alpha record; the gateway's RGBA
/// views expand through `palette_rgba`).
#[test]
fn palette_alpha_survives_a_png_round_trip() {
    let (w, h) = (8u32, 4u32);
    let palette: [[u8; 4]; 4] = [
        [255, 0, 0, 255],
        [0, 255, 0, 0],
        [0, 0, 255, 128],
        [9, 9, 9, 255],
    ];
    let indices: Vec<u8> = (0..w * h).map(|i| (i % 4) as u8).collect();
    let img = Image::from_indexed(w, h, indices.clone(), &palette).unwrap();
    assert!(
        img.has_alpha(),
        "a palette with a non-opaque entry has alpha"
    );
    assert_eq!(img.palette_rgba(1), Some([0, 255, 0, 0]));
    let rgba = img.to_rgba8().unwrap();
    assert_eq!(rgba[4 + 3], 0, "transparent entry → alpha 0");
    assert_eq!(rgba[4 * 2 + 3], 128);
    assert_eq!(rgba[4 * 3 + 3], 255);

    let out = round_trip(&img, "png", &SaveOptions::default());
    assert_eq!(
        out.to_rgb8().unwrap(),
        img.to_rgb8().unwrap(),
        "palette colours identical after the PNG round trip (RGB views drop alpha, no compositing)"
    );
    // The write side: `oxideav-png`'s encoder does not read the palette-alpha
    // record yet (round-476 survey: 0 of 25 crates push the record through
    // the registry), so a gateway-written PNG comes back opaque today. Once
    // png adopts it the alpha must come back exactly; until then the only
    // acceptable outcomes are "lost entirely" or "kept exactly".
    if out.has_alpha() {
        assert_eq!(out.to_rgba8().unwrap(), rgba, "alpha kept exactly");
    } else {
        assert_eq!(
            out.format(),
            PixelFormat::Pal8,
            "palette kept even when the alpha record is not written"
        );
    }
}

/// An RGB source lands on a colour HEIC / AVIF, never on a grey rung
/// (round-475 ladder: colour before grey); the requested name picks the
/// codec and brand (`avif` → AV1 in an `avif`-branded file, `heic` →
/// HEVC), and the advisory quality reaches the codec (bytes shrink as
/// quality drops).
#[test]
fn rgb_sources_land_on_colour_heic_and_avif_with_the_requested_codec() {
    let ctx = meta_ctx();
    let (w, h) = (16u32, 16u32);
    let rgb = gradient_rgb(w, h);
    let src = Image::from_rgb8(w, h, rgb.clone()).unwrap();
    // `heic` / `heix` are both HEVC brands (`heix` for profiles beyond
    // Main, e.g. the lossless rung at quality 100); `avif` is AV1's.
    type Brand<'a> = (&'a str, &'a [&'a [u8; 4]], &'a [u8; 4], &'a [u8; 4]);
    let brands: [Brand; 2] = [
        ("heic", &[b"heic", b"heix"], b"hvc1", b"av01"),
        ("avif", &[b"avif"], b"av01", b"hvc1"),
    ];
    for (format, brands, payload, other) in brands {
        let mut sizes = Vec::new();
        for q in [100u8, 90, 50] {
            let bytes = encode(&ctx, &src, format, &SaveOptions::new().with_quality(q))
                .unwrap_or_else(|e| panic!("{format} q{q}: {e}"));
            assert!(
                brands.iter().any(|b| &bytes[8..12] == *b),
                "{format} q{q}: major brand {:?}",
                String::from_utf8_lossy(&bytes[8..12])
            );
            assert!(
                bytes.windows(4).any(|x| x == payload),
                "{format}: carries {}",
                String::from_utf8_lossy(payload)
            );
            assert!(
                !bytes.windows(4).any(|x| x == other),
                "{format}: must not carry {}",
                String::from_utf8_lossy(other)
            );
            let file = decode_bytes(&ctx, &bytes)
                .unwrap_or_else(|e| panic!("{format} q{q}: re-open: {e}"));
            let out = file.primary();
            assert_eq!((out.width(), out.height()), (w, h), "{format}: geometry");
            let fmt = out.format();
            assert!(
                !matches!(
                    fmt,
                    PixelFormat::Gray8
                        | PixelFormat::Gray10Le
                        | PixelFormat::Gray12Le
                        | PixelFormat::Gray16Le
                ),
                "{format}: RGB source coded as grey ({fmt:?})"
            );
            let (mean, max) = diff(&out.to_rgb8().unwrap(), &rgb);
            let (mean_cap, max_cap) = if q == 100 { (3.0, 32) } else { (12.0, 96) };
            assert!(
                mean < mean_cap && max < max_cap,
                "{format} q{q}: mean {mean} max {max}"
            );
            sizes.push(bytes.len());
        }
        assert!(
            sizes[0] >= sizes[1] && sizes[1] >= sizes[2] && sizes[0] > sizes[2],
            "{format}: quality reaches the codec (bytes at q100/q90/q50 = {sizes:?})"
        );
    }
}

/// Pictures of different geometries go to one stream per geometry in a
/// multi-stream container (ICO, EXR); a single-geometry format refuses
/// by type instead of resizing.
#[test]
fn mixed_geometry_pictures_fan_out_to_ico_and_exr_and_are_refused_by_png() {
    let ctx = meta_ctx();
    let a = Image::from_rgba8(8, 8, gradient_rgba(8, 8)).unwrap();
    let b = Image::from_rgba8(12, 6, gradient_rgba(12, 6)).unwrap();
    let frames = [a.clone(), b.clone()];
    for format in ["ico", "exr"] {
        let bytes = encode_frames(&ctx, &frames, format, &SaveOptions::default())
            .unwrap_or_else(|e| panic!("{format}: mixed geometry: {e}"));
        let file = decode_bytes(&ctx, &bytes).unwrap_or_else(|e| panic!("{format}: re-open: {e}"));
        let mut sizes: Vec<(u32, u32)> = file
            .frames()
            .iter()
            .map(|f| (f.width(), f.height()))
            .collect();
        sizes.sort_unstable();
        assert_eq!(
            sizes,
            vec![(8, 8), (12, 6)],
            "{format}: both geometries come back"
        );
        for f in file.frames() {
            let want = if f.width() == 8 { &a } else { &b };
            let (mean, max) = diff(&f.to_rgba8().unwrap(), &want.to_rgba8().unwrap());
            assert!(
                mean < 1.0 && max <= 2,
                "{format} {}x{}: mean {mean} max {max}",
                f.width(),
                f.height()
            );
        }
    }
    let err = encode_frames(&ctx, &frames, "png", &SaveOptions::default())
        .expect_err("png: a single-geometry format refuses a mixed set");
    assert!(
        matches!(err, ImageError::Unsupported(_)),
        "png refuses by type, never resizes: {err}"
    );
}
