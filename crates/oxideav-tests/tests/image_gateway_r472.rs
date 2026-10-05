//! Round 472: the image crates that gained (or fixed) registry
//! containers, driven through `oxideav-image` over
//! `oxideav_meta::register_all`.
//!
//! The oracle for every case is the format crate's own Layer 1 surface
//! (`info` / `decode_rgba8` / `decode` / `decode_all`): the gateway must
//! report the same container, the same native layout, the same bytes
//! and the same frame count / delays. Formats with no vendored fixture
//! (gif, openexr, icer, jpegxs, the JP2 wrapper, Adobe-RGB JPEG) are
//! synthesised with the crate's Layer 1 encoder inside the test.

use std::path::{Path, PathBuf};
use std::time::Duration;

use oxideav_core::{PixelFormat, RuntimeContext, TimeBase};
use oxideav_image::{
    decode_bytes, decode_bytes_with, encode, encode_frames, open, Image, ImageFile, OpenOptions,
    SaveOptions,
};
use oxideav_tests::image::{heif_fixtures_root, meta_ctx, packed_planes};

fn crate_fixture(crate_name: &str, rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(crate_name)
        .join(rel)
}

fn read_fixture(crate_name: &str, rel: &str) -> Vec<u8> {
    let p = crate_fixture(crate_name, rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
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

/// A four-colour RGBA picture (fits any palette format losslessly);
/// `shift` moves the pattern so animation frames differ.
fn quad_rgba(w: u32, h: u32, shift: u32) -> Vec<u8> {
    const C: [[u8; 4]; 4] = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
    ];
    (0..h)
        .flat_map(|y| (0..w).flat_map(move |x| C[((x + y + shift) % 4) as usize]))
        .collect()
}

fn rgb_of(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks(4).flat_map(|p| p[..3].to_vec()).collect()
}

fn opaque(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks(4)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
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

/// The gateway picture's planes, rows tightly packed in plane order —
/// the shape every Layer 1 `into_raw` produces for its own decode.
fn native_bytes(img: &Image) -> Vec<u8> {
    packed_planes(img.frame(), img.format(), img.width(), img.height())
}

/// Layer 1 formats share core's variant names by contract; the mapping
/// to core is that name.
fn same_layout<F: std::fmt::Debug>(name: &str, layer1: F, gateway: PixelFormat) {
    assert_eq!(
        format!("{layer1:?}"),
        format!("{gateway:?}"),
        "{name}: Layer 1 layout vs gateway layout"
    );
}

fn tb(n: i64, d: i64) -> TimeBase {
    TimeBase::new(n, d)
}

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

/// `oxideav_meta::register_all` still links `oxideav-webp` 0.2.3 from
/// crates.io (the `oxideav-meta` and `oxideav-tiff` manifests pin
/// `"0.2"`, the in-tree crate is 0.3.0), and 0.2 has no container, so
/// the umbrella runtime cannot open a WebP file at all until those pins
/// move. The WebP cases register the in-tree crate on a fresh runtime.
fn webp_ctx() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_webp::register(&mut ctx);
    ctx
}

/// The umbrella runtime plus the in-tree WebP container (see
/// [`webp_ctx`]); the container registry replaces by name, so the probe
/// and demuxer are 0.3.0's.
fn meta_plus_webp_ctx() -> RuntimeContext {
    let mut ctx = meta_ctx();
    oxideav_webp::register(&mut ctx);
    ctx
}

fn decode_hint(ctx: &RuntimeContext, bytes: &[u8], ext: &str) -> ImageFile {
    decode_bytes_with(ctx, bytes, &OpenOptions::new().with_ext_hint(ext))
        .unwrap_or_else(|e| panic!("decode ({ext}): {e}"))
}

// ---------------------------------------------------------------------------
// GIF — synthesised through Layer 1 (no vendored named fixture).
// ---------------------------------------------------------------------------

fn gif_animation_bytes() -> (Vec<u8>, [u64; 3]) {
    let delays = [100u64, 200, 50];
    let frames: Vec<oxideav_gif::Frame> = delays
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let img = oxideav_gif::GifImage::from_rgba8(8, 6, quad_rgba(8, 6, i as u32)).unwrap();
            oxideav_gif::Frame::new(img, Some(ms(*d)))
        })
        .collect();
    let mut opts = oxideav_gif::EncodeOptions::new();
    opts.loop_count = Some(0);
    let bytes = oxideav_gif::encode_all(&frames, &opts).expect("gif encode_all");
    (bytes, delays)
}

#[test]
fn gif_still_is_pal8_with_its_palette_and_matches_layer_1() {
    let ctx = meta_ctx();
    let still = oxideav_gif::GifImage::from_rgba8(8, 6, quad_rgba(8, 6, 1)).unwrap();
    let bytes = oxideav_gif::encode(&still, &oxideav_gif::EncodeOptions::new()).expect("gif");
    let info = oxideav_gif::info(&bytes).unwrap();
    assert_eq!(info.frames, 1);
    let file = decode_bytes(&ctx, &bytes).expect("open gif");
    assert_eq!(file.container(), "gif");
    assert_eq!(file.len(), 1);
    let img = file.primary();
    same_layout("gif still", info.format, img.format());
    assert_eq!(img.format(), PixelFormat::Pal8);
    let pal = img.palette().expect("Pal8 carries its palette");
    assert!(
        pal.len() >= 4 * 3,
        "palette RGB triplets: {}",
        pal.len() / 3
    );
    assert_eq!(
        img.to_rgba8().unwrap(),
        oxideav_gif::decode_rgba8(&bytes).unwrap().data,
        "gif still: rgba vs Layer 1"
    );
    assert_eq!(img.to_rgba8().unwrap(), quad_rgba(8, 6, 1), "lossless");
    assert_eq!(img.delay(), None);
    // Gateway round trip of a still.
    let again = encode(&ctx, img, "gif", &SaveOptions::default()).expect("gif encode");
    assert_eq!(&again[..4], b"GIF8");
    let back = decode_bytes(&ctx, &again).expect("reopen");
    assert_eq!(back.container(), "gif");
    assert_eq!(back.primary().to_rgba8().unwrap(), quad_rgba(8, 6, 1));
}

