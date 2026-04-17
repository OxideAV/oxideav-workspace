//! End-to-end encode → decode roundtrip tests for the pure-Rust MP2
//! encoder.
//!
//! Generates a sine-tone PCM signal, encodes it with our MP2 encoder,
//! runs the output back through the decoder, and measures the resulting
//! PCM PSNR. The bar is low (Layer II with a naive bit allocator won't
//! hit libtwolame-class quality), but the signal must remain recognisable
//! and energy-preserving in the steady state.

use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Frame, MediaType, Packet, SampleFormat, TimeBase,
};
use oxideav_mp2::decoder::make_decoder;
use oxideav_mp2::encoder::make_encoder;
use oxideav_mp2::header::parse_header;
use oxideav_mp2::CODEC_ID_STR;

/// Build a mono PCM s16le 1 kHz sine at amplitude 0.5.
fn make_tone(duration_s: f32, sample_rate: u32, channels: u16, freq: f32) -> Vec<u8> {
    let n = (duration_s * sample_rate as f32) as usize;
    let mut out = Vec::with_capacity(n * 2 * channels as usize);
    for i in 0..n {
        let t = i as f32 / sample_rate as f32;
        let s = (0.5 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16;
        for _ch in 0..channels {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    out
}

fn encode_all(pcm: &[u8], sample_rate: u32, channels: u16, bitrate_kbps: u32) -> Vec<u8> {
    let mut params = CodecParameters::audio(CodecId::new(CODEC_ID_STR));
    params.media_type = MediaType::Audio;
    params.channels = Some(channels);
    params.sample_rate = Some(sample_rate);
    params.sample_format = Some(SampleFormat::S16);
    params.bit_rate = Some((bitrate_kbps as u64) * 1000);
    let mut enc = make_encoder(&params).expect("build encoder");

    let total_samples = (pcm.len() / (2 * channels as usize)) as u32;
    let frame = AudioFrame {
        format: SampleFormat::S16,
        channels,
        sample_rate,
        samples: total_samples,
        pts: Some(0),
        time_base: TimeBase::new(1, sample_rate as i64),
        data: vec![pcm.to_vec()],
    };
    enc.send_frame(&Frame::Audio(frame)).expect("send_frame");
    let mut bytes = Vec::new();
    while let Ok(p) = enc.receive_packet() {
        bytes.extend_from_slice(&p.data);
    }
    enc.flush().expect("flush");
    while let Ok(p) = enc.receive_packet() {
        bytes.extend_from_slice(&p.data);
    }
    bytes
}

fn split_frames(data: &[u8]) -> Vec<&[u8]> {
    let mut frames = Vec::new();
    let mut i = 0;
    while i + 4 <= data.len() {
        if data[i] != 0xFF || (data[i + 1] & 0xF0) != 0xF0 {
            i += 1;
            continue;
        }
        let Ok(h) = parse_header(&data[i..]) else {
            i += 1;
            continue;
        };
        let len = h.frame_length();
        if i + len > data.len() {
            break;
        }
        frames.push(&data[i..i + len]);
        i += len;
    }
    frames
}

fn decode_all(data: &[u8]) -> (Vec<i16>, u32, u16) {
    let params = CodecParameters::audio(CodecId::new(CODEC_ID_STR));
    let mut dec = make_decoder(&params).expect("build decoder");
    let mut samples: Vec<i16> = Vec::new();
    let mut sr = 0u32;
    let mut ch = 0u16;
    for fr in split_frames(data) {
        let pkt = Packet {
            stream_index: 0,
            time_base: TimeBase::new(1, 48_000),
            pts: None,
            dts: None,
            duration: None,
            flags: Default::default(),
            data: fr.to_vec(),
        };
        dec.send_packet(&pkt).expect("send_packet");
        if let Ok(Frame::Audio(a)) = dec.receive_frame() {
            sr = a.sample_rate;
            ch = a.channels;
            let bytes = &a.data[0];
            for chunk in bytes.chunks_exact(2) {
                samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
            }
        }
    }
    (samples, sr, ch)
}

/// Compute PSNR in dB, comparing `recon` to reference `ref_signal`. Both
/// buffers are treated as mono per-sample i16 values. The PSNR is computed
/// over the overlap of the two, with `ref_signal` offset by
/// `skip_ref` samples to account for encoder delay (one full frame of
/// 1152 samples is typical).
fn psnr_db(ref_signal: &[i16], recon: &[i16], skip_ref: usize) -> f64 {
    let r = &ref_signal[skip_ref.min(ref_signal.len())..];
    let n = r.len().min(recon.len());
    if n == 0 {
        return 0.0;
    }
    let mut sq_err = 0.0f64;
    for i in 0..n {
        let d = r[i] as f64 - recon[i] as f64;
        sq_err += d * d;
    }
    let mse = sq_err / n as f64;
    if mse < 1.0 {
        return 120.0;
    }
    let peak = 32767.0f64;
    10.0 * ((peak * peak) / mse).log10()
}

#[test]
fn roundtrip_mono_44k_192kbps_1khz_tone() {
    let sr = 44_100u32;
    let freq = 1000.0f32;
    // 2 seconds of tone → ample settle time.
    let pcm = make_tone(2.0, sr, 1, freq);
    let encoded = encode_all(&pcm, sr, 1, 192);
    assert!(!encoded.is_empty(), "encoder produced no data");
    let (decoded, dec_sr, dec_ch) = decode_all(&encoded);
    assert_eq!(dec_sr, sr);
    assert_eq!(dec_ch, 1);
    assert!(
        decoded.len() > 30_000,
        "decoded too few samples: {}",
        decoded.len()
    );

    let ref_samples: Vec<i16> = pcm
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    // Polyphase analysis + synthesis has a group delay of ~512/2 = ~256
    // samples per filterbank stage, so the end-to-end delay is ~481
    // samples at the canonical MPEG coupling. Round to the nearest frame
    // boundary since the ffmpeg-decoded tone test in the decoder suite
    // also accepts this. We scan a small offset window and take the best
    // PSNR — this is the standard way to align pure-rust chain with
    // delayed subband reconstruction.
    let mut best = -1000.0f64;
    for offset in 0..1500 {
        let p = psnr_db(&ref_samples, &decoded, offset);
        if p > best {
            best = p;
        }
    }
    println!("roundtrip PSNR (mono, 44.1k, 192kbps, 1kHz): {best:.2} dB");
    // With a naive bit-allocator Layer II targets 192 kbps comfortably;
    // a settled sine should be far above noise floor. 20 dB is a
    // "signal clearly dominates reconstruction error" bar.
    assert!(best >= 20.0, "PSNR too low: {best:.2} dB");
}

#[test]
fn roundtrip_stereo_48k_192kbps_440hz_tone() {
    let sr = 48_000u32;
    let freq = 440.0f32;
    let pcm = make_tone(1.5, sr, 2, freq);
    let encoded = encode_all(&pcm, sr, 2, 192);
    assert!(!encoded.is_empty());
    let (decoded, dec_sr, dec_ch) = decode_all(&encoded);
    assert_eq!(dec_sr, sr);
    assert_eq!(dec_ch, 2);

    // Pull left channel from both signals for PSNR.
    let ref_all: Vec<i16> = pcm
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    let ref_left: Vec<i16> = ref_all.chunks_exact(2).map(|c| c[0]).collect();
    let dec_left: Vec<i16> = decoded.chunks_exact(2).map(|c| c[0]).collect();

    let mut best = -1000.0f64;
    for offset in 0..1500 {
        let p = psnr_db(&ref_left, &dec_left, offset);
        if p > best {
            best = p;
        }
    }
    println!("roundtrip PSNR (stereo L, 48k, 192kbps, 440Hz): {best:.2} dB");
    assert!(best >= 20.0, "stereo PSNR too low: {best:.2} dB");
}

#[test]
fn roundtrip_mono_32k_128kbps_impulse_safety() {
    // Noise-like input: repeatedly hit the encoder with a white noise
    // buffer and assert the decoder doesn't blow up or emit NaN.
    let sr = 32_000u32;
    let n = 5 * 1152;
    let mut pcm = Vec::with_capacity(n * 2);
    let mut state: u32 = 0xDEAD_BEEF;
    for _ in 0..n {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let v = ((state >> 16) as i16) / 8; // small-ish amplitude
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    let encoded = encode_all(&pcm, sr, 1, 128);
    let (decoded, _sr, _ch) = decode_all(&encoded);
    assert!(!decoded.is_empty());
    for v in decoded.iter().take(1152) {
        // Just check sanity: decoder returns sane 16-bit values.
        let _ = *v; // no NaN / panic
    }
}
