//! `oxideav convert` encoder defaults, end to end through the binary:
//!
//! * `in.wav out.mp3` — the raw MPEG-audio muxer (Xing/Info-led) and
//!   the Layer III encoder at its default bit rate; an input rate MPEG
//!   audio cannot carry (96 kHz) is resampled to an accepted one;
//! * `in.wav out.opus` — any input rate: RFC 6716 rates (16 kHz) go to
//!   the encoder directly and are recorded as the `OpusHead` input
//!   rate, others (44.1 kHz) are resampled by the pipeline;
//! * `in.y4m out.webm` / `out.mkv` — raw pictures get VP9 with no codec
//!   or pixel-format flag (Y4M's `rawvideo` stream decodes, the
//!   container's default encoder is picked, 4:2:0 is negotiated).
//!
//! Outputs are checked by decoding them back through the library's own
//! software decoders, and — when installed — by `ffprobe` / `ffmpeg` /
//! `mpg123` as black-box oracles.

mod common;

use std::path::{Path, PathBuf};

use common::{ctx, oxideav, run_tool, scratch_file, tool};
use oxideav_core::{CodecParameters, Decoder, Frame, MediaType, RuntimeContext};

const TAG: &str = "convert-encoder-defaults";

/// A 16-bit PCM WAV of `secs` seconds: a `freq` Hz sine on every channel.
fn write_wav(name: &str, rate: u32, channels: u16, freq: f64, secs: f64) -> PathBuf {
    let path = scratch_file(TAG, name);
    let frames = (f64::from(rate) * secs) as usize;
    let mut pcm = Vec::with_capacity(frames * usize::from(channels) * 2);
    for i in 0..frames {
        let v =
            (12_000.0 * (std::f64::consts::TAU * freq * i as f64 / f64::from(rate)).sin()) as i16;
        for _ in 0..channels {
            pcm.extend_from_slice(&v.to_le_bytes());
        }
    }
    let block = channels * 2;
    let mut b = Vec::with_capacity(44 + pcm.len());
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    b.extend_from_slice(&block.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    b.extend_from_slice(&pcm);
    std::fs::write(&path, b).expect("write wav");
    path
}

/// A 4:2:0 Y4M clip: a moving gradient, `frames` pictures at 25 fps.
fn write_y4m(name: &str, w: usize, h: usize, frames: usize) -> PathBuf {
    let path = scratch_file(TAG, name);
    let mut b = format!("YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C420jpeg\n").into_bytes();
    for f in 0..frames {
        b.extend_from_slice(b"FRAME\n");
        for y in 0..h {
            for x in 0..w {
                b.push(((x * 3 + y * 2 + f * 4) % 220 + 16) as u8);
            }
        }
        for plane in 0..2 {
            for y in 0..h / 2 {
                for x in 0..w / 2 {
                    b.push((128 + (x + y + plane * 20) % 40) as u8 - 20);
                }
            }
        }
    }
    std::fs::write(&path, b).expect("write y4m");
    path
}

fn convert(input: &Path, output: &Path) {
    let r = oxideav(&["convert", input.to_str().unwrap(), output.to_str().unwrap()]);
    assert!(
        r.ok,
        "convert {} -> {} failed: {}",
        input.display(),
        output.display(),
        r.stderr
    );
}

/// The preferred software decoder for `params` (hardware backends may
/// outrank it in the registry but are not what this test exercises).
fn sw_decoder(ctx: &RuntimeContext, params: &CodecParameters) -> Box<dyn Decoder> {
    let imp = ctx
        .codecs
        .implementations(&params.codec_id)
        .iter()
        .filter(|i| i.make_decoder.is_some() && !i.caps.hardware_accelerated)
        .min_by_key(|i| i.caps.priority)
        .unwrap_or_else(|| panic!("no software decoder for {}", params.codec_id));
    (imp.make_decoder.unwrap())(params).expect("decoder")
}

/// What one stream of a file decodes to.
#[derive(Debug, Default)]
struct Decoded {
    codec: String,
    params: Option<CodecParameters>,
    frames: usize,
    /// Audio: samples per channel; first channel as i16 when the
    /// layout is S16 (interleaved or planar).
    samples: usize,
    pcm: Vec<i16>,
}

fn decode_stream(ctx: &RuntimeContext, path: &Path, kind: MediaType) -> Decoded {
    let mut f = std::fs::File::open(path).expect("open output");
    let ext = path.extension().and_then(|e| e.to_str());
    let name = ctx
        .containers
        .probe_input(&mut f, ext)
        .unwrap_or_else(|e| panic!("probe {}: {e}", path.display()));
    let mut dm = ctx
        .containers
        .open_demuxer(&name, Box::new(f), &ctx.codecs)
        .expect("demuxer");
    let stream = dm
        .streams()
        .iter()
        .find(|s| s.params.media_type == kind)
        .unwrap_or_else(|| panic!("{}: no {kind:?} stream", path.display()))
        .clone();
    let mut dec = sw_decoder(ctx, &stream.params);
    let mut out = Decoded {
        codec: stream.params.codec_id.to_string(),
        params: Some(stream.params.clone()),
        ..Default::default()
    };
    let channels = usize::from(stream.params.channels.unwrap_or(1));
    let take = |frame: Frame, out: &mut Decoded| {
        out.frames += 1;
        if let Frame::Audio(a) = frame {
            out.samples += a.samples as usize;
            // Planar → first plane; interleaved → every channels-th word.
            let step = if a.data.len() > 1 { 1 } else { channels };
            let words: Vec<i16> = a.data[0]
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            out.pcm.extend(words.into_iter().step_by(step));
        }
    };
    loop {
        match dm.next_packet() {
            Ok(p) if p.stream_index == stream.index => {
                dec.send_packet(&p).expect("send_packet");
                while let Ok(fr) = dec.receive_frame() {
                    take(fr, &mut out);
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = dec.flush();
    while let Ok(fr) = dec.receive_frame() {
        take(fr, &mut out);
    }
    out
}

/// Dominant frequency of `pcm` at `rate` by zero-crossing count.
fn zero_crossing_freq(pcm: &[i16], rate: u32) -> f64 {
    let s = &pcm[pcm.len() / 4..pcm.len() * 3 / 4];
    let crossings = s.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    crossings as f64 / 2.0 / (s.len() as f64 / f64::from(rate))
}

fn ffprobe_streams(path: &Path) -> Option<String> {
    let probe = tool("ffprobe")?;
    Some(
        run_tool(
            &probe,
            &[
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,sample_rate,channels,pix_fmt,width,height",
                "-of",
                "compact",
                path.to_str().unwrap(),
            ],
        )
        .unwrap_or_else(|e| panic!("ffprobe rejected {}: {e}", path.display())),
    )
}

fn ffmpeg_decodes(path: &Path) {
    if let Some(ff) = tool("ffmpeg") {
        run_tool(
            &ff,
            &[
                "-v",
                "error",
                "-i",
                path.to_str().unwrap(),
                "-f",
                "null",
                "-",
            ],
        )
        .unwrap_or_else(|e| panic!("ffmpeg failed to decode {}: {e}", path.display()));
    }
}

#[test]
fn wav_to_mp3_uses_the_mpeg_audio_muxer_and_decodes() {
    let input = write_wav("tone44.wav", 44_100, 2, 1_000.0, 1.0);
    let output = scratch_file(TAG, "tone44.mp3");
    convert(&input, &output);

    let bytes = std::fs::read(&output).unwrap();
    // First frame: MPEG-1 Layer III sync, carrying the Info tag.
    assert_eq!(bytes[0], 0xFF);
    assert_eq!(bytes[1] & 0xFE, 0xFA, "MPEG-1 Layer III header");
    assert!(
        bytes.windows(4).take(64).any(|w| w == b"Info"),
        "leading Info frame"
    );

    let ctx = ctx();
    let d = decode_stream(&ctx, &output, MediaType::Audio);
    assert_eq!(d.codec, "mp3");
    let p = d.params.unwrap();
    assert_eq!(p.sample_rate, Some(44_100));
    assert_eq!(p.channels, Some(2));
    // One second of audio (plus encoder delay / final-frame padding).
    assert!(
        (44_100..44_100 + 3 * 1_152).contains(&d.samples),
        "{} samples",
        d.samples
    );
    let f = zero_crossing_freq(&d.pcm, 44_100);
    assert!((f - 1_000.0).abs() < 20.0, "decoded tone at {f:.1} Hz");

    if let Some(s) = ffprobe_streams(&output) {
        assert!(s.contains("codec_name=mp3"), "{s}");
        assert!(s.contains("sample_rate=44100"), "{s}");
    }
    ffmpeg_decodes(&output);
    if let Some(mpg123) = tool("mpg123") {
        run_tool(&mpg123, &["-q", "-t", output.to_str().unwrap()])
            .unwrap_or_else(|e| panic!("mpg123 rejected the stream: {e}"));
    }
}

#[test]
fn wav_at_a_non_mpeg_rate_is_resampled_for_mp3() {
    let input = write_wav("tone96.wav", 96_000, 1, 2_000.0, 0.5);
    let output = scratch_file(TAG, "tone96.mp3");
    convert(&input, &output);
    let ctx = ctx();
    let d = decode_stream(&ctx, &output, MediaType::Audio);
    let p = d.params.unwrap();
    assert_eq!(p.sample_rate, Some(48_000), "96 kHz resampled to 48 kHz");
    let f = zero_crossing_freq(&d.pcm, 48_000);
    assert!((f - 2_000.0).abs() < 40.0, "decoded tone at {f:.1} Hz");
    ffmpeg_decodes(&output);
}

/// The RFC 7845 `OpusHead` input-sample-rate field of an Ogg Opus file.
fn opus_head_input_rate(bytes: &[u8]) -> u32 {
    let at = bytes
        .windows(8)
        .position(|w| w == b"OpusHead")
        .expect("OpusHead");
    u32::from_le_bytes(bytes[at + 12..at + 16].try_into().unwrap())
}

#[test]
fn wav_to_opus_accepts_rfc6716_rates_and_resamples_others() {
    let ctx = ctx();
    for (rate, channels, expect_head_rate) in [
        (16_000u32, 1u16, 16_000u32),
        (44_100, 2, 48_000),
        (48_000, 2, 48_000),
    ] {
        let input = write_wav(&format!("tone{rate}.wav"), rate, channels, 440.0, 1.0);
        let output = scratch_file(TAG, &format!("tone{rate}.opus"));
        convert(&input, &output);
        let bytes = std::fs::read(&output).unwrap();
        assert_eq!(&bytes[..4], b"OggS");
        assert_eq!(
            opus_head_input_rate(&bytes),
            expect_head_rate,
            "{rate} Hz input"
        );

        let d = decode_stream(&ctx, &output, MediaType::Audio);
        assert_eq!(d.codec, "opus");
        // The demuxer may surface the OpusHead input rate as the
        // decode rate (RFC 7845 §5.1 allows restoring it); measure at
        // whatever rate the decoder ran.
        let out_rate = d
            .params
            .as_ref()
            .and_then(|p| p.sample_rate)
            .unwrap_or(48_000);
        let f = zero_crossing_freq(&d.pcm, out_rate);
        assert!(
            (f - 440.0).abs() < 10.0,
            "{rate} Hz: decoded tone at {f:.1} Hz ({out_rate} Hz decode)"
        );
        // About one second (pre-skip trimmed, last packet padded).
        let secs = d.samples as f64 / f64::from(out_rate);
        assert!(
            (0.95..1.05).contains(&secs),
            "{rate} Hz: {secs:.3} s decoded"
        );
        if let Some(s) = ffprobe_streams(&output) {
            assert!(s.contains("codec_name=opus"), "{s}");
        }
        ffmpeg_decodes(&output);
    }
}

#[test]
fn y4m_to_webm_and_mkv_encode_vp9_without_flags() {
    let input = write_y4m("clip.y4m", 64, 48, 6);
    let ctx = ctx();
    for name in ["clip.webm", "clip.mkv"] {
        let output = scratch_file(TAG, name);
        convert(&input, &output);
        let d = decode_stream(&ctx, &output, MediaType::Video);
        assert_eq!(d.codec, "vp9", "{name}");
        assert_eq!(d.frames, 6, "{name}: every frame decodes");
        if let Some(s) = ffprobe_streams(&output) {
            assert!(s.contains("codec_name=vp9"), "{name}: {s}");
            assert!(s.contains("pix_fmt=yuv420p"), "{name}: {s}");
            assert!(s.contains("width=64") && s.contains("height=48"), "{s}");
        }
        ffmpeg_decodes(&output);
    }
}

#[test]
fn y4m_to_y4m_keeps_every_picture() {
    let (w, h, n) = (32usize, 16usize, 3usize);
    let input = write_y4m("raw.y4m", w, h, n);
    let output = scratch_file(TAG, "raw_out.y4m");
    convert(&input, &output);
    // Pictures = the bytes after each `FRAME…\n` marker line.
    let pictures = |v: &[u8]| -> Vec<Vec<u8>> {
        let size = w * h * 3 / 2;
        let mut at = v.iter().position(|&c| c == b'\n').unwrap() + 1;
        let mut out = Vec::new();
        while at < v.len() {
            assert!(v[at..].starts_with(b"FRAME"));
            at += v[at..].iter().position(|&c| c == b'\n').unwrap() + 1;
            out.push(v[at..at + size].to_vec());
            at += size;
        }
        out
    };
    let a = pictures(&std::fs::read(&input).unwrap());
    let b = pictures(&std::fs::read(&output).unwrap());
    assert_eq!(a.len(), n);
    assert_eq!(a, b);
}
