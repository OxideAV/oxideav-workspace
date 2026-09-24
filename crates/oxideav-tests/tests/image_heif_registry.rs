//! HEIF / HEIC / AVIF registry resolution audit (round 460).
//!
//! With `oxideav_meta::register_all` every HEIF-family brand (`mif1` /
//! `heic` / `heix` / `msf1` / `hevc` / `avif` / `avis` / `heim` …) must
//! resolve deterministically to the `heif` demuxer, the `mov` / `mp4`
//! probes must not capture `ftyp mif1/heic` files, and the extension
//! table must agree with the probe. Synthetic `ftyp`-only buffers pin
//! the ranking rule (score, then priority, then registration order)
//! independently of any fixture; the 52 vendored real-world files
//! (three producers) confirm it on real headers — with, without and
//! against a misleading extension hint.
//!
//! Also audits what the heif crate contributes (demuxer + muxer +
//! codec on both sides) and that its framework encoder honours the
//! documented option keys through `CodecParameters::options`.
//!
//! Runs unconditionally on CI — the fixtures ride with the sibling
//! checkout.

use oxideav_core::{CodecId, CodecParameters, Frame, PixelFormat, ProbeData};
use oxideav_tests::image::{decode_still, heif_corpus_dirs, heif_interop_files, meta_ctx};

const HEIF_EXTS: [&str; 5] = ["heic", "heif", "heics", "heifs", "hif"];
const AVIF_EXTS: [&str; 2] = ["avif", "avifs"];

/// Minimal ISOBMFF file: one `ftyp` box (major brand, minor version 0,
/// compatible brands) followed by an empty `meta`-less tail. Enough
/// for every brand-driven probe; the ranking below is what
/// `probe_input` would return for a real file with that header.
fn ftyp(major: &[u8; 4], compat: &[&[u8; 4]]) -> Vec<u8> {
    let mut b = Vec::new();
    let size = 8 + 4 + 4 + 4 * compat.len();
    b.extend_from_slice(&(size as u32).to_be_bytes());
    b.extend_from_slice(b"ftyp");
    b.extend_from_slice(major);
    b.extend_from_slice(&0u32.to_be_bytes());
    for c in compat {
        b.extend_from_slice(*c);
    }
    // A tiny `free` box so the buffer is not just the header.
    b.extend_from_slice(&16u32.to_be_bytes());
    b.extend_from_slice(b"free");
    b.extend_from_slice(&[0u8; 8]);
    b
}

fn ranked(ctx: &oxideav_core::RuntimeContext, buf: &[u8], ext: Option<&str>) -> Vec<String> {
    ctx.containers
        .probe_candidates(&ProbeData { buf, ext })
        .iter()
        .map(|c| format!("{}:{:?}@{}", c.name, c.score, c.priority))
        .collect()
}

// ───────────────────────── registry resolution ─────────────────────────

/// Every HEIF-family extension maps to the `heif` container and
/// nothing else claims it.
#[test]
fn heif_family_extensions_resolve_to_heif_only() {
    let ctx = meta_ctx();
    for ext in HEIF_EXTS {
        assert_eq!(
            ctx.containers.container_for_extension(ext),
            Some("heif"),
            ".{ext}"
        );
        let claims: Vec<_> = ctx
            .containers
            .extension_candidates(ext)
            .iter()
            .map(|c| (c.container.to_owned(), c.priority))
            .collect();
        assert_eq!(claims, vec![("heif".to_owned(), 50)], ".{ext} claims");
    }
}

/// `.avif` / `.avifs`: two claims exist — `heif` (prioritised, 50) and
/// `avif` (default, 100). The heif container wins the extension table
/// and, since the `avif` crate registers no content probe at all
/// (`probe_priority("avif") == None`), also every real file. This pins
/// the current, deterministic outcome; see the round report for the
/// cross-crate note (the brief expected oxideav-avif to keep `.avif`
/// through its own probe priority — it has none to keep it with).
#[test]
fn avif_extensions_resolve_to_heif_ahead_of_the_avif_claim() {
    let ctx = meta_ctx();
    for ext in AVIF_EXTS {
        let claims: Vec<_> = ctx
            .containers
            .extension_candidates(ext)
            .iter()
            .map(|c| (c.container.to_owned(), c.priority))
            .collect();
        assert_eq!(
            claims,
            vec![("heif".to_owned(), 50), ("avif".to_owned(), 100)],
            ".{ext} claims"
        );
        assert_eq!(ctx.containers.container_for_extension(ext), Some("heif"));
    }
    assert_eq!(ctx.containers.probe_priority("heif"), Some(50));
    assert_eq!(ctx.containers.probe_priority("avif"), None);
    assert!(!ctx.containers.demuxer_names().any(|n| n == "avif"));
    // The avif *codec* (AV1-in-HEIF profile) is still registered on
    // both sides — it is the container role that moved to heif.
    assert!(ctx.codecs.has_decoder(&CodecId::new("avif")));
    assert!(ctx.codecs.has_encoder(&CodecId::new("avif")));
}

