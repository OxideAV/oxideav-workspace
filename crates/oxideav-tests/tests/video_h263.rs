//! H.263 comparison tests against ffmpeg.
//!
//! Round 449 refresh: the r445 restoration drove this suite through
//! the codec registry — but `oxideav-h263`'s framework `register` is
//! still the round-1 no-op stub, so `first_encoder`/`first_decoder`
//! can never resolve and the suite only stayed green because the
//! oracle gate skipped everything on CI. The crate's real surface is
//! its direct picture API (`encode_intra_picture` /
//! `encode_inter_picture` → conformant §5 bitstreams;
//! `decode_sequence` → whole-ES decode with the reference chain
//! threaded), so the suite now consumes that. Wiring the framework
//! factories remains an `oxideav-h263` followup.
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