#[test]
fn gif_animation_frames_delays_loop_count_and_round_trip() {
    let ctx = meta_ctx();
    let (bytes, delays) = gif_animation_bytes();
    let oracle = oxideav_gif::decode_all(&bytes).expect("gif decode_all");
    assert_eq!(oracle.len(), 3);
    let file = decode_bytes(&ctx, &bytes).expect("open gif");
    assert_eq!(file.container(), "gif");
    assert_eq!(file.len(), oracle.len(), "frame count vs Layer 1");
    assert!(
        file.metadata()
            .iter()
            .any(|(k, v)| k == "loop_count" && v == "0"),
        "loop_count metadata: {:?}",
        file.metadata()
    );
    let mut t = Duration::ZERO;
    for (i, (img, want)) in file.frames().iter().zip(&oracle).enumerate() {
        assert_eq!(img.time_base(), tb(1, 100), "frame {i}: centiseconds");
        assert_eq!(img.delay(), Some(ms(delays[i])), "frame {i}: delay");
        assert_eq!(img.delay(), want.delay, "frame {i}: delay vs Layer 1");
        assert_eq!(img.timestamp(), Some(t), "frame {i}: timestamp");
        t += ms(delays[i]);
        assert_eq!(
            img.to_rgba8().unwrap(),
            want.image.to_rgba8(),
            "frame {i}: canvas vs Layer 1"
        );
        assert_eq!(img.to_rgba8().unwrap(), quad_rgba(8, 6, i as u32));
    }
    // encode_frames → gif muxer → same count, delays and pixels.
    let again = encode_frames(&ctx, file.frames(), "gif", &SaveOptions::default()).expect("mux");
    let back = decode_bytes(&ctx, &again).expect("reopen");
    assert_eq!(back.len(), 3);
    for (i, img) in back.frames().iter().enumerate() {
        assert_eq!(
            img.delay(),
            Some(ms(delays[i])),
            "re-muxed frame {i}: delay"
        );
        assert_eq!(
            img.to_rgba8().unwrap(),
            quad_rgba(8, 6, i as u32),
            "re-muxed frame {i}: pixels"
        );
    }
}

// ---------------------------------------------------------------------------
// WebP — vendored fixtures.
// ---------------------------------------------------------------------------

#[test]
fn webp_stills_match_layer_1_natively() {
    let ctx = webp_ctx();
    for (rel, want_fmt) in [
        ("tests/data/lossless-32x32-rgba.webp", PixelFormat::Rgba),
        ("tests/data/lossless-32x32-rgb.webp", PixelFormat::Rgba),
        ("tests/data/lossy-1x1.webp", PixelFormat::Yuv420P),
        (
            "tests/data/lossy-with-alpha-128x128.webp",
            PixelFormat::Yuva420P,
        ),
    ] {
        let bytes = read_fixture("oxideav-webp", rel);
        let info = oxideav_webp::info(&bytes).unwrap();
        let file = decode_hint(&ctx, &bytes, "webp");
        assert_eq!(file.container(), "webp", "{rel}");
        assert_eq!(file.len(), 1, "{rel}");
        let img = file.primary();
        same_layout(rel, info.format, img.format());
        assert_eq!(img.format(), want_fmt, "{rel}");
        assert_eq!((img.width(), img.height()), (info.width, info.height));
        // Native planes byte-exact with Layer 1's decode.
        let native = oxideav_webp::decode(&bytes).unwrap().into_raw();
        assert_eq!(native_bytes(img), native, "{rel}: native planes vs Layer 1");
        let rgba = img.to_rgba8().unwrap();
        let l1 = oxideav_webp::decode_rgba8(&bytes).unwrap().data;
        if want_fmt == PixelFormat::Rgba {
            assert_eq!(rgba, l1, "{rel}: rgba vs Layer 1");
        } else {
            // Lossy: Layer 1's own YCbCr → RGB and pixfmt's agree within
            // rounding; the alpha channel is exact.
            let (mean, max) = diff(&rgb_of(&rgba), &rgb_of(&l1));
            assert!(mean <= 1.0 && max <= 3, "{rel}: mean={mean:.3} max={max}");
            let a: Vec<u8> = rgba.chunks(4).map(|p| p[3]).collect();
            let b: Vec<u8> = l1.chunks(4).map(|p| p[3]).collect();
            assert_eq!(a, b, "{rel}: alpha exact");
        }
        assert_eq!(img.delay(), None, "{rel}: still");
    }
}

#[test]
fn webp_animation_frames_are_milliseconds_and_match_decode_all() {
    let ctx = webp_ctx();
    for rel in [
        "tests/data/animated-3-frames-rgb.webp",
        "tests/data/animated-with-alpha.webp",
    ] {
        let bytes = read_fixture("oxideav-webp", rel);
        let oracle = oxideav_webp::decode_all(&bytes).expect("decode_all");
        let file = decode_hint(&ctx, &bytes, "webp");
        assert_eq!(file.container(), "webp", "{rel}");
        assert_eq!(file.len(), oracle.len(), "{rel}: frame count");
        assert!(file.len() >= 2, "{rel}: an animation");
        assert!(
            file.metadata().iter().any(|(k, _)| k == "loop_count"),
            "{rel}: loop_count metadata"
        );
        for (i, (img, want)) in file.frames().iter().zip(&oracle).enumerate() {
            assert_eq!(img.time_base(), tb(1, 1000), "{rel} frame {i}: ms");
            assert_eq!(img.format(), PixelFormat::Rgba, "{rel} frame {i}");
            assert_eq!(img.delay(), want.delay, "{rel} frame {i}: delay");
            assert!(img.delay().is_some(), "{rel} frame {i}: timed");
            assert_eq!(
                img.to_rgba8().unwrap(),
                want.image.to_rgba8(),
                "{rel} frame {i}: canvas vs Layer 1"
            );
        }
    }
}