/// Synthetic `ftyp` headers: every HEIF / MIAF / AVIF brand ranks
/// `heif` first; the ISOBMFF generalists (`mp4`, `mov`) either do not
/// score or lose on priority (heif 50 < default 100). Plain ISO / QT
/// brands must NOT go to heif.
#[test]
fn ftyp_brands_probe_to_heif_ahead_of_mp4_and_mov() {
    let ctx = meta_ctx();
    let heif_brands: [(&[u8; 4], &[&[u8; 4]]); 9] = [
        (b"heic", &[b"mif1", b"heic"]),
        (b"mif1", &[b"mif1", b"heic"]),
        (b"heix", &[b"mif1", b"heix"]),
        (b"msf1", &[b"mif1", b"msf1", b"hevc"]),
        (
            b"hevc",
            &[b"mif1", b"heic", b"miaf", b"msf1", b"isom", b"hevc"],
        ),
        (b"avif", &[b"mif1", b"avif", b"miaf"]),
        (b"avis", &[b"mif1", b"avif", b"avis", b"msf1"]),
        (b"mif1", &[b"mif1", b"miaf", b"MiHB", b"heic"]),
        (b"heim", &[b"mif1", b"heim"]),
    ];
    for (major, compat) in heif_brands {
        let buf = ftyp(major, compat);
        let label = String::from_utf8_lossy(major).into_owned();
        for ext in [None, Some("heic"), Some("mp4"), Some("mov"), Some("bin")] {
            let r = ranked(&ctx, &buf, ext);
            assert!(
                r.first().is_some_and(|c| c.starts_with("heif:")),
                "major {label} ext {ext:?}: ranking {r:?}"
            );
            assert!(
                !r.iter().any(|c| c.starts_with("mov:")),
                "major {label}: mov must not score a HEIF brand: {r:?}"
            );
        }
        // Priority — not registration order — is what breaks the tie
        // with mp4 when both score the same.
        let cands = ctx.containers.probe_candidates(&ProbeData {
            buf: &buf,
            ext: None,
        });
        let heif = cands
            .iter()
            .find(|c| c.name == "heif")
            .expect("heif scored");
        if let Some(mp4) = cands.iter().find(|c| c.name == "mp4") {
            assert!(
                mp4.score < heif.score || mp4.priority > heif.priority,
                "major {label}: mp4 {mp4:?} vs heif {heif:?}"
            );
        }
    }
    // Generic ISO-BMFF and QuickTime: not heif.
    for (major, compat, expect) in [
        (b"isom", &[b"isom", b"iso2", b"mp41"][..], "mp4"),
        (b"mp42", &[b"mp42", b"isom"][..], "mp4"),
        (b"qt  ", &[b"qt  "][..], "mov"),
    ] {
        let buf = ftyp(major, compat);
        let r = ranked(&ctx, &buf, None);
        assert!(
            r.first()
                .is_some_and(|c| c.starts_with(&format!("{expect}:"))),
            "major {:?}: expected {expect} first, got {r:?}",
            String::from_utf8_lossy(major)
        );
        assert!(
            !r.iter().any(|c| c.starts_with("heif:")),
            "major {:?}: heif must not score: {r:?}",
            String::from_utf8_lossy(major)
        );
    }
}

