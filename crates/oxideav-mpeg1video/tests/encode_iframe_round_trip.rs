//! End-to-end I-frame encoder round-trip test.
//!
//! Pipeline:
//!   raw 64×64 yuv420p (one frame from `/tmp/mpeg1_in.yuv`)
//!     → our encoder → MPEG-1 elementary stream
//!     → our decoder → reconstructed yuv420p
//!     → measure pixel match against the input
//!
//! The fixture is generated with:
//!   ffmpeg -y -f lavfi -i "testsrc=size=64x64:rate=24:duration=0.04" \
//!          -f rawvideo -pix_fmt yuv420p /tmp/mpeg1_in.yuv
//!
//! Tests that can't find their fixture are skipped (logged, not failed) so
//! CI without ffmpeg still passes.

use std::path::Path;

use oxideav_core::{
    frame::VideoPlane, CodecId, CodecParameters, Error, Frame, Packet, PixelFormat, Rational,
    TimeBase, VideoFrame,
};
use oxideav_mpeg1video::{
    decoder::make_decoder,
    encoder::{make_encoder, DEFAULT_QUANT_SCALE},
    CODEC_ID_STR,
};

const W: u32 = 64;
const H: u32 = 64;
const FIXTURE: &str = "/tmp/mpeg1_in.yuv";

fn read_yuv_one_frame(path: &str, w: u32, h: u32) -> Option<VideoFrame> {
    if !Path::new(path).exists() {
        eprintln!("fixture {path} missing — skipping test");
        return None;
    }
    let bytes = std::fs::read(path).expect("read fixture");
    let y_size = (w * h) as usize;
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);
    let c_size = (cw * ch) as usize;
    let need = y_size + 2 * c_size;
    if bytes.len() < need {
        panic!("fixture too short: {} < {}", bytes.len(), need);
    }
    let y = bytes[..y_size].to_vec();
    let cb = bytes[y_size..y_size + c_size].to_vec();
    let cr = bytes[y_size + c_size..y_size + 2 * c_size].to_vec();
    Some(VideoFrame {
        format: PixelFormat::Yuv420P,
        width: w,
        height: h,
        pts: Some(0),
        time_base: TimeBase::new(1, 24),
        planes: vec![
            VideoPlane {
                stride: w as usize,
                data: y,
            },
            VideoPlane {
                stride: cw as usize,
                data: cb,
            },
            VideoPlane {
                stride: cw as usize,
                data: cr,
            },
        ],
    })
}

fn encode_one(frame: &VideoFrame) -> Vec<u8> {
    let mut params = CodecParameters::video(CodecId::new(CODEC_ID_STR));
    params.width = Some(W);
    params.height = Some(H);
    params.pixel_format = Some(PixelFormat::Yuv420P);
    params.frame_rate = Some(Rational::new(24, 1));
    params.bit_rate = Some(1_500_000);
    let mut enc = make_encoder(&params).expect("build encoder");
    enc.send_frame(&Frame::Video(frame.clone()))
        .expect("send_frame");
    enc.flush().expect("flush");
    let mut data = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(p) => data.extend_from_slice(&p.data),
            Err(Error::NeedMore) => break,
            Err(Error::Eof) => break,
            Err(e) => panic!("encoder error: {e}"),
        }
    }
    data
}

fn decode_one(bytes: &[u8]) -> VideoFrame {
    let params = CodecParameters::video(CodecId::new(CODEC_ID_STR));
    let mut dec = make_decoder(&params).expect("build decoder");
    let pkt = Packet::new(0, TimeBase::new(1, 24), bytes.to_vec()).with_pts(0);
    dec.send_packet(&pkt).expect("send_packet");
    dec.flush().expect("flush");
    loop {
        match dec.receive_frame() {
            Ok(Frame::Video(v)) => return v,
            Ok(_) => continue,
            Err(Error::NeedMore) | Err(Error::Eof) => panic!("decoder produced no frame"),
            Err(e) => panic!("decoder error: {e}"),
        }
    }
}

