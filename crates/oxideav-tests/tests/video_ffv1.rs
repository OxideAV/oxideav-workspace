//! FFV1 roundtrip comparison tests against ffmpeg.
//!
//! Round 445 restoration: `oxideav-ffv1` re-grew both directions to
//! saturation against RFC 9043 (v0/v1/v3, range coder + Golomb-Rice,
//! inter frames with carried coder state), reachable through the codec
//! registry (`register` → `first_encoder` / `first_decoder`), so the
//! harness neutralized when the old direct factories disappeared comes
//! back on the registry path.
//!
//! FFV1 is lossless, so every comparison here is **bit-exact**, and
//! the interop legs ride Matroska end-to-end as a genuine cross-crate
//! flow: our encoder's packets muxed through `oxideav-mkv` for ffmpeg
//! to decode, and ffmpeg's `.mkv` output demuxed through `oxideav-mkv`
//! for our registry decoder.
//!
//! With empty `extradata` and a mapped `PixelFormat`, the registry
//! encoder emits a v0/v1 stream whose §4.2 Parameters ride the first
//! keyframe in-band (RFC 9043 §4.4) — exactly the carriage shape the
//! decoder's `v0v1_dims` path parses back, and what a Matroska
//! `V_FFV1` track without CodecPrivate means.

use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, Packet, PixelFormat, ReadSeek, StreamInfo, TimeBase,
    VideoFrame, VideoPlane, WriteSeek,
};
use oxideav_tests::*;

const W: u32 = 96;
const H: u32 = 64;
const NFRAMES: usize = 4;

/// Deterministic YUV420P frame sequence: a gradient field with a
/// moving block so inter frames have real prediction work to do.
fn make_frames() -> Vec<VideoFrame> {
    let (w, h) = (W as usize, H as usize);
    let (cw, ch) = (w / 2, h / 2);
    (0..NFRAMES)
        .map(|idx| {
            let mut y = vec![0u8; w * h];
            let mut cb = vec![0u8; cw * ch];
            let mut cr = vec![0u8; cw * ch];
            for row in 0..h {
                for col in 0..w {
                    y[row * w + col] = ((row * 2 + col * 3 + idx * 5) % 251) as u8;
                }
            }
            // Moving 16x16 bright block.
            let bx = (idx * 12) % (w - 16);
            let by = (idx * 8) % (h - 16);
            for row in by..by + 16 {
                for col in bx..bx + 16 {
                    y[row * w + col] = 235;
                }
            }
            for row in 0..ch {
                for col in 0..cw {
                    cb[row * cw + col] = ((row + col * 2 + idx * 7) % 240 + 16) as u8;
                    cr[row * cw + col] = ((row * 3 + col + idx * 11) % 240 + 16) as u8;
                }
            }
            VideoFrame {
                pts: Some(idx as i64),
                planes: vec![
                    VideoPlane { stride: w, data: y },
                    VideoPlane {
                        stride: cw,
                        data: cb,
                    },
                    VideoPlane {
                        stride: cw,
                        data: cr,
                    },
                ],
            }
        })
        .collect()
}

/// Tightly-packed YUV420P bytes of a frame (Y then Cb then Cr).
fn frame_bytes(frame: &VideoFrame) -> Vec<u8> {
    let mut out = Vec::new();
    for plane in &frame.planes {
        out.extend_from_slice(&plane.data);
    }
    out
}

fn ffv1_params() -> CodecParameters {
    let mut params = CodecParameters::video(CodecId::new("ffv1"));
    params.width = Some(W);
    params.height = Some(H);
    params.pixel_format = Some(PixelFormat::Yuv420P);
    params
}

/// Encode the deterministic sequence through the registry encoder.
fn encode_with_ours() -> (CodecParameters, Vec<Packet>) {
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_ffv1::register(&mut reg);
    let mut enc = reg
        .codecs
        .first_encoder(&ffv1_params())
        .expect("registry builds an ffv1 encoder");
    let out_params = enc.output_params().clone();

    let mut packets = Vec::new();
    for frame in make_frames() {
        enc.send_frame(&Frame::Video(frame)).expect("send_frame");
        loop {
            match enc.receive_packet() {
                Ok(pkt) => packets.push(pkt),
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("encode error: {e:?}"),
            }
        }
    }
    enc.flush().expect("flush");
    loop {
        match enc.receive_packet() {
            Ok(pkt) => packets.push(pkt),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    }
    (out_params, packets)
}

/// Decode FFV1 packets through the registry decoder, returning tightly
/// packed YUV420P bytes per frame.
fn decode_with_ours(params: &CodecParameters, packets: &[Packet]) -> Vec<Vec<u8>> {
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_ffv1::register(&mut reg);
    let mut dec = reg
        .codecs
        .first_decoder(params)
        .expect("registry builds an ffv1 decoder");

    let mut frames = Vec::new();
    for pkt in packets {
        dec.send_packet(pkt).expect("send_packet");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Video(v)) => frames.push(frame_bytes(&v)),
                Ok(_) => {}
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("decode error: {e:?}"),
            }
        }
    }
    frames
}

