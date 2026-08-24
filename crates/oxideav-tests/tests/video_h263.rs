//! H.263 comparison tests against ffmpeg + framework registry legs.
//!
//! Round 451 refresh: the r450 h263 arc wired the REAL framework
//! registration (`oxideav_h263::register` installs decoder + encoder
//! factories, the `H263`/`S263` container tags, and the §5.1.1
//! Picture-Start-Code payload magics), closing the r449 followup
//! that kept this suite on the direct picture API alone. The direct
//! legs stay (`encode_intra_picture` → conformant §5 bitstreams;
//! `decode_sequence` → whole-ES decode with the reference chain
//! threaded); new registry legs pin tag/payload-magic resolution and
//! a full framework encode→decode GOP round trip.
//!
//! H.263 baseline supports the standard source formats only; QCIF
//! (176×144) is used throughout.

use oxideav_h263::{decode_sequence, encode_intra_picture, DecodeOptions, YuvFrame};

const W: usize = 176;
const H: usize = 144;
const NFRAMES: usize = 4;

/// Deterministic moving-gradient QCIF frames.
fn gradient_frames(n: usize) -> Vec<YuvFrame> {
    let (cw, ch) = (W / 2, H / 2);
    (0..n)
        .map(|f| {
            let mut y = Vec::with_capacity(W * H);
            for r in 0..H {
                for c in 0..W {
                    y.push(((c * 2 + r * 3 + f * 11) % 220) as u8 + 16);
                }
            }
            let mut cb = Vec::with_capacity(cw * ch);
            let mut cr = Vec::with_capacity(cw * ch);
            for r in 0..ch {
                for c in 0..cw {
                    cb.push(((c * 3 + r + f * 7) % 200) as u8 + 24);
                    cr.push(((c + r * 5 + f * 3) % 200) as u8 + 28);
                }
            }
            YuvFrame {
                y,
                cb,
                cr,
                luma_width: W,
                luma_height: H,
            }
        })
        .collect()
}

/// Encode an intra-only ES: one §5.1 picture per frame, TR advancing.
fn encode_intra_es(frames: &[YuvFrame], quant: u8) -> Vec<u8> {
    let mut es = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        es.extend_from_slice(
            &encode_intra_picture(f, quant, (i * 25) as u8).expect("encode intra picture"),
        );
    }
    es
}

fn y_psnr(a: &YuvFrame, b: &YuvFrame) -> f64 {
    oxideav_tests::video_y_psnr(&a.y, &b.y, W as u32, H as u32)
}

/// Self-roundtrip (no oracle): our intra encode → our sequence decode.
#[test]
fn encoder_self_roundtrip() {
    let frames = gradient_frames(NFRAMES);
    let es = encode_intra_es(&frames, 8);
    let decoded = decode_sequence(&es, DecodeOptions::default()).expect("decode our ES");
    assert_eq!(decoded.len(), NFRAMES);
    for (i, (got, want)) in decoded.iter().zip(frames.iter()).enumerate() {
        let psnr = y_psnr(got, want);
        eprintln!("  [H.263 self-roundtrip frame {i}] Y-PSNR={psnr:.1} dB");
        assert!(psnr > 28.0, "frame {i} Y-PSNR {psnr:.1} dB too low");
    }
}

