//! Polyphase windowed-sinc rate conversion.
//!
//! Given input/output sample rates `src_rate` / `dst_rate`, the resampler
//! builds an analytic polyphase filter bank at construction time:
//!
//! ```text
//! L = lcm(src_rate, dst_rate)
//! up   = L / src_rate           // rational upsample factor
//! down = L / dst_rate           // rational downsample factor
//! ```
//!
//! The bank has `up` phases and `taps_per_phase` taps per phase. The
//! prototype filter is a sinc with cutoff `1 / (2 * max(up, down))`,
//! windowed by a Kaiser window with `beta = 8.0` (~80 dB stopband). Per
//! output sample we identify the integer source-sample index plus a phase,
//! then convolve the corresponding `taps_per_phase` history samples with
//! that phase's tap row.
//!
//! State (the per-channel sample history) is preserved across `process`
//! calls so streaming yields the same output as a one-shot call.
//!
//! # Parameters
//! * `src_rate` — input sample rate in Hz.
//! * `dst_rate` — output sample rate in Hz.
//!
//! # Limitations
//! * Sample format is preserved (input format == output format).
//! * Channel count is preserved.

use crate::sample_convert::{decode_to_f32, encode_from_f32};
use crate::AudioFilter;
use oxideav_core::{AudioFrame, Error, Result};

const TAPS_PER_PHASE: usize = 32;
const KAISER_BETA: f32 = 8.0;

/// Polyphase windowed-sinc resampler.
pub struct Resample {
    src_rate: u32,
    dst_rate: u32,
    /// `up` (== L / src_rate) — number of phases.
    up: u32,
    /// `down` (== L / dst_rate) — phase increment per output sample.
    down: u32,
    /// Filter bank: `up * TAPS_PER_PHASE` floats. Row `p` contains the taps
    /// for phase `p`.
    taps: Vec<f32>,
    state: Option<ResampleState>,
}

struct ResampleState {
    channels: usize,
    /// Per-channel ring of recent input samples (length `TAPS_PER_PHASE`).
    history: Vec<Vec<f32>>,
    /// Per-channel write cursor (next slot to write).
    cursor: Vec<usize>,
    /// Number of input samples consumed so far.
    samples_in: u64,
    /// Number of output samples produced so far.
    samples_out: u64,
}

struct ProduceCfg<'a> {
    taps: &'a [f32],
    up: u32,
    down: u32,
    samples_in_before: u64,
    samples_out_before: u64,
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

fn lcm(a: u32, b: u32) -> u64 {
    if a == 0 || b == 0 {
        return 0;
    }
    (a as u64 / gcd(a, b) as u64) * b as u64
}

/// Modified Bessel function of the first kind, order 0. Series expansion
/// converges quickly for the small arguments we use (Kaiser window taps).
fn bessel_i0(x: f32) -> f32 {
    let mut sum = 1.0f64;
    let mut term = 1.0f64;
    let half_x_sq = (x as f64 * x as f64) / 4.0;
    for k in 1..50 {
        term *= half_x_sq / (k as f64 * k as f64);
        sum += term;
        if term < 1.0e-12 * sum {
            break;
        }
    }
    sum as f32
}

fn build_polyphase(up: u32, down: u32, taps_per_phase: usize, beta: f32) -> Vec<f32> {
    let total = (up as usize) * taps_per_phase;
    let center = (total as f32 - 1.0) / 2.0;
    // Cutoff in cycles per L-rate sample.
    let cutoff = 1.0 / (2.0 * up.max(down) as f32);
    let i0_beta = bessel_i0(beta);

    let mut proto = vec![0.0f32; total];
    for (n, slot) in proto.iter_mut().enumerate().take(total) {
        let m = n as f32 - center;
        let s = if m == 0.0 {
            2.0 * cutoff
        } else {
            let arg = 2.0 * std::f32::consts::PI * cutoff * m;
            arg.sin() / (std::f32::consts::PI * m)
        };
        let r = (n as f32 - center) / (total as f32 / 2.0);
        let w = if r.abs() >= 1.0 {
            0.0
        } else {
            bessel_i0(beta * (1.0 - r * r).sqrt()) / i0_beta
        };
        *slot = s * w;
    }

    // Polyphase decomposition: row `p`, column `k` holds proto[p + k*up].
    // Apply gain of `up` to compensate for the upsample-by-zeros operation.
    let mut bank = vec![0.0f32; total];
    for p in 0..(up as usize) {
        for k in 0..taps_per_phase {
            let proto_idx = p + k * (up as usize);
            bank[p * taps_per_phase + k] = proto[proto_idx] * up as f32;
        }
    }
    bank
}

