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

// ── Round 452: `Vp9GopConfig`-driven structured GOP → registry decode ──

/// Drain every frame the registry decoder surfaces for one packet.
fn drain_frames(dec: &mut Box<dyn oxideav_core::Decoder>, out: &mut Vec<VideoFrame>) {
    loop {
        match dec.receive_frame() {
            Ok(Frame::Video(v)) => out.push(v),
            Ok(other) => panic!("expected video, got {other:?}"),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("decode error: {e:?}"),
        }
    }
}

/// A hidden-alt-ref GOP (`altref_interval = 3`) from the batch
/// `Vp9GopConfig` entry: the coded sequence has more frames than the
/// display sequence (hidden ARFs + `show_existing_frame` bytes), yet
/// fed packet-by-packet to the registry decoder it surfaces exactly
/// `GOP` display frames with the source geometry and reasonable
/// fidelity. Segmentation on top of the ARF structure is exercised too.
#[test]
fn gop_config_altref_structure_decodes_via_registry() {
    use oxideav_vp9::{encode_vp9_lossy_sequence_with, Vp9GopConfig, Vp9Segmentation};
    let src = gop_frames();
    let refs: Vec<&[u8]> = src.iter().map(|f| f.as_slice()).collect();
    for seg in [Vp9Segmentation::Off, Vp9Segmentation::AdaptiveQuant] {
        let mut cfg = Vp9GopConfig::new(80);
        cfg.altref_interval = 3;
        cfg.segmentation = seg;
        let coded = encode_vp9_lossy_sequence_with(&refs, W, H, &cfg).expect("gop encode");
        assert!(
            coded.len() > src.len(),
            "{seg:?}: hidden ARF structure adds coded frames ({} vs {})",
            coded.len(),
            src.len()
        );
        assert!(
            coded.iter().any(|f| f.len() == 1),
            "{seg:?}: at least one 1-byte show_existing_frame packet"
        );

        let reg = registry();
        let mut dec = reg
            .codecs
            .first_decoder(&CodecParameters::video(CodecId::new("vp9")))
            .expect("vp9 decoder");
        let mut frames = Vec::new();
        for (i, data) in coded.iter().enumerate() {
            dec.send_packet(&Packet::new(
                0,
                oxideav_core::TimeBase::MILLIS,
                data.clone(),
            ))
            .unwrap_or_else(|e| panic!("{seg:?}: send packet {i}: {e:?}"));
            drain_frames(&mut dec, &mut frames);
        }
        dec.flush().expect("flush");
        drain_frames(&mut dec, &mut frames);
        assert_eq!(frames.len(), GOP, "{seg:?}: one output per display frame");
        for (i, (f, s)) in frames.iter().zip(&src).enumerate() {
            assert_eq!(f.planes.len(), 3);
            let y: Vec<u8> = f.planes[0]
                .data
                .chunks(f.planes[0].stride)
                .take(H as usize)
                .flat_map(|r| r[..W as usize].iter().copied())
                .collect();
            let psnr = video_y_psnr(&y, &s[..(W * H) as usize], W, H);
            assert!(psnr > 28.0, "{seg:?} frame {i}: Y-PSNR {psnr:.2} dB");
        }
    }
}

/// core 0.1.35 `Yuv440P` end-to-end: the registry encoder accepts a
/// 4:4:0 frame (full-width, half-height chroma) and the decoder labels
/// its output with the same format; plane geometry matches the core
/// helpers.
#[test]
fn registry_yuv440p_round_trip_keeps_label_and_geometry() {
    let fmt = PixelFormat::Yuv440P;
    let (w, h) = (W, H);
    let (cw, ch) = fmt.plane_dimensions(1, w, h).unwrap();
    let mk = |f: usize, pw: u32, ph: u32, k: usize| -> Vec<u8> {
        (0..(pw * ph) as usize)
            .map(|i| ((i * (3 + k) + f * 13) % 200) as u8 + 20)
            .collect()
    };
    let mut params = vp9_params(&[("q", "40")]);
    params.pixel_format = Some(fmt);
    let reg = registry();
    let mut enc = reg.codecs.first_encoder(&params).expect("vp9 440 encoder");
    let mut dec = oxideav_vp9::Vp9Decoder::new();
    let mut n = 0;
    for f in 0..3usize {
        let frame = Frame::Video(VideoFrame {
            pts: Some(f as i64),
            planes: vec![
                VideoPlane {
                    stride: w as usize,
                    data: mk(f, w, h, 0),
                },
                VideoPlane {
                    stride: cw as usize,
                    data: mk(f, cw, ch, 1),
                },
                VideoPlane {
                    stride: cw as usize,
                    data: mk(f, cw, ch, 2),
                },
            ],
        });
        enc.send_frame(&frame).expect("send 440 frame");
        let pkt = enc.receive_packet().expect("440 packet");
        oxideav_core::Decoder::send_packet(&mut dec, &pkt).expect("decode 440");
        let Frame::Video(v) = oxideav_core::Decoder::receive_frame(&mut dec).expect("440 frame")
        else {
            panic!("expected video");
        };
        assert_eq!(dec.pixel_format(), Some(fmt), "decoder labels 4:4:0 output");
        assert_eq!(v.planes.len(), 3);
        assert_eq!(v.planes[0].data.len() / v.planes[0].stride, h as usize);
        assert_eq!(
            v.planes[1].data.len() / v.planes[1].stride,
            ch as usize,
            "half-height Cb"
        );
        assert_eq!(
            v.planes[2].data.len() / v.planes[2].stride,
            ch as usize,
            "half-height Cr"
        );
        assert!(v.planes[1].stride >= cw as usize, "full-width chroma rows");
        n += 1;
    }
    assert_eq!(n, 3);
}

