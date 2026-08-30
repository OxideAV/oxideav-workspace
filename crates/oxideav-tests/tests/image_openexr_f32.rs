//! openexr on the core 0.1.35 float family, via `oxideav_meta`
//! (`openexr` feature, `image` preset): the registry encoder accepts
//! `RgbaF32Le` / `RgbF32Le` / `GrayF32Le` packed frames and the
//! registry decoder returns the same bytes (ZIP is lossless on
//! binary32 data); plane sizing agrees with the core geometry helpers.
//! `GbrpF32Le` is not an OpenEXR frame label today (no planar GBR
//! carriage) and is pinned as refused.

use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, PixelFormat, RuntimeContext, VideoFrame, VideoPlane,
};

fn registry() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_meta::register_all(&mut ctx);
    ctx
}

fn params(w: u32, h: u32, fmt: PixelFormat) -> CodecParameters {
    let mut p = CodecParameters::video(CodecId::new("openexr"));
    p.width = Some(w);
    p.height = Some(h);
    p.pixel_format = Some(fmt);
    p
}

fn float_image(w: u32, h: u32, comps: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h) as usize * comps * 4);
    for i in 0..(w * h) as usize {
        for c in 0..comps {
            // Scene-referred: values above 1.0 and tiny values both survive.
            let v = if c == 3 {
                1.0
            } else {
                (i as f32 * 0.37 + c as f32) * 0.01 - 0.5
            };
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

#[test]
fn f32_family_round_trips_byte_exact_via_registry() {
    let ctx = registry();
    let (w, h) = (17u32, 5u32);
    for (fmt, comps) in [
        (PixelFormat::RgbaF32Le, 4usize),
        (PixelFormat::RgbF32Le, 3),
        (PixelFormat::GrayF32Le, 1),
    ] {
        assert!(fmt.is_float());
        let data = float_image(w, h, comps);
        let stride = fmt.plane_row_bytes(0, w).unwrap();
        assert_eq!(stride, w as usize * comps * 4, "{fmt:?}: core stride");
        assert_eq!(
            data.len(),
            fmt.frame_size_bytes(w, h).unwrap(),
            "{fmt:?}: core frame size"
        );

        let mut enc = ctx
            .codecs
            .first_encoder(&params(w, h, fmt))
            .expect("openexr encoder");
        enc.send_frame(&Frame::Video(VideoFrame {
            pts: Some(1),
            planes: vec![VideoPlane {
                stride,
                data: data.clone(),
            }],
        }))
        .unwrap_or_else(|e| panic!("{fmt:?}: send_frame: {e:?}"));
        let pkt = enc.receive_packet().expect("exr packet");
        assert_eq!(
            &pkt.data[..4],
            &[0x76, 0x2f, 0x31, 0x01],
            "{fmt:?}: EXR magic"
        );

        let mut dec = ctx
            .codecs
            .first_decoder(&CodecParameters::video(CodecId::new("openexr")))
            .expect("openexr decoder");
        dec.send_packet(&pkt).expect("send exr");
        let Frame::Video(v) = dec.receive_frame().expect("exr frame") else {
            panic!("expected video");
        };
        assert_eq!(v.planes.len(), 1, "{fmt:?}: packed float plane");
        assert_eq!(
            v.planes[0].stride, stride,
            "{fmt:?}: decoded stride = core row bytes"
        );
        assert_eq!(
            v.planes[0].data, data,
            "{fmt:?}: binary32 samples byte-exact"
        );
        assert!(matches!(
            dec.receive_frame(),
            Err(Error::NeedMore | Error::Eof)
        ));
    }
}

#[test]
fn planar_gbr_f32_is_not_an_exr_frame_label() {
    let ctx = registry();
    let (w, h) = (4u32, 3u32);
    let fmt = PixelFormat::GbrpF32Le;
    let mut enc = ctx
        .codecs
        .first_encoder(&params(w, h, fmt))
        .expect("factory");
    let plane = || VideoPlane {
        stride: w as usize * 4,
        data: vec![0; (w * h * 4) as usize],
    };
    let r = enc.send_frame(&Frame::Video(VideoFrame {
        pts: None,
        planes: vec![plane(), plane(), plane()],
    }));
    assert!(r.is_err(), "GbrpF32Le has no OpenEXR channel mapping today");
    // The capability list names the float family the codec does carry.
    let impls = ctx.codecs.implementations(&CodecId::new("openexr"));
    assert!(!impls.is_empty());
    let accepted = &impls[0].caps.accepted_pixel_formats;
    eprintln!("openexr accepted: {accepted:?}");
}