impl Resample {
    /// Build a new resampler. Returns `Error::Unsupported` if either rate is
    /// zero or the rate ratio leads to an unreasonable LCM.
    pub fn new(src_rate: u32, dst_rate: u32) -> Result<Self> {
        if src_rate == 0 || dst_rate == 0 {
            return Err(Error::unsupported("resample rate must be non-zero"));
        }
        let l = lcm(src_rate, dst_rate);
        if l > 100_000_000 {
            return Err(Error::unsupported(
                "resample sample-rate ratio too extreme for LCM-polyphase design",
            ));
        }
        let up = (l / src_rate as u64) as u32;
        let down = (l / dst_rate as u64) as u32;
        let taps = build_polyphase(up, down, TAPS_PER_PHASE, KAISER_BETA);
        Ok(Self {
            src_rate,
            dst_rate,
            up,
            down,
            taps,
            state: None,
        })
    }

    fn ensure_state(&mut self, channels: usize) {
        let needs_rebuild = match &self.state {
            Some(s) => s.channels != channels,
            None => true,
        };
        if needs_rebuild {
            self.state = Some(ResampleState {
                channels,
                history: (0..channels).map(|_| vec![0.0; TAPS_PER_PHASE]).collect(),
                cursor: vec![0; channels],
                samples_in: 0,
                samples_out: 0,
            });
        }
    }

    #[inline]
    fn read_back(history: &[f32], cursor: usize, back: usize) -> f32 {
        let n = history.len();
        let idx = (cursor + n - 1 - back) % n;
        history[idx]
    }

    #[inline]
    fn push_sample(history: &mut [f32], cursor: &mut usize, sample: f32) {
        history[*cursor] = sample;
        *cursor = (*cursor + 1) % history.len();
    }

    /// Inner kernel parameters so we can avoid borrowing `&self` while we
    /// hold `&mut self.state` and stay under the clippy argument limit.
    fn produce_for_channel(
        cfg: ProduceCfg<'_>,
        ch_in: &[f32],
        history: &mut [f32],
        cursor: &mut usize,
        out_buf: &mut Vec<f32>,
    ) {
        let up_u64 = cfg.up as u64;
        let down_u64 = cfg.down as u64;

        for (i, x) in ch_in.iter().enumerate() {
            Self::push_sample(history, cursor, *x);
            let new_in_pos = cfg.samples_in_before + i as u64 + 1;
            loop {
                let next_out_idx = cfg.samples_out_before + out_buf.len() as u64;
                let phase_acc = next_out_idx * down_u64;
                let src_pos = phase_acc / up_u64;
                if src_pos + 1 > new_in_pos {
                    break;
                }
                let phase = (phase_acc % up_u64) as usize;
                let back0 = (new_in_pos - 1 - src_pos) as usize;
                if back0 >= TAPS_PER_PHASE {
                    out_buf.push(0.0);
                    continue;
                }
                let row = &cfg.taps[phase * TAPS_PER_PHASE..(phase + 1) * TAPS_PER_PHASE];
                let mut acc = 0.0f32;
                for (k, tap) in row.iter().enumerate().take(TAPS_PER_PHASE) {
                    let back = back0 + k;
                    if back >= TAPS_PER_PHASE {
                        break;
                    }
                    acc += *tap * Self::read_back(history, *cursor, back);
                }
                out_buf.push(acc);
            }
        }
    }
}

