//! VP8 comparison tests against ffmpeg.
//!
//! Round 449 refresh: the old prose claimed "VP8 has no encoder in
//! oxideav" — stale since the `oxideav-vp8` encoder reached
//! production status (SPLITMV, GOLDEN/ALTREF, two-pass, the works).
//! The decoder leg keeps its historical shape (ffmpeg-encoded IVF →
//! our registry demux + decode vs ffmpeg's own decode); the encoder
//! leg round-trips our framework `make_encoder` output through our
//! decoder and, when the oracle is present, through ffmpeg via an
//! IVF wrapping built with `oxideav-bitstream`.

use oxideav_core::{Error, Frame};

const W: u32 = 64;
const H: u32 = 64;

/// Encode with ffmpeg (libvpx), decode with both, compare Y-PSNR.
#[test]
fn decoder_vs_ffmpeg() {
    if !oxideav_tests::ffmpeg_available() {
        eprintln!("skip");
        return;
    }

    let tmp = oxideav_tests::tmp("video_vp8_dec");
    let _ = std::fs::create_dir_all(&tmp);
    let ivf_path = tmp.join("ffmpeg.ivf");
    let ref_yuv = tmp.join("ref.yuv");

    // Encode with ffmpeg's libvpx into IVF.
    assert!(
        oxideav_tests::ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=64x64:rate=10:duration=0.5",
            "-c:v",
            "libvpx",
            "-b:v",
            "200k",
            "-f",
            "ivf",
            ivf_path.to_str().unwrap(),
        ]),
        "ffmpeg failed to encode VP8"
    );

    // Decode with ffmpeg for reference.
    assert!(oxideav_tests::ffmpeg(&[
        "-i",
        ivf_path.to_str().unwrap(),
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        ref_yuv.to_str().unwrap(),
    ]));

    let ref_data = std::fs::read(&ref_yuv).expect("read ref yuv");
    let frame_sz = (W * H * 3 / 2) as usize;
    let ref_nframes = ref_data.len() / frame_sz;

    // Decode with our registry decoder. No fleet-level IVF container
    // demuxer exists (the historical suite assumed one and silently
    // never ran); split the IVF frames with `oxideav-bitstream`'s
    // public reader and feed each payload as one packet.
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_vp8::register(&mut reg);
    let ivf_data = std::fs::read(&ivf_path).expect("read ivf");
    let (header, ivf_frames) =
        oxideav_bitstream::ivf::parse_all(&ivf_data).expect("parse ffmpeg's IVF");
    assert_eq!(&header.fourcc, b"VP80", "ffmpeg must emit a VP8 IVF");

    let params = {
        let mut p = oxideav_core::CodecParameters::video(oxideav_core::CodecId::new("vp8"));
        p.width = Some(W);
        p.height = Some(H);
        p
    };
    let mut dec = reg.codecs.first_decoder(&params).expect("make decoder");

    let mut our_frames: Vec<Vec<u8>> = Vec::new();
    let tb = oxideav_core::TimeBase::new(1, 10);
    for f in &ivf_frames {
        let pkt = oxideav_core::Packet::new(0, tb, f.payload.to_vec());
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
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("decode error: {e}"),
            }
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
    eprintln!(
        "  VP8 decoder: decoded {} frames (ref has {ref_nframes})",
        our_frames.len()
    );
    assert!(count > 0, "no frames decoded");

    let mut total_psnr = 0.0f64;
    let mut min_psnr = f64::INFINITY;
    for i in 0..count {
        let ref_y = &ref_data[i * frame_sz..i * frame_sz + (W * H) as usize];
        let our_y = &our_frames[i];
        let psnr = oxideav_tests::video_y_psnr(our_y, ref_y, W, H);
        eprintln!("  [VP8 decoder frame {i}] PSNR={psnr:.1} dB");
        total_psnr += psnr;
        if psnr < min_psnr {
            min_psnr = psnr;
        }
        // Two conforming VP8 decoders reconstruct the same stream
        // (near-)identically — the old 5 dB "does not crash" floor
        // predated the decoder's production status.
        assert!(
            psnr > 40.0,
            "VP8 decoder frame {i} PSNR {psnr:.1} dB < 40 dB"
        );
    }
    let avg_psnr = total_psnr / count as f64;
    eprintln!("  VP8 decoder average PSNR={avg_psnr:.1} dB, min={min_psnr:.1} dB");
}

/// Deterministic moving-gradient frames (packed planar YUV420).
fn gradient_frames(n: usize) -> Vec<Vec<u8>> {
    let (w, h) = (W as usize, H as usize);
    let (cw, ch) = (w / 2, h / 2);
    (0..n)
        .map(|f| {
            let mut buf = Vec::with_capacity(w * h + 2 * cw * ch);
            for y in 0..h {
                for x in 0..w {
                    buf.push(((x * 2 + y * 3 + f * 13) % 220) as u8 + 16);
                }
            }
            for y in 0..ch {
                for x in 0..cw {
                    buf.push(((x * 5 + y + f * 7) % 200) as u8 + 24);
                }
            }
            for y in 0..ch {
                for x in 0..cw {
                    buf.push(((x + y * 4 + f * 3) % 200) as u8 + 28);
                }
            }
            buf
        })
        .collect()
}

