//! Speex wideband (16 kHz) encoder ↔ decoder roundtrip.
//!
//! Encode a synthetic wideband chirp with the `speex` encoder running
//! in WB mode (NB-mode-5 + WB-mode-1 spectral-folding, 42-byte packets),
//! decode with the in-tree WB decoder, and assert the output has
//! finite energy and tracks the input spectrum.
//!
//! ### Quality expectations
//!
//! WB sub-mode 1 is the lowest-rate extension layer (36 bits/frame).
//! The high band (4–8 kHz) is reconstructed by spectrally folding the
//! NB innovation — effectively duplicating the NB band's noise-like
//! component into the high band and relying on LPC shaping to carve
//! out the formants. That means:
//!
//! - Pure high-frequency tones (> 4 kHz) without any energy below 4 kHz
//!   encode through the NB layer as low-frequency aliases (from QMF
//!   analysis the mirror-image) and come out sounding rough.
//! - Mixed-spectrum signals (speech-like, or chirps crossing 4 kHz)
//!   recover a plausible wideband envelope.
//!
//! The roundtrip PSNR we measure here is computed on a gain-corrected
//! residual (best linear gain removed before noise measurement), the
//! same metric the NB roundtrip uses. We set the floor at **12 dB** —
//! below the NB floor because the folding layer adds an extra noise
//! term in the 4–8 kHz band that the NB test doesn't see. 12 dB
//! cleanly separates "the encoder is working" (typically measures
//! 15–20 dB on speech-like input) from "total garbage" (< 3 dB).
//!
//! ### Known gaps
//!
//! - Only sub-mode 1 is emitted. The stochastic-codebook sub-modes
//!   (2/3/4) aren't implemented — they'd require a split-VQ search on
//!   the high-band residual, which is a much bigger lift.
//! - The folding-gain quantiser uses raw residual energy without
//!   perceptual weighting, so voice-like inputs with prominent
//!   sibilance above 4 kHz may sound dimmer than reference Speex
//!   output.

use oxideav_codec::Decoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, Packet, SampleFormat, TimeBase,
};
use oxideav_speex::decoder::make_decoder;
use oxideav_speex::encoder::make_encoder;
use oxideav_speex::wb_decoder::WB_FULL_FRAME_SIZE;

fn build_input(total_frames: usize) -> Vec<i16> {
    // Speech-like wideband signal: 3 Hz syllable envelope applied to a
    // multi-harmonic carrier (200 / 600 / 1600 / 3200 Hz) plus a
    // little high-band energy (4500 Hz) that only the WB extension
    // can carry. Amplitude kept moderate so the NB synthesis filter
    // stays comfortably under saturation.
    let sr = 16_000.0f32;
    let n = total_frames * WB_FULL_FRAME_SIZE;
    let mut out = Vec::with_capacity(n);
    let mut rng = 0x12345u32;
    for i in 0..n {
        let t = i as f32 / sr;
        let env = 0.5 + 0.5 * (2.0 * std::f32::consts::PI * 3.0 * t).sin().abs();
        let carrier = (2.0 * std::f32::consts::PI * 200.0 * t).sin()
            + 0.5 * (2.0 * std::f32::consts::PI * 600.0 * t).sin()
            + 0.25 * (2.0 * std::f32::consts::PI * 1600.0 * t).sin()
            + 0.15 * (2.0 * std::f32::consts::PI * 3200.0 * t).sin()
            + 0.1 * (2.0 * std::f32::consts::PI * 4500.0 * t).sin();
        rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
        let noise = (((rng >> 16) & 0x7FFF) as f32 / 32768.0) - 0.5;
        let s = 3000.0 * env * carrier + 150.0 * noise;
        out.push(s.round().clamp(-32768.0, 32767.0) as i16);
    }
    out
}

fn audio_frame_s16(samples: &[i16]) -> AudioFrame {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    AudioFrame {
        format: SampleFormat::S16,
        channels: 1,
        sample_rate: 16_000,
        samples: samples.len() as u32,
        pts: None,
        time_base: TimeBase::new(1, 16_000),
        data: vec![bytes],
    }
}

fn decode_all(decoder: &mut Box<dyn Decoder>, packets: &[Packet]) -> Vec<i16> {
    let mut pcm = Vec::new();
    for p in packets {
        decoder.send_packet(p).expect("send_packet");
        loop {
            match decoder.receive_frame() {
                Ok(Frame::Audio(af)) => {
                    let bytes = &af.data[0];
                    for chunk in bytes.chunks_exact(2) {
                        pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
                    }
                }
                Ok(_) => {}
                Err(Error::NeedMore) | Err(Error::Eof) => break,
                Err(e) => panic!("decode error: {e}"),
            }
        }
    }
    let _ = decoder.flush();
    pcm
}

fn rms_i16(x: &[i16]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    let sum: f64 = x.iter().map(|&v| (v as f64) * (v as f64)).sum();
    ((sum / x.len() as f64).sqrt()) as f32
}