#[test]
fn webp_round_trips_through_the_gateway() {
    let ctx = webp_ctx();
    let (w, h) = (9u32, 7u32);
    let rgba = gradient_rgba(w, h);
    let src = Image::from_rgba8(w, h, rgba.clone()).unwrap();
    // The `webp` container's payload codecs are `webp_vp8l` / `webp_vp8`;
    // the gateway's default-codec table resolves a *container* named
    // `webp` to a codec named `webp`, which has no encoder (the prefixed
    // lookup only runs for codec-only targets — oxideav-image followup),
    // so the codec is named explicitly.
    let vp8l = SaveOptions::new().with_codec("webp_vp8l");
    // Lossless still.
    let bytes = encode(&ctx, &src, "webp", &vp8l).expect("webp");
    let back = decode_bytes(&ctx, &bytes).expect("reopen");
    assert_eq!(back.container(), "webp");
    assert_eq!(back.primary().to_rgba8().unwrap(), rgba, "lossless rgba");
    // Lossy still: webp_vp8 at a quality, and the file still opens.
    let lossy = encode(
        &ctx,
        &src,
        "webp",
        &SaveOptions::new().with_codec("webp_vp8").with_quality(80),
    )
    .expect("vp8");
    let back = decode_bytes(&ctx, &lossy).expect("reopen lossy");
    assert!(
        matches!(
            back.primary().format(),
            PixelFormat::Yuv420P | PixelFormat::Yuva420P
        ),
        "{:?}",
        back.primary().format()
    );
    // Animation: three frames with distinct delays, lossless.
    let delays = [100u64, 200, 50];
    let frames: Vec<Image> = delays
        .iter()
        .enumerate()
        .map(|(i, d)| {
            Image::from_rgba8(w, h, quad_rgba(w, h, i as u32))
                .unwrap()
                .with_delay(Some(ms(*d)))
        })
        .collect();
    let anim = encode_frames(&ctx, &frames, "webp", &vp8l).expect("anim");
    assert_eq!(
        oxideav_webp::info(&anim).unwrap().frames,
        3,
        "Layer 1 sees 3"
    );
    let back = decode_bytes(&ctx, &anim).expect("reopen anim");
    assert_eq!(back.len(), 3);
    for (i, img) in back.frames().iter().enumerate() {
        assert_eq!(img.delay(), Some(ms(delays[i])), "frame {i}: delay");
        assert_eq!(
            img.to_rgba8().unwrap(),
            quad_rgba(w, h, i as u32),
            "frame {i}: pixels"
        );
    }
    let l1 = oxideav_webp::decode_all(&anim).unwrap();
    for (i, f) in l1.iter().enumerate() {
        assert_eq!(f.delay, Some(ms(delays[i])), "Layer 1 frame {i}: delay");
    }
}

// ---------------------------------------------------------------------------
// QOI — vendored fixtures.
// ---------------------------------------------------------------------------

#[test]
fn qoi_fixtures_open_with_a_colour_signal_and_match_layer_1() {
    let ctx = meta_ctx();
    for rel in [
        "tests/fixtures/testcard.qoi",
        "tests/fixtures/testcard_rgba.qoi",
        "tests/fixtures/edgecase.qoi",
    ] {
        let path = crate_fixture("oxideav-qoi", rel);
        let bytes = std::fs::read(&path).unwrap();
        let info = oxideav_qoi::info(&bytes).unwrap();
        let file = open(&ctx, &path).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert_eq!(file.container(), "qoi", "{rel}");
        let img = file.primary();
        same_layout(rel, info.format, img.format());
        assert!(img.color_signal().is_some(), "{rel}: colour signal stamped");
        assert_eq!(
            img.to_rgba8().unwrap(),
            oxideav_qoi::decode_rgba8(&bytes).unwrap().data,
            "{rel}: rgba vs Layer 1"
        );
        assert_eq!(
            native_bytes(img),
            oxideav_qoi::decode(&bytes).unwrap().into_raw(),
            "{rel}: native vs Layer 1"
        );
    }
    // Round trip both layouts.
    let rgba = gradient_rgba(9, 7);
    let src = Image::from_rgba8(9, 7, rgba.clone()).unwrap();
    for (layout, want) in [
        (PixelFormat::Rgba, rgba.clone()),
        (PixelFormat::Rgb24, opaque(&rgba)),
    ] {
        let input = src.to_format(layout).unwrap();
        let bytes = encode(&ctx, &input, "qoi", &SaveOptions::default()).expect("qoi");
        let back = decode_bytes(&ctx, &bytes).expect("reopen qoi");
        assert_eq!(back.container(), "qoi");
        assert_eq!(back.primary().format(), layout, "{layout:?}");
        assert_eq!(back.primary().to_rgba8().unwrap(), want, "{layout:?}");
    }
}

// ---------------------------------------------------------------------------
// OpenEXR — synthesised through Layer 1.
// ---------------------------------------------------------------------------

fn exr_samples(w: u32, h: u32, comps: usize, seed: f32) -> Vec<f32> {
    (0..(w * h) as usize * comps)
        .map(|i| ((i as f32) * 0.37 + seed).sin() * 2.0 + 0.5)
        .collect()
}