/// core 0.1.35 `Yuv440P` on real bitstreams: the staged 4:4:0 docs
/// fixtures decode through the framework decoder with the 4:4:0 label
/// and match the reference planes byte-exact (full-width, half-height
/// chroma, tightly packed). Skips when the docs corpus is not staged.
#[test]
fn docs_yuv440_fixtures_decode_with_440_label() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/video/vp9/fixtures");
    let cases = [
        ("profile-1-yuv440-8bit-inter", PixelFormat::Yuv440P, 1usize),
        ("lossy-440-gop", PixelFormat::Yuv440P, 1),
        ("lossy-hbd12-440-gop", PixelFormat::Yuv440P12Le, 2),
    ];
    let mut seen = 0;
    for (name, fmt, bps) in cases {
        let dir = root.join(name);
        let (Ok(ivf), Ok(expected)) = (
            std::fs::read(dir.join("input.ivf")),
            std::fs::read(dir.join("expected.yuv")),
        ) else {
            eprintln!("skipping {name}: docs fixture not staged");
            continue;
        };
        let (hdr, frames) = oxideav_bitstream::ivf::parse_all(&ivf).expect("ivf");
        let (w, h) = (hdr.width as u32, hdr.height as u32);
        let mut dec = oxideav_vp9::Vp9Decoder::new();
        let mut out = Vec::new();
        let mut shown = 0usize;
        for (i, f) in frames.iter().enumerate() {
            oxideav_core::Decoder::send_packet(
                &mut dec,
                &Packet::new(0, oxideav_core::TimeBase::MILLIS, f.payload.to_vec()),
            )
            .unwrap_or_else(|e| panic!("{name}: frame {i}: {e:?}"));
            loop {
                match oxideav_core::Decoder::receive_frame(&mut dec) {
                    Ok(Frame::Video(v)) => {
                        assert_eq!(
                            dec.pixel_format(),
                            Some(fmt),
                            "{name}: label at frame {shown}"
                        );
                        assert_eq!(v.planes.len(), 3, "{name}");
                        for (p, plane) in v.planes.iter().enumerate() {
                            let (pw, ph) = fmt.plane_dimensions(p, w, h).unwrap();
                            let row = fmt.plane_row_bytes(p, w).unwrap();
                            assert_eq!(row, pw as usize * bps);
                            assert!(plane.stride >= row, "{name}: plane {p} stride");
                            for r in plane.data.chunks(plane.stride).take(ph as usize) {
                                out.extend_from_slice(&r[..row]);
                            }
                        }
                        shown += 1;
                    }
                    Ok(other) => panic!("{name}: {other:?}"),
                    Err(Error::NeedMore | Error::Eof) => break,
                    Err(e) => panic!("{name}: frame {i}: {e:?}"),
                }
            }
        }
        assert!(shown > 0, "{name}: at least one shown frame");
        assert_eq!(
            out.len(),
            fmt.frame_size_bytes(w, h).unwrap() * shown,
            "{name}: tight 4:4:0 frame size × shown"
        );
        assert_eq!(out.len(), expected.len(), "{name}: reference length");
        assert!(
            out == expected,
            "{name}: 4:4:0 planes must match the reference byte-exact"
        );
        seen += 1;
    }
    let _ = seen;
}
