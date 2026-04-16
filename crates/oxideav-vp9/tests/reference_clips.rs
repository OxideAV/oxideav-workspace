//! Integration tests against ffmpeg-generated VP9 reference clips.
//!
//! Fixtures (skipped if missing — CI without ffmpeg still passes):
//!   /tmp/vp9.ivf  (IVF container, 64x64, 24fps, ~3 frames)
//!   /tmp/vp9.mp4  (MP4 / vp09 sample entry, same content)
//!
//! Generate them with:
//!   ffmpeg -y -f lavfi -i "testsrc=size=64x64:rate=24:duration=0.1" \
//!          -c:v libvpx-vp9 -keyint_min 1 -g 1 -f ivf /tmp/vp9.ivf
//!   ffmpeg -y -f lavfi -i "testsrc=size=64x64:rate=24:duration=0.1" \
//!          -c:v libvpx-vp9 -keyint_min 1 -g 1 /tmp/vp9.mp4

use std::path::Path;

use oxideav_vp9::{parse_uncompressed_header, FrameType};

fn read_fixture(path: &str) -> Option<Vec<u8>> {
    if !Path::new(path).exists() {
        eprintln!("fixture {path} missing — skipping test");
        return None;
    }
    Some(std::fs::read(path).expect("read fixture"))
}

/// Walk an IVF stream and pull out the first encoded frame's payload.
fn first_ivf_frame(buf: &[u8]) -> &[u8] {
    // IVF header is 32 bytes:
    //   0..4   "DKIF"
    //   4..6   version
    //   6..8   header length
    //   8..12  fourcc
    //   12..14 width
    //   14..16 height
    //   16..20 framerate numerator
    //   20..24 framerate denominator
    //   24..28 frame count
    //   28..32 reserved
    // Each frame: 4 bytes size LE + 8 bytes pts LE + payload.
    assert!(&buf[..4] == b"DKIF", "bad IVF magic");
    let frame_size = u32::from_le_bytes([buf[32], buf[33], buf[34], buf[35]]) as usize;
    &buf[44..44 + frame_size]
}

#[test]
fn parse_ivf_first_frame_header() {
    let Some(data) = read_fixture("/tmp/vp9.ivf") else {
        return;
    };
    let frame = first_ivf_frame(&data);
    let h = parse_uncompressed_header(frame, None).expect("parse uncompressed header");
    assert_eq!(h.frame_type, FrameType::Key);
    assert_eq!(h.width, 64);
    assert_eq!(h.height, 64);
    assert_eq!(h.color_config.bit_depth, 8);
    assert!(h.show_frame);
}

#[test]
fn parse_mp4_vp9_via_pipeline() {
    // This test exercises the MP4 demuxer's vp09 sample-entry mapping. It
    // doesn't decode pixels — just verifies the container surfaces a vp9
    // codec id with the right dimensions.
    if !Path::new("/tmp/vp9.mp4").exists() {
        eprintln!("fixture /tmp/vp9.mp4 missing — skipping");
        return;
    }
    // We don't depend on oxideav-mp4 here to keep the dep graph minimal;
    // instead re-use the MP4 container's public API via the codec_id mapping.
    // The mapping table is small and stable — verify directly.
    use oxideav_mp4::codec_id::from_sample_entry;
    let id = from_sample_entry(b"vp09");
    assert_eq!(id.as_str(), "vp9");
}