#[test]
fn openexr_single_and_multi_part_files_through_the_gateway() {
    let ctx = meta_ctx();
    let (w, h) = (6u32, 4u32);
    // Single part, RGB float.
    let img = oxideav_openexr::ExrImage::from_f32(
        w,
        h,
        oxideav_openexr::ExrPixelFormat::RgbF32Le,
        &exr_samples(w, h, 3, 0.0),
    )
    .unwrap();
    let bytes =
        oxideav_openexr::encode(&img, &oxideav_openexr::EncodeOptions::default()).expect("exr");
    let info = oxideav_openexr::info(&bytes).unwrap();
    let file = decode_hint(&ctx, &bytes, "exr");
    assert_eq!(file.container(), "openexr");
    assert_eq!(file.len(), 1);
    let g = file.primary();
    same_layout("exr", info.format, g.format());
    assert_eq!(g.format(), PixelFormat::RgbF32Le);
    assert_eq!(
        g.to_packed(PixelFormat::RgbF32Le).unwrap(),
        oxideav_openexr::decode(&bytes).unwrap().into_raw(),
        "float view byte-exact vs Layer 1"
    );
    assert_eq!(g.delay(), None);
    assert_eq!(g.time_base(), tb(1, 1), "untimed");
    // Multi-part: two viewable parts → two pictures, untimed. (Both parts
    // share the layout: the demuxer declares one stream labelled with the
    // first part's layout, so a part with another channel set would be
    // mislabelled — openexr followup.)
    let parts: Vec<oxideav_openexr::Frame> = (0..2u32)
        .map(|i| {
            let img = oxideav_openexr::ExrImage::from_f32(
                w,
                h,
                oxideav_openexr::ExrPixelFormat::RgbF32Le,
                &exr_samples(w, h, 3, i as f32),
            )
            .unwrap();
            oxideav_openexr::Frame::new(img, i)
        })
        .collect();
    let multi = oxideav_openexr::encode_all(&parts, &oxideav_openexr::EncodeOptions::default())
        .expect("multi-part");
    let oracle = oxideav_openexr::decode_all(&multi).unwrap();
    assert_eq!(oracle.len(), 2);
    let file = decode_hint(&ctx, &multi, "exr");
    assert_eq!(file.container(), "openexr");
    assert_eq!(file.len(), oracle.len(), "one picture per viewable part");
    for (i, (g, want)) in file.frames().iter().zip(&oracle).enumerate() {
        same_layout(&format!("part {i}"), want.image.format(), g.format());
        assert_eq!(g.delay(), None, "part {i}: untimed");
        assert_eq!(g.time_base(), tb(1, 1), "part {i}");
        assert_eq!(
            g.to_packed(g.format()).unwrap(),
            want.image.clone().into_raw(),
            "part {i}: float bytes vs Layer 1"
        );
    }
    // Gateway round trip of a float picture (ZIP is lossless on FLOAT).
    let raw: Vec<u8> = exr_samples(w, h, 4, 2.0)
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let src = Image::from_raw(w, h, PixelFormat::RgbaF32Le, raw.clone()).unwrap();
    let bytes = encode(&ctx, &src, "exr", &SaveOptions::default()).expect("encode exr");
    let back = decode_bytes(&ctx, &bytes).expect("reopen exr");
    assert_eq!(back.container(), "openexr");
    assert_eq!(back.primary().format(), PixelFormat::RgbaF32Le);
    assert_eq!(
        back.primary().to_packed(PixelFormat::RgbaF32Le).unwrap(),
        raw,
        "float round trip exact"
    );
    // encode_frames → multi-part file → same part count.
    let two = [src.clone(), src.clone()];
    let bytes = encode_frames(&ctx, &two, "exr", &SaveOptions::default()).expect("parts");
    let back = decode_bytes(&ctx, &bytes).expect("reopen parts");
    assert_eq!(back.len(), 2, "two parts");
    assert_eq!(oxideav_openexr::decode_all(&bytes).unwrap().len(), 2);
    assert!(back.frames().iter().all(|g| g.delay().is_none()));
}

// ---------------------------------------------------------------------------
// PICT — vendored fixture (structural probe, `Rgba`).
// ---------------------------------------------------------------------------

#[test]
fn pict_fixture_opens_by_structure_and_matches_layer_1() {
    let ctx = meta_ctx();
    let path = crate_fixture(
        "oxideav-pict",
        "tests/fixtures/imagemagick_gradient_64x48.pict",
    );
    let bytes = std::fs::read(&path).unwrap();
    let info = oxideav_pict::info(&bytes).unwrap();
    let file = open(&ctx, &path).expect("open pict");
    assert_eq!(file.container(), "pict");
    let img = file.primary();
    same_layout("pict", info.format, img.format());
    assert_eq!(img.format(), PixelFormat::Rgba);
    assert_eq!((img.width(), img.height()), (64, 48));
    assert_eq!(
        img.to_rgba8().unwrap(),
        oxideav_pict::decode_rgba8(&bytes).unwrap().data,
        "pict: rgba vs Layer 1"
    );
    // No extension hint: the structural probe alone finds it.
    let bare = decode_bytes(&ctx, &bytes).expect("probe without hint");
    assert_eq!(bare.container(), "pict");
    // Round trip.
    let rgba = gradient_rgba(9, 7);
    let src = Image::from_rgba8(9, 7, rgba.clone()).unwrap();
    let out = encode(&ctx, &src, "pict", &SaveOptions::default()).expect("encode pict");
    let back = decode_hint(&ctx, &out, "pict");
    assert_eq!(back.container(), "pict");
    // QuickDraw stores no alpha: the picture comes back opaque (documented).
    assert_eq!(
        back.primary().to_rgba8().unwrap(),
        opaque(&rgba),
        "pict round trip"
    );
}

// ---------------------------------------------------------------------------
// ICER — synthesised through Layer 1 (2-D stream and ICER-3D cube).
// ---------------------------------------------------------------------------

fn gray_band(w: u32, h: u32, band: u32) -> Vec<u8> {
    (0..h)
        .flat_map(|y| (0..w).map(move |x| ((x * 29 + y * 13 + band * 71) % 251) as u8))
        .collect()
}

fn icer_gray(w: u32, h: u32, band: u32) -> oxideav_icer::IcerImage {
    oxideav_icer::IcerImage::new(
        w,
        h,
        oxideav_icer::IcerPixelFormat::Gray8,
        vec![oxideav_icer::Plane::new(w as usize, gray_band(w, h, band))],
    )
    .unwrap()
}

