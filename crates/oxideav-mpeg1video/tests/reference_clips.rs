//! Integration tests against ffmpeg-generated reference clips.
//!
//! These tests expect fixtures at:
//!   /tmp/ref-mpeg1-tiny.m1v    (64x64,   25fps, 1s)
//!   /tmp/ref-mpeg1-gop.m1v     (128x96,  25fps, 2s with -g 10, I+P only)
//!   /tmp/ref-mpeg1-gop-p.m1v   (128x96,  25fps, 2s with -g 10, I+P only, explicit -bf 0)
//!   /tmp/ref-mpeg1-gop-b.m1v   (128x96,  25fps, 2s with -g 10 -bf 2, I/P/B)
//!
//! Generate them with:
//!   ffmpeg -y -f lavfi -i testsrc=d=1:s=64x64:r=25 -c:v mpeg1video -b:v 200k       /tmp/ref-mpeg1-tiny.m1v
//!   ffmpeg -y -f lavfi -i testsrc=d=2:s=128x96:r=25 -c:v mpeg1video -g 10          /tmp/ref-mpeg1-gop.m1v
//!   ffmpeg -y -f lavfi -i testsrc=d=2:s=128x96:r=25 -c:v mpeg1video -g 10 -bf 0    /tmp/ref-mpeg1-gop-p.m1v
//!   ffmpeg -y -f lavfi -i testsrc=d=2:s=128x96:r=25 -c:v mpeg1video -g 10 -bf 2    /tmp/ref-mpeg1-gop-b.m1v
//!
//! Tests that can't find their fixture are skipped (logged, not failed), so
//! CI without ffmpeg still passes.

use std::path::Path;

use oxideav_core::{CodecId, CodecParameters, Error, Frame, Packet, TimeBase};
use oxideav_mpeg1video::{
    bitreader::BitReader,
    decoder::{codec_parameters_from_sequence_header, make_decoder},
    headers::{parse_sequence_header, PictureType},
    start_codes::{self, PICTURE_START_CODE, SEQUENCE_HEADER_CODE},
};

fn read_fixture(path: &str) -> Option<Vec<u8>> {
    if !Path::new(path).exists() {
        eprintln!("fixture {path} missing — skipping test");
        return None;
    }
    Some(std::fs::read(path).expect("read fixture"))
}

#[test]
fn parse_sequence_header_tiny() {
    let Some(data) = read_fixture("/tmp/ref-mpeg1-tiny.m1v") else {
        return;
    };
    // First start code should be the sequence header.
    let mut iter = start_codes::iter_start_codes(&data);
    let (pos, code) = iter.next().expect("sequence header start code");
    assert_eq!(code, SEQUENCE_HEADER_CODE);
    let mut br = BitReader::new(&data[pos + 4..]);
    let sh = parse_sequence_header(&mut br).expect("parse sequence header");
    assert_eq!(sh.horizontal_size, 64);
    assert_eq!(sh.vertical_size, 64);
    let params = codec_parameters_from_sequence_header(&sh);
    assert_eq!(params.width, Some(64));
    assert_eq!(params.height, Some(64));
    let fr = params.frame_rate.expect("frame rate");
    assert_eq!(fr.num, 25);
    assert_eq!(fr.den, 1);
}

#[test]
fn parse_first_picture_header_tiny() {
    let Some(data) = read_fixture("/tmp/ref-mpeg1-tiny.m1v") else {
        return;
    };
    // Find first picture_start_code (0x00).
    let (pos, code) = start_codes::iter_start_codes(&data)
        .find(|(_, c)| *c == PICTURE_START_CODE)
        .expect("picture start code");
    assert_eq!(code, PICTURE_START_CODE);
    let mut br = BitReader::new(&data[pos + 4..]);
    let ph =
        oxideav_mpeg1video::headers::parse_picture_header(&mut br).expect("parse picture header");
    // First picture in an MPEG-1 sequence is always an I-picture.
    assert_eq!(ph.picture_type, PictureType::I);
}

/// Milestone 2: decode a single I-frame. Exercises the full parse +
/// macroblock/block decode path end-to-end.
#[test]
fn decode_first_i_frame_tiny() {
    let Some(data) = read_fixture("/tmp/ref-mpeg1-tiny.m1v") else {
        return;
    };
    let params = CodecParameters::video(CodecId::new(oxideav_mpeg1video::CODEC_ID_STR));
    let mut decoder = make_decoder(&params).expect("build decoder");
    let packet = Packet::new(0, TimeBase::new(1, 90_000), data);
    if let Err(e) = decoder.send_packet(&packet) {
        eprintln!("send_packet err: {e}");
    }
    let _ = decoder.flush();

    let frame = match decoder.receive_frame() {
        Ok(f) => f,
        Err(Error::NeedMore) => panic!("NeedMore after flush"),
        Err(Error::Eof) => panic!("decoder returned EOF before any frame"),
        Err(e) => panic!("decoder error: {e}"),
    };

    match frame {
        Frame::Video(vf) => {
            assert_eq!(vf.width, 64);
            assert_eq!(vf.height, 64);
            assert_eq!(vf.planes.len(), 3);
            let y_plane = &vf.planes[0];
            let mean_y: u64 =
                y_plane.data.iter().map(|&b| b as u64).sum::<u64>() / y_plane.data.len() as u64;
            eprintln!("mean Y = {mean_y}");
            assert!(
                mean_y > 30,
                "mean Y {mean_y} too low — expected testsrc colour bars"
            );
            // Regression guard for the "EOB after 63 AC coefficients" bug: the
            // bottom-right luma block of the first macroblock (pixels rows
            // 8..15, cols 8..15) has a high mean (~162 for the testsrc colour
            // bars). If the AC-loop exits without consuming the trailing EOB
            // marker, block 3 DC ends up misdecoded and drops to ~44.
            let stride = y_plane.stride;
            let block3_mean: u32 = (8..16)
                .flat_map(|r| (8..16).map(move |c| y_plane.data[r * stride + c] as u32))
                .sum::<u32>()
                / 64;
            eprintln!("block 3 (bottom-right luma of MB0) mean = {block3_mean}");
            assert!(
                block3_mean > 120,
                "block 3 mean {block3_mean} too low — EOB/AC sync bug?"
            );
        }
        _ => panic!("expected video frame"),
    }
}