/// Gain-corrected segmental SNR. Scans a short alignment window to pick
/// the best time offset (decoder has a QMF + sub-frame group delay of
/// up to ~96 samples at 16 kHz), then computes the best linear gain
/// that minimises ||ref - g·tst||^2 and reports the remaining noise
/// energy under that correction.
fn snr_db(reference: &[i16], test: &[i16]) -> f32 {
    let n = reference.len().min(test.len());
    // Warm-up: skip the first ~200 samples so the QMF synth memory
    // settles and the NB decoder's first sub-frame (which emits ~zero
    // from a cold-start) doesn't dominate the SNR.
    let warm = 400;
    let max_shift = 240usize; // ~15 ms of search — generous for WB
    if n <= warm + max_shift + 64 {
        return 0.0;
    }
    let mut best = f32::NEG_INFINITY;
    for shift in 0..max_shift {
        let end = n.saturating_sub(max_shift);
        if end <= warm {
            break;
        }
        let ref_slice = &reference[warm..end];
        let tst_slice = &test[warm + shift..end + shift];
        let sig_pow: f64 = ref_slice
            .iter()
            .map(|&v| (v as f64) * (v as f64))
            .sum::<f64>()
            / ref_slice.len() as f64;
        let rt: f64 = ref_slice
            .iter()
            .zip(tst_slice.iter())
            .map(|(&a, &b)| (a as f64) * (b as f64))
            .sum::<f64>();
        let tt: f64 = tst_slice.iter().map(|&b| (b as f64) * (b as f64)).sum();
        let g = if tt > 1e-9 { rt / tt } else { 1.0 };
        let noise_pow: f64 = ref_slice
            .iter()
            .zip(tst_slice.iter())
            .map(|(&a, &b)| {
                let d = (a as f64) - g * (b as f64);
                d * d
            })
            .sum::<f64>()
            / ref_slice.len() as f64;
        if noise_pow < 1e-9 {
            return 200.0;
        }
        let snr = 10.0 * (sig_pow / noise_pow).log10();
        if snr as f32 > best {
            best = snr as f32;
        }
    }
    best
}

#[test]
fn encode_decode_wb_zero_input_stays_quiet() {
    // Sanity: 16 kHz zeros should decode back to (near-)silence.
    let input = vec![0i16; 10 * WB_FULL_FRAME_SIZE];
    let mut params = CodecParameters::audio(CodecId::new("speex"));
    params.sample_rate = Some(16_000);
    params.channels = Some(1);
    params.sample_format = Some(SampleFormat::S16);
    let mut enc = make_encoder(&params).expect("speex wb encoder");
    enc.send_frame(&Frame::Audio(audio_frame_s16(&input)))
        .unwrap();
    enc.flush().unwrap();
    let mut packets = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(p) => packets.push(p),
            Err(Error::NeedMore) | Err(Error::Eof) => break,
            Err(e) => panic!("{e}"),
        }
    }
    let mut dec_params = enc.output_params().clone();
    dec_params.codec_id = CodecId::new("speex");
    let mut dec = make_decoder(&dec_params).expect("speex wb decoder");
    let decoded = decode_all(&mut dec, &packets);
    let out_rms = rms_i16(&decoded);
    assert!(
        out_rms < 500.0,
        "zero-input WB encode should decode to (near-)silence, got RMS {out_rms}"
    );
}

#[test]
fn encode_decode_wb_roundtrip_is_coherent() {
    // ~0.5 second of periodic speech-like wideband audio (25 frames
    // × 20 ms).
    let input = build_input(25);
    let input_rms = rms_i16(&input);
    assert!(
        input_rms > 100.0,
        "synthetic WB input should be loud enough"
    );

    let mut params = CodecParameters::audio(CodecId::new("speex"));
    params.sample_rate = Some(16_000);
    params.channels = Some(1);
    params.sample_format = Some(SampleFormat::S16);
    let mut enc = make_encoder(&params).expect("speex wb encoder");
    enc.send_frame(&Frame::Audio(audio_frame_s16(&input)))
        .expect("send_frame");
    enc.flush().expect("flush encoder");

    let mut packets = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(p) => packets.push(p),
            Err(Error::NeedMore) | Err(Error::Eof) => break,
            Err(e) => panic!("encoder receive_packet: {e}"),
        }
    }
    let n_frames = input.len() / WB_FULL_FRAME_SIZE;
    assert_eq!(
        packets.len(),
        n_frames,
        "encoder should emit one packet per codec frame (got {}, expected {})",
        packets.len(),
        n_frames,
    );
    for p in &packets {
        assert_eq!(
            p.data.len(),
            42,
            "WB NB-5 + WB-1 packets are 336 bits = 42 bytes"
        );
    }

    let mut dec_params = enc.output_params().clone();
    dec_params.codec_id = CodecId::new("speex");
    let mut dec = make_decoder(&dec_params).expect("speex wb decoder");
    let decoded = decode_all(&mut dec, &packets);
    assert!(
        decoded.len() >= n_frames * WB_FULL_FRAME_SIZE - WB_FULL_FRAME_SIZE,
        "decoder should produce ~{} samples, got {}",
        n_frames * WB_FULL_FRAME_SIZE,
        decoded.len()
    );

    // Output must be non-silent and finite (i16 storage already
    // bounds it).
    let out_rms = rms_i16(&decoded);
    eprintln!("input RMS = {input_rms}, decoded RMS = {out_rms}");
    assert!(
        out_rms > 10.0,
        "decoded WB PCM should have non-negligible energy (RMS > 10), got {out_rms}"
    );

    // Gain-corrected SNR — at least 12 dB on this metric means the
    // spectral shape was preserved. WB adds noise above 4 kHz from
    // the spectral-folding reconstruction, so the floor sits below
    // the NB test's 8 dB (measured on pure-NB output).
    let snr = snr_db(&input, &decoded);
    eprintln!("WB encoder↔decoder gain-corrected SNR ≈ {snr:.1} dB (PSNR floor target: 12 dB)");
    assert!(
        snr > 12.0,
        "round-trip WB SNR should clear 12 dB, got {snr:.1} dB"
    );
}