#[test]
fn icer_stream_and_cube_through_the_gateway() {
    let ctx = meta_ctx();
    let (w, h) = (16u32, 12u32);
    let opts = oxideav_icer::EncodeOptions::default();
    // 2-D stream.
    let bytes = oxideav_icer::encode(&icer_gray(w, h, 0), &opts).expect("icer");
    let info = oxideav_icer::info(&bytes).unwrap();
    let file = decode_hint(&ctx, &bytes, "icer");
    assert_eq!(file.container(), "icer");
    assert_eq!(file.len(), 1);
    let g = file.primary();
    same_layout("icer 2d", info.format, g.format());
    assert_eq!(g.format(), PixelFormat::Gray8);
    assert_eq!(
        native_bytes(g),
        oxideav_icer::decode(&bytes).unwrap().into_raw(),
        "icer 2d: planes vs Layer 1"
    );
    assert_eq!(
        native_bytes(g),
        gray_band(w, h, 0),
        "default encode is lossless"
    );
    assert_eq!(g.delay(), None);
    // ICER-3D cube: one packet, one picture per band.
    let bands: Vec<oxideav_icer::Frame> = (0..3u32)
        .map(|i| oxideav_icer::Frame::new(icer_gray(w, h, i), i))
        .collect();
    let cube = oxideav_icer::encode_all(&bands, &opts).expect("cube");
    let oracle = oxideav_icer::decode_all(&cube).expect("decode_all cube");
    assert_eq!(oracle.len(), 3);
    let file = decode_hint(&ctx, &cube, "icer");
    assert_eq!(file.container(), "icer");
    assert_eq!(file.len(), oracle.len(), "one picture per band");
    for (i, (g, want)) in file.frames().iter().zip(&oracle).enumerate() {
        assert_eq!(g.format(), PixelFormat::Gray8, "band {i}");
        assert_eq!(g.delay(), None, "band {i}: untimed");
        assert_eq!(g.time_base(), tb(1, 1), "band {i}");
        assert_eq!(
            native_bytes(g),
            want.image.clone().into_raw(),
            "band {i}: vs Layer 1"
        );
        assert_eq!(
            native_bytes(g),
            gray_band(w, h, i as u32),
            "band {i}: exact"
        );
    }
    // Gateway round trip: a still and a three-band cube.
    let src = Image::from_raw(w, h, PixelFormat::Gray8, gray_band(w, h, 5)).unwrap();
    let out = encode(&ctx, &src, "icer", &SaveOptions::default()).expect("encode icer");
    let back = decode_hint(&ctx, &out, "icer");
    assert_eq!(back.primary().format(), PixelFormat::Gray8);
    assert_eq!(
        native_bytes(back.primary()),
        gray_band(w, h, 5),
        "still round trip"
    );
    let frames: Vec<Image> = (0..3u32)
        .map(|i| Image::from_raw(w, h, PixelFormat::Gray8, gray_band(w, h, 10 + i)).unwrap())
        .collect();
    let out = encode_frames(&ctx, &frames, "icer", &SaveOptions::default()).expect("cube mux");
    let back = decode_hint(&ctx, &out, "icer");
    assert_eq!(back.len(), 3, "cube round trip: band count");
    for (i, g) in back.frames().iter().enumerate() {
        assert_eq!(g.delay(), None, "band {i}");
        assert_eq!(
            native_bytes(g),
            gray_band(w, h, 10 + i as u32),
            "band {i}: pixels"
        );
    }
}

// ---------------------------------------------------------------------------
// JPEG 2000 — vendored bare codestreams + a synthesised JP2 wrapper.
// ---------------------------------------------------------------------------

#[test]
fn jpeg2000_codestream_and_jp2_wrapper_through_the_gateway() {
    let ctx = meta_ctx();
    // Bare codestream (`.j2c`) → the `jpeg2000` container.
    let path = crate_fixture("oxideav-jpeg2000", "tests/fixtures/ht_rgb24_rev.j2c");
    let bytes = std::fs::read(&path).unwrap();
    let info = oxideav_jpeg2000::info(&bytes).unwrap();
    let file = open(&ctx, &path).expect("open j2c");
    assert_eq!(file.container(), "jpeg2000");
    let img = file.primary();
    same_layout("j2c", info.format, img.format());
    assert_eq!(img.format(), PixelFormat::Rgb24);
    let rgba = img.to_rgba8().unwrap();
    assert_eq!(
        rgba,
        oxideav_jpeg2000::decode_rgba8(&bytes).unwrap().data,
        "j2c: rgba vs Layer 1"
    );
    // The reversible fixture's PPM reference decodes to the same pixels.
    let reference = open(
        &ctx,
        crate_fixture("oxideav-jpeg2000", "tests/fixtures/ht_rgb24_rev_ref.ppm"),
    )
    .expect("ppm");
    assert_eq!(
        reference.primary().to_rgba8().unwrap(),
        rgba,
        "j2c vs ppm reference"
    );
    // JP2 wrapper synthesised by Layer 1 (its default container).
    let (w, h) = (9u32, 7u32);
    let src_rgba = gradient_rgba(w, h);
    let jp2 = oxideav_jpeg2000::encode_rgb8(
        w,
        h,
        &rgb_of(&src_rgba),
        &oxideav_jpeg2000::EncodeOptions::default(),
    )
    .expect("jp2");
    assert_eq!(&jp2[4..8], b"jP  ", "JP2 signature box");
    let info = oxideav_jpeg2000::info(&jp2).unwrap();
    let file = decode_bytes(&ctx, &jp2).expect("open jp2");
    assert_eq!(file.container(), "jp2");
    same_layout("jp2", info.format, file.primary().format());
    assert_eq!(
        file.primary().to_rgba8().unwrap(),
        oxideav_jpeg2000::decode_rgba8(&jp2).unwrap().data,
        "jp2: rgba vs Layer 1"
    );
    assert_eq!(
        file.primary().to_rgba8().unwrap(),
        opaque(&src_rgba),
        "jp2: lossless"
    );
    // Gateway round trips: both muxers, reversible by default. The `jp2`
    // container's payload codec is `jpeg2000`, which the gateway's
    // default-codec table does not know yet (oxideav-image followup), so
    // the codec is named explicitly.
    let src = Image::from_rgb8(w, h, rgb_of(&src_rgba)).unwrap();
    for (format, container) in [
        ("jp2", "jp2"),
        ("j2k", "jpeg2000"),
        ("jpeg2000", "jpeg2000"),
    ] {
        let out = encode(
            &ctx,
            &src,
            format,
            &SaveOptions::new().with_codec("jpeg2000"),
        )
        .unwrap_or_else(|e| panic!("{format}: encode: {e}"));
        let back = decode_hint(&ctx, &out, format);
        assert_eq!(back.container(), container, "{format}");
        assert_eq!(
            back.primary().to_rgba8().unwrap(),
            opaque(&src_rgba),
            "{format}: round trip exact"
        );
    }
}

// ---------------------------------------------------------------------------
// JPEG XS — synthesised through Layer 1 (bare codestream and `.jxs` boxes).
// ---------------------------------------------------------------------------

