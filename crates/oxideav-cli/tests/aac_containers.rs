//! End-to-end AAC through containers with the `oxideav` binary.
//!
//! Regressions covered:
//!
//! * AAC from MP4 / Matroska / QuickTime failed to decode ("packet has
//!   neither an ADTS nor a LOAS syncword"): the decoder ignored the
//!   AudioSpecificConfig in extradata and required ADTS framing; the
//!   QuickTime demuxer also handed over raw extension atoms instead of
//!   the ASC.
//! * `.aac` (ADTS) had no container at all.
//! * AAC could not be muxed into MP4 ("aac stream missing extradata"),
//!   and Matroska output stored ADTS headers inside its frames.
//! * HE-AAC in Matroska declaring only the core rate decoded to a WAV
//!   labelled with the core rate but holding double-rate samples.
//!
//! Inputs are the FFmpeg-generated fixtures in `tests/fixtures/aac/`
//! (see the README there for the command lines); outputs are checked by
//! decoding them back with the binary and — when `ffmpeg` / `ffprobe`
//! are installed — with FFmpeg as a black-box oracle.

use std::path::{Path, PathBuf};
use std::process::Command;

const RATE: u32 = 44_100;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_oxideav")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/aac")
        .join(name)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oxideav_cli_aac_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    let _ = std::fs::remove_file(&p);
    p
}

fn run_ok(args: &[&std::ffi::OsStr]) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn oxideav");
    assert!(
        out.status.success(),
        "oxideav {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn transcode(input: &Path, output: &Path, codec_audio: Option<&str>) {
    let mut args: Vec<&std::ffi::OsStr> = vec!["transcode".as_ref()];
    if let Some(c) = codec_audio {
        args.push("--codec-audio".as_ref());
        args.push(c.as_ref());
    }
    args.push(input.as_os_str());
    args.push(output.as_os_str());
    run_ok(&args);
}

/// The fixture source: `0.4·sin(2π(200 + 600t)t) + 0.2·sin(2π·1234.5t)`.
fn source_sample(i: usize) -> f64 {
    let t = i as f64 / f64::from(RATE);
    0.4 * (std::f64::consts::TAU * (200.0 + 600.0 * t) * t).sin()
        + 0.2 * (std::f64::consts::TAU * 1234.5 * t).sin()
}

fn source(samples: usize) -> Vec<i16> {
    (0..samples)
        .map(|i| (source_sample(i) * 32767.0).round() as i16)
        .collect()
}

/// Minimal RIFF/WAVE 16-bit PCM reader → (rate, channels, interleaved).
fn read_wav(path: &Path) -> (u32, u16, Vec<i16>) {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_eq!(&b[0..4], b"RIFF");
    assert_eq!(&b[8..12], b"WAVE");
    let (mut rate, mut ch, mut bits) = (0u32, 0u16, 0u16);
    let mut pos = 12;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let len = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &b[pos + 8..(pos + 8 + len).min(b.len())];
        if id == b"fmt " {
            ch = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            assert_eq!(bits, 16, "expected 16-bit PCM");
            let pcm = body
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]))
                .collect();
            return (rate, ch, pcm);
        }
        pos += 8 + len + (len & 1);
    }
    panic!("{}: no data chunk", path.display());
}

