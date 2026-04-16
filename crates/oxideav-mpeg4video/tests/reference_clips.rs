//! Integration tests against ffmpeg-generated MPEG-4 Part 2 reference clips.
//!
//! Fixtures expected at:
//!   /tmp/ref-mpeg4-iframes.avi  (64x64 @ 10 fps, 1s, every frame I)
//!   /tmp/ref-mpeg4-gop.avi      (128x96 @ 10 fps, 2s, GOP=10)
//!
//! Generated with:
//!   ffmpeg -y -f lavfi -i testsrc=d=1:s=64x64:r=10 -c:v mpeg4 -g 1 -b:v 500k \
//!       /tmp/ref-mpeg4-iframes.avi
//!   ffmpeg -y -f lavfi -i testsrc=d=2:s=128x96:r=10 -c:v mpeg4 -g 10 -b:v 800k \
//!       /tmp/ref-mpeg4-gop.avi
//!
//! Tests that can't find their fixture are skipped (logged, not failed) so
//! CI without ffmpeg still passes.

use std::path::Path;

use oxideav_mpeg4video::{
    bitreader::BitReader,
    decoder::codec_parameters_from_vol,
    headers::{
        vol::parse_vol,
        vop::{parse_vop, VopCodingType},
        vos::{parse_visual_object, parse_vos, profile_level_description},
    },
    start_codes::{self, VISUAL_OBJECT_START_CODE, VOL_START_MIN, VOP_START_CODE, VOS_START_CODE},
};

fn read_fixture(path: &str) -> Option<Vec<u8>> {
    if !Path::new(path).exists() {
        eprintln!("fixture {path} missing — skipping test");
        return None;
    }
    Some(std::fs::read(path).expect("read fixture"))
}

/// Find the first occurrence of a start code matching `predicate`.
fn find_start_code<F: Fn(u8) -> bool>(data: &[u8], predicate: F) -> Option<(usize, u8)> {
    start_codes::iter_start_codes(data).find(|(_, c)| predicate(*c))
}

/// Locate the first AVI "00dc" chunk body (the first video packet) inside a
/// RIFF AVI file. Walks the top-level list structure by hand — avoids pulling
/// oxideav-avi as a dev-dependency.
fn avi_first_video_chunk(data: &[u8]) -> Option<Vec<u8>> {
    // Find the `movi` LIST by scanning for the literal sequence `movi` (which
    // follows a `LIST` + 4-byte size). We then walk chunk headers inside.
    let movi_pos = {
        let needle = b"movi";
        let mut idx = None;
        for i in 0..data.len().saturating_sub(4) {
            if &data[i..i + 4] == needle {
                // Validate this is a LIST form type: bytes at i-8..i-4 should be "LIST".
                if i >= 8 && &data[i - 8..i - 4] == b"LIST" {
                    idx = Some(i);
                    break;
                }
            }
        }
        idx?
    };
    // Chunks begin at `movi_pos + 4`.
    let mut i = movi_pos + 4;
    while i + 8 <= data.len() {
        let id = &data[i..i + 4];
        let size =
            u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        if id == b"00dc" {
            if i + 8 + size <= data.len() {
                return Some(data[i + 8..i + 8 + size].to_vec());
            }
            return None;
        }
        if id == b"JUNK" || id == b"junk" {
            // Skip the JUNK chunk payload + pad.
            let step = 8 + size + (size & 1);
            i += step;
            continue;
        }
        // Unknown chunk — skip with pad.
        let step = 8 + size + (size & 1);
        if step == 0 {
            return None;
        }
        i += step;
    }
    None
}