#[test]
fn jpegxs_codestream_and_box_file_through_the_gateway() {
    let ctx = meta_ctx();
    let (w, h) = (16u32, 8u32);
    let src_rgba = gradient_rgba(w, h);
    let rgb = rgb_of(&src_rgba);
    for (boxed, container, ext) in [(false, "jpegxs", "jpegxs"), (true, "jxs", "jxs")] {
        let opts = oxideav_jpegxs::EncodeOptions::default().with_boxed(boxed);
        let bytes = oxideav_jpegxs::encode_rgb8(w, h, &rgb, &opts).expect("jxs encode");
        let info = oxideav_jpegxs::info(&bytes).unwrap();
        let file = decode_hint(&ctx, &bytes, ext);
        assert_eq!(file.container(), container, "boxed={boxed}");
        let img = file.primary();
        same_layout(container, info.format, img.format());
        assert_eq!(
            native_bytes(img),
            oxideav_jpegxs::decode(&bytes).unwrap().into_raw(),
            "{container}: native vs Layer 1"
        );
        assert_eq!(
            img.to_rgba8().unwrap(),
            oxideav_jpegxs::decode_rgba8(&bytes).unwrap().data,
            "{container}: rgba vs Layer 1"
        );
        assert_eq!(
            img.to_rgba8().unwrap(),
            opaque(&src_rgba),
            "{container}: lossless"
        );
        assert_eq!(img.delay(), None);
    }
    // Gateway round trips through both muxers (`jxs` carries the
    // `jpegxs` codec; the gateway's default-codec table does not know
    // that yet — oxideav-image followup — so the codec is named).
    let src = Image::from_rgb8(w, h, rgb.clone()).unwrap();
    for (format, container) in [("jxs", "jxs"), ("jpegxs", "jpegxs")] {
        let out = encode(&ctx, &src, format, &SaveOptions::new().with_codec("jpegxs"))
            .unwrap_or_else(|e| panic!("{format}: encode: {e}"));
        let back = decode_hint(&ctx, &out, format);
        assert_eq!(back.container(), container, "{format}");
        assert_eq!(
            back.primary().to_rgba8().unwrap(),
            opaque(&src_rgba),
            "{format}: round trip exact"
        );
    }
}

// ---------------------------------------------------------------------------
// JPEG XL — vendored fixtures (demuxer only).
// ---------------------------------------------------------------------------

#[test]
fn jpegxl_stills_and_animation_through_the_gateway() {
    let ctx = meta_ctx();
    for rel in [
        "tests/fixtures/alpha_64x64.jxl",
        "tests/fixtures/palette_32x32.jxl",
        "tests/fixtures/gray_64x64_lossless.jxl",
        "tests/fixtures/pixel_1x1.jxl",
    ] {
        let path = crate_fixture("oxideav-jpegxl", rel);
        let bytes = std::fs::read(&path).unwrap();
        let info = oxideav_jpegxl::info(&bytes).unwrap();
        let file = open(&ctx, &path).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert_eq!(file.container(), "jpegxl", "{rel}");
        assert_eq!(file.len(), 1, "{rel}");
        let img = file.primary();
        same_layout(rel, info.format, img.format());
        assert_eq!(
            (img.width(), img.height()),
            (info.width, info.height),
            "{rel}"
        );
        assert_eq!(
            img.to_rgba8().unwrap(),
            oxideav_jpegxl::decode_rgba8(&bytes).unwrap().data,
            "{rel}: rgba vs Layer 1"
        );
        assert_eq!(
            native_bytes(img),
            oxideav_jpegxl::decode(&bytes).unwrap().into_raw(),
            "{rel}: native vs Layer 1"
        );
        assert_eq!(img.delay(), None, "{rel}: still");
    }
    // Animation: one picture per presented frame, the file's own tick.
    let path = crate_fixture("oxideav-jpegxl", "tests/fixtures/animation_3frame.jxl");
    let bytes = std::fs::read(&path).unwrap();
    let oracle = oxideav_jpegxl::decode_all(&bytes).expect("decode_all");
    assert_eq!(oracle.len(), 3);
    let file = open(&ctx, &path).expect("open animation");
    assert_eq!(file.container(), "jpegxl");
    assert_eq!(file.len(), oracle.len(), "frame count vs Layer 1");
    let mut t = Duration::ZERO;
    for (i, (img, want)) in file.frames().iter().zip(&oracle).enumerate() {
        assert_eq!(img.delay(), want.delay, "frame {i}: delay vs Layer 1");
        assert!(img.delay().is_some(), "frame {i}: timed");
        let (_, dur) = img.raw_timing();
        assert_eq!(dur, Some(i64::from(want.ticks)), "frame {i}: ticks");
        assert_eq!(img.timestamp(), Some(t), "frame {i}: timestamp");
        t += img.delay().unwrap();
        assert_eq!(
            img.to_rgba8().unwrap(),
            want.image.to_rgba8(),
            "frame {i}: pixels vs Layer 1"
        );
    }
    // Decoder-only crate: the gateway cannot write it, and says so
    // without panicking.
    let src = Image::from_rgba8(4, 4, gradient_rgba(4, 4)).unwrap();
    assert!(encode(&ctx, &src, "jxl", &SaveOptions::default()).is_err());
}

// ---------------------------------------------------------------------------
// HEIF — burst items through the demuxer; lossless RGB round trip.
// ---------------------------------------------------------------------------

