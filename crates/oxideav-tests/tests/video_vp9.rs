//! VP9 whole-GOP decode through the codec registry.
//!
//! Round 445 promoted the §7.2.6 chain framing to the default public
//! GOP path and wired real framework factories into `register()`:
//! `Vp9Encoder` rides the chained default GOP (keyframe + shown
//! non-error-resilient P-frames), `Vp9Decoder` streams packets
//! through the incremental sequence decoder with a per-packet Annex-B
//! split. This suite drives both halves through
//! `first_encoder` / `first_decoder` — the registry surface, not the
//! crate's batch entry points — and cross-checks the emitted stream
//! against ffmpeg via a minimal IVF wrapping built with
//! `oxideav-bitstream`'s public IVF writer.

use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, Packet, PixelFormat, VideoFrame, VideoPlane,
};
use oxideav_tests::*;

const W: u32 = 64;
const H: u32 = 48;
const GOP: usize = 5;

/// Deterministic moving-gradient GOP: packed planar YUV420 frames with
/// real inter-frame motion so P-frames carry non-trivial residual.
fn gop_frames() -> Vec<Vec<u8>> {
    let (w, h) = (W as usize, H as usize);
    let (cw, ch) = (w / 2, h / 2);
    (0..GOP)
        .map(|f| {
            let mut buf = Vec::with_capacity(w * h + 2 * cw * ch);
            for y in 0..h {
                for x in 0..w {
                    buf.push(((x * 3 + y * 5 + f * 17) % 240) as u8 + 8);
                }
            }
            for y in 0..ch {
                for x in 0..cw {
                    buf.push(((x * 7 + y + f * 11) % 200) as u8 + 20);
                }
            }
            for y in 0..ch {
                for x in 0..cw {
                    buf.push(((x + y * 9 + f * 5) % 200) as u8 + 30);
                }
            }
            buf
        })
        .collect()
}

/// Wrap a packed planar YUV420 buffer as a framework `VideoFrame`.
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

fn vp9_params(extra_options: &[(&str, &str)]) -> CodecParameters {
    let mut params = CodecParameters::video(CodecId::new("vp9"));
    params.width = Some(W);
    params.height = Some(H);
    params.pixel_format = Some(PixelFormat::Yuv420P);
    for (k, v) in extra_options {
        params.options.insert(*k, *v);
    }
    params
}

fn registry() -> oxideav_core::RuntimeContext {
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_vp9::register(&mut reg);
    reg
}

/// Encode the GOP through `first_encoder`, returning the packets.
fn encode_gop(params: &CodecParameters) -> Vec<Packet> {
    let reg = registry();
    let mut enc = reg.codecs.first_encoder(params).expect("make vp9 encoder");
    for (i, planar) in gop_frames().iter().enumerate() {
        enc.send_frame(&video_frame(planar, i as i64))
            .expect("send");
    }
    enc.flush().expect("flush");
    let mut packets = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(p) => packets.push(p),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    }
    packets
}

/// Decode packets through `first_decoder`, returning packed planar
/// YUV420 buffers.
fn decode_gop(params: &CodecParameters, packets: &[Packet]) -> Vec<Vec<u8>> {
    let reg = registry();
    let mut dec = reg.codecs.first_decoder(params).expect("make vp9 decoder");
    let mut out = Vec::new();
    for p in packets {
        dec.send_packet(p).expect("send packet");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Video(v)) => {
                    let mut planar = Vec::new();
                    for plane in &v.planes {
                        planar.extend_from_slice(&plane.data);
                    }
                    out.push(planar);
                }
                Ok(_) => panic!("expected video frames"),
                Err(Error::NeedMore) => break,
                Err(e) => panic!("decode error: {e:?}"),
            }
        }
    }
    dec.flush().expect("flush");
    out
}

/// Registry lossless GOP: keyframe + chained P-frames, byte-exact.
#[test]
fn registry_lossless_gop_roundtrip() {
    let params = vp9_params(&[("lossless", "true")]);
    let packets = encode_gop(&params);
    assert_eq!(packets.len(), GOP, "one packet per frame");
    assert!(packets[0].flags.keyframe, "frame 0 must be the keyframe");
    assert!(
        packets[1..].iter().all(|p| !p.flags.keyframe),
        "chained framing codes later frames as P-frames"
    );

    let decoded = decode_gop(&params, &packets);
    assert_eq!(decoded.len(), GOP);
    for (i, (got, want)) in decoded.iter().zip(gop_frames().iter()).enumerate() {
        assert_eq!(got, want, "lossless GOP frame {i} not byte-exact");
    }
}

/// Registry lossy GOP: the `q` option flows through
/// `Vp9EncoderOptions`; the whole chained GOP decodes with sane
/// fidelity.
#[test]
fn registry_lossy_gop_decodes() {
    let params = vp9_params(&[("q", "60")]);
    let packets = encode_gop(&params);
    assert_eq!(packets.len(), GOP);
    assert!(packets[0].flags.keyframe);

    let decoded = decode_gop(&params, &packets);
    assert_eq!(decoded.len(), GOP);
    for (i, (got, want)) in decoded.iter().zip(gop_frames().iter()).enumerate() {
        let psnr = video_y_psnr(got, want, W, H);
        eprintln!("  vp9 lossy GOP frame {i}: Y-PSNR {psnr:.1} dB");
        assert!(psnr > 30.0, "frame {i} Y-PSNR {psnr:.1} dB too low");
    }
}

/// Oracle leg: the registry encoder's lossless chained GOP, wrapped in
/// IVF via `oxideav-bitstream`, decodes byte-exact in ffmpeg.
#[test]
fn registry_lossless_gop_vs_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let params = vp9_params(&[("lossless", "true")]);
    let packets = encode_gop(&params);

    let mut ivf = Vec::new();
    oxideav_bitstream::ivf::write_header(
        &mut ivf,
        oxideav_bitstream::ivf::IvfHeader {
            fourcc: oxideav_bitstream::ivf::IVF_FOURCC_VP90,
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
    let ivf_path = tmp("oxideav-vp9-gop.ivf");
    std::fs::write(&ivf_path, &ivf).expect("write ivf");

    let yuv_path = tmp("oxideav-vp9-gop-ffmpeg.yuv");
    assert!(
        ffmpeg(&[
            "-i",
            ivf_path.to_str().unwrap(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            yuv_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our VP9 stream"
    );

    let raw = std::fs::read(&yuv_path).expect("read yuv");
    let frame_size = (W * H * 3 / 2) as usize;
    assert_eq!(
        raw.len(),
        frame_size * GOP,
        "ffmpeg must decode every frame of the chained GOP"
    );
    for (i, want) in gop_frames().iter().enumerate() {
        let got = &raw[i * frame_size..(i + 1) * frame_size];
        assert_eq!(got, &want[..], "frame {i}: ffmpeg decode not byte-exact");
    }
}
