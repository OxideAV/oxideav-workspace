//! End-to-end FFV1 bit-exact round-trip tests via the `Decoder` / `Encoder`
//! trait surfaces.

use oxideav_core::frame::VideoPlane;
use oxideav_core::{
    CodecId, CodecParameters, Frame, MediaType, PixelFormat, Rational, TimeBase, VideoFrame,
};
use oxideav_ffv1::decoder::make_decoder;
use oxideav_ffv1::encoder::make_encoder;

fn make_params(pix: PixelFormat, width: u32, height: u32) -> CodecParameters {
    CodecParameters {
        codec_id: CodecId::new("ffv1"),
        media_type: MediaType::Video,
        sample_rate: None,
        channels: None,
        sample_format: None,
        width: Some(width),
        height: Some(height),
        pixel_format: Some(pix),
        frame_rate: Some(Rational::new(30, 1)),
        extradata: Vec::new(),
        bit_rate: None,
    }
}

fn synth_yuv420(width: u32, height: u32) -> VideoFrame {
    let w = width as usize;
    let h = height as usize;
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);
    // Deterministic pattern, no RNG dependency.
    let mut y = vec![0u8; w * h];
    for j in 0..h {
        for i in 0..w {
            y[j * w + i] = ((i * 7 + j * 11 + 16) & 0xFF) as u8;
        }
    }
    let mut u = vec![0u8; cw * ch];
    let mut v = vec![0u8; cw * ch];
    for j in 0..ch {
        for i in 0..cw {
            u[j * cw + i] = ((i * 19 + j * 3 + 64) & 0xFF) as u8;
            v[j * cw + i] = ((i * 5 + j * 23 + 128) & 0xFF) as u8;
        }
    }
    VideoFrame {
        format: PixelFormat::Yuv420P,
        width,
        height,
        pts: Some(0),
        time_base: TimeBase::new(1, 30),
        planes: vec![
            VideoPlane { stride: w, data: y },
            VideoPlane {
                stride: cw,
                data: u,
            },
            VideoPlane {
                stride: cw,
                data: v,
            },
        ],
    }
}

fn synth_yuv444(width: u32, height: u32) -> VideoFrame {
    let w = width as usize;
    let h = height as usize;
    let mut y = vec![0u8; w * h];
    let mut u = vec![0u8; w * h];
    let mut v = vec![0u8; w * h];
    for j in 0..h {
        for i in 0..w {
            let idx = j * w + i;
            y[idx] = ((i * 3 + j * 7) & 0xFF) as u8;
            u[idx] = ((i * 11 + j * 5 + 50) & 0xFF) as u8;
            v[idx] = ((i * 13 + j * 17 + 100) & 0xFF) as u8;
        }
    }
    VideoFrame {
        format: PixelFormat::Yuv444P,
        width,
        height,
        pts: Some(0),
        time_base: TimeBase::new(1, 30),
        planes: vec![
            VideoPlane { stride: w, data: y },
            VideoPlane { stride: w, data: u },
            VideoPlane { stride: w, data: v },
        ],
    }
}

fn assert_frames_equal(a: &VideoFrame, b: &VideoFrame) {
    assert_eq!(a.format, b.format, "pixel format");
    assert_eq!(a.width, b.width, "width");
    assert_eq!(a.height, b.height, "height");
    assert_eq!(a.planes.len(), b.planes.len(), "plane count");
    for (i, (pa, pb)) in a.planes.iter().zip(b.planes.iter()).enumerate() {
        // Compare the `width × height` active region, not the raw data array
        // — strides may differ if encoder and decoder disagree on padding.
        let (w, h) = match (i, a.format) {
            (0, _) => (a.width as usize, a.height as usize),
            (_, PixelFormat::Yuv420P) => (
                (a.width as usize).div_ceil(2),
                (a.height as usize).div_ceil(2),
            ),
            (_, PixelFormat::Yuv444P) => (a.width as usize, a.height as usize),
            _ => panic!("unhandled format/plane combo"),
        };
        for y in 0..h {
            let row_a = &pa.data[y * pa.stride..y * pa.stride + w];
            let row_b = &pb.data[y * pb.stride..y * pb.stride + w];
            assert_eq!(row_a, row_b, "plane {} row {} mismatch", i, y);
        }
    }
}

