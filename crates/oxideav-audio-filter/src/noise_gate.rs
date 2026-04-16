//! Per-sample noise gate.
//!
//! A noise gate attenuates audio that falls below a threshold. The internal
//! envelope follower watches the absolute value of the signal and either
//! ramps the output gain toward `1.0` (when above threshold) or toward `0.0`
//! (when continuously below threshold for `hold` samples).
//!
//! For multi-channel input the channels share a single linked envelope: the
//! per-sample drive value is `max(|x_ch_0|, |x_ch_1|, ...)`. This keeps the
//! channels gated together so a stereo image does not collapse when only one
//! side dips below the threshold.
//!
//! # Parameters
//!
//! * `threshold_db` — gate opens when |signal| > 10^(threshold_db / 20). A
//!   typical value is `-40.0` dBFS.
//! * `attack_ms` — time over which the gain ramps from current to `1.0`
//!   when the signal exceeds the threshold.
//! * `release_ms` — time over which the gain ramps from current to `0.0`
//!   after the hold period elapses.
//! * `hold_ms` — how long the signal must remain below threshold before
//!   the release ramp begins.

use crate::sample_convert::{decode_to_f32, encode_from_f32};
use crate::AudioFilter;
use oxideav_core::{AudioFrame, Result};

#[derive(Debug, Clone)]
pub struct NoiseGate {
    threshold_db: f32,
    attack_ms: f32,
    release_ms: f32,
    hold_ms: f32,
    // Cached state, updated lazily when the sample rate changes
    state: Option<GateState>,
}

#[derive(Debug, Clone)]
struct GateState {
    sample_rate: u32,
    threshold_lin: f32,
    attack_step: f32,
    release_step: f32,
    hold_samples: u32,
    gain: f32,
    below_count: u32,
}

impl NoiseGate {
    pub fn new(threshold_db: f32, attack_ms: f32, release_ms: f32, hold_ms: f32) -> Self {
        Self {
            threshold_db,
            attack_ms,
            release_ms,
            hold_ms,
            state: None,
        }
    }

    fn ensure_state(&mut self, sample_rate: u32) {
        let needs_rebuild = match &self.state {
            Some(s) => s.sample_rate != sample_rate,
            None => true,
        };
        if needs_rebuild {
            let threshold_lin = 10.0f32.powf(self.threshold_db / 20.0);
            let attack_samples = ((self.attack_ms / 1000.0) * sample_rate as f32).max(1.0);
            let release_samples = ((self.release_ms / 1000.0) * sample_rate as f32).max(1.0);
            let hold_samples = ((self.hold_ms / 1000.0) * sample_rate as f32).max(0.0) as u32;
            self.state = Some(GateState {
                sample_rate,
                threshold_lin,
                attack_step: 1.0 / attack_samples,
                release_step: 1.0 / release_samples,
                hold_samples,
                gain: 0.0,
                below_count: 0,
            });
        }
    }
}

impl AudioFilter for NoiseGate {
    fn process(&mut self, input: &AudioFrame) -> Result<Vec<AudioFrame>> {
        self.ensure_state(input.sample_rate);
        let mut channels = decode_to_f32(input)?;
        let n_samples = channels.first().map(|c| c.len()).unwrap_or(0);
        let n_chan = channels.len();
        let state = self.state.as_mut().expect("ensure_state succeeded");

        for s in 0..n_samples {
            let mut drive = 0.0f32;
            for ch in channels.iter().take(n_chan) {
                let abs = ch[s].abs();
                if abs > drive {
                    drive = abs;
                }
            }

            if drive > state.threshold_lin {
                state.below_count = 0;
                state.gain = (state.gain + state.attack_step).min(1.0);
            } else {
                state.below_count = state.below_count.saturating_add(1);
                if state.below_count > state.hold_samples {
                    state.gain = (state.gain - state.release_step).max(0.0);
                }
            }

            for ch in channels.iter_mut().take(n_chan) {
                ch[s] *= state.gain;
            }
        }

        let out = encode_from_f32(input, &channels)?;
        Ok(vec![out])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{SampleFormat, TimeBase};

    fn make_f32_mono(samples: &[f32]) -> AudioFrame {
        let mut bytes = Vec::with_capacity(samples.len() * 4);
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        AudioFrame {
            format: SampleFormat::F32,
            channels: 1,
            sample_rate: 48_000,
            samples: samples.len() as u32,
            pts: None,
            time_base: TimeBase::new(1, 48_000),
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
    fn quiet_signal_is_attenuated() {
        // -60 dBFS noise gate at -40 dBFS threshold
        let samples = vec![0.0001f32; 48_000];
        let frame = make_f32_mono(&samples);
        let mut g = NoiseGate::new(-40.0, 1.0, 1.0, 0.0);
        let out = g.process(&frame).unwrap();
        let got = read_f32(&out[0]);
        // After many samples gain should have collapsed to 0
        let last = *got.last().unwrap();
        assert!(last.abs() < 1.0e-6, "expected gate closed, got {}", last);
    }

    #[test]
    fn loud_signal_passes_through() {
        // Half-scale tone, well above -40 dBFS
        let mut samples = Vec::with_capacity(2_000);
        for i in 0..2_000 {
            samples.push(0.5 * ((i as f32) * 0.1).sin());
        }
        let frame = make_f32_mono(&samples);
        let mut g = NoiseGate::new(-40.0, 1.0, 50.0, 5.0);
        let out = g.process(&frame).unwrap();
        let got = read_f32(&out[0]);
        // After attack ramp the loud tone should reach near full amplitude
        let tail = &got[got.len() - 100..];
        let peak = tail.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
        assert!(peak > 0.4, "expected open gate, peak={}", peak);
    }
}
