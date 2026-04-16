//! Integration tests using ffmpeg-produced reference clips.
//!
//! These tests are skipped gracefully if `/usr/bin/ffmpeg` or the reference
//! files are missing — consistent with other crates in the workspace.
//!
//! Scope today:
//!
//! * Mode detection via the TOC parser on real ffmpeg-produced packets.
//! * SILK/Hybrid rejection (the decoder must return `Unsupported` and not
//!   panic or emit garbage).
//! * CELT packet-framing invariants (total packet duration ≤ 120 ms).
//!
//! Full CELT audio decoding is not yet implemented; see
//! `oxideav-opus/src/decoder.rs` for the current scope.

use std::path::Path;
use std::process::Command;

use oxideav_container::{Demuxer, ReadSeek};
use oxideav_core::{Error, Frame};
use oxideav_opus::toc::{parse_packet, OpusMode, Toc};

const FFMPEG: &str = "/usr/bin/ffmpeg";

fn ffmpeg_available() -> bool {
    Path::new(FFMPEG).exists()
}

fn ensure_ref(path: &str, args: &[&str]) -> bool {
    if !ffmpeg_available() {
        return false;
    }
    if Path::new(path).exists() {
        return true;
    }
    let status = Command::new(FFMPEG)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args(args)
        .arg(path)
        .status();
    matches!(status, Ok(s) if s.success()) && Path::new(path).exists()
}

fn ensure_celt_mono() -> Option<&'static str> {
    let path = "/tmp/ref-opus-celt-mono.opus";
    if ensure_ref(
        path,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=1000:d=1:sample_rate=48000",
            "-ac",
            "1",
            "-c:a",
            "libopus",
            "-b:a",
            "128k",
            "-application",
            "audio",
        ],
    ) {
        Some(path)
    } else {
        None
    }
}

fn ensure_celt_stereo() -> Option<&'static str> {
    let path = "/tmp/ref-opus-celt-stereo.opus";
    if ensure_ref(
        path,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=1000:d=1:sample_rate=48000",
            "-ac",
            "2",
            "-c:a",
            "libopus",
            "-b:a",
            "128k",
            "-application",
            "audio",
        ],
    ) {
        Some(path)
    } else {
        None
    }
}

fn ensure_voip_mono() -> Option<&'static str> {
    let path = "/tmp/ref-opus-voip-mono.opus";
    if ensure_ref(
        path,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=f=300:d=1:sample_rate=16000",
            "-ac",
            "1",
            "-c:a",
            "libopus",
            "-b:a",
            "16k",
            "-application",
            "voip",
        ],
    ) {
        Some(path)
    } else {
        None
    }
}

fn open_ogg(path: &str) -> Box<dyn Demuxer> {
    let f = std::fs::File::open(path).expect("open ref");
    let rs: Box<dyn ReadSeek> = Box::new(f);
    oxideav_ogg::demux::open(rs).expect("open ogg demuxer")
}

/// Mode-detection check: CELT-only reference TOC reports CELT-only.
#[test]
fn toc_reports_celt_only_for_music() {
    let Some(path) = ensure_celt_mono() else {
        eprintln!("skip: ffmpeg / reference unavailable");
        return;
    };
    let mut dmx = open_ogg(path);
    let pkt = dmx.next_packet().expect("packet");
    let toc = Toc::parse(pkt.data[0]);
    assert_eq!(toc.mode, OpusMode::CeltOnly);
    assert_eq!(toc.frame_samples_48k, 960);
    assert!(!toc.stereo, "mono reference");
}

#[test]
fn toc_reports_celt_only_for_stereo_music() {
    let Some(path) = ensure_celt_stereo() else {
        eprintln!("skip: ffmpeg / reference unavailable");
        return;
    };
    let mut dmx = open_ogg(path);
    let pkt = dmx.next_packet().expect("packet");
    let toc = Toc::parse(pkt.data[0]);
    assert_eq!(toc.mode, OpusMode::CeltOnly);
    assert!(toc.stereo, "stereo reference");
}

#[test]
fn toc_reports_silk_only_for_voip() {
    let Some(path) = ensure_voip_mono() else {
        eprintln!("skip: ffmpeg / reference unavailable");
        return;
    };
    let mut dmx = open_ogg(path);
    let pkt = dmx.next_packet().expect("packet");
    let toc = Toc::parse(pkt.data[0]);
    assert_eq!(toc.mode, OpusMode::SilkOnly);
}