/// Encode `frames` with our framework VP8 encoder, returning packets.
fn encode_with_ours(frames: &[Vec<u8>]) -> Vec<oxideav_core::Packet> {
    use oxideav_core::{CodecId, CodecParameters, PixelFormat, VideoFrame, VideoPlane};
    let (w, h) = (W as usize, H as usize);
    let (cw, ch) = (w / 2, h / 2);
    let mut params = CodecParameters::video(CodecId::new("vp8"));
    params.width = Some(W);
    params.height = Some(H);
    params.pixel_format = Some(PixelFormat::Yuv420P);
    let mut enc = oxideav_vp8::make_encoder(&params).expect("make vp8 encoder");
    let mut packets = Vec::new();
    for (i, planar) in frames.iter().enumerate() {
        let frame = Frame::Video(VideoFrame {
            pts: Some(i as i64),
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
        });
        enc.send_frame(&frame).expect("send frame");
        loop {
            match enc.receive_packet() {
                Ok(p) => packets.push(p),
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("encode error: {e:?}"),
            }
        }
    }
    enc.flush().expect("flush");
    loop {
        match enc.receive_packet() {
            Ok(p) => packets.push(p),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    }
    packets
}

/// Encoder self-roundtrip (no oracle): our framework encoder → our
/// registry decoder, Y-PSNR gated per frame.
#[test]
fn encoder_self_roundtrip() {
    use oxideav_core::{CodecId, CodecParameters};
    let frames = gradient_frames(4);
    let packets = encode_with_ours(&frames);
    assert_eq!(packets.len(), frames.len(), "one packet per frame");
    assert!(packets[0].flags.keyframe, "frame 0 is the keyframe");

    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_vp8::register(&mut reg);
    let params = CodecParameters::video(CodecId::new("vp8"));
    let mut dec = reg.codecs.first_decoder(&params).expect("make vp8 decoder");
    let mut decoded = Vec::new();
    for p in &packets {
        dec.send_packet(p).expect("send");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Video(v)) => {
                    let mut y = Vec::with_capacity((W * H) as usize);
                    for row in 0..H as usize {
                        let start = row * v.planes[0].stride;
                        y.extend_from_slice(&v.planes[0].data[start..start + W as usize]);
                    }
                    decoded.push(y);
                }
                Ok(_) => {}
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("decode error: {e:?}"),
            }
        }
    }
    assert_eq!(decoded.len(), frames.len());
    for (i, (got, want)) in decoded.iter().zip(frames.iter()).enumerate() {
        let psnr = oxideav_tests::video_y_psnr(got, want, W, H);
        eprintln!("  [VP8 encoder self-roundtrip frame {i}] Y-PSNR={psnr:.1} dB");
        assert!(psnr > 30.0, "frame {i} Y-PSNR {psnr:.1} dB too low");
    }
}

/// Encoder oracle leg: our stream wrapped as IVF (`VP80`), decoded by
/// ffmpeg, Y-PSNR gated against the encoder's input.
#[test]
fn encoder_vs_ffmpeg_decode() {
    if !oxideav_tests::ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }
    let frames = gradient_frames(4);
    let packets = encode_with_ours(&frames);

    let mut ivf = Vec::new();
    oxideav_bitstream::ivf::write_header(
        &mut ivf,
        oxideav_bitstream::ivf::IvfHeader {
            fourcc: oxideav_bitstream::ivf::IVF_FOURCC_VP80,
            width: W as u16,
            height: H as u16,
            framerate_num: 30,
            framerate_den: 1,
            frame_count: packets.len() as u32,
        },
    );
    for (i, p) in packets.iter().enumerate() {
        oxideav_bitstream::ivf::write_frame(&mut ivf, i as u64, &p.data).expect("ivf frame");
    }
    let ivf_path = oxideav_tests::tmp("oxideav-vp8-enc.ivf");
    std::fs::write(&ivf_path, &ivf).expect("write ivf");

    let yuv_path = oxideav_tests::tmp("oxideav-vp8-enc-ffmpeg.yuv");
    assert!(
        oxideav_tests::ffmpeg(&[
            "-i",
            ivf_path.to_str().unwrap(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            yuv_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our VP8 stream"
    );
    let raw = std::fs::read(&yuv_path).expect("read yuv");
    let frame_sz = (W * H * 3 / 2) as usize;
    assert_eq!(raw.len(), frame_sz * frames.len(), "frame count");
    for (i, want) in frames.iter().enumerate() {
        let got_y = &raw[i * frame_sz..i * frame_sz + (W * H) as usize];
        let psnr = oxideav_tests::video_y_psnr(got_y, want, W, H);
        eprintln!("  [VP8 encoder vs ffmpeg frame {i}] Y-PSNR={psnr:.1} dB");
        assert!(psnr > 30.0, "frame {i} Y-PSNR {psnr:.1} dB too low");
    }
}
