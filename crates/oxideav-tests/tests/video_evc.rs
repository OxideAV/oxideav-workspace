//! evc encoder → registry decoder (round 452): B-slice (`b=1`) and
//! multi-reference (`refs`) configurations through the string-keyed
//! `CodecParameters::options`, with no extradata on the decode side
//! (SPS/PPS ride in-band on every IDR access unit).

use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, PixelFormat, RuntimeContext, VideoFrame, VideoPlane,
};
use oxideav_tests::video_y_psnr;

const W: u32 = 64;
const H: u32 = 48;

fn registry() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_evc::register(&mut ctx.codecs);
    ctx
}

fn scene(t: usize) -> Vec<u8> {
    let (w, h) = (W as usize, H as usize);
    let (cw, ch) = (w / 2, h / 2);
    let mut buf = Vec::with_capacity(w * h + 2 * cw * ch);
    for y in 0..h {
        for x in 0..w {
            let v = ((x + t * 2) * 3 + y * 2) % 200 + 20;
            let blob = if (x as i32 - 20 - t as i32 * 3).abs() < 6 && (y as i32 - 24).abs() < 6 {
                60
            } else {
                0
            };
            buf.push((v + blob).min(235) as u8);
        }
    }
    for y in 0..ch {
        for x in 0..cw {
            buf.push(((x * 5 + y * 3 + t) % 120) as u8 + 64);
        }
    }
    for y in 0..ch {
        for x in 0..cw {
            buf.push(((x * 2 + y * 7 + t * 3) % 120) as u8 + 64);
        }
    }
    buf
}

fn video_frame(planar: &[u8], pts: i64) -> Frame {
    let (w, h) = (W as usize, H as usize);
    let (cw, ch) = (w / 2, h / 2);
    Frame::Video(VideoFrame {
        pts: Some(pts),
        planes: vec![
            VideoPlane {
                stride: w,
                data: planar[..w * h].to_vec(),
            },
            VideoPlane {
                stride: cw,
                data: planar[w * h..w * h + cw * ch].to_vec(),
            },
            VideoPlane {
                stride: cw,
                data: planar[w * h + cw * ch..].to_vec(),
            },
        ],
    })
}

fn params(opts: &[(&str, &str)]) -> CodecParameters {
    let mut p = CodecParameters::video(CodecId::new("evc"));
    p.width = Some(W);
    p.height = Some(H);
    p.pixel_format = Some(PixelFormat::Yuv420P);
    for (k, v) in opts {
        p.options.insert(*k, *v);
    }
    p
}

fn y_of(v: &VideoFrame) -> Vec<u8> {
    v.planes[0]
        .data
        .chunks(v.planes[0].stride)
        .take(H as usize)
        .flat_map(|r| r[..W as usize].iter().copied())
        .collect()
}

/// Encode `n` frames under `opts`, decode each packet immediately
/// through the registry decoder (no reordering delay in these
/// low-delay configurations), return (keyframe flags, decoded frames).
fn round_trip(
    ctx: &RuntimeContext,
    opts: &[(&str, &str)],
    n: usize,
) -> (Vec<bool>, Vec<VideoFrame>, usize) {
    let mut enc = ctx
        .codecs
        .first_encoder(&params(opts))
        .expect("evc encoder");
    let mut dec = ctx
        .codecs
        .first_decoder(&CodecParameters::video(CodecId::new("evc")))
        .expect("evc decoder");
    let mut keys = Vec::new();
    let mut frames = Vec::new();
    let mut bytes = 0usize;
    for t in 0..n {
        enc.send_frame(&video_frame(&scene(t), t as i64))
            .expect("send_frame");
        let pkt = enc.receive_packet().expect("packet");
        assert!(
            matches!(enc.receive_packet(), Err(Error::NeedMore)),
            "one packet per frame"
        );
        keys.push(pkt.flags.keyframe);
        bytes += pkt.data.len();
        dec.send_packet(&pkt).expect("send_packet");
        let Frame::Video(v) = dec.receive_frame().expect("frame") else {
            panic!("expected video");
        };
        assert_eq!(v.planes.len(), 3);
        frames.push(v);
    }
    (keys, frames, bytes)
}

#[test]
fn b_slices_and_multi_ref_decode_via_registry() {
    let ctx = registry();
    let n = 9usize;
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("p_single_ref", &[("gop", "6"), ("refs", "1"), ("qp", "24")]),
        ("p_multi_ref", &[("gop", "6"), ("refs", "3"), ("qp", "24")]),
        (
            "b_low_delay",
            &[("gop", "6"), ("refs", "2"), ("b", "1"), ("qp", "24")],
        ),
        (
            "b_baseline_deblock",
            &[
                ("gop", "5"),
                ("refs", "2"),
                ("b", "1"),
                ("qp", "30"),
                ("cm_init", "0"),
                ("deblock", "1"),
            ],
        ),
        (
            "b_max_refs",
            &[("gop", "9"), ("refs", "5"), ("b", "true"), ("qp", "20")],
        ),
    ];
    for (label, opts) in cases {
        let gop: usize = opts
            .iter()
            .find(|(k, _)| *k == "gop")
            .unwrap()
            .1
            .parse()
            .unwrap();
        let (keys, frames, bytes) = round_trip(&ctx, opts, n);
        for (t, k) in keys.iter().enumerate() {
            assert_eq!(*k, t % gop == 0, "{label}: keyframe flag at {t}");
        }
        assert_eq!(frames.len(), n, "{label}: every AU decodes");
        for (t, f) in frames.iter().enumerate() {
            let psnr = video_y_psnr(&y_of(f), &scene(t)[..(W * H) as usize], W, H);
            assert!(psnr > 30.0, "{label} frame {t}: Y-PSNR {psnr:.2} dB");
        }
        eprintln!("{label}: {bytes} B for {n} frames");
    }
}

/// The inter tools do work: at equal QP a multi-ref/B configuration
/// must not be larger than all-intra, and the option validator rejects
/// out-of-range `refs` / `gop` at factory time.
#[test]
fn inter_tools_reduce_size_and_options_are_validated() {
    let ctx = registry();
    let (_, _, intra) = round_trip(&ctx, &[("gop", "1"), ("qp", "24")], 6);
    let (_, _, inter) = round_trip(
        &ctx,
        &[("gop", "6"), ("refs", "3"), ("b", "1"), ("qp", "24")],
        6,
    );
    assert!(
        inter < intra,
        "inter {inter} B should beat all-intra {intra} B"
    );
    for bad in [
        ("refs", "0"),
        ("refs", "6"),
        ("gop", "0"),
        ("qp", "52"),
        ("b", "maybe"),
    ] {
        assert!(
            ctx.codecs.first_encoder(&params(&[bad])).is_err(),
            "{bad:?} must be refused"
        );
    }
    // Odd geometry is refused.
    let mut odd = params(&[]);
    odd.width = Some(63);
    assert!(ctx.codecs.first_encoder(&odd).is_err());
}