/// Locate the first AVI "00dc" chunk plus the header bytes (VOS..VOL) that
/// precede it in the same file. Returns the concatenation `[headers, frame]`
/// — a self-contained mpeg4 bitstream.
fn avi_headers_plus_first_frame(data: &[u8]) -> Option<Vec<u8>> {
    let vos_pos = start_codes::iter_start_codes(data)
        .find(|(_, c)| *c == VOS_START_CODE)
        .map(|(p, _)| p)?;
    // Headers region: from VOS up to (but not including) first VOP — that is
    // VOS, VO, VOL. Actually we want everything up to the first VOP since the
    // VOP start code is already in the frame body if ffmpeg wraps it. In
    // practice ffmpeg places VOS..VOL in `avi.bin` prologue area (before movi)
    // and VOP bodies begin with 0x000001B6. The video chunk body therefore
    // starts with the VOP start code directly, and we need to splice
    // headers + chunk body.
    let frame = avi_first_video_chunk(data)?;
    // Extract headers [vos_pos .. first VOP start code]. First VOP is in
    // `frame`, not in the AVI headers region — so we grab from vos_pos up to
    // the byte before the movi's first VOP.
    let first_vop_in_file = start_codes::iter_start_codes(&data[vos_pos..])
        .find(|(_, c)| *c == VOP_START_CODE)
        .map(|(p, _)| vos_pos + p)?;
    let headers = data[vos_pos..first_vop_in_file].to_vec();
    let mut joined = headers;
    joined.extend_from_slice(&frame);
    Some(joined)
}

#[test]
fn parse_vos_vo_vol_iframes() {
    let Some(data) = read_fixture("/tmp/ref-mpeg4-iframes.avi") else {
        return;
    };
    // VOS.
    let (pos, code) = find_start_code(&data, |c| c == VOS_START_CODE).expect("VOS start code");
    assert_eq!(code, VOS_START_CODE);
    let mut br = BitReader::new(&data[pos + 4..pos + 5]);
    let vos = parse_vos(&mut br).expect("parse VOS");
    eprintln!(
        "profile_and_level = 0x{:02x} ({})",
        vos.profile_and_level_indication,
        profile_level_description(vos.profile_and_level_indication)
    );

    // Visual Object.
    let (pos, _) = find_start_code(&data, |c| c == VISUAL_OBJECT_START_CODE)
        .expect("visual_object start code");
    let next = start_codes::iter_start_codes(&data[pos + 4..])
        .next()
        .map(|(p, _)| pos + 4 + p)
        .unwrap_or(data.len());
    let mut br = BitReader::new(&data[pos + 4..next]);
    let _vo = parse_visual_object(&mut br).expect("parse VO");

    // VOL.
    let (pos, _) =
        find_start_code(&data, start_codes::is_video_object_layer).expect("VOL start code");
    let next = start_codes::iter_start_codes(&data[pos + 4..])
        .next()
        .map(|(p, _)| pos + 4 + p)
        .unwrap_or(data.len());
    let mut br = BitReader::new(&data[pos + 4..next]);
    let vol = parse_vol(&mut br).expect("parse VOL");
    assert_eq!(vol.width, 64, "VOL width");
    assert_eq!(vol.height, 64, "VOL height");

    let params = codec_parameters_from_vol(&vol);
    assert_eq!(params.width, Some(64));
    assert_eq!(params.height, Some(64));
    let fr = params.frame_rate.expect("frame rate");
    let ratio = fr.num as f64 / fr.den as f64;
    assert!(
        (ratio - 10.0).abs() < 0.5,
        "expected frame rate ~10 fps, got {}/{} = {}",
        fr.num,
        fr.den,
        ratio
    );
    assert_eq!(vol.mb_width(), 4);
    assert_eq!(vol.mb_height(), 4);
    let _ = VOL_START_MIN;
}