/// Encoder oracle leg: our intra-only ES decoded by ffmpeg.
#[test]
fn encoder_vs_ffmpeg_decode() {
    if !oxideav_tests::ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }
    let frames = gradient_frames(NFRAMES);
    let es = encode_intra_es(&frames, 8);
    let es_path = oxideav_tests::tmp("oxideav-h263-enc.h263");
    std::fs::write(&es_path, &es).expect("write es");

    let yuv_path = oxideav_tests::tmp("oxideav-h263-enc-ffmpeg.yuv");
    assert!(
        oxideav_tests::ffmpeg(&[
            "-f",
            "h263",
            "-i",
            es_path.to_str().unwrap(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            yuv_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our H.263 stream"
    );
    let raw = std::fs::read(&yuv_path).expect("read yuv");
    let frame_sz = W * H * 3 / 2;
    assert_eq!(
        raw.len(),
        frame_sz * NFRAMES,
        "ffmpeg decodes every picture"
    );
    for (i, want) in frames.iter().enumerate() {
        let got_y = &raw[i * frame_sz..i * frame_sz + W * H];
        let psnr = oxideav_tests::video_y_psnr(got_y, &want.y, W as u32, H as u32);
        eprintln!("  [H.263 encoder vs ffmpeg frame {i}] Y-PSNR={psnr:.1} dB");
        assert!(psnr > 28.0, "frame {i} Y-PSNR {psnr:.1} dB too low");
    }
}

/// Decoder oracle leg: ffmpeg-encoded H.263 ES (I+P pictures), our
/// `decode_sequence` vs ffmpeg's own decode of the same stream.
#[test]
fn decoder_vs_ffmpeg() {
    if !oxideav_tests::ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }
    let tmp = oxideav_tests::tmp("video_h263_dec");
    let _ = std::fs::create_dir_all(&tmp);
    let es_path = tmp.join("ffmpeg.h263");
    let ref_yuv = tmp.join("ref.yuv");

    assert!(
        oxideav_tests::ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=176x144:rate=10:duration=0.5",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "h263",
            "-b:v",
            "400k",
            "-f",
            "h263",
            es_path.to_str().unwrap(),
        ]),
        "ffmpeg failed to encode H.263"
    );
    assert!(oxideav_tests::ffmpeg(&[
        "-f",
        "h263",
        "-i",
        es_path.to_str().unwrap(),
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        ref_yuv.to_str().unwrap(),
    ]));

    let es = std::fs::read(&es_path).expect("read es");
    let ours = decode_sequence(&es, DecodeOptions::default()).expect("decode ffmpeg's ES");
    let ref_data = std::fs::read(&ref_yuv).expect("read ref yuv");
    let frame_sz = W * H * 3 / 2;
    let ref_nframes = ref_data.len() / frame_sz;
    let count = ours.len().min(ref_nframes);
    assert!(count > 0, "no frames decoded");
    eprintln!(
        "  H.263 decoder: decoded {} frames (ref has {ref_nframes})",
        ours.len()
    );

    for (i, got) in ours.iter().take(count).enumerate() {
        let ref_y = &ref_data[i * frame_sz..i * frame_sz + W * H];
        let psnr = oxideav_tests::video_y_psnr(&got.y, ref_y, W as u32, H as u32);
        eprintln!("  [H.263 decoder frame {i}] Y-PSNR={psnr:.1} dB");
        assert!(psnr > 40.0, "frame {i} Y-PSNR {psnr:.1} dB < 40 dB");
    }
}

// ══════════════════ registry (framework) legs — round 451 ══════════════════

/// A fresh registry with only the h263 crate's `register` applied.
fn h263_registry() -> oxideav_core::RuntimeContext {
    let mut ctx = oxideav_core::RuntimeContext::new();
    oxideav_h263::register(&mut ctx);
    ctx
}

/// The container-facing claims land in the registry: both FourCC tags
/// (`H263` AVI-family, `S263` 3GP/MP4 sample entry) resolve to the
/// codec id, and a raw elementary stream resolves through the §5.1.1
/// byte-aligned Picture-Start-Code payload magic.
#[test]
fn registry_resolves_tags_and_payload_magic() {
    use oxideav_core::CodecTag;
    let ctx = h263_registry();
    for raw in [b"H263", b"S263"] {
        let tag = CodecTag::fourcc(raw);
        let probe = oxideav_core::ProbeContext::new(&tag);
        let id = ctx
            .codecs
            .resolve_tag_ref(&probe)
            .unwrap_or_else(|| panic!("tag {raw:?} must resolve"));
        assert_eq!(id.as_str(), "h263", "tag {raw:?}");
    }

    // A real encoded picture starts 00 00 8x — the registered magic.
    let frames = gradient_frames(1);
    let es = encode_intra_es(&frames, 8);
    assert!(es.len() > 3 && es[0] == 0 && es[1] == 0 && (es[2] & 0xFC) == 0x80);
    let id = ctx
        .codecs
        .resolve_payload_magic_ref(&es)
        .expect("PSC payload magic must resolve");
    assert_eq!(id.as_str(), "h263");
    assert!(ctx.codecs.has_decoder(&oxideav_core::CodecId::new("h263")));
    assert!(ctx.codecs.has_encoder(&oxideav_core::CodecId::new("h263")));
}

