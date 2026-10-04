//! Conversions whose output container constrains what it carries,
//! end to end through the built binary:
//!
//! * `.ogg` from an H.264 + AAC (or FLAC) input: the Ogg muxer refuses
//!   codecs without an Ogg mapping at open, so stream selection drops
//!   the video and the audio is kept — stream-copied when Ogg maps it
//!   (FLAC), re-encoded to Vorbis otherwise (AAC). Regression: the
//!   muxer accepted every stream and wrote a file no reader could
//!   identify.
//! * `.y4m` from RGB FFV1: the planner converts the decoded GBR pictures
//!   to a layout the Y4M muxer can describe (planar YUV 4:4:4).
//!   Regression: "Gbrp8 cannot be expressed as a Y4M colorspace".
//! * `transcode` without codec flags applies `convert`'s per-container
//!   defaults: `.flac` encodes FLAC, `.ogg` Vorbis, `.y4m` rawvideo.
//!   Regression: audio defaulted to PCM, which `.flac` / `.ogg` refuse.
//! * PCM in MP4 (ISO/IEC 23003-5 `ipcm` / `fpcm`): WAV → MP4 → WAV is
//!   sample-exact, and ffprobe / ffmpeg (when installed) read our files
//!   and produce files we read identically.
//!
//! Inputs that need an external encoder are generated with ffmpeg when
//! it is installed (the tests skip those parts otherwise):
//!
//! ```text
//! ffmpeg -f lavfi -i testsrc2=size=96x64:rate=25 -f lavfi -i sine=frequency=440:sample_rate=48000 \
//!        -t 1 -c:v libx264 -pix_fmt yuv420p -c:a aac in.mkv
//! ffmpeg -f lavfi -i testsrc2=size=64x48:rate=25 -frames:v 5 -c:v ffv1 -pix_fmt gbrp rgb.mkv
//! ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000 -ac 2 -t 0.5 -c:a pcm_s24le ff.mp4
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::*;

const TAG: &str = "container-conversions";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn ffmpeg_has(what: &str, list: &str) -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", list])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains(what))
        .unwrap_or(false)
}

fn ffmpeg(args: &[&str]) {
    let st = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .expect("spawn ffmpeg");
    assert!(st.success(), "ffmpeg {args:?}");
}

/// ffmpeg's decode of the first audio stream as interleaved f64.
fn ffmpeg_audio(path: &Path) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", "f64le", "-acodec", "pcm_f64le", "-"])
        .output()
        .expect("spawn ffmpeg");
    assert!(out.status.success(), "ffmpeg decode {}", path.display());
    out.stdout
}

/// `codec_name,codec_tag_string` per stream, from ffprobe.
fn ffprobe_codecs(path: &Path) -> Option<Vec<String>> {
    let probe = tool("ffprobe")?;
    let out = run_tool(
        &probe,
        &[
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name,codec_tag_string",
            "-of",
            "csv=p=0",
            path.to_str().unwrap(),
        ],
    )
    .unwrap_or_else(|e| panic!("ffprobe {}: {e}", path.display()));
    Some(out.lines().map(str::to_string).collect())
}

/// `oxideav probe` stream lines.
fn probed(path: &Path) -> Vec<String> {
    let r = oxideav(&["probe", path.to_str().unwrap()]);
    assert!(r.ok, "probe {}: {}", path.display(), r.stderr);
    r.stdout
        .lines()
        .filter(|l| l.trim_start().starts_with("Stream #"))
        .map(str::to_string)
        .collect()
}

fn run_ok(args: &[&str]) -> Run {
    let r = oxideav(args);
    assert!(r.ok, "oxideav {args:?}: {}", r.stderr);
    r
}

#[test]
fn h264_and_flac_to_ogg_keeps_the_flac_audio() {
    let input = fixture("h264_64x48_two_flac.mkv");
    for cmd in ["convert", "transcode"] {
        let out = scratch_file(TAG, &format!("flac_{cmd}.ogg"));
        let r = run_ok(&[cmd, input.to_str().unwrap(), out.to_str().unwrap()]);
        assert!(
            r.stderr.contains("dropping video stream #0 (h264)"),
            "{cmd}: {}",
            r.stderr
        );
        let streams = probed(&out);
        assert!(!streams.is_empty(), "{cmd}: no streams");
        for s in &streams {
            assert!(
                s.contains("[Audio]") && s.contains("codec=flac"),
                "{cmd}: {s}"
            );
        }
        if let Some(codecs) = ffprobe_codecs(&out) {
            assert!(
                !codecs.is_empty() && codecs.iter().all(|c| c.starts_with("flac")),
                "{cmd}: ffprobe {codecs:?}"
            );
        }
    }
}