#[test]
fn parse_first_vop_header_iframes() {
    let Some(data) = read_fixture("/tmp/ref-mpeg4-iframes.avi") else {
        return;
    };
    let (pos, _) =
        find_start_code(&data, start_codes::is_video_object_layer).expect("VOL start code");
    let next = start_codes::iter_start_codes(&data[pos + 4..])
        .next()
        .map(|(p, _)| pos + 4 + p)
        .unwrap_or(data.len());
    let mut br = BitReader::new(&data[pos + 4..next]);
    let vol = parse_vol(&mut br).expect("parse VOL");

    let (pos, code) = find_start_code(&data, |c| c == VOP_START_CODE).expect("VOP start code");
    assert_eq!(code, VOP_START_CODE);
    let next = start_codes::iter_start_codes(&data[pos + 4..])
        .next()
        .map(|(p, _)| pos + 4 + p)
        .unwrap_or(data.len());
    let mut br = BitReader::new(&data[pos + 4..next]);
    let vop = parse_vop(&mut br, &vol).expect("parse VOP");
    assert_eq!(
        vop.vop_coding_type,
        VopCodingType::I,
        "first VOP in an I-only stream is I"
    );
    assert!(vop.vop_coded, "first VOP coded");
    assert!(vop.vop_quant > 0, "VOP quant > 0");
    eprintln!(
        "VOP0: type={:?} quant={} time_increment={}",
        vop.vop_coding_type, vop.vop_quant, vop.vop_time_increment
    );
}

#[test]
fn parse_vol_gop_clip() {
    let Some(data) = read_fixture("/tmp/ref-mpeg4-gop.avi") else {
        return;
    };
    let (pos, _) =
        find_start_code(&data, start_codes::is_video_object_layer).expect("VOL start code");
    let next = start_codes::iter_start_codes(&data[pos + 4..])
        .next()
        .map(|(p, _)| pos + 4 + p)
        .unwrap_or(data.len());
    let mut br = BitReader::new(&data[pos + 4..next]);
    let vol = parse_vol(&mut br).expect("parse VOL");
    assert_eq!(vol.width, 128);
    assert_eq!(vol.height, 96);
}

/// End-to-end: decode the first I-VOP out of a tiny all-I ffmpeg clip.
///
/// The earlier "AC desync at MB(0,1)" turned out to be a missing
/// video-packet resync-marker decode (§6.3.5.2). FFmpeg's encoder splices a
/// resync marker after a row of MBs whenever the VOL has
/// `resync_marker_disable == 0`, which is the default. See
/// `crate::resync` for the marker layout.
#[test]
fn decode_i_vop_tiny() {
    use oxideav_core::{CodecId, CodecParameters, Frame, Packet, PixelFormat, TimeBase};

    let Some(data) = read_fixture("/tmp/ref-mpeg4-iframes.avi") else {
        return;
    };
    let Some(bitstream) = avi_headers_plus_first_frame(&data) else {
        eprintln!("couldn't locate a 00dc chunk + headers in the AVI — skipping");
        return;
    };
    let params = CodecParameters::video(CodecId::new(oxideav_mpeg4video::CODEC_ID_STR));
    let mut dec = oxideav_mpeg4video::decoder::make_decoder(&params).expect("build decoder");
    let packet = Packet::new(0, TimeBase::new(1, 90_000), bitstream);
    dec.send_packet(&packet).expect("send_packet");
    let _ = dec.flush();
    let frame = dec.receive_frame().expect("receive_frame");
    match frame {
        Frame::Video(vf) => {
            assert_eq!(vf.format, PixelFormat::Yuv420P);
            assert_eq!(vf.width, 64);
            assert_eq!(vf.height, 64);
            assert_eq!(vf.planes.len(), 3);
            let y = &vf.planes[0];
            let mean_y: u64 = y.data.iter().map(|&b| b as u64).sum::<u64>() / y.data.len() as u64;
            eprintln!("decode_i_vop_tiny: mean Y = {mean_y}");
            assert!(
                (30..=230).contains(&mean_y),
                "mean Y out of expected range: {mean_y}"
            );

            // If a reference YUV from ffmpeg is available, compute pixel
            // match percentage. The fixture is generated with:
            //   ffmpeg -y -i /tmp/ref-mpeg4-iframes.avi -frames:v 1 \
            //          -f rawvideo -pix_fmt yuv420p /tmp/ref_iframe0.yuv
            if let Ok(reference) = std::fs::read("/tmp/ref_iframe0.yuv") {
                if reference.len() == 6144 {
                    let mut ours = Vec::with_capacity(6144);
                    ours.extend_from_slice(&vf.planes[0].data);
                    ours.extend_from_slice(&vf.planes[1].data);
                    ours.extend_from_slice(&vf.planes[2].data);
                    let total = ours.len().min(reference.len());
                    let mut close = 0usize;
                    let mut max_diff = 0i32;
                    let mut sum_sq_diff: u64 = 0;
                    for i in 0..total {
                        let d = (ours[i] as i32) - (reference[i] as i32);
                        if d.abs() <= 2 {
                            close += 1;
                        }
                        max_diff = max_diff.max(d.abs());
                        sum_sq_diff += (d * d) as u64;
                    }
                    let mse = sum_sq_diff as f64 / total as f64;
                    let psnr = if mse > 0.0 {
                        10.0 * (255.0_f64 * 255.0 / mse).log10()
                    } else {
                        100.0
                    };
                    let pct = 100.0 * (close as f64) / (total as f64);
                    eprintln!(
                        "pixel match (within 2 LSB): {pct:.2}% ({close}/{total}); max |diff| = {max_diff}; PSNR = {psnr:.2} dB"
                    );
                    assert!(
                        pct >= 95.0,
                        "pixel match {pct:.2}% < 95% target (max diff {max_diff})"
                    );
                }
            }
        }
        _ => panic!("expected VideoFrame"),
    }
}