fn write_wav(path: &Path, channels: u16, pcm: &[i16]) {
    let data_len = (pcm.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + pcm.len() * 2);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * u32::from(channels) * 2).to_le_bytes());
    b.extend_from_slice(&(channels * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

/// Best SNR (dB) of channel 0 of `dec` against mono `src` over lags
/// `0..3000` (decoders differ in how much encoder delay they keep).
fn snr_db(src: &[i16], dec: &[i16], channels: usize) -> (f64, usize) {
    let n_dec = dec.len() / channels;
    let lo = src.len() / 5;
    let hi = src.len() * 4 / 5;
    let mut best = (f64::INFINITY, 0usize, 1.0f64);
    for lag in 0..3000 {
        if hi + lag > n_dec {
            break;
        }
        let (mut err, mut sig) = (0.0f64, 0.0f64);
        for t in (lo..hi).step_by(3) {
            let a = f64::from(src[t]);
            let d = a - f64::from(dec[(t + lag) * channels]);
            err += d * d;
            sig += a * a;
        }
        if err < best.0 {
            best = (err, lag, sig);
        }
    }
    (10.0 * (best.2 / best.0.max(1.0)).log10(), best.1)
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn aac_in_containers_decodes_to_wav() {
    let src = source(RATE as usize / 2);
    for name in ["lc_mono.m4a", "lc_mono.mkv", "lc_mono.mov", "lc_mono.aac"] {
        let out = scratch(&format!("{name}.wav"));
        transcode(&fixture(name), &out, None);
        let (rate, ch, pcm) = read_wav(&out);
        assert_eq!((rate, ch), (RATE, 1), "{name}: output geometry");
        assert!(pcm.len() >= src.len(), "{name}: {} samples", pcm.len());
        let (snr, lag) = snr_db(&src, &pcm, 1);
        // 64 kb/s mono AAC-LC of a chirp: comfortably above 20 dB.
        assert!(snr > 20.0, "{name}: SNR {snr:.1} dB (lag {lag})");
    }
}

#[test]
fn he_aac_declaring_the_core_rate_decodes_at_that_rate() {
    let out = scratch("he_implicit_core_rate.wav");
    transcode(&fixture("he_implicit_core_rate.mkv"), &out, None);
    let (rate, ch, pcm) = read_wav(&out);
    assert_eq!((rate, ch), (22_050, 2));
    // The 440 Hz source tone must read as 440 Hz at the labelled rate
    // (double-rate samples under a 22.05 kHz label would read 220 Hz).
    let left: Vec<i16> = pcm.iter().step_by(2).copied().collect();
    let mid = &left[left.len() / 4..left.len() * 3 / 4];
    let crossings = mid.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    let hz = crossings as f64 / 2.0 / (mid.len() as f64 / 22_050.0);
    assert!((hz - 440.0).abs() < 15.0, "measured {hz:.1} Hz");
}

#[test]
fn wav_encodes_to_aac_in_every_container() {
    let n = RATE as usize;
    let mono = source(n);
    let stereo: Vec<i16> = mono.iter().flat_map(|&s| [s, s / 2]).collect();
    let input = scratch("in.wav");
    write_wav(&input, 2, &stereo);
    let ffmpeg = have("ffmpeg") && have("ffprobe");

    for ext in ["m4a", "mp4", "mkv", "mov", "aac"] {
        let out = scratch(&format!("out.{ext}"));
        transcode(&input, &out, Some("aac"));

        // Back through our own demuxer + decoder.
        let back = scratch(&format!("out_{ext}.wav"));
        transcode(&out, &back, None);
        let (rate, ch, pcm) = read_wav(&back);
        assert_eq!((rate, ch), (RATE, 2), "{ext}: geometry");
        let (snr, lag) = snr_db(&mono, &pcm, 2);
        // Encoder start-up delay: one frame (1024) from the pure-Rust
        // encoder, 2112 from AudioToolbox when it is the selected AAC
        // encoder (macOS).
        assert!(
            lag == 1024 || lag == 2112,
            "{ext}: unexpected encoder delay {lag}"
        );
        assert!(snr > 30.0, "{ext}: SNR {snr:.1} dB");

        // Black-box oracle: an independent reader must see AAC and
        // decode it to the same signal.
        if ffmpeg {
            let probe = Command::new("ffprobe")
                .args(["-v", "error", "-select_streams", "a:0"])
                .args(["-show_entries", "stream=codec_name,sample_rate,channels"])
                .args(["-of", "default=nw=1"])
                .arg(&out)
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&probe.stdout);
            assert!(text.contains("codec_name=aac"), "{ext}: ffprobe {text}");
            assert!(text.contains("sample_rate=44100"), "{ext}: ffprobe {text}");
            assert!(text.contains("channels=2"), "{ext}: ffprobe {text}");
            let ff_wav = scratch(&format!("ff_{ext}.wav"));
            let st = Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-xerror", "-i"])
                .arg(&out)
                .args(["-c:a", "pcm_s16le"])
                .arg(&ff_wav)
                .status()
                .unwrap();
            assert!(st.success(), "{ext}: ffmpeg could not decode our output");
            let (_, ffch, ffpcm) = read_wav(&ff_wav);
            let (ffsnr, _) = snr_db(&mono, &ffpcm, ffch as usize);
            assert!(ffsnr > 30.0, "{ext}: ffmpeg-decoded SNR {ffsnr:.1} dB");
        }
    }
}