#[test]
fn h264_and_aac_mkv_to_ogg_is_vorbis_audio() {
    if !(ffmpeg_has("libx264", "-encoders") && ffmpeg_has(" aac ", "-encoders")) {
        eprintln!("skipped: ffmpeg with libx264 + aac not available");
        return;
    }
    let input = scratch_file(TAG, "h264_aac.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=96x64:rate=25",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000",
        "-t",
        "1",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        input.to_str().unwrap(),
    ]);
    for cmd in ["convert", "transcode"] {
        let out = scratch_file(TAG, &format!("aac_{cmd}.ogg"));
        let r = run_ok(&[cmd, input.to_str().unwrap(), out.to_str().unwrap()]);
        assert!(
            r.stderr.contains("dropping video stream #0 (h264)"),
            "{cmd}: {}",
            r.stderr
        );
        let streams = probed(&out);
        assert_eq!(streams.len(), 1, "{cmd}: {streams:?}");
        assert!(streams[0].contains("codec=vorbis"), "{cmd}: {streams:?}");
        assert_eq!(
            ffprobe_codecs(&out).unwrap_or_default(),
            vec!["vorbis,[0][0][0][0]".to_string()],
            "{cmd}"
        );
        // About one second of 48 kHz mono audio decodes back.
        let samples = ffmpeg_audio(&out).len() / 8;
        assert!(
            (44_000..=52_000).contains(&samples),
            "{cmd}: {samples} samples"
        );
    }
}

#[test]
fn rgb_ffv1_to_y4m_converts_to_planar_yuv() {
    if !ffmpeg_has("ffv1", "-encoders") {
        eprintln!("skipped: ffmpeg with ffv1 not available");
        return;
    }
    let input = scratch_file(TAG, "rgb_ffv1.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=64x48:rate=25",
        "-frames:v",
        "5",
        "-c:v",
        "ffv1",
        "-pix_fmt",
        "gbrp",
        input.to_str().unwrap(),
    ]);
    let rgb = |p: &Path| -> Vec<u8> {
        let out = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(p)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .expect("ffmpeg");
        assert!(out.status.success());
        out.stdout
    };
    let want = rgb(&input);
    assert_eq!(want.len(), 64 * 48 * 3 * 5);
    for cmd in ["convert", "transcode"] {
        let out = scratch_file(TAG, &format!("rgb_{cmd}.y4m"));
        run_ok(&[cmd, input.to_str().unwrap(), out.to_str().unwrap()]);
        let data = std::fs::read(&out).unwrap();
        let header =
            String::from_utf8_lossy(&data[..data.iter().position(|&b| b == b'\n').unwrap()])
                .into_owned();
        // 4:4:4 keeps the full colour resolution of the RGB source.
        assert!(header.contains(" C444"), "{cmd}: {header}");
        let got = rgb(&out);
        assert_eq!(got.len(), want.len(), "{cmd}: frames");
        let mse = got
            .iter()
            .zip(&want)
            .map(|(&a, &b)| f64::from(a as i32 - b as i32).powi(2))
            .sum::<f64>()
            / want.len() as f64;
        let psnr = 10.0 * (255.0f64.powi(2) / mse.max(1e-9)).log10();
        assert!(
            psnr > 40.0,
            "{cmd}: RGB → YUV 4:4:4 → RGB PSNR {psnr:.1} dB"
        );
    }
}

