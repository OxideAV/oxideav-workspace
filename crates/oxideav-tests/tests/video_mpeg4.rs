//! MPEG-4 Part 2 (Visual) roundtrip comparison tests against ffmpeg.
//!
//! - `encoder_roundtrip`: encode with ours, decode with ffmpeg, compare Y-PSNR.
//! - `decoder_vs_ffmpeg`: encode with ffmpeg, decode with both, compare Y-PSNR.

use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, PixelFormat, Rational, VideoFrame, VideoPlane,
};

const W: u32 = 64;
const H: u32 = 64;
const NFRAMES: usize = 5;

fn make_yuv_frame(raw: &[u8], idx: usize, w: u32, h: u32) -> VideoFrame {
    let y_sz = (w * h) as usize;
    let cw = (w / 2) as usize;
    let ch = (h / 2) as usize;
    let c_sz = cw * ch;
    let frame_sz = y_sz + 2 * c_sz;
    let base = idx * frame_sz;
    VideoFrame {
        pts: Some(idx as i64),
        planes: vec![
            VideoPlane {
                stride: w as usize,
                data: raw[base..base + y_sz].to_vec(),
            },
            VideoPlane {
                stride: cw,
                data: raw[base + y_sz..base + y_sz + c_sz].to_vec(),
            },
            VideoPlane {
                stride: cw,
                data: raw[base + y_sz + c_sz..base + frame_sz].to_vec(),
            },
        ],
    }
}

/// Encode 5 frames with our MPEG-4 encoder, decode with ffmpeg, compare.
#[test]
fn encoder_roundtrip() {
    if !oxideav_tests::ffmpeg_available() {
        eprintln!("skip");
        return;
    }

    let tmp = oxideav_tests::tmp("video_mpeg4_enc");
    let _ = std::fs::create_dir_all(&tmp);
    let ref_yuv = tmp.join("ref.yuv");

    assert!(oxideav_tests::ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=64x64:rate=10:duration=0.5",
        "-pix_fmt",
        "yuv420p",
        "-f",
        "rawvideo",
        ref_yuv.to_str().unwrap(),
    ]));

    let raw = std::fs::read(&ref_yuv).expect("read ref yuv");
    let frame_sz = (W * H * 3 / 2) as usize;
    assert!(raw.len() >= NFRAMES * frame_sz);

    // Encode with our MPEG-4 encoder.
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_meta::register_all(&mut reg);
    let mut params = CodecParameters::video(CodecId::new("mpeg4video"));
    params.width = Some(W);
    params.height = Some(H);
    params.pixel_format = Some(PixelFormat::Yuv420P);
    params.frame_rate = Some(Rational::new(10, 1));

    let mut enc = reg.codecs.first_encoder(&params).expect("make encoder");

    let mut es_data = Vec::new();
    for i in 0..NFRAMES {
        let frame = make_yuv_frame(&raw, i, W, H);
        enc.send_frame(&Frame::Video(frame)).expect("send_frame");
        loop {
            match enc.receive_packet() {
                Ok(p) => es_data.extend_from_slice(&p.data),
                Err(Error::NeedMore) => break,
                Err(Error::Eof) => break,
                Err(e) => panic!("encoder error: {e}"),
            }
        }
    }
    enc.flush().expect("flush");
    while let Ok(p) = enc.receive_packet() {
        es_data.extend_from_slice(&p.data);
    }

    // Decode with ffmpeg.
    let es_path = tmp.join("ours.m4v");
    let decoded_yuv = tmp.join("decoded.yuv");
    std::fs::write(&es_path, &es_data).expect("write es");

    assert!(oxideav_tests::ffmpeg(&[
        "-f",
        "m4v",
        "-i",
        es_path.to_str().unwrap(),
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        decoded_yuv.to_str().unwrap(),
    ]));

    let decoded = std::fs::read(&decoded_yuv).expect("read decoded yuv");
    let decoded_nframes = decoded.len() / frame_sz;

    for i in 0..decoded_nframes.min(NFRAMES) {
        let orig_y = &raw[i * frame_sz..i * frame_sz + (W * H) as usize];
        let dec_y = &decoded[i * frame_sz..i * frame_sz + (W * H) as usize];
        let psnr = oxideav_tests::video_y_psnr(orig_y, dec_y, W, H);
        eprintln!("  [MPEG-4 encoder frame {i}] PSNR={psnr:.1} dB");
        assert!(
            psnr > 25.0,
            "MPEG-4 encoder frame {i} PSNR {psnr:.1} dB < 25 dB threshold"
        );
    }
}