impl AudioFilter for Resample {
    fn process(&mut self, input: &AudioFrame) -> Result<Vec<AudioFrame>> {
        if input.sample_rate != self.src_rate {
            return Err(Error::invalid(
                "Resample: input frame sample_rate does not match constructor",
            ));
        }
        let channels = decode_to_f32(input)?;
        let n_chan = channels.len();
        self.ensure_state(n_chan);

        let taps = &self.taps;
        let up = self.up;
        let down = self.down;

        let state = self.state.as_mut().expect("state ensured above");
        let samples_in_before = state.samples_in;
        let samples_in_after = samples_in_before + channels[0].len() as u64;
        let samples_out_before = state.samples_out;

        let mut out_per_channel: Vec<Vec<f32>> = (0..n_chan).map(|_| Vec::new()).collect();
        for (ch, out_ch) in out_per_channel.iter_mut().enumerate().take(n_chan) {
            let history = &mut state.history[ch];
            let cursor_ref = &mut state.cursor[ch];
            Self::produce_for_channel(
                ProduceCfg {
                    taps,
                    up,
                    down,
                    samples_in_before,
                    samples_out_before,
                },
                &channels[ch],
                history,
                cursor_ref,
                out_ch,
            );
        }
        let produced = out_per_channel[0].len() as u64;
        state.samples_in = samples_in_after;
        state.samples_out += produced;

        if produced == 0 {
            return Ok(Vec::new());
        }

        let mut out_template = input.clone();
        out_template.sample_rate = self.dst_rate;
        out_template.time_base = oxideav_core::TimeBase::new(1, self.dst_rate as i64);
        let frame = encode_from_f32(&out_template, &out_per_channel)?;
        Ok(vec![frame])
    }

