//! Round-trip a synthetic 128x128 Pal8 frame through the encoder +
//! container + demuxer + decoder and confirm palette + indices survive.

mod common;

use std::io::Cursor;

use oxideav_codec::CodecRegistry;
use oxideav_container::{ContainerRegistry, WriteSeek};
use oxideav_core::{
    CodecId, CodecParameters, Frame, MediaType, PixelFormat, TimeBase, VideoFrame, VideoPlane,
};
use oxideav_gif::{register_codecs, register_containers, GIF_CODEC_ID};

use common::SharedSink;

fn build_palette(n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(256 * 4);
    for i in 0..256 {
        if i < n {
            let r = ((i * 37) & 0xFF) as u8;
            let g = ((i * 71) & 0xFF) as u8;
            let b = ((i * 113) & 0xFF) as u8;
            out.extend_from_slice(&[r, g, b, 0xFF]);
        } else {
            out.extend_from_slice(&[0, 0, 0, 0xFF]);
        }
    }
    out
}

fn build_pal8_frame(w: u32, h: u32, n_colors: usize) -> VideoFrame {
    let mut indices = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = ((x ^ y) as usize) % n_colors;
            indices.push(v as u8);
        }
    }
    let palette = build_palette(n_colors);
    VideoFrame {
        format: PixelFormat::Pal8,
        width: w,
        height: h,
        pts: Some(0),
        time_base: TimeBase::new(1, 100),
        planes: vec![
            VideoPlane {
                stride: w as usize,
                data: indices,
            },
            VideoPlane {
                stride: 256 * 4,
                data: palette,
            },
        ],
    }
}

#[test]
fn static_gif_preserves_indices_and_palette() {
    let w = 128u32;
    let h = 128u32;
    let n_colors = 32;
    let frame_in = build_pal8_frame(w, h, n_colors);

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
    encoder
        .send_frame(&Frame::Video(frame_in.clone()))
        .expect("send");
    encoder.flush().expect("flush");
    let pkt = encoder.receive_packet().expect("pkt");
    let encoder_params = encoder.output_params().clone();

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
        muxer.write_packet(&pkt).expect("pkt");
        muxer.write_trailer().expect("trl");
    }
    let buf: Vec<u8> = sink_data.lock().unwrap().clone();

    assert!(buf.starts_with(b"GIF89a"), "output is not a GIF89a");
    assert_eq!(
        buf.last().copied(),
        Some(0x3B),
        "file does not end with the GIF trailer"
    );

    let cursor = Cursor::new(buf.clone());
    let boxed: Box<dyn oxideav_container::ReadSeek> = Box::new(cursor);
    let mut demuxer = containers.open_demuxer("gif", boxed).expect("demux");
    let si = demuxer.streams()[0].clone();
    assert_eq!(si.params.width, Some(w));
    assert_eq!(si.params.height, Some(h));

    let mut decoder = codecs.make_decoder(&si.params).expect("decoder");
    let out_pkt = demuxer.next_packet().expect("next_packet");
    decoder.send_packet(&out_pkt).expect("send");
    let out_frame = match decoder.receive_frame().expect("recv") {
        Frame::Video(v) => v,
        _ => panic!("non-video frame"),
    };

    assert_eq!(out_frame.format, PixelFormat::Pal8);
    assert_eq!(out_frame.width, w);
    assert_eq!(out_frame.height, h);

    let indices_in = &frame_in.planes[0].data;
    let indices_out = &out_frame.planes[0].data;
    assert_eq!(
        indices_in.len(),
        indices_out.len(),
        "index plane length differs"
    );
    for y in 0..h as usize {
        for x in 0..w as usize {
            let a = indices_in[y * w as usize + x];
            let b = indices_out[y * w as usize + x];
            assert_eq!(a, b, "pixel ({}, {}) differs: in={} out={}", x, y, a, b);
        }
    }

    let pal_in = &frame_in.planes[1].data;
    let pal_out = &out_frame.planes[1].data;
    for i in 0..n_colors {
        let off = i * 4;
        assert_eq!(
            &pal_in[off..off + 3],
            &pal_out[off..off + 3],
            "palette entry {} RGB differs",
            i
        );
    }
}