/// Encode with ffmpeg, decode with both, compare Y-PSNR.
#[test]
fn decoder_vs_ffmpeg() {
    if !oxideav_tests::ffmpeg_available() {
        eprintln!("skip");
        return;
    }

    let tmp = oxideav_tests::tmp("video_mpeg4_dec");
    let _ = std::fs::create_dir_all(&tmp);
    let avi_path = tmp.join("ffmpeg.avi");
    let ref_yuv = tmp.join("ref.yuv");

    // Encode with ffmpeg's MPEG-4 into AVI.
    assert!(oxideav_tests::ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=64x64:rate=10:duration=0.5",
        "-c:v",
        "mpeg4",
        "-q:v",
        "5",
        "-f",
        "avi",
        avi_path.to_str().unwrap(),
    ]));

    // Decode with ffmpeg for reference.
    assert!(oxideav_tests::ffmpeg(&[
        "-i",
        avi_path.to_str().unwrap(),
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        ref_yuv.to_str().unwrap(),
    ]));

    let ref_data = std::fs::read(&ref_yuv).expect("read ref yuv");
    let frame_sz = (W * H * 3 / 2) as usize;
    let ref_nframes = ref_data.len() / frame_sz;

    // Decode with our decoder.
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_meta::register_all(&mut reg);
    let avi_data = std::fs::read(&avi_path).expect("read avi");
    let mut file: Box<dyn oxideav::core::ReadSeek> = Box::new(std::io::Cursor::new(avi_data));
    let format = reg
        .containers
        .probe_input(&mut *file, Some("avi"))
        .expect("probe");
    let mut dmx = reg
        .containers
        .open_demuxer(&format, file, &reg.codecs)
        .expect("open demuxer");

    let video_idx = dmx
        .streams()
        .iter()
        .position(|s| s.params.width.is_some())
        .expect("no video stream");
    let params = dmx.streams()[video_idx].params.clone();
    let mut dec = reg.codecs.first_decoder(&params).expect("make decoder");

    let mut our_frames: Vec<Vec<u8>> = Vec::new();
    loop {
        match dmx.next_packet() {
            Ok(pkt) => {
                if pkt.stream_index != video_idx as u32 {
                    continue;
                }
                dec.send_packet(&pkt).expect("send_packet");
                loop {
                    match dec.receive_frame() {
                        Ok(Frame::Video(v)) => {
                            let mut y = Vec::with_capacity((W * H) as usize);
                            for row in 0..H as usize {
                                let start = row * v.planes[0].stride;
                                y.extend_from_slice(&v.planes[0].data[start..start + W as usize]);
                            }
                            our_frames.push(y);
                        }
                        Ok(_) => {}
                        Err(Error::NeedMore) => break,
                        Err(Error::Eof) => break,
                        Err(e) => panic!("decoder error: {e}"),
                    }
                }
            }
            Err(Error::Eof) => break,
            Err(e) => panic!("demuxer error: {e}"),
        }
    }

    dec.flush().expect("flush");
    loop {
        match dec.receive_frame() {
            Ok(Frame::Video(v)) => {
                let mut y = Vec::with_capacity((W * H) as usize);
                for row in 0..H as usize {
                    let start = row * v.planes[0].stride;
                    y.extend_from_slice(&v.planes[0].data[start..start + W as usize]);
                }
                our_frames.push(y);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    let count = our_frames.len().min(ref_nframes);
    eprintln!("  MPEG-4 decoder: decoded {count} frames (ref has {ref_nframes})");
    assert!(count > 0, "no frames decoded");

    for i in 0..count {
        let ref_y = &ref_data[i * frame_sz..i * frame_sz + (W * H) as usize];
        let our_y = &our_frames[i];
        let psnr = oxideav_tests::video_y_psnr(our_y, ref_y, W, H);
        eprintln!("  [MPEG-4 decoder frame {i}] PSNR={psnr:.1} dB");
        assert!(
            psnr > 25.0,
            "MPEG-4 decoder frame {i} PSNR {psnr:.1} dB < 25 dB threshold"
        );
    }
}

// ── Round 452: registry encoder options → registry decoder (no oracle) ──

fn mpeg4_registry() -> oxideav_core::RuntimeContext {
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_mpeg4video::register(&mut reg);
    reg
}

fn synth_yuv(idx: usize) -> Vec<u8> {
    let (w, h) = (W as usize, H as usize);
    let mut raw = vec![0u8; w * h * 3 / 2];
    for y in 0..h {
        for x in 0..w {
            raw[y * w + x] = (((x + idx * 3) ^ (y * 2)) % 220) as u8 + 16;
        }
    }
    for i in 0..(w * h / 4) {
        raw[w * h + i] = ((i + idx * 5) % 100) as u8 + 80;
        raw[w * h + w * h / 4 + i] = ((i * 3 + idx) % 100) as u8 + 90;
    }
    raw
}

/// Our encoder with the r452 registry options → our decoder, no
/// oracle. Each option set must produce a decodable stream whose VOL
/// header carries the requested tools and whose frames land within a
/// fidelity floor. `rvlc` without `data-partitioned` is rejected at
/// factory time (the encoder's own invariant).
#[test]
fn registry_options_encode_decode_round_trip() {
    let reg = mpeg4_registry();
    let base = || {
        let mut p = CodecParameters::video(CodecId::new("mpeg4video"));
        p.width = Some(W);
        p.height = Some(H);
        p.pixel_format = Some(PixelFormat::Yuv420P);
        p.frame_rate = Some(Rational::new(10, 1));
        p
    };

    // Invalid combination is refused by the factory.
    let mut bad = base();
    bad.options = oxideav_core::CodecOptions::default().set("rvlc", "true");
    assert!(
        reg.codecs.first_encoder(&bad).is_err(),
        "rvlc requires data-partitioned"
    );

    let cases: &[(&str, &[(&str, &str)])] = &[
        ("mb-aq", &[("mb-aq", "true"), ("qp", "6")]),
        ("packet-bits", &[("packet-bits", "400")]),
        (
            "rvlc",
            &[
                ("data-partitioned", "true"),
                ("rvlc", "true"),
                ("packet-bits", "500"),
            ],
        ),
        (
            "all+bf",
            &[
                ("mb-aq", "true"),
                ("packet-bits", "600"),
                ("data-partitioned", "true"),
                ("rvlc", "true"),
                ("bf", "2"),
            ],
        ),
    ];
    for (label, opts) in cases {
        let mut p = base();
        for (k, v) in *opts {
            p.options.insert(*k, *v);
        }
        let mut enc = reg
            .codecs
            .first_encoder(&p)
            .unwrap_or_else(|e| panic!("{label}: encoder: {e:?}"));
        let src: Vec<Vec<u8>> = (0..NFRAMES).map(synth_yuv).collect();
        for (i, raw) in src.iter().enumerate() {
            let mut vf = make_yuv_frame(raw, 0, W, H);
            vf.pts = Some(i as i64);
            enc.send_frame(&Frame::Video(vf))
                .unwrap_or_else(|e| panic!("{label}: send {i}: {e:?}"));
        }
        enc.flush().expect("flush");
        let mut packets = Vec::new();
        loop {
            match enc.receive_packet() {
                Ok(pk) => packets.push(pk),
                Err(Error::Eof | Error::NeedMore) => break,
                Err(e) => panic!("{label}: receive: {e:?}"),
            }
        }
        assert_eq!(packets.len(), NFRAMES, "{label}: one packet per VOP");
        assert!(packets[0].flags.keyframe, "{label}: first VOP is intra");
        // Config headers are in-band on the first packet (VOS start code).
        assert!(
            packets[0].data.starts_with(&[0, 0, 1, 0xB0]),
            "{label}: VOS header leads the stream"
        );
        let extradata = &enc.output_params().extradata;
        assert!(
            !extradata.is_empty() && packets[0].data.starts_with(extradata),
            "{label}: output_params.extradata mirrors the in-band headers"
        );

        // Decoder needs no extradata: everything is in-band.
        let mut dec = reg
            .codecs
            .first_decoder(&CodecParameters::video(CodecId::new("mpeg4video")))
            .expect("decoder");
        let mut frames = Vec::new();
        for pk in &packets {
            dec.send_packet(pk)
                .unwrap_or_else(|e| panic!("{label}: decode: {e:?}"));
            loop {
                match dec.receive_frame() {
                    Ok(Frame::Video(v)) => frames.push(v),
                    Ok(_) => panic!("video only"),
                    Err(Error::NeedMore | Error::Eof) => break,
                    Err(e) => panic!("{label}: receive_frame: {e:?}"),
                }
            }
        }
        dec.flush().expect("flush");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Video(v)) => frames.push(v),
                Ok(_) => panic!("video only"),
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("{label}: drain: {e:?}"),
            }
        }
        assert_eq!(frames.len(), NFRAMES, "{label}: every VOP decodes");
        // With B-VOPs the decoder reorders to display order; sort by pts
        // so the fidelity check compares like with like.
        frames.sort_by_key(|f| f.pts.unwrap_or(0));
        for (i, (f, raw)) in frames.iter().zip(&src).enumerate() {
            let y: Vec<u8> = f.planes[0]
                .data
                .chunks(f.planes[0].stride)
                .take(H as usize)
                .flat_map(|r| r[..W as usize].iter().copied())
                .collect();
            let psnr = oxideav_tests::video_y_psnr(&y, &raw[..(W * H) as usize], W, H);
            assert!(psnr > 25.0, "{label} frame {i}: Y-PSNR {psnr:.2} dB");
        }
    }
}
