//! Integration tests against ffmpeg-generated H.264 reference clips.
//!
//! Fixtures expected at:
//!   /tmp/h264_iframe.mp4   (128x96 baseline, every-frame I, MP4 / avcC)
//!   /tmp/h264.es           (same content, Annex B elementary stream)
//!   /tmp/h264_iframe.yuv   (raw YUV420P decoded reference)
//!
//! Generated with:
//!   ffmpeg -y -f lavfi -i "testsrc=size=128x96:rate=24:duration=0.1" \
//!       -pix_fmt yuv420p -c:v libx264 -profile:v baseline -g 1 \
//!       /tmp/h264_iframe.mp4
//!   ffmpeg -y -i /tmp/h264_iframe.mp4 -c:v copy -bsf:v h264_mp4toannexb \
//!       -f h264 /tmp/h264.es
//!   ffmpeg -y -i /tmp/h264_iframe.mp4 -f rawvideo -pix_fmt yuv420p \
//!       /tmp/h264_iframe.yuv
//!
//! Tests skip (with a printed note) when their fixture is missing so CI
//! without ffmpeg still passes.

use std::path::Path;

use oxideav_h264::nal::{extract_rbsp, split_annex_b, AvcConfig, NalHeader, NalUnitType};
use oxideav_h264::pps::parse_pps;
use oxideav_h264::sps::parse_sps;

fn read_fixture(path: &str) -> Option<Vec<u8>> {
    if !Path::new(path).exists() {
        eprintln!("fixture {path} missing — skipping test");
        return None;
    }
    Some(std::fs::read(path).expect("read fixture"))
}

fn find_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut i = 0;
    while i + 8 <= data.len() {
        let size = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        let kind = &data[i + 4..i + 8];
        if kind == fourcc {
            let end = i + size.max(8);
            if end > data.len() {
                return Some(&data[i + 8..]);
            }
            return Some(&data[i + 8..end]);
        }
        if size < 8 {
            return None;
        }
        i += size;
    }
    None
}

/// Walk into the MP4 box hierarchy moov/trak/mdia/minf/stbl/stsd to find
/// the avc1 sample entry, then recover the avcC body. Hand-rolled to avoid
/// depending on `oxideav-mp4` in unit-test territory.
fn extract_avcc_from_mp4(data: &[u8]) -> Option<Vec<u8>> {
    let moov = find_box(data, b"moov")?;
    let trak = find_box(moov, b"trak")?;
    let mdia = find_box(trak, b"mdia")?;
    let minf = find_box(mdia, b"minf")?;
    let stbl = find_box(minf, b"stbl")?;
    let stsd = find_box(stbl, b"stsd")?;
    // stsd has 4-byte fullbox flags + 4-byte entry_count, then sample entries.
    if stsd.len() < 8 {
        return None;
    }
    let entries = &stsd[8..];
    // First sample entry: 4-byte size + 4-byte fourcc + body.
    if entries.len() < 8 {
        return None;
    }
    let entry_size = u32::from_be_bytes([entries[0], entries[1], entries[2], entries[3]]) as usize;
    let fourcc = &entries[4..8];
    if fourcc != b"avc1" && fourcc != b"avc3" {
        return None;
    }
    let entry_body = &entries[8..entry_size.min(entries.len())];
    // VisualSampleEntry header is 78 bytes; child boxes follow.
    if entry_body.len() <= 78 {
        return None;
    }
    let avcc_search = &entry_body[78..];
    find_box(avcc_search, b"avcC").map(|b| b.to_vec())
}

#[test]
fn parse_avcc_from_mp4() {
    let data = match read_fixture("/tmp/h264_iframe.mp4") {
        Some(d) => d,
        None => return,
    };
    let avcc = extract_avcc_from_mp4(&data).expect("locate avcC");
    let cfg = AvcConfig::parse(&avcc).expect("parse avcC");
    assert_eq!(cfg.length_size, 4, "x264 avcC always uses lengthSize=4");
    assert_eq!(cfg.profile_indication, 66, "baseline = 66");
    assert!(!cfg.sps.is_empty());
    assert!(!cfg.pps.is_empty());

    // Parse the SPS and verify the size matches what ffmpeg encoded (128x96).
    let sps_nalu = &cfg.sps[0];
    let header = NalHeader::parse(sps_nalu[0]).unwrap();
    assert_eq!(header.nal_unit_type, NalUnitType::Sps);
    let rbsp = extract_rbsp(&sps_nalu[1..]);
    let sps = parse_sps(&header, &rbsp).expect("parse SPS");
    assert_eq!(sps.profile_idc, 66);
    let (w, h) = sps.visible_size();
    assert_eq!((w, h), (128, 96));

    // Parse the PPS.
    let pps_nalu = &cfg.pps[0];
    let pps_header = NalHeader::parse(pps_nalu[0]).unwrap();
    assert_eq!(pps_header.nal_unit_type, NalUnitType::Pps);
    let pps_rbsp = extract_rbsp(&pps_nalu[1..]);
    let pps = parse_pps(&pps_header, &pps_rbsp, Some(&sps)).expect("parse PPS");
    assert_eq!(pps.seq_parameter_set_id, sps.seq_parameter_set_id);
    // Baseline => CAVLC.
    assert!(!pps.entropy_coding_mode_flag);
}