fn roundtrip_one(frame: VideoFrame) {
    let pix = frame.format;
    let params = make_params(pix, frame.width, frame.height);

    let mut enc = make_encoder(&params).expect("make_encoder");
    enc.send_frame(&Frame::Video(frame.clone()))
        .expect("send_frame");
    let pkt = enc.receive_packet().expect("receive_packet");
    assert!(pkt.flags.keyframe);

    // Hand off the encoder's output CodecParameters (with extradata) to the
    // decoder so the configuration record matches.
    let dec_params = enc.output_params().clone();
    let mut dec = make_decoder(&dec_params).expect("make_decoder");
    dec.send_packet(&pkt).expect("send_packet");
    let out = dec.receive_frame().expect("receive_frame");
    match out {
        Frame::Video(v) => assert_frames_equal(&v, &frame),
        _ => panic!("decoder returned non-video frame"),
    }
}

#[test]
fn yuv420_16x16_roundtrip() {
    roundtrip_one(synth_yuv420(16, 16));
}

#[test]
fn yuv420_64x48_roundtrip() {
    roundtrip_one(synth_yuv420(64, 48));
}

#[test]
fn yuv420_odd_dimensions_roundtrip() {
    roundtrip_one(synth_yuv420(17, 11));
}

#[test]
fn yuv444_32x32_roundtrip() {
    roundtrip_one(synth_yuv444(32, 32));
}

#[test]
fn yuv444_64x48_roundtrip() {
    roundtrip_one(synth_yuv444(64, 48));
}

#[test]
fn yuv420_all_zero_roundtrip() {
    // Highly-compressible flat content — stresses the highest-probability
    // states in the range coder and the context-0 (flat) pathway.
    let w = 128u32;
    let h = 80u32;
    let wu = w as usize;
    let hu = h as usize;
    let cw = wu.div_ceil(2);
    let ch = hu.div_ceil(2);
    let frame = VideoFrame {
        format: PixelFormat::Yuv420P,
        width: w,
        height: h,
        pts: Some(0),
        time_base: TimeBase::new(1, 30),
        planes: vec![
            VideoPlane {
                stride: wu,
                data: vec![0u8; wu * hu],
            },
            VideoPlane {
                stride: cw,
                data: vec![128u8; cw * ch],
            },
            VideoPlane {
                stride: cw,
                data: vec![128u8; cw * ch],
            },
        ],
    };
    roundtrip_one(frame);
}

#[test]
fn yuv444_128x96_large_random_roundtrip() {
    // Bigger frame with pseudo-random samples — exercises many contexts.
    let w = 128u32;
    let h = 96u32;
    let wu = w as usize;
    let hu = h as usize;
    let mut rng: u32 = 0xc0ffee00;
    let mut rand = || {
        rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (rng >> 16) as u8
    };
    let y: Vec<u8> = (0..wu * hu).map(|_| rand()).collect();
    let u: Vec<u8> = (0..wu * hu).map(|_| rand()).collect();
    let v: Vec<u8> = (0..wu * hu).map(|_| rand()).collect();
    let frame = VideoFrame {
        format: PixelFormat::Yuv444P,
        width: w,
        height: h,
        pts: Some(0),
        time_base: TimeBase::new(1, 30),
        planes: vec![
            VideoPlane {
                stride: wu,
                data: y,
            },
            VideoPlane {
                stride: wu,
                data: u,
            },
            VideoPlane {
                stride: wu,
                data: v,
            },
        ],
    };
    roundtrip_one(frame);
}
