//! Decode an H.263 I-frame carried inside a 3GP / MP4 container.
//!
//! Generate the fixture with:
//!
//! ```sh
//! ffmpeg -y -f lavfi -i "testsrc=size=128x96:rate=10:duration=0.1" \
//!     -c:v h263 -qscale:v 5 -an /tmp/h263_iframe.3gp
//! ffmpeg -y -i /tmp/h263_iframe.3gp -f rawvideo -pix_fmt yuv420p \
//!     /tmp/h263_iframe_3gp.yuv
//! ```
//!
//! When the fixture is missing the test logs a warning and passes.

use std::path::Path;

use oxideav_codec::Decoder;
use oxideav_core::packet::PacketFlags;
use oxideav_core::{CodecId, Frame, Packet, TimeBase};
use oxideav_h263::decoder::H263Decoder;

fn read_optional(path: &str) -> Option<Vec<u8>> {
    if !Path::new(path).exists() {
        eprintln!("fixture {path} missing — skipping test");
        return None;
    }
    Some(std::fs::read(path).expect("read fixture"))
}

/// Walk the MP4 bytes by hand to extract the first sample of the H.263
/// track. We do this without depending on `oxideav-mp4` to keep the test
/// self-contained.
fn first_h263_sample_from_3gp(data: &[u8]) -> Option<Vec<u8>> {
    // The 3GP file has a `mdat` box near the start (or after `moov`). The
    // first H.263 sample begins with a PSC (`00 00 80...`). Find that.
    for i in 0..data.len().saturating_sub(3) {
        if data[i] == 0x00 && data[i + 1] == 0x00 && (data[i + 2] & 0xFC) == 0x80 {
            // We need the END too — find the next PSC after this one or use
            // a reasonable cap.
            let mut end = data.len();
            for j in (i + 3)..data.len().saturating_sub(3) {
                if data[j] == 0x00 && data[j + 1] == 0x00 && (data[j + 2] & 0xFC) == 0x80 {
                    end = j;
                    break;
                }
            }
            return Some(data[i..end].to_vec());
        }
    }
    None
}

#[test]
fn decode_3gp_h263_iframe_via_decoder() {
    let Some(file) = read_optional("/tmp/h263_iframe.3gp") else {
        return;
    };
    let Some(es) = first_h263_sample_from_3gp(&file) else {
        eprintln!("could not locate first H.263 sample inside 3GP");
        return;
    };

    let codec_id = CodecId::new(oxideav_h263::CODEC_ID_STR);
    let mut decoder = H263Decoder::new(codec_id);
    decoder
        .send_packet(&Packet {
            stream_index: 0,
            data: es,
            pts: Some(0),
            dts: Some(0),
            duration: None,
            time_base: TimeBase::new(1, 90_000),
            flags: PacketFlags {
                keyframe: true,
                ..PacketFlags::default()
            },
        })
        .expect("send_packet");
    decoder.flush().unwrap();

    let frame = decoder.receive_frame().expect("receive_frame");
    let Frame::Video(vf) = frame else {
        panic!("expected video");
    };
    assert_eq!(vf.width, 128);
    assert_eq!(vf.height, 96);
    assert_eq!(vf.planes.len(), 3);
    // Sanity check — the first pixel of the testsrc is a non-black colour.
    let any_nonzero = vf.planes[0].data.iter().any(|&p| p != 0);
    assert!(any_nonzero, "decoded Y plane is all zeros");
}