#[test]
fn transcode_uses_the_container_default_codecs() {
    let input = fixture("h264_64x48_two_flac.mkv");
    // `.flac` names the FLAC encoder; no `--codec-audio` needed.
    let out = scratch_file(TAG, "default.flac");
    run_ok(&["transcode", input.to_str().unwrap(), out.to_str().unwrap()]);
    let streams = probed(&out);
    assert_eq!(streams.len(), 1, "{streams:?}");
    assert!(streams[0].contains("codec=flac"), "{streams:?}");
    // `.wav`: compressed audio → 16-bit PCM.
    let out = scratch_file(TAG, "default.wav");
    run_ok(&["transcode", input.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(probed(&out)[0].contains("codec=pcm_s16le"));
    // `.y4m`: H.264 → rawvideo pictures (a stream copy cannot fit).
    let out = scratch_file(TAG, "default.y4m");
    run_ok(&["transcode", input.to_str().unwrap(), out.to_str().unwrap()]);
    let streams = probed(&out);
    assert!(
        streams[0].contains("codec=rawvideo") && streams[0].contains("64x48"),
        "{streams:?}"
    );
    // An explicit flag still wins.
    let out = scratch_file(TAG, "explicit.mkv");
    run_ok(&[
        "transcode",
        "--codec-audio",
        "pcm_s16le",
        input.to_str().unwrap(),
        out.to_str().unwrap(),
    ]);
    let streams = probed(&out);
    assert!(
        streams.iter().any(|s| s.contains("codec=pcm_s16le")),
        "{streams:?}"
    );
    assert!(
        streams.iter().any(|s| s.contains("codec=h264")),
        "{streams:?}"
    );
}

/// A `bits`-bit PCM WAV (`format` 1 = integer, 3 = float) with a 440 Hz
/// sine on every channel.
fn write_wav(name: &str, rate: u32, channels: u16, bits: u16, format: u16) -> PathBuf {
    let path = scratch_file(TAG, name);
    let frames = rate as usize / 4;
    let mut pcm = Vec::new();
    for i in 0..frames {
        let v = 0.4 * (std::f64::consts::TAU * 440.0 * i as f64 / f64::from(rate)).sin();
        for c in 0..channels {
            let v = v * (1.0 - 0.1 * f64::from(c));
            match (format, bits) {
                (3, 32) => pcm.extend_from_slice(&(v as f32).to_le_bytes()),
                (3, 64) => pcm.extend_from_slice(&v.to_le_bytes()),
                (_, 16) => pcm.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes()),
                (_, 24) => pcm.extend_from_slice(&((v * 8_388_607.0) as i32).to_le_bytes()[..3]),
                _ => pcm.extend_from_slice(&((v * 2_147_483_647.0) as i32).to_le_bytes()),
            }
        }
    }
    let block = channels * bits / 8;
    let mut b = Vec::with_capacity(44 + pcm.len());
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&format.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    b.extend_from_slice(&block.to_le_bytes());
    b.extend_from_slice(&bits.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    b.extend_from_slice(&pcm);
    std::fs::write(&path, b).unwrap();
    path
}

/// The `data` chunk of a WAV file.
fn wav_data(path: &Path) -> Vec<u8> {
    let b = std::fs::read(path).unwrap();
    let mut pos = 12;
    while pos + 8 <= b.len() {
        let len = u32::from_le_bytes([b[pos + 4], b[pos + 5], b[pos + 6], b[pos + 7]]) as usize;
        if &b[pos..pos + 4] == b"data" {
            return b[pos + 8..(pos + 8 + len).min(b.len())].to_vec();
        }
        pos += 8 + len + (len & 1);
    }
    panic!("no data chunk in {}", path.display());
}

#[test]
fn pcm_in_mp4_round_trips_and_interoperates() {
    let ffmpeg_ok = tool("ffmpeg").is_some() && tool("ffprobe").is_some();
    for (name, rate, channels, bits, format, codec, tag) in [
        ("s16", 48_000u32, 2u16, 16u16, 1u16, "pcm_s16le", "ipcm"),
        ("s24", 44_100, 1, 24, 1, "pcm_s24le", "ipcm"),
        ("s32", 96_000, 2, 32, 1, "pcm_s32le", "ipcm"),
        ("f32", 192_000, 2, 32, 3, "pcm_f32le", "fpcm"),
        ("f64", 8_000, 6, 64, 3, "pcm_f64le", "fpcm"),
    ] {
        let wav = write_wav(&format!("{name}.wav"), rate, channels, bits, format);
        let mp4 = scratch_file(TAG, &format!("{name}.mp4"));
        run_ok(&["transcode", wav.to_str().unwrap(), mp4.to_str().unwrap()]);
        let streams = probed(&mp4);
        assert!(
            streams[0].contains(&format!("codec={codec}"))
                && streams[0].contains(&format!("{rate} Hz")),
            "{name}: {streams:?}"
        );
        // Back to WAV in the same layout: sample-exact.
        let back = scratch_file(TAG, &format!("{name}_back.wav"));
        run_ok(&[
            "transcode",
            "--codec-audio",
            codec,
            mp4.to_str().unwrap(),
            back.to_str().unwrap(),
        ]);
        assert!(wav_data(&back) == wav_data(&wav), "{name}: round trip");
        if ffmpeg_ok {
            assert_eq!(
                ffprobe_codecs(&mp4).unwrap(),
                vec![format!("{codec},{tag}")],
                "{name}: ffprobe"
            );
            assert!(
                ffmpeg_audio(&mp4) == ffmpeg_audio(&wav),
                "{name}: ffmpeg decode"
            );
        }
    }
    if !ffmpeg_ok {
        eprintln!("ffmpeg interop skipped: ffmpeg / ffprobe not available");
        return;
    }
    // ffmpeg's own ipcm / fpcm files (both byte orders) read back
    // identically to ffmpeg's decode.
    for codec in [
        "pcm_s16be",
        "pcm_s24le",
        "pcm_s32be",
        "pcm_f32le",
        "pcm_f64be",
    ] {
        let src = scratch_file(TAG, &format!("ff_{codec}.mp4"));
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-ac",
            "2",
            "-t",
            "0.5",
            "-c:a",
            codec,
            src.to_str().unwrap(),
        ]);
        let wav = scratch_file(TAG, &format!("ff_{codec}.wav"));
        run_ok(&[
            "transcode",
            "--codec-audio",
            "pcm_f64le",
            src.to_str().unwrap(),
            wav.to_str().unwrap(),
        ]);
        assert!(ffmpeg_audio(&wav) == ffmpeg_audio(&src), "{codec}");
    }
}