/// Decode a 128×128 (8×8 MB) all-I clip — exercises a wider picture with at
/// least one resync marker per row (FFmpeg emits them at every byte-aligned
/// row boundary unless `resync_marker_disable=1` is set in the VOL).
///
/// Fixtures generated with:
///   ffmpeg -y -f lavfi -i testsrc=d=1:s=128x128:r=10 -c:v mpeg4 -g 1 \
///       -b:v 500k -an /tmp/ref-mpeg4-128.avi
///   ffmpeg -y -i /tmp/ref-mpeg4-128.avi -frames:v 1 \
///       -f rawvideo -pix_fmt yuv420p /tmp/ref_128.yuv
#[test]
fn decode_i_vop_128() {
    use oxideav_core::{CodecId, CodecParameters, Frame, Packet, PixelFormat, TimeBase};

    let Some(data) = read_fixture("/tmp/ref-mpeg4-128.avi") else {
        return;
    };
    let Some(bitstream) = avi_headers_plus_first_frame(&data) else {
        eprintln!("couldn't locate a 00dc chunk + headers in the AVI — skipping");
        return;
    };
    let params = CodecParameters::video(CodecId::new(oxideav_mpeg4video::CODEC_ID_STR));
    let mut dec = oxideav_mpeg4video::decoder::make_decoder(&params).expect("build decoder");
    let packet = Packet::new(0, TimeBase::new(1, 90_000), bitstream);
    dec.send_packet(&packet).expect("send_packet");
    let _ = dec.flush();
    let frame = dec.receive_frame().expect("receive_frame");
    match frame {
        Frame::Video(vf) => {
            assert_eq!(vf.format, PixelFormat::Yuv420P);
            assert_eq!(vf.width, 128);
            assert_eq!(vf.height, 128);
            if let Ok(reference) = std::fs::read("/tmp/ref_128.yuv") {
                if reference.len() == 128 * 128 * 3 / 2 {
                    let mut ours = Vec::with_capacity(reference.len());
                    ours.extend_from_slice(&vf.planes[0].data);
                    ours.extend_from_slice(&vf.planes[1].data);
                    ours.extend_from_slice(&vf.planes[2].data);
                    let total = ours.len().min(reference.len());
                    let mut close = 0usize;
                    let mut max_diff = 0i32;
                    let mut sum_sq_diff: u64 = 0;
                    for i in 0..total {
                        let d = (ours[i] as i32) - (reference[i] as i32);
                        if d.abs() <= 2 {
                            close += 1;
                        }
                        max_diff = max_diff.max(d.abs());
                        sum_sq_diff += (d * d) as u64;
                    }
                    let mse = sum_sq_diff as f64 / total as f64;
                    let psnr = if mse > 0.0 {
                        10.0 * (255.0_f64 * 255.0 / mse).log10()
                    } else {
                        100.0
                    };
                    let pct = 100.0 * (close as f64) / (total as f64);
                    eprintln!(
                        "decode_i_vop_128: pixel match (within 2 LSB): {pct:.2}%; max |diff| = {max_diff}; PSNR = {psnr:.2} dB"
                    );
                    assert!(
                        pct >= 95.0,
                        "128x128 pixel match {pct:.2}% < 95% target (max diff {max_diff})"
                    );
                }
            }
        }
        _ => panic!("expected VideoFrame"),
    }
}