    fn flush(&mut self) -> Result<Vec<AudioFrame>> {
        let n_chan = match &self.state {
            Some(s) => s.channels,
            None => return Ok(Vec::new()),
        };
        // Push half a tap window of zeros to flush the tail.
        let pad = TAPS_PER_PHASE / 2;
        let template = AudioFrame {
            format: oxideav_core::SampleFormat::F32,
            channels: n_chan as u16,
            sample_rate: self.src_rate,
            samples: 0,
            pts: None,
            time_base: oxideav_core::TimeBase::new(1, self.src_rate as i64),
            data: vec![Vec::new()],
        };

        let taps = &self.taps;
        let up = self.up;
        let down = self.down;

        let state = self.state.as_mut().expect("state checked");
        let samples_in_before = state.samples_in;
        let samples_out_before = state.samples_out;
        let mut out_per_channel: Vec<Vec<f32>> = (0..n_chan).map(|_| Vec::new()).collect();

        for (ch, out_ch) in out_per_channel.iter_mut().enumerate().take(n_chan) {
            let history = &mut state.history[ch];
            let cursor_ref = &mut state.cursor[ch];
            let zero_pad = vec![0.0f32; pad];
            Self::produce_for_channel(
                ProduceCfg {
                    taps,
                    up,
                    down,
                    samples_in_before,
                    samples_out_before,
                },
                &zero_pad,
                history,
                cursor_ref,
                out_ch,
            );
        }
        let produced = out_per_channel[0].len() as u64;
        state.samples_in += pad as u64;
        state.samples_out += produced;

        if produced == 0 {
            return Ok(Vec::new());
        }

        let mut out_template = template;
        out_template.sample_rate = self.dst_rate;
        out_template.time_base = oxideav_core::TimeBase::new(1, self.dst_rate as i64);
        let frame = encode_from_f32(&out_template, &out_per_channel)?;
        Ok(vec![frame])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{SampleFormat, TimeBase};

    fn sine_f32(freq: f32, rate: u32, n: usize) -> AudioFrame {
        let mut bytes = Vec::with_capacity(n * 4);
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let s = (2.0 * std::f32::consts::PI * freq * t).sin();
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
    fn round_trip_48k_44k_48k_within_40db() {
        // Round-trip a 1 kHz sine through 48000 → 44100 → 48000 and verify
        // the reconstruction error is below -40 dB. Because the resampler
        // applies a non-integer group delay, integer-sample RMS comparison
        // would be dominated by sub-sample misalignment. Instead we project
        // the round-tripped signal onto sin/cos at the same frequency and
        // measure the residual.
        let n = 48_000;
        let freq = 1000.0_f64;
        let rate = 48_000.0_f64;
        let original_frame = sine_f32(freq as f32, 48_000, n);

        let mut down = Resample::new(48_000, 44_100).unwrap();
        let mut mid_frames = down.process(&original_frame).unwrap();
        mid_frames.extend(down.flush().unwrap());

        let mut up = Resample::new(44_100, 48_000).unwrap();
        let mut out_frames: Vec<AudioFrame> = Vec::new();
        for f in &mid_frames {
            let outs = up.process(f).unwrap();
            out_frames.extend(outs);
        }
        out_frames.extend(up.flush().unwrap());

        let mut result: Vec<f32> = Vec::new();
        for f in &out_frames {
            result.extend(read_f32(f));
        }

        let mid_start = 5_000;
        let mid_end = (n - 5_000).min(result.len());
        assert!(
            mid_end > mid_start + 1_000,
            "round-trip output too short: {} samples",
            result.len()
        );

        // Least-squares fit of (a*sin + b*cos) to the round-tripped signal.
        let mut sum_s2 = 0.0f64;
        let mut sum_c2 = 0.0f64;
        let mut sum_xs = 0.0f64;
        let mut sum_xc = 0.0f64;
        for (i, x) in result.iter().enumerate().take(mid_end).skip(mid_start) {
            let t = i as f64 / rate;
            let s = (2.0 * std::f64::consts::PI * freq * t).sin();
            let c = (2.0 * std::f64::consts::PI * freq * t).cos();
            let x = *x as f64;
            sum_s2 += s * s;
            sum_c2 += c * c;
            sum_xs += x * s;
            sum_xc += x * c;
        }
        let a = sum_xs / sum_s2;
        let b = sum_xc / sum_c2;
        let mag = (a * a + b * b).sqrt();

        let mut sum_r2 = 0.0f64;
        let mut sum_x2 = 0.0f64;
        for (i, x) in result.iter().enumerate().take(mid_end).skip(mid_start) {
            let t = i as f64 / rate;
            let s = (2.0 * std::f64::consts::PI * freq * t).sin();
            let c = (2.0 * std::f64::consts::PI * freq * t).cos();
            let model = a * s + b * c;
            let x = *x as f64;
            let resid = x - model;
            sum_r2 += resid * resid;
            sum_x2 += x * x;
        }
        let count = (mid_end - mid_start) as f64;
        let rms_resid = (sum_r2 / count).sqrt();
        let rms_signal = (sum_x2 / count).sqrt();
        let snr_db = 20.0 * (rms_resid / rms_signal).log10();
        eprintln!(
            "round-trip: amplitude={:.6}, residual SNR={:.2} dB",
            mag, snr_db
        );
        assert!(
            (mag - 1.0).abs() < 0.01,
            "round-trip amplitude {} is far from unity",
            mag
        );
        assert!(
            snr_db < -40.0,
            "round-trip residual SNR {} dB exceeded -40 dB threshold",
            snr_db
        );
    }

    #[test]
    fn output_rate_matches_target() {
        let n = 9_600;
        let frame = sine_f32(440.0, 48_000, n);
        let mut r = Resample::new(48_000, 24_000).unwrap();
        let outs = r.process(&frame).unwrap();
        let total: usize = outs.iter().map(|f| f.samples as usize).sum();
        assert!(
            (total as i32 - 4_800).abs() < 64,
            "expected ~4800 samples, got {}",
            total
        );
        for f in &outs {
            assert_eq!(f.sample_rate, 24_000);
        }
    }
}