fn pixel_match(orig: &VideoFrame, recon: &VideoFrame, tolerance: i32) -> f64 {
    assert_eq!(orig.width, recon.width);
    assert_eq!(orig.height, recon.height);
    assert_eq!(orig.planes.len(), recon.planes.len());
    let mut total: u64 = 0;
    let mut matched: u64 = 0;
    for (o, r) in orig.planes.iter().zip(recon.planes.iter()) {
        for (a, b) in o.data.iter().zip(r.data.iter()) {
            total += 1;
            if (*a as i32 - *b as i32).abs() <= tolerance {
                matched += 1;
            }
        }
    }
    matched as f64 / total as f64
}

fn mean_abs_diff(orig: &VideoFrame, recon: &VideoFrame) -> f64 {
    let mut total = 0u64;
    let mut count = 0u64;
    for (o, r) in orig.planes.iter().zip(recon.planes.iter()) {
        for (a, b) in o.data.iter().zip(r.data.iter()) {
            total += (*a as i32 - *b as i32).unsigned_abs() as u64;
            count += 1;
        }
    }
    total as f64 / count as f64
}

#[test]
fn iframe_round_trip_64x64() {
    let Some(frame) = read_yuv_one_frame(FIXTURE, W, H) else {
        return;
    };
    let bytes = encode_one(&frame);
    eprintln!("encoded {} bytes for 64x64 I-frame", bytes.len());
    assert!(!bytes.is_empty(), "encoder produced no bytes");
    // Sanity-check: starts with the sequence header start code.
    assert_eq!(&bytes[..4], &[0x00, 0x00, 0x01, 0xB3]);

    let recon = decode_one(&bytes);
    let mad = mean_abs_diff(&frame, &recon);
    let pct1 = pixel_match(&frame, &recon, 1) * 100.0;
    let pct8 = pixel_match(&frame, &recon, 8) * 100.0;
    eprintln!(
        "round-trip MAD={mad:.2}, pct(±1)={pct1:.2}%, pct(±8)={pct8:.2}%, quant_scale={}",
        DEFAULT_QUANT_SCALE
    );
    // A simple textbook encoder + reasonable quant_scale=8 should keep the
    // mean absolute pixel difference modest. We deliberately set a loose
    // tolerance — the decoder is bit-exact w.r.t. the encoded stream, but
    // FDCT/IDCT are f32 and intra quantisation introduces lossy rounding.
    assert!(
        pct8 >= 99.0,
        "round-trip ≤±8 match {pct8:.2}% < 99% (MAD={mad:.2})"
    );
}

#[test]
fn ffmpeg_decodes_our_output() {
    let Some(frame) = read_yuv_one_frame(FIXTURE, W, H) else {
        return;
    };
    let Some(_) = which("ffmpeg") else {
        eprintln!("ffmpeg not found — skipping ffmpeg interop test");
        return;
    };
    let bytes = encode_one(&frame);
    let in_path = "/tmp/mpeg1_oxideav.m1v";
    let out_path = "/tmp/mpeg1_oxideav_decoded.yuv";
    std::fs::write(in_path, &bytes).expect("write encoded m1v");
    let _ = std::fs::remove_file(out_path);

    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "mpegvideo",
            "-i",
            in_path,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            out_path,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "ffmpeg failed to decode our output");

    let ff = read_yuv_one_frame(out_path, W, H).expect("read ffmpeg output");
    let pct8 = pixel_match(&frame, &ff, 8) * 100.0;
    let pct16 = pixel_match(&frame, &ff, 16) * 100.0;
    let mad = mean_abs_diff(&frame, &ff);
    eprintln!("ffmpeg-decoded round-trip MAD={mad:.2}, pct(±8)={pct8:.2}%, pct(±16)={pct16:.2}%");
    assert!(
        pct16 >= 95.0,
        "ffmpeg round-trip ≤±16 match {pct16:.2}% < 95% (MAD={mad:.2})"
    );
}

fn which(prog: &str) -> Option<String> {
    let p = std::process::Command::new("which")
        .arg(prog)
        .output()
        .ok()?;
    if p.status.success() {
        let s = String::from_utf8_lossy(&p.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}