#[test]
fn heif_burst_items_are_untimed_pictures_matching_decode_all() {
    let ctx = meta_ctx();
    let path = heif_fixtures_root().join("corpus/multi-image-burst-3/input.heic");
    let bytes = std::fs::read(&path).unwrap();
    let oracle = oxideav_heif::decode_all(&bytes).expect("decode_all");
    assert_eq!(oracle.len(), 3, "Layer 1 burst count");
    let file = open(&ctx, &path).expect("open burst");
    assert_eq!(file.container(), "heif");
    assert_eq!(file.len(), 3, "three burst items through the demuxer");
    for (i, (img, want)) in file.frames().iter().zip(&oracle).enumerate() {
        assert_eq!(img.delay(), None, "item {i}: untimed");
        assert_eq!(want.delay, None, "item {i}: Layer 1 untimed");
        assert_eq!(img.time_base(), tb(1, 1), "item {i}: 1/1 convention");
        assert_eq!(
            (img.width(), img.height()),
            (want.image.width(), want.image.height()),
            "item {i}: geometry"
        );
        // heif's Layer 1 layout carries the range in `color`, the core
        // name folds it in (`YuvJ*`): map through the crate's own bridge.
        assert_eq!(
            want.image.format().to_core_labelled(&want.image.color),
            img.format(),
            "item {i}: Layer 1 layout vs gateway layout"
        );
        assert_eq!(
            native_bytes(img),
            want.image.clone().into_raw(),
            "item {i}: native planes vs Layer 1"
        );
        // The per-item oracle renders (colour within rounding).
        let png = open(&ctx, path.with_file_name(format!("expected_{i}.png"))).expect("png");
        let (mean, max) = diff(
            &rgb_of(&img.to_rgba8().unwrap()),
            &rgb_of(&png.primary().to_rgba8().unwrap()),
        );
        assert!(
            mean <= 1.5 && max <= 12,
            "item {i}: mean={mean:.3} max={max}"
        );
    }
}

#[test]
fn heif_lossless_rgb_round_trip_is_identity_gbrp() {
    let ctx = meta_ctx();
    let schema = oxideav_image::encoder_options(&ctx, "heic", &SaveOptions::default()).unwrap();
    assert!(
        schema.iter().any(|f| f.name == "mode"),
        "heif encoder declares `mode`"
    );
    let (w, h) = (9u32, 7u32);
    let rgba = gradient_rgba(w, h);
    let src = Image::from_rgb8(w, h, rgb_of(&rgba)).unwrap();
    let bytes = encode(
        &ctx,
        &src,
        "heic",
        &SaveOptions::new().with_option("mode", "pcm"),
    )
    .expect("heic pcm");
    let back = decode_bytes(&ctx, &bytes).expect("reopen");
    assert_eq!(back.container(), "heif");
    let img = back.primary();
    assert_eq!(
        img.format(),
        PixelFormat::Gbrp8,
        "identity-matrix 4:4:4 item"
    );
    assert_eq!(img.to_rgba8().unwrap(), opaque(&rgba), "0 LSB round trip");
    // Layer 1 reads the same file to the same pixels.
    assert_eq!(
        oxideav_heif::decode_rgba8(&bytes).unwrap().data,
        opaque(&rgba)
    );
}

// ---------------------------------------------------------------------------
// JPEG — `quality` schema published; Adobe RGB labelled `Rgb24`.
// ---------------------------------------------------------------------------

#[test]
fn jpeg_quality_schema_and_adobe_rgb_label() {
    let ctx = meta_ctx();
    let schema = oxideav_image::encoder_options(&ctx, "jpg", &SaveOptions::default()).unwrap();
    assert!(
        schema.iter().any(|f| f.name == "quality"),
        "mjpeg publishes `quality`: {:?}",
        schema.iter().map(|f| f.name).collect::<Vec<_>>()
    );
    let img = Image::from_rgba8(32, 32, gradient_rgba(32, 32)).unwrap();
    let lo = encode(&ctx, &img, "jpg", &SaveOptions::new().with_quality(10)).expect("q10");
    let hi = encode(&ctx, &img, "jpg", &SaveOptions::new().with_quality(95)).expect("q95");
    assert_ne!(lo, hi, "SaveOptions::quality reaches the encoder");
    let want = rgb_of(&img.to_rgba8().unwrap());
    let err = |b: &[u8]| {
        diff(
            &rgb_of(&decode_bytes(&ctx, b).unwrap().primary().to_rgba8().unwrap()),
            &want,
        )
        .0
    };
    assert!(err(&hi) < err(&lo), "q95 closer than q10");
    // Adobe APP14 transform 0: RGB samples, labelled Rgb24 by the demuxer.
    let (w, h) = (16u32, 8u32);
    let rgba = gradient_rgba(w, h);
    let opts = oxideav_mjpeg::EncodeOptions::default()
        .with_signalling(oxideav_mjpeg::ColorSignalling::Rgb)
        .with_quality(100);
    let bytes = oxideav_mjpeg::encode_rgb8(w, h, &rgb_of(&rgba), &opts).expect("adobe rgb");
    let info = oxideav_mjpeg::info(&bytes).unwrap();
    same_layout("adobe", info.format, PixelFormat::Rgb24);
    let file = decode_bytes(&ctx, &bytes).expect("open adobe rgb jpeg");
    assert_eq!(file.container(), "jpeg");
    let img = file.primary();
    assert_eq!(
        img.format(),
        PixelFormat::Rgb24,
        "demuxer labels what the decoder emits"
    );
    assert_eq!(
        img.to_rgba8().unwrap(),
        oxideav_mjpeg::decode_rgba8(&bytes).unwrap().data,
        "adobe rgb: rgba vs Layer 1"
    );
    let (mean, max) = diff(&img.to_rgba8().unwrap(), &opaque(&rgba));
    assert!(
        mean <= 2.0 && max <= 16,
        "q100 RGB close to source: mean={mean:.3} max={max}"
    );
}

// ---------------------------------------------------------------------------
// APNG — the muxer honours the stream time base.
// ---------------------------------------------------------------------------

