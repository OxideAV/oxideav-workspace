//! Volume / gain filter.
//!
//! Multiplies every sample by a constant linear gain. Output is hard-clipped
//! to the destination format's full-scale range — silently, without a
//! soft-knee or look-ahead. Use [`Volume::from_db`] for decibel-relative
//! settings.

use crate::sample_convert::{decode_to_f32, encode_from_f32};
use crate::AudioFilter;
use oxideav_core::{AudioFrame, Result};

/// Per-sample multiplicative gain.
///
/// # Parameters
/// * `gain` — linear amplification factor; `1.0` is unity, `0.5` is -6 dB,
///   `2.0` is +6 dB. Negative values invert phase.
#[derive(Debug, Clone)]
pub struct Volume {
    gain: f32,
}

impl Volume {
    /// Create a new gain stage with a linear multiplier.
    pub fn new(gain: f32) -> Self {
        Self { gain }
    }

    /// Create a new gain stage from a decibel value: `gain = 10^(db/20)`.
    pub fn from_db(db: f32) -> Self {
        Self {
            gain: 10.0f32.powf(db / 20.0),
        }
    }

    /// Current linear gain.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Update the linear gain.
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }
}

impl AudioFilter for Volume {
    fn process(&mut self, input: &AudioFrame) -> Result<Vec<AudioFrame>> {
        let mut channels = decode_to_f32(input)?;
        for ch in channels.iter_mut() {
            for s in ch.iter_mut() {
                *s = (*s * self.gain).clamp(-1.0, 1.0);
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

    fn s16_mono(samples: &[i16]) -> AudioFrame {
        let mut data = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            data.extend_from_slice(&s.to_le_bytes());
        }
        AudioFrame {
            format: SampleFormat::S16,
            channels: 1,
            sample_rate: 48_000,
            samples: samples.len() as u32,
            pts: None,
            time_base: TimeBase::new(1, 48_000),
            data: vec![data],
        }
    }

    fn read_s16(frame: &AudioFrame) -> Vec<i16> {
        frame.data[0]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    #[test]
    fn unity_gain_is_identity() {
        let frame = s16_mono(&[0, 100, -100, 16384, -16384]);
        let mut v = Volume::new(1.0);
        let out = v.process(&frame).unwrap();
        assert_eq!(out.len(), 1);
        let got = read_s16(&out[0]);
        for (a, b) in got.iter().zip([0i16, 100, -100, 16384, -16384].iter()) {
            assert!((*a - *b).abs() <= 1);
        }
    }

    #[test]
    fn from_db_doubles_at_6db() {
        let frame = s16_mono(&[1000, -1000, 5000]);
        let mut v = Volume::from_db(6.0);
        let out = v.process(&frame).unwrap();
        let got = read_s16(&out[0]);
        // 6.0 dB linear ≈ 1.9953x; allow a 1 % tolerance.
        assert!((got[0] as f32 - 1000.0 * 1.9953).abs() < 50.0);
        assert!((got[1] as f32 + 1000.0 * 1.9953).abs() < 50.0);
        assert!((got[2] as f32 - 5000.0 * 1.9953).abs() < 50.0);
    }

    #[test]
    fn clipping_caps_at_full_scale() {
        let frame = s16_mono(&[20000, -20000]);
        let mut v = Volume::new(4.0);
        let out = v.process(&frame).unwrap();
        let got = read_s16(&out[0]);
        // Conversion path: i16 -> f32 (/32768) -> mult -> clamp(-1, 1) ->
        // back to i16 via *32767. A clamped sample becomes ±32767.
        assert_eq!(got[0], i16::MAX);
        assert_eq!(got[1], -i16::MAX);
    }
}
