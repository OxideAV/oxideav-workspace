//! Integration tests using ffmpeg-produced reference clips.
//!
//! Skipped gracefully if `/usr/bin/ffmpeg` (or the libspeex encoder
//! plug-in) is unavailable — consistent with sister crates.
//!
//! Coverage today:
//!   * NB 8 kHz mono, 24 kbps tone — should decode to PCM with non-trivial
//!     RMS and a clear 440 Hz peak (Goertzel).
//!   * WB 16 kHz attempt — currently returns `Unsupported` from the
//!     decoder factory; assert the error is reported cleanly without
//!     panic and the test is `#[ignore]`d on success expectations until
//!     SB-CELP lands.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::process::Command;

use oxideav_codec::Decoder;
use oxideav_container::{Demuxer, ReadSeek};
use oxideav_core::{Error, Frame};
use oxideav_speex::decoder::make_decoder;

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

fn ensure_nb_24k() -> Option<&'static str> {
    let path = "/tmp/speex_nb.spx";
    if ensure_ref(
        path,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-ar",
            "8000",
            "-ac",
            "1",
            "-c:a",
            "libspeex",
            "-b:a",
            "24k",
        ],
    ) {
        Some(path)
    } else {
        None
    }
}

fn ensure_wb_27k() -> Option<&'static str> {
    let path = "/tmp/speex_wb.spx";
    if ensure_ref(
        path,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-ar",
            "16000",
            "-ac",
            "1",
            "-c:a",
            "libspeex",
            "-b:a",
            "27k",
        ],
    ) {
        Some(path)
    } else {
        None
    }
}

fn open_speex_file(path: &str) -> (Box<dyn Decoder>, Box<dyn Demuxer>) {
    let f = File::open(path).expect("open speex file");
    let bf: Box<dyn ReadSeek> = Box::new(BufReader::new(f));
    let demux = oxideav_ogg::demux::open(bf).expect("ogg open");
    let dec = make_decoder(&demux.streams()[0].params).expect("speex decoder");
    (dec, demux)
}

fn decode_to_f32(path: &str) -> (Vec<f32>, u32) {
    let (mut dec, mut demux) = open_speex_file(path);
    let sr = demux.streams()[0].params.sample_rate.unwrap_or(8_000);
    let mut pcm = Vec::<f32>::new();
    loop {
        let pkt = match demux.next_packet() {
            Ok(p) => p,
            Err(Error::Eof) => break,
            Err(e) => panic!("demux error: {e}"),
        };
        if let Err(e) = dec.send_packet(&pkt) {
            panic!("send_packet: {e}");
        }
        loop {
            match dec.receive_frame() {
                Ok(Frame::Audio(af)) => {
                    // S16 interleaved -> f32 in [-1, 1].
                    let bytes = &af.data[0];
                    for chunk in bytes.chunks_exact(2) {
                        let s = i16::from_le_bytes([chunk[0], chunk[1]]);
                        pcm.push(s as f32 / 32768.0);
                    }
                }
                Ok(_) => {}
                Err(Error::NeedMore) => break,
                Err(Error::Eof) => break,
                Err(e) => panic!("receive_frame: {e}"),
            }
        }
    }
    let _ = dec.flush();
    (pcm, sr)
}

/// Goertzel's algorithm — returns the magnitude squared of the
/// frequency bin centred on `target_hz` for a signal of length `n`
/// sampled at `sample_rate`.
fn goertzel(samples: &[f32], sample_rate: u32, target_hz: f32) -> f32 {
    let n = samples.len();
    let k = (0.5 + (n as f32 * target_hz) / sample_rate as f32) as i32;
    let omega = 2.0 * std::f32::consts::PI * k as f32 / n as f32;
    let coeff = 2.0 * omega.cos();
    let (mut q1, mut q2) = (0.0f32, 0.0f32);
    for &x in samples {
        let q0 = coeff * q1 - q2 + x;
        q2 = q1;
        q1 = q0;
    }
    q1 * q1 + q2 * q2 - q1 * q2 * coeff
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|x| x * x).sum();
    (sum / samples.len() as f32).sqrt()
}

/// Average power over [start_hz, end_hz) outside `target_hz` — used for a
/// signal-to-noise ratio (Goertzel ratio) check.
fn off_target_power(samples: &[f32], sr: u32, target: f32, span: f32) -> f32 {
    let mut sum = 0.0f32;
    let mut n = 0;
    let mut hz = 100.0;
    while hz < (sr as f32 / 2.0) - span {
        if (hz - target).abs() > span {
            sum += goertzel(samples, sr, hz);
            n += 1;
        }
        hz += span;
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f32
    }
}

#[test]
fn decode_nb_440_tone_is_audible() {
    let Some(path) = ensure_nb_24k() else {
        eprintln!("skipping: ffmpeg/libspeex unavailable");
        return;
    };

    let (pcm, sr) = decode_to_f32(path);
    assert_eq!(sr, 8_000, "NB sample rate");
    assert!(
        pcm.len() >= 4 * 160,
        "expected >= 4 frames of audio, got {} samples",
        pcm.len()
    );

    // Discard the first 50 ms (~400 samples) — the LPC filter cold-starts
    // and the open-loop pitch estimator hasn't settled.
    let warmup = (sr as usize) / 20;
    let warm = &pcm[warmup..];

    let r = rms(warm);
    eprintln!("NB decode RMS = {r}");
    assert!(
        r > 0.05,
        "decoded PCM should be audible (RMS > 0.05), got {r}"
    );

    // Goertzel: 440 Hz peak should dominate the off-target average by a
    // substantial margin.
    let on = goertzel(warm, sr, 440.0);
    let off = off_target_power(warm, sr, 440.0, 100.0);
    let ratio = on / off.max(1e-9);
    eprintln!("NB 440 Hz Goertzel ratio = {ratio:.2} (on={on}, off={off})");
    assert!(
        ratio > 5.0,
        "440 Hz tone should dominate (ratio > 5x), got {ratio}"
    );
}

#[test]
fn wb_decoder_returns_unsupported_for_now() {
    // WB synthesis (sb_celp) is not yet implemented — see decoder.rs and
    // the gap notes in the report. Until QMF is in, this clip is rejected
    // up-front from the factory. The point of this test is to make sure
    // the rejection path stays clean (no panic, no garbage data).
    let Some(path) = ensure_wb_27k() else {
        eprintln!("skipping: ffmpeg/libspeex unavailable");
        return;
    };

    let f = File::open(path).expect("open wb fixture");
    let bf: Box<dyn ReadSeek> = Box::new(BufReader::new(f));
    let demux = oxideav_ogg::demux::open(bf).expect("ogg open");
    let stream = &demux.streams()[0];
    match make_decoder(&stream.params) {
        Ok(_) => {
            // If we got here, WB decode landed — great. Don't fail.
            eprintln!("wideband decoder is now supported");
        }
        Err(Error::Unsupported(msg)) => {
            assert!(
                msg.contains("wideband") || msg.contains("WB") || msg.contains("sub-band"),
                "WB rejection should mention the codec, got: {msg}"
            );
        }
        Err(e) => panic!("WB factory should return Unsupported, got {e}"),
    }
}
