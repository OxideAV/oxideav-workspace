//! Integration tests for the AVI demuxer's `seek_to` (idx1-backed).
//!
//! We build a small AVI in-memory via the muxer (which always emits idx1),
//! reopen it with the demuxer, seek mid-video, and assert the next packet
//! lands at a pts ≥ the landed timestamp.

use std::io::Cursor;

use oxideav_container::{ReadSeek, WriteSeek};
use oxideav_core::{
    CodecId, CodecParameters, Error, MediaType, Packet, PixelFormat, Rational, SampleFormat,
    StreamInfo, TimeBase,
};

/// Build a deterministic JPEG-ish payload. The demuxer round-trips the bytes
/// verbatim so we don't actually need a valid JPEG — just something unique
/// per-frame to make PTS ordering visible.
fn fake_video_packet(i: u32) -> Vec<u8> {
    // Use a valid-looking 2-byte SOI prefix (0xFFD8) so the muxer's codec
    // checks (if any) don't complain. The rest is per-frame counter.
    let mut v = vec![0xFFu8, 0xD8];
    v.extend_from_slice(&i.to_le_bytes());
    // Pad to an even length so no pad byte hides in the middle of the index.
    v.extend_from_slice(&[0u8; 14]);
    v
}

fn pcm_payload(frames: usize, phase: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        let l = (i as u16).wrapping_add(phase) as i16;
        let r = (i as u16).wrapping_mul(3).wrapping_add(phase) as i16;
        out.extend_from_slice(&l.to_le_bytes());
        out.extend_from_slice(&r.to_le_bytes());
    }
    out
}

#[test]
fn seek_to_video_midway_lands_on_keyframe() {
    // Two streams: video (MJPEG, 25 fps) + audio (PCM s16 stereo, 48 kHz).
    let time_base_v = TimeBase::new(1, 25);
    let mut vparams = CodecParameters::video(CodecId::new("mjpeg"));
    vparams.media_type = MediaType::Video;
    vparams.width = Some(64);
    vparams.height = Some(64);
    vparams.pixel_format = Some(PixelFormat::Yuv420P);
    vparams.frame_rate = Some(Rational::new(25, 1));
    let video_stream = StreamInfo {
        index: 0,
        time_base: time_base_v,
        duration: None,
        start_time: Some(0),
        params: vparams,
    };

    let time_base_a = TimeBase::new(1, 48_000);
    let mut aparams = CodecParameters::audio(CodecId::new("pcm_s16le"));
    aparams.channels = Some(2);
    aparams.sample_rate = Some(48_000);
    aparams.sample_format = Some(SampleFormat::S16);
    let audio_stream = StreamInfo {
        index: 1,
        time_base: time_base_a,
        duration: None,
        start_time: Some(0),
        params: aparams,
    };

    let streams = [video_stream.clone(), audio_stream.clone()];

    // Mux into an in-memory buffer. All video frames flagged as keyframes
    // (every MJPEG frame is a keyframe). Use a temp file so we can hand
    // the demuxer a fresh `Box<dyn ReadSeek>` independent of the writer.
    let tmp = std::env::temp_dir().join("oxideav-avi-seek-video.avi");
    {
        let f = std::fs::File::create(&tmp).unwrap();
        let writer: Box<dyn WriteSeek> = Box::new(f);
        let mut mux = oxideav_avi::muxer::open(writer, &streams).unwrap();
        mux.write_header().unwrap();

        // 10 video frames + matching audio packets per frame (1920 samples
        // per frame at 48 kHz / 25 fps = one video-frame worth of audio).
        for i in 0..10u32 {
            let vdata = fake_video_packet(i);
            let mut vpkt = Packet::new(0, time_base_v, vdata);
            vpkt.pts = Some(i as i64);
            vpkt.flags.keyframe = true;
            mux.write_packet(&vpkt).unwrap();

            let adata = pcm_payload(1920, i as u16);
            let mut apkt = Packet::new(1, time_base_a, adata);
            apkt.pts = Some((i as i64) * 1920);
            apkt.flags.keyframe = true;
            mux.write_packet(&apkt).unwrap();
        }
        mux.write_trailer().unwrap();
    }

    // Demux and seek.
    let reader: Box<dyn ReadSeek> = Box::new(std::fs::File::open(&tmp).unwrap());
    let mut dmx = oxideav_avi::demuxer::open(reader).unwrap();
    assert_eq!(dmx.streams().len(), 2);

    // Seek video stream to pts 5. The muxer flagged every video chunk as a
    // keyframe, so the landed pts should equal 5.
    let landed = dmx.seek_to(0, 5).expect("seek_to must succeed with idx1");
    assert_eq!(landed, 5, "expected exact landing on frame-5 keyframe");

    // Next packet pulled from the demuxer is on stream 0 with pts ≥ landed.
    // (In this stream layout the next chunk after a video keyframe is the
    // video chunk itself — interleaving is video/audio/video/audio/... in
    // the muxer, so offset of idx[i].video precedes idx[i].audio.)
    let mut saw_video_at_or_after = false;
    for _ in 0..4 {
        let pkt = dmx.next_packet().expect("packet after seek");
        if pkt.stream_index == 0 {
            let pts = pkt.pts.expect("pts set");
            assert!(pts >= landed, "post-seek video pts {pts} < landed {landed}");
            saw_video_at_or_after = true;
            break;
        }
    }
    assert!(
        saw_video_at_or_after,
        "did not observe a video packet at or after landed pts"
    );
}

#[test]
fn seek_to_without_idx1_is_unsupported() {
    // Build a minimal AVI, then strip the idx1 chunk to simulate a file
    // written by a muxer that didn't emit a legacy index.
    let mut vparams = CodecParameters::video(CodecId::new("mjpeg"));
    vparams.media_type = MediaType::Video;
    vparams.width = Some(32);
    vparams.height = Some(32);
    vparams.pixel_format = Some(PixelFormat::Yuv420P);
    vparams.frame_rate = Some(Rational::new(25, 1));
    let stream = StreamInfo {
        index: 0,
        time_base: TimeBase::new(1, 25),
        duration: None,
        start_time: Some(0),
        params: vparams,
    };

    let tmp = std::env::temp_dir().join("oxideav-avi-seek-noidx.avi");
    {
        let f = std::fs::File::create(&tmp).unwrap();
        let writer: Box<dyn WriteSeek> = Box::new(f);
        let mut mux = oxideav_avi::muxer::open(writer, std::slice::from_ref(&stream)).unwrap();
        mux.write_header().unwrap();
        let mut pkt = Packet::new(0, stream.time_base, fake_video_packet(0));
        pkt.pts = Some(0);
        pkt.flags.keyframe = true;
        mux.write_packet(&pkt).unwrap();
        mux.write_trailer().unwrap();
    }

    // Zero out the "idx1" FourCC so the demuxer walks past it as an
    // unknown chunk. We don't alter the size field — the demuxer will
    // skip the body either way.
    let mut backing = std::fs::read(&tmp).unwrap();
    let mut pos = None;
    for (i, w) in backing.windows(4).enumerate() {
        if w == b"idx1" {
            pos = Some(i);
            break;
        }
    }
    let pos = pos.expect("idx1 must have been written by muxer");
    backing[pos..pos + 4].copy_from_slice(b"JUNK");

    let reader: Box<dyn ReadSeek> = Box::new(Cursor::new(backing));
    let mut dmx = oxideav_avi::demuxer::open(reader).unwrap();
    match dmx.seek_to(0, 0) {
        Err(Error::Unsupported(_)) => {}
        other => panic!("expected Unsupported without idx1, got {other:?}"),
    }
}
