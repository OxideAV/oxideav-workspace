//! Encode → decode round-trip test for the MP3 CBR encoder.
//!
//! Generates a 1-second 440 Hz mono sine wave at 44.1 kHz, encodes it
//! at 128 kbps, decodes the resulting bitstream with our own decoder,
//! and uses a Goertzel filter to verify the dominant frequency component
//! is at 440 Hz with comfortable energy concentration.

use oxideav_core::{AudioFrame, CodecId, CodecParameters, Frame, Packet, SampleFormat, TimeBase};
use oxideav_mp3::decoder::make_decoder;
use oxideav_mp3::encoder::make_encoder;
use oxideav_mp3::frame::parse_frame_header;
use oxideav_mp3::CODEC_ID_STR;

fn build_sine_pcm(freq: f32, sample_rate: u32, duration_s: f32) -> Vec<i16> {
    let n = (sample_rate as f32 * duration_s) as usize;
    let mut out = Vec::with_capacity(n);
    let two_pi = 2.0 * std::f32::consts::PI;
    for i in 0..n {
        let t = i as f32 / sample_rate as f32;
        let s = (two_pi * freq * t).sin() * 0.5; // half-scale
        let q = (s * 32767.0) as i16;
        out.push(q);
    }
    out
}

/// Run a Goertzel resonator at `freq` over `pcm` and return
/// (target_bin_power, total_energy). Ratio = power/energy is the
/// fraction of energy concentrated in that bin.
fn goertzel(pcm: &[f32], sample_rate: u32, freq: f32) -> (f32, f32) {
    let n = pcm.len();
    let k = (n as f32 * freq / sample_rate as f32).round();
    let omega = 2.0 * std::f32::consts::PI * k / n as f32;
    let coeff = 2.0 * omega.cos();
    let mut s_prev = 0.0f32;
    let mut s_prev2 = 0.0f32;
    for &x in pcm {
        let s = x + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }
    let power = s_prev2 * s_prev2 + s_prev * s_prev - coeff * s_prev * s_prev2;
    let energy: f32 = pcm.iter().map(|x| x * x).sum();
    (power, energy)
}

fn encode_to_bytes(pcm: &[i16], sample_rate: u32, channels: u16, bitrate_bps: u64) -> Vec<u8> {
    let mut params = CodecParameters::audio(CodecId::new(CODEC_ID_STR));
    params.channels = Some(channels);
    params.sample_rate = Some(sample_rate);
    params.sample_format = Some(SampleFormat::S16);
    params.bit_rate = Some(bitrate_bps);

    let mut enc = make_encoder(&params).expect("encoder");
    let tb = TimeBase::new(1, sample_rate as i64);

    // Feed in chunks of 1152 samples per channel.
    let chunk = 1152 * channels as usize;
    let mut bytes_in: Vec<u8> = Vec::with_capacity(pcm.len() * 2);
    for &s in pcm {
        bytes_in.extend_from_slice(&s.to_le_bytes());
    }
    let mut pts: i64 = 0;
    for slice in bytes_in.chunks(chunk * 2) {
        let n_samples = slice.len() / (2 * channels as usize);
        let frame = AudioFrame {
            format: SampleFormat::S16,
            channels,
            sample_rate,
            samples: n_samples as u32,
            pts: Some(pts),
            time_base: tb,
            data: vec![slice.to_vec()],
        };
        enc.send_frame(&Frame::Audio(frame)).expect("send_frame");
        pts += n_samples as i64;
    }
    enc.flush().expect("flush");

    let mut out: Vec<u8> = Vec::new();
    while let Ok(p) = enc.receive_packet() {
        out.extend_from_slice(&p.data);
    }
    out
}

