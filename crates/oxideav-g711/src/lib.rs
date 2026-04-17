//! ITU-T G.711 µ-law / A-law PCM codec.
//!
//! G.711 is the classic PSTN audio codec — 8 kHz mono, 8-bit samples that
//! round-trip to ~13-bit linear quality via a logarithmic companding curve.
//! Two variants, selected by codec id:
//!
//! - **µ-law** (North America / Japan): codec ids `pcm_mulaw`, `ulaw`,
//!   `g711u`.
//! - **A-law** (rest of the world): codec ids `pcm_alaw`, `alaw`, `g711a`.
//!
//! Both variants are **byte-for-sample**: one encoded byte on input
//! yields one S16 PCM sample on output, and vice versa. The spec defines
//! G.711 at 8 kHz but the implementation works at any sample rate the
//! caller provides — the companding math is independent of rate.
//!
//! # Algorithm
//!
//! Decoding uses a compile-time 256-entry lookup table generated from the
//! ITU-T G.711 bit layout (see [`tables`]). Encoding is arithmetic: sign
//! extraction → bias + segment search → mantissa extraction → on-wire
//! inversion. There is no signal processing state, so each byte /sample
//! is independent and packets may be arbitrary-length.
//!
//! # Registration
//!
//! [`register`] wires up both laws under each of their aliases via
//! `CodecRegistry::register_both` — i.e. `pcm_mulaw`, `ulaw`, and `g711u`
//! all resolve to the same [`mulaw::UlawDecoder`] / [`mulaw::UlawEncoder`]
//! pair, and likewise for A-law.

#![deny(unsafe_code)]
#![allow(clippy::needless_range_loop)]

pub mod alaw;
pub mod mulaw;
pub mod tables;

use oxideav_codec::CodecRegistry;
use oxideav_core::{CodecCapabilities, CodecId};

/// Canonical codec id for µ-law (matches FFmpeg's `pcm_mulaw`).
pub const CODEC_ID_MULAW: &str = "pcm_mulaw";

/// Canonical codec id for A-law (matches FFmpeg's `pcm_alaw`).
pub const CODEC_ID_ALAW: &str = "pcm_alaw";

/// Aliases that resolve to the µ-law implementation.
pub const MULAW_ALIASES: &[&str] = &["pcm_mulaw", "ulaw", "g711u"];

/// Aliases that resolve to the A-law implementation.
pub const ALAW_ALIASES: &[&str] = &["pcm_alaw", "alaw", "g711a"];

/// Register every G.711 codec id + alias for both decode and encode.
pub fn register(reg: &mut CodecRegistry) {
    // µ-law: one registration per alias so calls with any of them resolve
    // cleanly.
    for alias in MULAW_ALIASES {
        let caps = CodecCapabilities::audio("g711_mulaw_sw")
            .with_lossy(true)
            .with_intra_only(true)
            .with_max_channels(1);
        reg.register_both(
            CodecId::new(*alias),
            caps,
            mulaw::make_decoder,
            mulaw::make_encoder,
        );
    }

    // A-law: same story.
    for alias in ALAW_ALIASES {
        let caps = CodecCapabilities::audio("g711_alaw_sw")
            .with_lossy(true)
            .with_intra_only(true)
            .with_max_channels(1);
        reg.register_both(
            CodecId::new(*alias),
            caps,
            alaw::make_decoder,
            alaw::make_encoder,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{CodecParameters, Frame, Packet, SampleFormat, TimeBase};

    fn params(id: &str) -> CodecParameters {
        let mut p = CodecParameters::audio(CodecId::new(id));
        p.sample_rate = Some(8_000);
        p.channels = Some(1);
        p.sample_format = Some(SampleFormat::S16);
        p
    }

    #[test]
    fn register_all_aliases() {
        let mut reg = CodecRegistry::new();
        register(&mut reg);
        for alias in MULAW_ALIASES.iter().chain(ALAW_ALIASES.iter()) {
            let id = CodecId::new(*alias);
            assert!(reg.has_decoder(&id), "no decoder for alias {alias}");
            assert!(reg.has_encoder(&id), "no encoder for alias {alias}");
        }
    }

    #[test]
    fn mulaw_aliases_resolve_to_same_impl() {
        let mut reg = CodecRegistry::new();
        register(&mut reg);
        // Build a decoder for every alias and feed it the same byte. All
        // must produce the same S16 result.
        let input = vec![0x55u8, 0xAA, 0x80, 0x00];
        let mut results = Vec::new();
        for alias in MULAW_ALIASES {
            let p = params(alias);
            let mut dec = reg.make_decoder(&p).expect("make_decoder");
            let pkt = Packet::new(0, TimeBase::new(1, 8_000), input.clone());
            dec.send_packet(&pkt).unwrap();
            let Frame::Audio(af) = dec.receive_frame().unwrap() else {
                panic!("expected audio frame");
            };
            results.push(af.data[0].clone());
        }
        for r in &results[1..] {
            assert_eq!(r, &results[0]);
        }
    }

    #[test]
    fn mulaw_roundtrip_samples() {
        let mut reg = CodecRegistry::new();
        register(&mut reg);
        let p = params(CODEC_ID_MULAW);
        let mut enc = reg.make_encoder(&p).expect("make_encoder");
        let mut dec = reg.make_decoder(&p).expect("make_decoder");

        let samples: Vec<i16> = vec![0, 1, -1, 100, -100, 10000, -10000, 32000, -32000];
        let mut pcm_bytes = Vec::with_capacity(samples.len() * 2);
        for &s in &samples {
            pcm_bytes.extend_from_slice(&s.to_le_bytes());
        }
        let input = Frame::Audio(oxideav_core::AudioFrame {
            format: SampleFormat::S16,
            channels: 1,
            sample_rate: 8_000,
            samples: samples.len() as u32,
            pts: Some(0),
            time_base: TimeBase::new(1, 8_000),
            data: vec![pcm_bytes],
        });
        enc.send_frame(&input).unwrap();
        let pkt = enc.receive_packet().unwrap();
        assert_eq!(pkt.data.len(), samples.len());
        dec.send_packet(&pkt).unwrap();
        let Frame::Audio(af) = dec.receive_frame().unwrap() else {
            panic!("expected audio frame");
        };
        assert_eq!(af.samples as usize, samples.len());
    }
}
