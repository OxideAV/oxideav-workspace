//! Single-tap delay echo with feedback and wet/dry mix.
//!
//! For each channel a circular delay line of `delay_ms` worth of samples is
//! kept across `process` calls. Output is computed as
//!
//! ```text
//! out  = dry * (1 - mix) + delayed * mix
//! line = dry + delayed * feedback
//! ```
//!
//! # Parameters
//! * `delay_ms` — delay length in milliseconds (must be > 0).
//! * `feedback` — fraction of the delayed signal fed back into the line,
//!   in `[0.0, 1.0]`. Higher values produce longer-tailed echoes.
//! * `mix` — wet/dry blend in `[0.0, 1.0]`. `0.0` is fully dry, `1.0` is
//!   fully wet.

use crate::sample_convert::{decode_to_f32, encode_from_f32};
use crate::AudioFilter;
use oxideav_core::{AudioFrame, Result};

#[derive(Debug, Clone)]
pub struct Echo {
    delay_ms: f32,
    feedback: f32,
    mix: f32,
    state: Option<EchoState>,
}

#[derive(Debug, Clone)]
struct EchoState {
    sample_rate: u32,
    channels: usize,
    /// One ring buffer per channel.
    lines: Vec<Vec<f32>>,
    /// Per-channel write index into the ring buffer.
    write_idx: Vec<usize>,
}

impl Echo {
    pub fn new(delay_ms: f32, feedback: f32, mix: f32) -> Self {
        Self {
            delay_ms,
            feedback: feedback.clamp(0.0, 1.0),
            mix: mix.clamp(0.0, 1.0),
            state: None,
        }
    }

    fn ensure_state(&mut self, sample_rate: u32, channels: usize) {
        let needs_rebuild = match &self.state {
            Some(s) => s.sample_rate != sample_rate || s.channels != channels,
            None => true,
        };
        if needs_rebuild {
            let line_len = ((self.delay_ms / 1000.0) * sample_rate as f32).max(1.0) as usize;
            self.state = Some(EchoState {
                sample_rate,
                channels,
                lines: (0..channels).map(|_| vec![0.0; line_len]).collect(),
                write_idx: vec![0; channels],
            });
        }
    }
}

impl AudioFilter for Echo {
    fn process(&mut self, input: &AudioFrame) -> Result<Vec<AudioFrame>> {
        let n_chan = input.channels as usize;
        self.ensure_state(input.sample_rate, n_chan);
        let mut channels = decode_to_f32(input)?;
        let n_samples = channels.first().map(|c| c.len()).unwrap_or(0);
        let dry_mix = 1.0 - self.mix;

        let state = self.state.as_mut().expect("state ensured above");

        for (ch, buf) in channels.iter_mut().enumerate().take(n_chan) {
            let line = &mut state.lines[ch];
            let line_len = line.len();
            let mut idx = state.write_idx[ch];
            for sample in buf.iter_mut().take(n_samples) {
                let dry = *sample;
                let delayed = line[idx];
                let out = dry * dry_mix + delayed * self.mix;
                line[idx] = dry + delayed * self.feedback;
                *sample = out;
                idx += 1;
                if idx >= line_len {
                    idx = 0;
                }
            }
            state.write_idx[ch] = idx;
        }

        let out = encode_from_f32(input, &channels)?;
        Ok(vec![out])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{SampleFormat, TimeBase};

    fn impulse_f32(n: usize, rate: u32) -> AudioFrame {
        let mut samples = vec![0.0f32; n];
        samples[0] = 1.0;
        let mut bytes = Vec::with_capacity(n * 4);
        for s in &samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        AudioFrame {
            format: SampleFormat::F32,
            channels: 1,
            sample_rate: rate,
            samples: n as u32,
            pts: None,
            time_base: TimeBase::new(1, rate as i64),
            data: vec![bytes],
        }
    }

    fn read_f32(frame: &AudioFrame) -> Vec<f32> {
        frame.data[0]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    #[test]
    fn impulse_produces_delayed_copy() {
        // 10 ms delay at 48 kHz = 480 samples
        let mut e = Echo::new(10.0, 0.0, 0.5);
        let frame = impulse_f32(2000, 48_000);
        let out = e.process(&frame).unwrap();
        let got = read_f32(&out[0]);
        // dry sample at index 0 is multiplied by (1 - 0.5) = 0.5
        assert!((got[0] - 0.5).abs() < 1.0e-5);
        // echo at delay index = 480 with mix=0.5
        assert!((got[480] - 0.5).abs() < 1.0e-3);
        // surrounding samples are 0
        assert!(got[10].abs() < 1.0e-6);
    }

    #[test]
    fn feedback_creates_repeating_echoes() {
        let mut e = Echo::new(10.0, 0.5, 1.0); // wet only, feedback=0.5
        let frame = impulse_f32(4000, 48_000);
        let out = e.process(&frame).unwrap();
        let got = read_f32(&out[0]);
        // dry suppressed (mix=1 means out = delayed), so got[0] = 0
        assert!(got[0].abs() < 1.0e-6);
        // first echo at 480
        assert!((got[480] - 1.0).abs() < 1.0e-4);
        // second echo at 960 = feedback^1 = 0.5
        assert!((got[960] - 0.5).abs() < 1.0e-3);
        // third at 1440 = 0.25
        assert!((got[1440] - 0.25).abs() < 1.0e-3);
    }
}