#[test]
fn split_annex_b_es_and_parse() {
    let data = match read_fixture("/tmp/h264.es") {
        Some(d) => d,
        None => return,
    };
    let nalus = split_annex_b(&data);
    // We expect at least: SPS, PPS, IDR slice (and possibly SEI, AUD).
    assert!(
        nalus.len() >= 3,
        "expected at least SPS+PPS+slice, got {} NALUs",
        nalus.len()
    );

    let mut sps_seen = 0;
    let mut pps_seen = 0;
    let mut idr_seen = 0;
    for nalu in &nalus {
        let h = NalHeader::parse(nalu[0]).unwrap();
        match h.nal_unit_type {
            NalUnitType::Sps => {
                sps_seen += 1;
                let rbsp = extract_rbsp(&nalu[1..]);
                let sps = parse_sps(&h, &rbsp).expect("SPS parse");
                assert_eq!(sps.profile_idc, 66);
            }
            NalUnitType::Pps => {
                pps_seen += 1;
            }
            NalUnitType::SliceIdr => idr_seen += 1,
            _ => {}
        }
    }
    assert!(sps_seen >= 1, "no SPS NAL units found");
    assert!(pps_seen >= 1, "no PPS NAL units found");
    assert!(idr_seen >= 1, "no IDR slice NAL units found");
}

#[test]
fn slice_header_parse_for_idr() {
    // Drive the front-end decoder through an MP4 IDR sample.
    use oxideav_codec::Decoder;
    use oxideav_core::{CodecId, CodecParameters, Packet, TimeBase};
    use oxideav_h264::decoder::H264Decoder;

    let mp4 = match read_fixture("/tmp/h264_iframe.mp4") {
        Some(d) => d,
        None => return,
    };
    let avcc = extract_avcc_from_mp4(&mp4).expect("avcC");
    let mut params = CodecParameters::video(CodecId::new("h264"));
    params.extradata = avcc.clone();
    let mut dec = H264Decoder::new(CodecId::new("h264"));
    dec.set_avc_config(&avcc).expect("avcC ingest");

    // Build a minimal IDR-only packet from the AVCC `mdat`. We can't easily
    // walk the moov to find sample boundaries, so we approximate by taking
    // the entire `mdat` payload — for an I-only clip with one IDR per frame
    // this is conservative: the decoder will see length-prefixed slices
    // back-to-back and try to ingest them.
    let mdat = find_box(&mp4, b"mdat").expect("mdat");
    // Construct a Packet and send.
    let pkt = Packet::new(0, TimeBase::new(1, 90_000), mdat.to_vec())
        .with_pts(0)
        .with_dts(0)
        .with_keyframe(true);
    // The decoder will likely return Unsupported on the first slice (I-slice
    // pixel reconstruction not yet implemented). That's fine — what we want
    // to verify here is that the slice *header* parses and the error message
    // is the expected unsupported one.
    let res = dec.send_packet(&pkt);
    let last = dec.last_slice_headers();
    if let Err(e) = &res {
        let msg = format!("{e}");
        assert!(
            msg.contains("CABAC")
                || msg.contains("I-slice macroblock decode")
                || msg.contains("interlaced"),
            "unexpected unsupported message: {msg}",
        );
    }
    if let Some(sh) = last.first() {
        // Slice type for baseline IDR is 7 (or 2 mod 5 == I).
        assert!(matches!(sh.slice_type, oxideav_h264::slice::SliceType::I));
        assert!(sh.is_idr);
        assert_eq!(sh.first_mb_in_slice, 0);
    }
}