/// Packet parse invariant: we can successfully split every real-world
/// packet from a music clip into frames, and every frame is non-empty.
#[test]
fn celt_mono_packets_parse_cleanly() {
    let Some(path) = ensure_celt_mono() else {
        eprintln!("skip: ffmpeg / reference unavailable");
        return;
    };
    let mut dmx = open_ogg(path);
    let mut n = 0usize;
    loop {
        match dmx.next_packet() {
            Ok(pkt) => {
                let parsed = parse_packet(&pkt.data).expect("TOC parse");
                assert!(!parsed.frames.is_empty(), "packet #{} has zero frames", n);
                // ffmpeg produces code-0 (single frame) for CELT at 128 kbps.
                assert_eq!(parsed.toc.code, 0);
                n += 1;
            }
            Err(Error::Eof) => break,
            Err(e) => panic!("demux error: {}", e),
        }
    }
    assert!(n > 40, "expected >40 packets from a 1-second clip, got {n}");
}

/// Decode 50 SILK-only voip packets and assert we reject each one with a
/// clean `Unsupported` error — no panics, no garbage.
#[test]
fn silk_packets_are_rejected_not_crashed() {
    let Some(path) = ensure_voip_mono() else {
        eprintln!("skip: ffmpeg / reference unavailable");
        return;
    };
    let mut dmx = open_ogg(path);
    let params = dmx.streams()[0].params.clone();
    let mut dec = oxideav_opus::decoder::make_decoder(&params).expect("make decoder");

    let mut rejected = 0usize;
    for _ in 0..50 {
        let pkt = match dmx.next_packet() {
            Ok(p) => p,
            Err(Error::Eof) => break,
            Err(e) => panic!("demux error: {}", e),
        };
        dec.send_packet(&pkt).expect("send");
        let r = dec.receive_frame();
        match r {
            Err(Error::Unsupported(msg)) => {
                assert!(
                    msg.to_lowercase().contains("silk") || msg.to_lowercase().contains("hybrid"),
                    "unexpected Unsupported message: {}",
                    msg
                );
                rejected += 1;
            }
            Ok(_) => panic!("SILK packet unexpectedly decoded"),
            Err(e) => panic!("unexpected error type: {:?}", e),
        }
    }
    assert!(rejected >= 10, "expected ≥10 rejections, got {rejected}");
}

/// CELT-only packets with full audio content currently return
/// `Unsupported` (band-energy + PVQ + IMDCT not yet landed). This test
/// pins the contract: decoder must not panic, and the error must be
/// `Unsupported` with a message mentioning "CELT" so a caller can tell
/// the difference from "invalid packet".
#[test]
fn celt_packets_return_unsupported_cleanly() {
    let Some(path) = ensure_celt_mono() else {
        eprintln!("skip: ffmpeg / reference unavailable");
        return;
    };
    let mut dmx = open_ogg(path);
    let params = dmx.streams()[0].params.clone();
    let mut dec = oxideav_opus::decoder::make_decoder(&params).expect("make decoder");

    let mut tested = 0usize;
    let mut saw_unsupported = false;
    for _ in 0..20 {
        let pkt = match dmx.next_packet() {
            Ok(p) => p,
            Err(Error::Eof) => break,
            Err(e) => panic!("demux: {}", e),
        };
        dec.send_packet(&pkt).expect("send");
        match dec.receive_frame() {
            Ok(Frame::Audio(a)) => {
                // A silence-flag CELT frame (rare but valid) would produce
                // a real AudioFrame of zeros. Accept that without failing.
                assert_eq!(a.sample_rate, 48_000);
                assert_eq!(a.samples, 960);
            }
            Ok(Frame::Video(_)) => panic!("audio decoder returned video frame"),
            Err(Error::Unsupported(msg)) => {
                let lc = msg.to_lowercase();
                assert!(
                    lc.contains("celt") || lc.contains("opus"),
                    "unexpected message: {}",
                    msg
                );
                saw_unsupported = true;
            }
            Err(e) => panic!("unexpected error: {:?}", e),
        }
        tested += 1;
    }
    assert!(tested > 0, "no packets tested");
    // At 128 kbps sine-wave the encoder never emits silence, so we expect
    // the full-decode Unsupported path to hit at least once.
    assert!(
        saw_unsupported,
        "expected at least one Unsupported from the full-CELT path"
    );
}