#[test]
fn apng_muxer_honours_a_millisecond_time_base() {
    let ctx = meta_ctx();
    let delays = [100u64, 200, 50, 33];
    let frames: Vec<Image> = delays
        .iter()
        .enumerate()
        .map(|(i, d)| {
            Image::from_rgba8(6, 4, quad_rgba(6, 4, i as u32))
                .unwrap()
                .with_delay(Some(ms(*d)))
        })
        .collect();
    for tb_opt in [None, Some(tb(1, 1000))] {
        let mut opts = SaveOptions::default();
        if let Some(t) = tb_opt {
            opts = opts.with_time_base(t);
        }
        // The APNG default tick is 1/100 s, so 33 ms lands on 30 ms there;
        // a millisecond stream time base keeps it.
        let want_delay = |d: u64| {
            if tb_opt.is_some() {
                ms(d)
            } else {
                ms(d / 10 * 10)
            }
        };
        let bytes = encode_frames(&ctx, &frames, "png", &opts).expect("apng");
        let oracle = oxideav_png::decode_all(&bytes).expect("Layer 1 decode_all");
        assert_eq!(oracle.len(), 4, "{tb_opt:?}");
        let back = decode_bytes(&ctx, &bytes).expect("reopen");
        assert_eq!(back.container(), "png");
        assert_eq!(back.len(), 4, "{tb_opt:?}");
        for (i, (img, want)) in back.frames().iter().zip(&oracle).enumerate() {
            assert_eq!(
                img.delay(),
                Some(want_delay(delays[i])),
                "{tb_opt:?} frame {i}: delay"
            );
            assert_eq!(
                want.delay,
                Some(want_delay(delays[i])),
                "{tb_opt:?} frame {i}: Layer 1"
            );
            assert_eq!(
                img.to_rgba8().unwrap(),
                want.image.to_rgba8(),
                "{tb_opt:?} frame {i}: pixels vs Layer 1"
            );
            assert_eq!(img.to_rgba8().unwrap(), quad_rgba(6, 4, i as u32));
        }
    }
}

// ---------------------------------------------------------------------------
// Probe negatives: every fixture under every other format's hint.
// ---------------------------------------------------------------------------

/// Every fixture the round-472 suite reads, plus the synthesised ones.
fn all_fixture_bytes() -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = [
        ("oxideav-webp", "tests/data/lossless-32x32-rgba.webp"),
        ("oxideav-webp", "tests/data/lossy-1x1.webp"),
        ("oxideav-webp", "tests/data/animated-3-frames-rgb.webp"),
        ("oxideav-qoi", "tests/fixtures/edgecase.qoi"),
        (
            "oxideav-pict",
            "tests/fixtures/imagemagick_gradient_64x48.pict",
        ),
        ("oxideav-jpeg2000", "tests/fixtures/ht_rgb24_rev.j2c"),
        ("oxideav-jpeg2000", "tests/fixtures/ht_rgb24_rev_ref.ppm"),
        ("oxideav-jpegxl", "tests/fixtures/alpha_64x64.jxl"),
        ("oxideav-jpegxl", "tests/fixtures/animation_3frame.jxl"),
        ("oxideav-jpegxl", "tests/fixtures/pixel_1x1.jxl"),
        ("oxideav-mjpeg", "tests/fixtures/sampling/samp_4x2.jpg"),
        (
            "oxideav-mjpeg",
            "tests/fixtures/noninterleaved/seqni37_2x1_y.pgm",
        ),
        ("oxideav-hdr", "tests/fixtures/gradient_32x16_newrle.hdr"),
        ("oxideav-dds", "tests/fixtures/grad8.dds"),
        ("oxideav-avif", "tests/fixtures/red.avif"),
    ]
    .iter()
    .filter_map(|(c, rel)| {
        let p = crate_fixture(c, rel);
        std::fs::read(&p).ok().map(|b| (rel.to_string(), b))
    })
    .collect();
    v.push((
        "multi-image-burst-3/input.heic".into(),
        std::fs::read(heif_fixtures_root().join("corpus/multi-image-burst-3/input.heic")).unwrap(),
    ));
    v.push(("synth.gif".into(), gif_animation_bytes().0));
    let exr = oxideav_openexr::ExrImage::from_f32(
        4,
        3,
        oxideav_openexr::ExrPixelFormat::GrayF32Le,
        &exr_samples(4, 3, 1, 0.5),
    )
    .unwrap();
    v.push((
        "synth.exr".into(),
        oxideav_openexr::encode(&exr, &oxideav_openexr::EncodeOptions::default()).unwrap(),
    ));
    v.push((
        "synth.icer".into(),
        oxideav_icer::encode(&icer_gray(8, 8, 0), &oxideav_icer::EncodeOptions::default()).unwrap(),
    ));
    let rgb = rgb_of(&gradient_rgba(8, 8));
    v.push((
        "synth.jxs".into(),
        oxideav_jpegxs::encode_rgb8(
            8,
            8,
            &rgb,
            &oxideav_jpegxs::EncodeOptions::default().with_boxed(true),
        )
        .unwrap(),
    ));
    v.push((
        "synth.jp2".into(),
        oxideav_jpeg2000::encode_rgb8(8, 8, &rgb, &oxideav_jpeg2000::EncodeOptions::default())
            .unwrap(),
    ));
    v
}

#[test]
fn foreign_fixtures_under_every_hint_never_panic() {
    let ctx = meta_plus_webp_ctx();
    // WebP pictures are decoded by the in-tree crate alone: with
    // `register_all` first, `first_decoder` still hands webp packets to
    // the linked 0.2.3 decoder, which cannot read a per-ANMF packet.
    let wctx = webp_ctx();
    let hints = [
        "gif", "webp", "qoi", "exr", "pict", "pic", "icer", "j2c", "jp2", "jpegxs", "jxs", "jxl",
        "heic", "jpg", "png", "ppm", "hdr", "dds", "bmp", "tga", "tif",
    ];
    let fixtures = all_fixture_bytes();
    assert!(fixtures.len() >= 15, "fixtures present: {}", fixtures.len());
    let mut unopened: Vec<String> = Vec::new();
    for (name, bytes) in &fixtures {
        // No hint: the content probe alone; then every foreign hint.
        let own = if name.ends_with(".webp") { &wctx } else { &ctx };
        if let Err(e) = decode_bytes(own, bytes) {
            unopened.push(format!("{name}: {e}"));
        }
        for hint in hints {
            let r = decode_bytes_with(&ctx, bytes, &OpenOptions::new().with_ext_hint(hint));
            // Ok or Err — the point is that nothing panics; a wrong hint
            // never selects a container on its own.
            if let Ok(f) = r {
                assert!(
                    !f.is_empty(),
                    "{name} under {hint}: a file always has a picture"
                );
            }
        }
        // Truncated copies must fail cleanly as well.
        for cut in [bytes.len() / 2, bytes.len().min(16), 1] {
            let _ = decode_bytes(&ctx, &bytes[..cut]);
        }
    }
    assert!(
        unopened.is_empty(),
        "every fixture opens without a hint:\n{}",
        unopened.join("\n")
    );
}