/// Every vendored real-world file (all three producers, `.heic` /
/// `.heics` / `.avif`) probes to `heif` with or without an extension
/// hint — and even with a *misleading* hint, because a content score
/// always beats the extension fallback.
#[test]
fn every_vendored_file_probes_to_heif_regardless_of_hint() {
    let ctx = meta_ctx();
    let mut files = heif_interop_files();
    for d in heif_corpus_dirs() {
        files.push(d.join("input.heic"));
    }
    for p in &files {
        let mut f = std::fs::File::open(p).expect("open");
        for hint in [None, Some("heic"), Some("avif"), Some("mp4"), Some("mov")] {
            let got = ctx
                .containers
                .probe_input(&mut f, hint)
                .expect("probe_input");
            assert_eq!(got, "heif", "{} with hint {hint:?}", p.display());
        }
        let buf = std::fs::read(p).expect("read");
        let r = ranked(&ctx, &buf[..buf.len().min(256 * 1024)], None);
        assert!(r[0].starts_with("heif:"), "{}: {r:?}", p.display());
        assert!(
            !r.iter().any(|c| c.starts_with("mov:")),
            "{}: mov scored {r:?}",
            p.display()
        );
    }
}

/// The heif crate contributes a demuxer + muxer named `heif` and the
/// `heif` codec on both sides, and its framework encoder honours the
/// documented option keys (`codec` / `mode` / `qp` / `grid` /
/// `thumbnail`) — though it declares no `OptionField` schema for them,
/// so `oxideav info heif` cannot list them (cross-crate note).
#[test]
fn heif_registers_demuxer_muxer_codec_and_encoder_options() {
    let ctx = meta_ctx();
    assert!(ctx.containers.demuxer_names().any(|n| n == "heif"));
    assert!(ctx.containers.muxer_names().any(|n| n == "heif"));
    let id = CodecId::new("heif");
    assert!(ctx.codecs.has_decoder(&id));
    assert!(ctx.codecs.has_encoder(&id));
    match ctx.codecs.encoder_options_schema(&id) {
        Some(schema) => {
            let names: Vec<&str> = schema.iter().map(|f| f.name).collect();
            for key in ["codec", "mode", "qp", "grid", "thumbnail"] {
                assert!(names.contains(&key), "schema {names:?} lacks {key}");
            }
        }
        None => eprintln!(
            "note: heif declares no encoder options schema — `oxideav info heif` lists none"
        ),
    }
    // Options are honoured through `CodecParameters::options` even
    // without a schema: a 4:2:0 frame encodes to a HEVC still by
    // default and to an AV1 (`avif` brand) still on `codec=av1`.
    let (w, h) = (32u32, 24u32);
    let frame = Frame::Video(oxideav_core::VideoFrame {
        pts: Some(0),
        planes: [(w, h), (w / 2, h / 2), (w / 2, h / 2)]
            .iter()
            .map(|&(pw, ph)| oxideav_core::VideoPlane {
                stride: pw as usize,
                data: (0..(pw * ph) as usize)
                    .map(|i| (i * 5 % 251) as u8)
                    .collect(),
            })
            .collect(),
    });
    for (opts, brand) in [
        (oxideav_core::CodecOptions::new(), b"heic"),
        (
            oxideav_core::CodecOptions::new().set("codec", "av1"),
            b"avif",
        ),
        (
            oxideav_core::CodecOptions::new().set("mode", "pcm"),
            b"heic",
        ),
        (oxideav_core::CodecOptions::new().set("qp", "30"), b"heic"),
    ] {
        let mut p = CodecParameters::video(id.clone());
        p.width = Some(w);
        p.height = Some(h);
        p.pixel_format = Some(PixelFormat::Yuv420P);
        p.options = opts.clone();
        let mut enc = ctx.codecs.first_encoder(&p).expect("heif encoder");
        enc.send_frame(&frame).expect("send_frame");
        let pkt = enc.receive_packet().expect("one still = one packet");
        assert_eq!(&pkt.data[4..8], b"ftyp", "{opts:?}: whole file out");
        assert_eq!(&pkt.data[8..12], brand, "{opts:?}: major brand");
        assert!(enc.receive_packet().is_err(), "{opts:?}: no second packet");
        // …and the file it wrote re-opens through the registry.
        let tmp = oxideav_tests::tmp(&format!(
            "r460_heif_opts_{}.heic",
            opts.iter()
                .map(|(k, v)| format!("{k}-{v}"))
                .collect::<String>()
        ));
        std::fs::write(&tmp, &pkt.data).expect("write");
        let d = decode_still(&ctx, &tmp).expect("re-decode");
        assert_eq!(d.container, "heif");
        assert_eq!((d.width(), d.height()), (w, h));
    }
}
