//! jpeg2000 registry `Encoder` driven purely by `CodecParameters`
//! (round 452): packed input formats, the `container` option (bare
//! J2K codestream vs JP2 wrapper), lossy/HT switches — every variant
//! decodes back through the registry decoder, which sniffs the
//! framing itself and needs no parameters.

use oxideav_core::{
    CodecId, CodecOptions, CodecParameters, Error, Frame, PixelFormat, RuntimeContext, VideoFrame,
    VideoPlane,
};

const JP2_SIGNATURE: [u8; 12] = [
    0, 0, 0, 0x0c, b'j', b'P', b' ', b' ', 0x0d, 0x0a, 0x87, 0x0a,
];

fn registry() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_jpeg2000::register(&mut ctx);
    ctx
}

fn image(w: u32, h: u32, ncomp: usize, bytes_per: usize) -> Vec<u8> {
    (0..(w * h) as usize * ncomp * bytes_per)
        .map(|i| ((i * 37 + (i / 7) * 11) % 251) as u8)
        .collect()
}

fn frame(w: u32, data: &[u8], ncomp: usize, bytes_per: usize) -> Frame {
    Frame::Video(VideoFrame {
        pts: Some(7),
        planes: vec![VideoPlane {
            stride: w as usize * ncomp * bytes_per,
            data: data.to_vec(),
        }],
    })
}

fn encode(ctx: &RuntimeContext, params: &CodecParameters, f: &Frame) -> oxideav_core::Packet {
    let mut enc = ctx.codecs.first_encoder(params).expect("jpeg2000 encoder");
    enc.send_frame(f).expect("send_frame");
    let pkt = enc.receive_packet().expect("receive_packet");
    assert!(pkt.flags.keyframe, "intra-only: every packet is a keyframe");
    assert_eq!(pkt.pts, Some(7));
    pkt
}

fn decode(ctx: &RuntimeContext, pkt: &oxideav_core::Packet) -> VideoFrame {
    let mut dec = ctx
        .codecs
        .first_decoder(&CodecParameters::video(CodecId::new("jpeg2000")))
        .expect("jpeg2000 decoder");
    dec.send_packet(pkt).expect("send_packet");
    let Frame::Video(v) = dec.receive_frame().expect("receive_frame") else {
        panic!("expected video");
    };
    dec.flush().expect("flush");
    assert!(matches!(dec.receive_frame(), Err(Error::Eof)));
    v
}

fn params(w: u32, h: u32, fmt: Option<PixelFormat>, opts: CodecOptions) -> CodecParameters {
    let mut p = CodecParameters::video(CodecId::new("jpeg2000"));
    p.width = Some(w);
    p.height = Some(h);
    p.pixel_format = fmt;
    p.options = opts;
    p
}

/// Packed formats: Gray8 / Rgb24 / Rgba / Gray16Le / Rgb48Le round-trip
/// byte-exact under the default lossless (5-3 + RCT) mode, and the
/// decoder re-labels the output with the canonical packed format.
#[test]
fn packed_formats_round_trip_lossless_via_registry() {
    let ctx = registry();
    let (w, h) = (23u32, 11u32);
    let cases = [
        (PixelFormat::Gray8, 1usize, 1usize, PixelFormat::Gray8),
        (PixelFormat::Rgb24, 3, 1, PixelFormat::Rgb24),
        (PixelFormat::Rgba, 4, 1, PixelFormat::Rgba),
        (PixelFormat::Gray16Le, 1, 2, PixelFormat::Gray16Le),
        (PixelFormat::Rgb48Le, 3, 2, PixelFormat::Rgb48Le),
    ];
    for (fmt, ncomp, bp, out_fmt) in cases {
        let data = image(w, h, ncomp, bp);
        let pkt = encode(
            &ctx,
            &params(w, h, Some(fmt), CodecOptions::default()),
            &frame(w, &data, ncomp, bp),
        );
        assert_eq!(
            &pkt.data[..2],
            &[0xFF, 0x4F],
            "{fmt:?}: SOC marker (bare J2K)"
        );
        let v = decode(&ctx, &pkt);
        assert_eq!(v.planes.len(), 1, "{fmt:?}: packed output");
        assert_eq!(v.planes[0].stride, w as usize * ncomp * bp, "{fmt:?}");
        assert_eq!(v.planes[0].data, data, "{fmt:?}: lossless byte-exact");
        assert_eq!(
            out_fmt.plane_row_bytes(0, w),
            Some(v.planes[0].stride),
            "{fmt:?}: core geometry"
        );
    }
    // BGR input is re-ordered on the way in and decodes as RGB.
    let data = image(w, h, 3, 1);
    let pkt = encode(
        &ctx,
        &params(w, h, Some(PixelFormat::Bgr24), CodecOptions::default()),
        &frame(w, &data, 3, 1),
    );
    let v = decode(&ctx, &pkt);
    let swapped: Vec<u8> = data
        .chunks_exact(3)
        .flat_map(|p| [p[2], p[1], p[0]])
        .collect();
    assert_eq!(v.planes[0].data, swapped, "Bgr24 in → Rgb24 out");
    // Unspecified pixel format is inferred from stride / width.
    let pkt = encode(
        &ctx,
        &params(w, h, None, CodecOptions::default()),
        &frame(w, &data, 3, 1),
    );
    assert_eq!(decode(&ctx, &pkt).planes[0].data, data);
    // Planar input is refused (packed formats only).
    let mut enc = ctx
        .codecs
        .first_encoder(&params(
            w,
            h,
            Some(PixelFormat::Rgb24),
            CodecOptions::default(),
        ))
        .unwrap();
    let planar = Frame::Video(VideoFrame {
        pts: None,
        planes: vec![
            VideoPlane {
                stride: w as usize,
                data: vec![0; (w * h) as usize],
            },
            VideoPlane {
                stride: w as usize,
                data: vec![0; (w * h) as usize],
            },
            VideoPlane {
                stride: w as usize,
                data: vec![0; (w * h) as usize],
            },
        ],
    });
    assert!(
        enc.send_frame(&planar).is_err(),
        "planar frames are rejected"
    );
}