/// Self-roundtrip that runs without ffmpeg: registry encode → registry
/// decode must reproduce every pixel bit-for-bit (FFV1 is lossless).
#[test]
fn registry_roundtrip_is_bit_exact() {
    let (out_params, packets) = encode_with_ours();
    assert_eq!(packets.len(), NFRAMES, "one coded Frame per input frame");
    assert!(packets[0].is_keyframe(), "first Frame must be a keyframe");

    let decoded = decode_with_ours(&out_params, &packets);
    assert_eq!(decoded.len(), NFRAMES);
    for (idx, (ours, original)) in decoded.iter().zip(make_frames().iter()).enumerate() {
        assert_eq!(
            ours,
            &frame_bytes(original),
            "frame {idx} did not round-trip bit-exact"
        );
    }
}

/// Encoder interop: our FFV1 packets muxed through `oxideav-mkv`,
/// decoded by ffmpeg, compared bit-exact against the input frames.
#[test]
fn encoder_vs_ffmpeg_via_matroska() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let (out_params, packets) = encode_with_ours();

    // Mux through the registry's Matroska muxer.
    let mkv_path = tmp("oxideav-ffv1-enc-test.mkv");
    {
        let mut reg = oxideav_core::RuntimeContext::new();
        oxideav_mkv::register(&mut reg);
        let stream = StreamInfo {
            index: 0,
            time_base: TimeBase::new(1, 25),
            duration: None,
            start_time: Some(0),
            params: out_params,
        };
        let f = std::fs::File::create(&mkv_path).expect("create mkv");
        let ws: Box<dyn WriteSeek> = Box::new(f);
        let mut mux = reg
            .containers
            .open_muxer("matroska", ws, &[stream])
            .expect("open matroska muxer");
        mux.write_header().expect("write header");
        for pkt in &packets {
            mux.write_packet(pkt).expect("write packet");
        }
        mux.write_trailer().expect("write trailer");
    }

    let decoded_path = tmp("oxideav-ffv1-enc-ffmpeg.yuv");
    assert!(
        ffmpeg(&[
            "-i",
            mkv_path.to_str().unwrap(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our FFV1-in-Matroska stream"
    );

    let raw = std::fs::read(&decoded_path).expect("read yuv");
    let frame_size = (W * H * 3 / 2) as usize;
    assert_eq!(
        raw.len(),
        frame_size * NFRAMES,
        "ffmpeg decoded a different frame count"
    );
    for (idx, original) in make_frames().iter().enumerate() {
        assert_eq!(
            &raw[idx * frame_size..(idx + 1) * frame_size],
            frame_bytes(original).as_slice(),
            "frame {idx}: ffmpeg's decode of our stream is not bit-exact"
        );
    }
}

/// Decoder interop: ffmpeg-encoded FFV1 in Matroska, demuxed through
/// `oxideav-mkv` and decoded by our registry decoder, compared
/// bit-exact against ffmpeg's own decode.
#[test]
fn decoder_vs_ffmpeg_via_matroska() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    // Deterministic input written from our frame generator.
    let input_path = tmp("oxideav-ffv1-dec-input.yuv");
    {
        let mut raw = Vec::new();
        for frame in make_frames() {
            raw.extend(frame_bytes(&frame));
        }
        std::fs::write(&input_path, &raw).expect("write input yuv");
    }

    let size = format!("{W}x{H}");
    let mkv_path = tmp("oxideav-ffv1-dec-test.mkv");
    assert!(
        ffmpeg(&[
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            "-video_size",
            &size,
            "-framerate",
            "25",
            "-i",
            input_path.to_str().unwrap(),
            "-c:v",
            "ffv1",
            mkv_path.to_str().unwrap(),
        ]),
        "ffmpeg ffv1 encode failed"
    );

    // ffmpeg's own decode (the reference pixels — must equal the input,
    // FFV1 being lossless, but compare against the dump to be safe).
    let ffmpeg_decoded_path = tmp("oxideav-ffv1-dec-ffmpeg.yuv");
    assert!(
        ffmpeg(&[
            "-i",
            mkv_path.to_str().unwrap(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            ffmpeg_decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg decode failed"
    );
    let reference = std::fs::read(&ffmpeg_decoded_path).expect("read reference yuv");

    // Our side: registry demux (matroska) → registry decode (ffv1).
    let mkv_data = std::fs::read(&mkv_path).expect("read mkv");
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_ffv1::register(&mut reg);
    oxideav_mkv::register(&mut reg);
    let mut file: Box<dyn ReadSeek> = Box::new(std::io::Cursor::new(mkv_data));
    let format = reg
        .containers
        .probe_input(&mut *file, Some("mkv"))
        .expect("probe mkv");
    assert_eq!(format, "matroska");
    let mut dmx = reg
        .containers
        .open_demuxer(&format, file, &oxideav_core::NullCodecResolver)
        .expect("open matroska demuxer");
    let params = dmx.streams()[0].params.clone();
    assert_eq!(params.codec_id.as_str(), "ffv1", "track must map to ffv1");
    let mut dec = reg
        .codecs
        .first_decoder(&params)
        .expect("make ffv1 decoder");

    let mut ours = Vec::new();
    loop {
        let pkt = match dmx.next_packet() {
            Ok(p) => p,
            Err(Error::Eof) => break,
            Err(e) => panic!("demux error: {e:?}"),
        };
        dec.send_packet(&pkt).expect("send");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Video(v)) => ours.extend(frame_bytes(&v)),
                Ok(_) => {}
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("decode error: {e:?}"),
            }
        }
    }

    assert_eq!(
        ours.len(),
        reference.len(),
        "decoded byte count differs from the reference decode"
    );
    assert_eq!(
        ours, reference,
        "our FFV1 decode of ffmpeg's stream is not bit-exact"
    );
}