fn decode_all_frames(path: &str) -> Option<Vec<oxideav_core::VideoFrame>> {
    let data = read_fixture(path)?;
    let params = CodecParameters::video(CodecId::new(oxideav_mpeg1video::CODEC_ID_STR));
    let mut decoder = make_decoder(&params).expect("build decoder");
    let packet = Packet::new(0, TimeBase::new(1, 90_000), data).with_pts(0);
    if let Err(e) = decoder.send_packet(&packet) {
        panic!("send_packet err: {e}");
    }
    let _ = decoder.flush();

    let mut frames = Vec::new();
    loop {
        match decoder.receive_frame() {
            Ok(Frame::Video(v)) => frames.push(v),
            Ok(_) => {}
            Err(Error::Eof) => break,
            Err(Error::NeedMore) => break,
            Err(e) => panic!("decoder error: {e}"),
        }
    }
    Some(frames)
}

fn mean_luma(frame: &oxideav_core::VideoFrame) -> u64 {
    let y = &frame.planes[0];
    y.data.iter().map(|&b| b as u64).sum::<u64>() / y.data.len() as u64
}

/// Milestone 3: decode the first few frames of a GOP clip and verify each
/// produces a sensible (non-black) mean Y. Exercises the P-frame path via
/// motion compensation and non-intra block residuals.
#[test]
fn decode_gop_clip_p_frames_nonzero() {
    // Prefer the explicit P-only clip if available; fall back to the default
    // GOP clip (which in most ffmpeg configurations is also I+P with no B).
    let path = if Path::new("/tmp/ref-mpeg1-gop-p.m1v").exists() {
        "/tmp/ref-mpeg1-gop-p.m1v"
    } else {
        "/tmp/ref-mpeg1-gop.m1v"
    };
    let Some(frames) = decode_all_frames(path) else {
        return;
    };
    assert!(
        frames.len() >= 4,
        "expected at least 4 frames, got {}",
        frames.len()
    );
    for (i, f) in frames.iter().take(4).enumerate() {
        let my = mean_luma(f);
        eprintln!("frame {i}: mean Y = {my}");
        assert!(my > 30, "frame {i} mean Y {my} too low — expected testsrc");
    }
}

/// Milestone 4: decode a GOP with B frames. Verify PTSes are monotonic
/// increasing (display order) and that the decode_order-derived
/// temporal_reference stream is NOT monotonic (proving B-reordering
/// happened).
#[test]
fn decode_gop_clip_b_frames_ordered() {
    let Some(frames) = decode_all_frames("/tmp/ref-mpeg1-gop-b.m1v") else {
        return;
    };
    assert!(
        frames.len() >= 6,
        "expected at least 6 frames, got {}",
        frames.len()
    );
    // Collect PTSes; they should be monotonic.
    let pts: Vec<i64> = frames.iter().filter_map(|f| f.pts).collect();
    assert!(!pts.is_empty(), "no frames carried a PTS");
    for w in pts.windows(2) {
        assert!(w[0] < w[1], "PTSes not monotonic: {pts:?}");
    }
    // Decode-order temporal_reference: read raw picture headers from the
    // bitstream and record their temporal_reference. The B-reordering
    // test is: this sequence is NOT monotonic (P arrives before its
    // preceding Bs, so its TR > next B's TR).
    let data = std::fs::read("/tmp/ref-mpeg1-gop-b.m1v").expect("read fixture");
    let mut tr_decode_order = Vec::new();
    for (pos, code) in start_codes::iter_start_codes(&data) {
        if code == PICTURE_START_CODE {
            let mut br = BitReader::new(&data[pos + 4..]);
            if let Ok(ph) = oxideav_mpeg1video::headers::parse_picture_header(&mut br) {
                tr_decode_order.push(ph.temporal_reference);
            }
        }
    }
    eprintln!("decode-order TRs: {tr_decode_order:?}");
    eprintln!("display-order PTSes: {pts:?}");
    let is_monotonic = tr_decode_order.windows(2).all(|w| w[0] < w[1]);
    assert!(
        !is_monotonic,
        "decode-order TR sequence is monotonic — expected B-reordering jumps: {tr_decode_order:?}"
    );
}