/// `container=jp2` wraps the codestream in the JP2 box structure; the
/// registry decoder sniffs it and round-trips identically. `j2k` (the
/// default) stays bare; an unknown container value is refused.
#[test]
fn container_option_selects_jp2_wrapper() {
    let ctx = registry();
    let (w, h) = (16u32, 9u32);
    let data = image(w, h, 3, 1);
    let f = frame(w, &data, 3, 1);
    for (container, ht) in [("jp2", "false"), ("jp2", "true"), ("jph", "true")] {
        let opts = CodecOptions::default()
            .set("container", container)
            .set("ht", ht);
        let pkt = encode(&ctx, &params(w, h, Some(PixelFormat::Rgb24), opts), &f);
        assert_eq!(
            &pkt.data[..12],
            &JP2_SIGNATURE,
            "{container}/ht={ht}: JP2 signature box"
        );
        assert!(
            pkt.data.windows(4).any(|b| b == b"jp2h"),
            "{container}: jp2h header box"
        );
        assert!(
            pkt.data.windows(4).any(|b| b == b"jp2c"),
            "{container}: jp2c codestream box"
        );
        assert_eq!(
            decode(&ctx, &pkt).planes[0].data,
            data,
            "{container}/ht={ht}: exact"
        );
    }
    let bare = encode(
        &ctx,
        &params(
            w,
            h,
            Some(PixelFormat::Rgb24),
            CodecOptions::default().set("container", "j2c"),
        ),
        &f,
    );
    assert_eq!(&bare.data[..2], &[0xFF, 0x4F]);
    assert_eq!(decode(&ctx, &bare).planes[0].data, data);

    // Options are validated when the first frame is encoded (the
    // factory only captures the parameter set): an unknown container
    // value surfaces as a `send_frame` error.
    let bogus = params(
        w,
        h,
        Some(PixelFormat::Rgb24),
        CodecOptions::default().set("container", "tiff"),
    );
    let mut enc = ctx
        .codecs
        .first_encoder(&bogus)
        .expect("factory defers option validation");
    assert!(
        enc.send_frame(&f).is_err(),
        "unknown container is invalid at encode time"
    );
}

/// Lossy 9-7 + ICT under the registry: smaller than lossless, decodes
/// close to the source; `layers` / `levels` / `progression` accepted;
/// a malformed option value is an error at factory time.
#[test]
fn lossy_and_structure_options_via_registry() {
    let ctx = registry();
    let (w, h) = (48u32, 32u32);
    // Smooth content so the lossy path has something to compress.
    let data: Vec<u8> = (0..(w * h) as usize)
        .flat_map(|i| {
            let (x, y) = ((i % w as usize) as u8, (i / w as usize) as u8);
            [x * 5, y * 7, x.wrapping_add(y) * 3]
        })
        .collect();
    let f = frame(w, &data, 3, 1);
    let lossless = encode(
        &ctx,
        &params(w, h, Some(PixelFormat::Rgb24), CodecOptions::default()),
        &f,
    );
    let opts = CodecOptions::default()
        .set("lossless", "false")
        .set("layers", "3")
        .set("levels", "3")
        .set("progression", "rpcl")
        .set("psnr", "38");
    let lossy = encode(&ctx, &params(w, h, Some(PixelFormat::Rgb24), opts), &f);
    let v = decode(&ctx, &lossy);
    assert_eq!(v.planes[0].data.len(), data.len());
    let mse = v.planes[0]
        .data
        .iter()
        .zip(&data)
        .map(|(&a, &b)| (a as f64 - b as f64).powi(2))
        .sum::<f64>()
        / data.len() as f64;
    let psnr = 10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10();
    eprintln!(
        "jpeg2000 lossy: {} B vs lossless {} B, PSNR {psnr:.2} dB",
        lossy.data.len(),
        lossless.data.len()
    );
    assert!(psnr > 34.0, "lossy PSNR {psnr:.2} dB");
    assert!(lossy.data.len() < lossless.data.len(), "lossy is smaller");

    let bad = params(
        w,
        h,
        Some(PixelFormat::Rgb24),
        CodecOptions::default().set("levels", "many"),
    );
    let mut enc = ctx
        .codecs
        .first_encoder(&bad)
        .expect("factory defers option validation");
    assert!(
        enc.send_frame(&f).is_err(),
        "malformed option value fails at encode time"
    );
}