fn decode_to_pcm(bitstream: &[u8], sample_rate: u32) -> Vec<f32> {
    let params = CodecParameters::audio(CodecId::new(CODEC_ID_STR));
    let mut dec = make_decoder(&params).expect("decoder");
    let tb = TimeBase::new(1, sample_rate as i64);
    let mut pcm: Vec<f32> = Vec::new();
    let mut pos = 0usize;
    while pos + 4 <= bitstream.len() {
        let Ok(hdr) = parse_frame_header(&bitstream[pos..]) else {
            break;
        };
        let Some(flen) = hdr.frame_bytes() else { break };
        let flen = flen as usize;
        if pos + flen > bitstream.len() {
            break;
        }
        let pkt = Packet::new(0, tb, bitstream[pos..pos + flen].to_vec());
        if let Err(e) = dec.send_packet(&pkt) {
            eprintln!("decoder send_packet err at pos={pos}: {e:?}");
            pos += flen;
            continue;
        }
        match dec.receive_frame() {
            Ok(Frame::Audio(a)) => {
                for chunk in a.data[0].chunks_exact(2) {
                    let s = i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0;
                    pcm.push(s);
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("decoder receive_frame err at pos={pos}: {e:?}");
            }
        }
        pos += flen;
    }
    pcm
}

/// Compute SNR-style ratio: power at the target frequency divided by
/// the average power across a set of off-target "noise" bins.
fn snr_ratio(pcm: &[f32], sample_rate: u32, target: f32, noise_bins: &[f32]) -> f32 {
    let (p_target, _) = goertzel(pcm, sample_rate, target);
    let mut acc = 0.0f32;
    for &f in noise_bins {
        let (p, _) = goertzel(pcm, sample_rate, f);
        acc += p;
    }
    let avg_noise = acc / noise_bins.len().max(1) as f32 + 1e-12;
    p_target / avg_noise
}

#[test]
fn encode_decode_440hz_mono_44100() {
    let sample_rate = 44_100u32;
    let pcm = build_sine_pcm(440.0, sample_rate, 1.0);
    let bytes = encode_to_bytes(&pcm, sample_rate, 1, 128_000);

    // Sanity: file should be roughly 128_000 / 8 = 16_000 bytes/s for 1s.
    assert!(
        bytes.len() > 8_000 && bytes.len() < 30_000,
        "unexpected MP3 size: {}",
        bytes.len()
    );

    // Round-trip decode through our own decoder.
    let decoded = decode_to_pcm(&bytes, sample_rate);
    assert!(
        decoded.len() >= 4 * 1152,
        "too few samples decoded: {}",
        decoded.len()
    );

    // Skip warm-up frames (decoder + encoder padding).
    let warmup = 4 * 1152;
    let analysis = &decoded[warmup..];
    let noise_bins = [180.0_f32, 320.0, 1500.0, 3000.0, 7000.0];
    let ratio = snr_ratio(analysis, sample_rate, 440.0, &noise_bins);
    eprintln!(
        "440Hz mono 44.1k 128kbps own-decode SNR ratio: {ratio:.2} bytes={}",
        bytes.len()
    );
    assert!(
        ratio >= 30.0,
        "440Hz SNR ratio too low after own-decoder round-trip: {ratio:.2}"
    );
}

#[test]
fn encode_decode_440hz_stereo_44100() {
    let sample_rate = 44_100u32;
    // Build a stereo PCM with 440Hz on both channels (interleaved L,R).
    let mono = build_sine_pcm(440.0, sample_rate, 1.0);
    let mut stereo: Vec<i16> = Vec::with_capacity(mono.len() * 2);
    for &s in &mono {
        stereo.push(s);
        stereo.push(s);
    }
    let bytes = encode_to_bytes(&stereo, sample_rate, 2, 192_000);
    assert!(bytes.len() > 16_000 && bytes.len() < 40_000);
    let decoded = decode_to_pcm(&bytes, sample_rate);
    assert!(decoded.len() >= 4 * 1152 * 2);
    // De-interleave left channel.
    let warmup = 4 * 1152 * 2;
    let l: Vec<f32> = decoded[warmup..].chunks_exact(2).map(|p| p[0]).collect();
    let noise_bins = [180.0_f32, 320.0, 1500.0, 3000.0, 7000.0];
    let ratio = snr_ratio(&l, sample_rate, 440.0, &noise_bins);
    eprintln!("stereo 440Hz own-decode L-channel SNR ratio: {ratio:.2}");
    assert!(ratio >= 30.0, "stereo SNR ratio too low: {ratio:.2}");
}

#[test]
fn encode_decode_440hz_mono_48000() {
    let sample_rate = 48_000u32;
    let pcm = build_sine_pcm(440.0, sample_rate, 1.0);
    let bytes = encode_to_bytes(&pcm, sample_rate, 1, 128_000);
    assert!(bytes.len() > 8_000 && bytes.len() < 30_000);
    let decoded = decode_to_pcm(&bytes, sample_rate);
    assert!(decoded.len() >= 4 * 1152);
    let warmup = 4 * 1152;
    let analysis = &decoded[warmup..];
    let noise_bins = [180.0_f32, 320.0, 1500.0, 3000.0, 7000.0];
    let ratio = snr_ratio(analysis, sample_rate, 440.0, &noise_bins);
    eprintln!("440Hz mono 48k 128kbps own-decode SNR ratio: {ratio:.2}");
    assert!(ratio >= 30.0, "48k SNR ratio too low: {ratio:.2}");
}

/// Run an ffmpeg decode and return the PCM samples (interleaved if
/// multi-channel). Returns None on missing ffmpeg.
fn ffmpeg_decode(mp3_bytes: &[u8], suffix: &str) -> Option<Vec<f32>> {
    use std::process::{Command, Stdio};
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        return None;
    }
    let tmp_mp3 = std::env::temp_dir().join(format!("oxideav_mp3_enc_{suffix}.mp3"));
    let tmp_wav = std::env::temp_dir().join(format!("oxideav_mp3_enc_{suffix}.wav"));
    std::fs::write(&tmp_mp3, mp3_bytes).expect("write mp3");
    let out = Command::new("ffmpeg")
        .arg("-y")
        .arg("-loglevel")
        .arg("warning")
        .arg("-i")
        .arg(&tmp_mp3)
        .arg("-f")
        .arg("wav")
        .arg(&tmp_wav)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("ffmpeg run");
    assert!(out.status.success(), "ffmpeg failed: {:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let suspicious_lines: Vec<&str> = stderr
        .lines()
        .filter(|l| !l.contains("Estimating duration from bitrate"))
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert!(
        suspicious_lines.is_empty(),
        "ffmpeg emitted warnings: {suspicious_lines:?}"
    );
    let wav = std::fs::read(&tmp_wav).expect("read wav");
    let data_off = wav
        .windows(4)
        .position(|w| w == b"data")
        .expect("WAV data tag")
        + 8;
    let mut decoded: Vec<f32> = Vec::new();
    for ch in wav[data_off..].chunks_exact(2) {
        decoded.push(i16::from_le_bytes([ch[0], ch[1]]) as f32 / 32768.0);
    }
    Some(decoded)
}

/// ffmpeg interop check (mono). Skipped silently when ffmpeg is
/// unavailable — keeps CI portable.
#[test]
fn encode_decode_440hz_mono_via_ffmpeg() {
    let sample_rate = 44_100u32;
    let pcm = build_sine_pcm(440.0, sample_rate, 1.0);
    let bytes = encode_to_bytes(&pcm, sample_rate, 1, 128_000);
    let Some(decoded) = ffmpeg_decode(&bytes, "mono44k") else {
        eprintln!("ffmpeg not available — skipping interop check");
        return;
    };
    assert!(decoded.len() >= 4 * 1152);
    let warmup = 4 * 1152;
    let analysis = &decoded[warmup..];
    let noise_bins = [180.0_f32, 320.0, 1500.0, 3000.0, 7000.0];
    let ratio = snr_ratio(analysis, sample_rate, 440.0, &noise_bins);
    eprintln!("440Hz mono ffmpeg-decoded SNR ratio: {ratio:.2}");
    assert!(
        ratio >= 30.0,
        "440Hz SNR ratio too low via ffmpeg: {ratio:.2}"
    );
}

/// ffmpeg interop check (stereo).
#[test]
fn encode_decode_440hz_stereo_via_ffmpeg() {
    let sample_rate = 44_100u32;
    let mono = build_sine_pcm(440.0, sample_rate, 1.0);
    let mut stereo: Vec<i16> = Vec::with_capacity(mono.len() * 2);
    for &s in &mono {
        stereo.push(s);
        stereo.push(s);
    }
    let bytes = encode_to_bytes(&stereo, sample_rate, 2, 192_000);
    let Some(decoded) = ffmpeg_decode(&bytes, "stereo44k") else {
        eprintln!("ffmpeg not available — skipping interop check");
        return;
    };
    assert!(decoded.len() >= 4 * 1152 * 2);
    let warmup = 4 * 1152 * 2;
    let l: Vec<f32> = decoded[warmup..].chunks_exact(2).map(|p| p[0]).collect();
    let noise_bins = [180.0_f32, 320.0, 1500.0, 3000.0, 7000.0];
    let ratio = snr_ratio(&l, sample_rate, 440.0, &noise_bins);
    eprintln!("440Hz stereo ffmpeg-decoded L-channel SNR ratio: {ratio:.2}");
    assert!(
        ratio >= 30.0,
        "stereo SNR ratio too low via ffmpeg: {ratio:.2}"
    );
}