/// Full framework GOP round trip: registry-resolved encoder (closed
/// loop, I+P, `gop=4`) → packets with the keyframe cadence → registry
/// -resolved decoder → pixel-domain comparison against the input.
#[test]
fn registry_encode_decode_gop_round_trip() {
    use oxideav_core::{CodecId, CodecParameters, Error, Frame, Packet, PixelFormat, TimeBase};
    const N: usize = 8;
    let ctx = h263_registry();
    let frames = gradient_frames(N);

    let mut enc_params = CodecParameters::video(CodecId::new("h263"));
    enc_params.width = Some(W as u32);
    enc_params.height = Some(H as u32);
    enc_params.pixel_format = Some(PixelFormat::Yuv420P);
    enc_params.options = oxideav_core::CodecOptions::new()
        .set("gop", "4")
        .set("quant", "8");
    let mut enc = ctx
        .codecs
        .first_encoder(&enc_params)
        .expect("registry must resolve an h263 encoder");

    let tb = TimeBase::MICROS;
    let mut packets: Vec<Packet> = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        let vf = oxideav_core::VideoFrame {
            pts: Some(i as i64 * 100_000),
            planes: vec![
                oxideav_core::VideoPlane {
                    stride: W,
                    data: f.y.clone(),
                },
                oxideav_core::VideoPlane {
                    stride: W / 2,
                    data: f.cb.clone(),
                },
                oxideav_core::VideoPlane {
                    stride: W / 2,
                    data: f.cr.clone(),
                },
            ],
        };
        enc.send_frame(&Frame::Video(vf)).expect("send frame");
        loop {
            match enc.receive_packet() {
                Ok(p) => packets.push(p),
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("encode error: {e:?}"),
            }
        }
    }
    enc.flush().expect("flush encoder");
    loop {
        match enc.receive_packet() {
            Ok(p) => packets.push(p),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    }
    assert_eq!(packets.len(), N, "one packet per frame");
    let kf: Vec<usize> = packets
        .iter()
        .enumerate()
        .filter(|(_, p)| p.flags.keyframe)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(kf, vec![0, 4], "gop=4 keyframe cadence");

    // Decoder options are a separate schema — the encoder's `gop` /
    // `quant` knobs are not decoder options.
    let mut dec_params = CodecParameters::video(CodecId::new("h263"));
    dec_params.width = Some(W as u32);
    dec_params.height = Some(H as u32);
    dec_params.pixel_format = Some(PixelFormat::Yuv420P);
    let mut dec = ctx
        .codecs
        .first_decoder(&dec_params)
        .expect("registry must resolve an h263 decoder");
    let mut decoded: Vec<oxideav_core::VideoFrame> = Vec::new();
    let drain = |dec: &mut Box<dyn oxideav_core::Decoder>,
                 out: &mut Vec<oxideav_core::VideoFrame>| loop {
        match dec.receive_frame() {
            Ok(Frame::Video(v)) => out.push(v),
            Ok(other) => panic!("unexpected frame {other:?}"),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("decode error: {e:?}"),
        }
    };
    for p in &packets {
        let mut q = Packet::new(0, tb, p.data.clone());
        q.pts = p.pts;
        dec.send_packet(&q).expect("send packet");
        drain(&mut dec, &mut decoded);
    }
    dec.flush().expect("flush decoder");
    drain(&mut dec, &mut decoded);
    assert_eq!(decoded.len(), N, "every packet decodes to one picture");

    for (i, (got, want)) in decoded.iter().zip(frames.iter()).enumerate() {
        let planes = got.image_planes();
        assert_eq!(planes.len(), 3, "frame {i}: 3 planar 4:2:0 planes");
        let y: Vec<u8> = planes[0]
            .data
            .chunks(planes[0].stride)
            .take(H)
            .flat_map(|row| row[..W].iter().copied())
            .collect();
        let psnr = oxideav_tests::video_y_psnr(&y, &want.y, W as u32, H as u32);
        eprintln!("  [H.263 framework round trip frame {i}] Y-PSNR={psnr:.1} dB");
        assert!(psnr > 28.0, "frame {i} Y-PSNR {psnr:.1} dB too low");
    }
}