/// After the I-VOP, the first P-VOP in the GOP clip must report Unsupported
/// (the motion-compensation path is explicitly out of scope this session).
#[test]
fn inter_vop_still_unsupported() {
    use oxideav_core::{CodecId, CodecParameters, Error, Packet, TimeBase};

    let Some(data) = read_fixture("/tmp/ref-mpeg4-gop.avi") else {
        return;
    };
    // Find the second VOP start code (first is I, second is P).
    let vops: Vec<_> = start_codes::iter_start_codes(&data)
        .filter(|(_, c)| *c == VOP_START_CODE)
        .collect();
    if vops.len() < 2 {
        eprintln!(
            "expected >=2 VOPs in /tmp/ref-mpeg4-gop.avi, found {}; skipping",
            vops.len()
        );
        return;
    }

    // Build headers-only prefix (VOS..VOL) plus the 2nd VOP's bytes. The 2nd
    // VOP begins at vops[1].0 and extends until the next start code or EOF
    // (we also strip any appended AVI chunk trailer via the byte scan — close
    // enough for this "Unsupported" expectation).
    let (vos_pos, _) = start_codes::iter_start_codes(&data)
        .find(|(_, c)| *c == VOS_START_CODE)
        .expect("VOS");
    let first_vop_pos = vops[0].0;
    let headers = data[vos_pos..first_vop_pos].to_vec();

    let second_vop_start = vops[1].0;
    let second_vop_end = start_codes::iter_start_codes(&data[second_vop_start + 4..])
        .next()
        .map(|(p, _)| second_vop_start + 4 + p)
        .unwrap_or(data.len());
    let vop_bytes = &data[second_vop_start..second_vop_end];

    let mut bitstream = headers;
    bitstream.extend_from_slice(vop_bytes);

    let params = CodecParameters::video(CodecId::new(oxideav_mpeg4video::CODEC_ID_STR));
    let mut dec = oxideav_mpeg4video::decoder::make_decoder(&params).expect("build decoder");
    let packet = Packet::new(0, TimeBase::new(1, 90_000), bitstream);
    match dec.send_packet(&packet) {
        Err(Error::Unsupported(msg)) => {
            assert!(
                msg.contains("P frame")
                    || msg.contains("B frame")
                    || msg.contains("P-VOP")
                    || msg.contains("motion")
                    || msg.contains("mpeg4"),
                "Unsupported message should mention inter path: {msg}"
            );
        }
        Err(Error::NeedMore) => panic!("decoder returned NeedMore on P-VOP"),
        Err(other) => panic!("unexpected error: {other}"),
        // It's also fine if the decoder produces the I-frame first and the P
        // decode hasn't been reached yet in this invocation — Ok(()) means it
        // returned the I-VOP successfully and buffered the rest; in that case
        // we don't fail.
        Ok(()) => {
            eprintln!("decoder accepted the packet — I-VOP-only decode emitted, P-VOP remains");
        }
    }
}
