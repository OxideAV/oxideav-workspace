//! Integration test against an ffmpeg-generated H.263 elementary-stream
//! reference clip.
//!
//! Generate the fixtures with:
//!
//! ```sh
//! ffmpeg -y -f lavfi -i "testsrc=size=128x96:rate=10:duration=0.1" \
//!     -c:v h263 -qscale:v 5 -an -f h263 /tmp/h263_iframe.es
//! ffmpeg -y -i /tmp/h263_iframe.es -f rawvideo -pix_fmt yuv420p \
//!     /tmp/h263_iframe.yuv
//! ```
//!
//! When the fixtures aren't present the test logs a warning and passes — so
//! CI without ffmpeg still goes green.

use std::path::Path;

use oxideav_codec::Decoder;
use oxideav_core::packet::PacketFlags;
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, TimeBase};
use oxideav_h263::decoder::H263Decoder;

fn read_optional(path: &str) -> Option<Vec<u8>> {
    if !Path::new(path).exists() {
        eprintln!("fixture {path} missing — skipping test");
        return None;
    }
    Some(std::fs::read(path).expect("read fixture"))
}

fn decode_and_compare(es_path: &str, yuv_path: &str, w: u32, h: u32) {
    let Some(es) = read_optional(es_path) else {
        return;
    };
    let Some(ref_yuv) = read_optional(yuv_path) else {
        return;
    };

    let codec_id = CodecId::new(oxideav_h263::CODEC_ID_STR);
    let _params = CodecParameters::video(codec_id.clone());
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

    let frame = decoder.receive_frame().expect("receive first frame");
    let Frame::Video(vf) = frame else {
        panic!("expected video frame");
    };
    assert_eq!(vf.width, w);
    assert_eq!(vf.height, h);

    // Compare per-plane against the 4:2:0 reference. ffmpeg's reference is
    // packed Y, then Cb, then Cr.
    let w = vf.width as usize;
    let h = vf.height as usize;
    let cw = w / 2;
    let ch = h / 2;
    let y_ref = &ref_yuv[..w * h];
    let cb_ref = &ref_yuv[w * h..w * h + cw * ch];
    let cr_ref = &ref_yuv[w * h + cw * ch..];

    let y_plane = &vf.planes[0];
    let cb_plane = &vf.planes[1];
    let cr_plane = &vf.planes[2];

    let mut total = 0usize;
    let mut within = 0usize;
    let mut max_err: u32 = 0;
    let mut compare = |dec: &[u8], stride: usize, refp: &[u8], pw: usize, ph: usize| {
        for row in 0..ph {
            for col in 0..pw {
                let a = dec[row * stride + col] as i32;
                let b = refp[row * pw + col] as i32;
                let d = (a - b).unsigned_abs();
                total += 1;
                if d <= 2 {
                    within += 1;
                }
                if d > max_err {
                    max_err = d;
                }
            }
        }
    };
    compare(&y_plane.data, y_plane.stride, y_ref, w, h);
    compare(&cb_plane.data, cb_plane.stride, cb_ref, cw, ch);
    compare(&cr_plane.data, cr_plane.stride, cr_ref, cw, ch);

    let pct = within as f32 / total as f32;
    eprintln!(
        "h263 {w}x{h}: {within}/{total} pixels within 2 LSB ({:.2}%), max abs err = {max_err}",
        100.0 * pct
    );
    assert!(
        pct >= 0.95,
        "fewer than 95% of pixels within 2 LSB ({:.2}%, max abs err {max_err})",
        100.0 * pct
    );
}

#[test]
fn decode_first_iframe_matches_reference_within_2lsb() {
    decode_and_compare("/tmp/h263_iframe.es", "/tmp/h263_iframe.yuv", 128, 96);
}

#[test]
fn decode_qcif_iframe_matches_reference_within_2lsb() {
    decode_and_compare("/tmp/h263_qcif.es", "/tmp/h263_qcif.yuv", 176, 144);
}

#[test]
fn decode_cif_iframe_matches_reference_within_2lsb() {
    decode_and_compare("/tmp/h263_cif.es", "/tmp/h263_cif.yuv", 352, 288);
}

#[test]
fn decode_q15_qcif_iframe_matches_reference_within_2lsb() {
    decode_and_compare("/tmp/h263_q15.es", "/tmp/h263_q15.yuv", 176, 144);
}
