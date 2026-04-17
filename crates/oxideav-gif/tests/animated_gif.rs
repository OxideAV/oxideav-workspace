//! Animated GIF round-trip: build 5 synthetic frames with varying
//! delays, encode, mux, demux, decode, and check every frame survives
//! with its palette indices + delay timing intact.

mod common;

use std::io::Cursor;

use oxideav_codec::CodecRegistry;
use oxideav_container::{ContainerRegistry, WriteSeek};
use oxideav_core::{
    CodecId, CodecParameters, Frame, MediaType, PixelFormat, TimeBase, VideoFrame, VideoPlane,
};
use oxideav_gif::{register_codecs, register_containers, GIF_CODEC_ID};

use common::SharedSink;

fn build_palette() -> Vec<u8> {
    let mut out = Vec::with_capacity(256 * 4);
    for i in 0..256 {
        out.extend_from_slice(&[i as u8, (255 - i) as u8, ((i * 17) & 0xFF) as u8, 0xFF]);
    }
    out
}

fn build_frame(w: u32, h: u32, phase: u32, pts_cs: i64) -> VideoFrame {
    let mut indices = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = ((x + y + phase) & 0x1F) as u8;
            indices.push(v);
        }
    }
    VideoFrame {
        format: PixelFormat::Pal8,
        width: w,
        height: h,
        pts: Some(pts_cs),
        time_base: TimeBase::new(1, 100),
        planes: vec![
            VideoPlane {
                stride: w as usize,
                data: indices,
            },
            VideoPlane {
                stride: 256 * 4,
                data: build_palette(),
            },
        ],
    }
}

#[test]
fn animated_5_frames_roundtrip() {
    let w = 64u32;
    let h = 64u32;
    let n_frames = 5u32;

    // Varying per-frame delays (in centiseconds): 5, 12, 20, 8, 15.
    // Absolute pts in cs = cumulative sum starting at 0.
    let per_frame_delays: [i64; 5] = [5, 12, 20, 8, 15];
    let mut pts_cs: Vec<i64> = Vec::with_capacity(n_frames as usize);
    let mut cursor = 0i64;
    for d in &per_frame_delays {
        pts_cs.push(cursor);
        cursor += d;
    }
    let input_frames: Vec<VideoFrame> = (0..n_frames)
        .map(|i| build_frame(w, h, i * 3, pts_cs[i as usize]))
        .collect();

    // Encode all frames.
    let mut codecs = CodecRegistry::new();
    register_codecs(&mut codecs);
    let params_enc = {
        let mut p = CodecParameters::video(CodecId::new(GIF_CODEC_ID));
        p.media_type = MediaType::Video;
        p.width = Some(w);
        p.height = Some(h);
        p.pixel_format = Some(PixelFormat::Pal8);
        p
    };
    let mut encoder = codecs.make_encoder(&params_enc).expect("encoder");
    let mut packets: Vec<oxideav_core::Packet> = Vec::new();
    for f in &input_frames {
        encoder.send_frame(&Frame::Video(f.clone())).expect("send");
        while let Ok(pkt) = encoder.receive_packet() {
            packets.push(pkt);
        }
    }
    encoder.flush().expect("flush");
    while let Ok(pkt) = encoder.receive_packet() {
        packets.push(pkt);
    }
    assert_eq!(packets.len() as u32, n_frames);
    let encoder_params = encoder.output_params().clone();

    // Mux.
    let mut containers = ContainerRegistry::new();
    register_containers(&mut containers);
    let (sink, sink_data) = SharedSink::new();
    {
        let boxed: Box<dyn WriteSeek> = Box::new(sink);
        let si = oxideav_core::StreamInfo {
            index: 0,
            time_base: TimeBase::new(1, 100),
            duration: None,
            start_time: Some(0),
            params: encoder_params.clone(),
        };
        let mut muxer = containers
            .open_muxer("gif", boxed, std::slice::from_ref(&si))
            .expect("muxer");
        muxer.write_header().expect("hdr");
        for pkt in &packets {
            muxer.write_packet(pkt).expect("pkt");
        }
        muxer.write_trailer().expect("trl");
    }
    let buf: Vec<u8> = sink_data.lock().unwrap().clone();

    // Sanity: should contain NETSCAPE2.0 since >1 frame.
    let needle = b"NETSCAPE2.0";
    let mut found = false;
    for win in buf.windows(needle.len()) {
        if win == needle {
            found = true;
            break;
        }
    }
    assert!(found, "animated GIF should contain NETSCAPE2.0 app ext");

    // Demux + decode.
    let cursor = Cursor::new(buf.clone());
    let boxed: Box<dyn oxideav_container::ReadSeek> = Box::new(cursor);
    let mut demuxer = containers.open_demuxer("gif", boxed).expect("demux");
    let si = demuxer.streams()[0].clone();
    assert_eq!(si.params.width, Some(w));
    assert_eq!(si.params.height, Some(h));
    assert_eq!(si.time_base, TimeBase::new(1, 100));

    let mut decoder = codecs.make_decoder(&si.params).expect("decoder");

    let mut decoded: Vec<VideoFrame> = Vec::new();
    let mut durations: Vec<i64> = Vec::new();
    for _ in 0..n_frames {
        let pkt = demuxer.next_packet().expect("pkt");
        durations.push(pkt.duration.unwrap_or(0));
        decoder.send_packet(&pkt).expect("send");
        let f = match decoder.receive_frame().expect("recv") {
            Frame::Video(v) => v,
            _ => panic!("non-video"),
        };
        decoded.push(f);
    }

    // Per-frame delays should track the input pts-deltas, except for
    // the last frame which uses the encoder's default delay.
    for i in 0..(n_frames as usize - 1) {
        assert_eq!(
            durations[i], per_frame_delays[i],
            "frame {} delay mismatch",
            i
        );
    }
    assert_eq!(
        durations[n_frames as usize - 1],
        oxideav_gif::DEFAULT_DELAY_CS as i64,
        "trailing frame should use default delay"
    );

    // Every decoded frame should match the corresponding input frame's
    // indices (they've been composited onto the canvas, but since
    // x=y=0 and w/h cover the whole canvas, the composite is
    // index-identity).
    for (i, (got, want)) in decoded.iter().zip(input_frames.iter()).enumerate() {
        assert_eq!(got.width, w);
        assert_eq!(got.height, h);
        assert_eq!(
            got.planes[0].data, want.planes[0].data,
            "frame {} indices differ",
            i
        );
    }
}
