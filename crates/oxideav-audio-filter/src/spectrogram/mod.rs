//! Streaming spectrogram renderer.
//!
//! Feeds [`AudioFrame`]s incrementally through a Hann/Hamming/Blackman
//! windowed STFT, accumulates per-FFT-column magnitudes, and finally renders
//! a `width x height` RGB image (optionally written to a PNG file).
//!
//! Multi-channel input is mixed down to mono. Magnitudes are converted to
//! dBFS and clamped to `db_range`. Time downsampling uses max-pooling over
//! consecutive STFT columns; frequency mapping is linear by default.
//!
//! # Defaults
//!
//! * `fft_size`: 1024
//! * `hop_size`: 256
//! * `window`: Hann
//! * `db_range`: (-90.0, 0.0)
//! * `width`: 800
//! * `height`: 256
//! * `colormap`: Viridis

use crate::fft::real_fft;
use crate::sample_convert::decode_to_f32;
use oxideav_core::{AudioFrame, Error, Result};

mod colormaps;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    Hann,
    Hamming,
    Blackman,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colormap {
    Grayscale,
    Viridis,
    Magma,
}

/// Spectrogram configuration.
#[derive(Clone, Debug)]
pub struct SpectrogramOptions {
    pub fft_size: usize,
    pub hop_size: usize,
    pub window: Window,
    pub db_range: (f32, f32),
    pub width: u32,
    pub height: u32,
    pub colormap: Colormap,
}

impl Default for SpectrogramOptions {
    fn default() -> Self {
        Self {
            fft_size: 1024,
            hop_size: 256,
            window: Window::Hann,
            db_range: (-90.0, 0.0),
            width: 800,
            height: 256,
            colormap: Colormap::Viridis,
        }
    }
}

pub struct Spectrogram {
    opts: SpectrogramOptions,
    window: Vec<f32>,
    pending: Vec<f32>,
    /// Each column is `fft_size / 2 + 1` magnitudes (post-window FFT mag).
    columns: Vec<Vec<f32>>,
}

impl Spectrogram {
    pub fn new(opts: SpectrogramOptions) -> Result<Self> {
        if !opts.fft_size.is_power_of_two() || opts.fft_size < 8 {
            return Err(Error::invalid(
                "spectrogram fft_size must be a power of two >= 8",
            ));
        }
        if opts.hop_size == 0 || opts.hop_size > opts.fft_size {
            return Err(Error::invalid(
                "spectrogram hop_size must be in (0, fft_size]",
            ));
        }
        if opts.width == 0 || opts.height == 0 {
            return Err(Error::invalid("spectrogram width/height must be non-zero"));
        }
        let window = build_window(opts.window, opts.fft_size);
        Ok(Self {
            opts,
            window,
            pending: Vec::new(),
            columns: Vec::new(),
        })
    }

    /// Feed one audio frame. Multi-channel input is averaged to mono.
    pub fn feed(&mut self, frame: &AudioFrame) -> Result<()> {
        let channels = decode_to_f32(frame)?;
        let n_chan = channels.len();
        let n_samples = channels.first().map(|c| c.len()).unwrap_or(0);
        if n_chan == 0 || n_samples == 0 {
            return Ok(());
        }
        let inv_n = 1.0 / n_chan as f32;
        for s in 0..n_samples {
            let mut sum = 0.0;
            for ch in channels.iter().take(n_chan) {
                sum += ch[s];
            }
            self.pending.push(sum * inv_n);
        }

        // Drain windows
        while self.pending.len() >= self.opts.fft_size {
            let mut block = vec![0.0f32; self.opts.fft_size];
            for (i, slot) in block.iter_mut().enumerate().take(self.opts.fft_size) {
                *slot = self.pending[i] * self.window[i];
            }
            let bins = real_fft(&block);
            let mags: Vec<f32> = bins.iter().map(|c| c.magnitude()).collect();
            self.columns.push(mags);
            self.pending.drain(..self.opts.hop_size);
        }
        Ok(())
    }

    /// Render the accumulated columns into a `width * height * 3` RGB byte
    /// vector.
    pub fn finalize_rgb(&self) -> Vec<u8> {
        let w = self.opts.width as usize;
        let h = self.opts.height as usize;
        let mut out = vec![0u8; w * h * 3];
        if self.columns.is_empty() {
            return out;
        }
        let n_cols = self.columns.len();
        let n_freq = self.opts.fft_size / 2 + 1;

        // Time mapping (max-pool over column ranges)
        // For each output column x, source range is [x*n_cols/w, (x+1)*n_cols/w)
        // For each output row y (top = high freq), source range is similar
        for x in 0..w {
            let s0 = (x * n_cols) / w;
            let s1 = (((x + 1) * n_cols) / w).max(s0 + 1).min(n_cols);
            for y in 0..h {
                // y=0 is top row (high freq); flip so high freq at top
                let yy = h - 1 - y;
                let f0 = (yy * n_freq) / h;
                let f1 = (((yy + 1) * n_freq) / h).max(f0 + 1).min(n_freq);

                let mut max_mag = 0.0f32;
                for cx in s0..s1 {
                    let col = &self.columns[cx];
                    for m in col.iter().take(f1).skip(f0) {
                        if *m > max_mag {
                            max_mag = *m;
                        }
                    }
                }

                // Convert magnitude to dBFS. Reference: 0 dBFS == fft_size/2
                // (full-scale sine gives mag = N/2 in each bin).
                let ref_mag = self.opts.fft_size as f32 / 2.0;
                let db = if max_mag <= 1.0e-12 {
                    -200.0
                } else {
                    20.0 * (max_mag / ref_mag).log10()
                };
                let (lo, hi) = self.opts.db_range;
                let t = ((db - lo) / (hi - lo)).clamp(0.0, 1.0);
                let idx = (t * 255.0) as u8;
                let (r, g, b) = colormap_lookup(self.opts.colormap, idx);
                let off = (y * w + x) * 3;
                out[off] = r;
                out[off + 1] = g;
                out[off + 2] = b;
            }
        }
        out
    }

    /// Write the rendered spectrogram out as a sequence of RGB bytes.
    pub fn write_rgb<W: std::io::Write>(&self, w: &mut W) -> std::io::Result<()> {
        let rgb = self.finalize_rgb();
        w.write_all(&rgb)
    }

    /// Encode the rendered spectrogram as a PNG file at `path`.
    pub fn finalize_png(&self, path: &std::path::Path) -> Result<()> {
        let rgb = self.finalize_rgb();
        let file = std::fs::File::create(path)?;
        let w = std::io::BufWriter::new(file);
        let mut encoder = png::Encoder::new(w, self.opts.width, self.opts.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| Error::other(format!("png header: {}", e)))?;
        writer
            .write_image_data(&rgb)
            .map_err(|e| Error::other(format!("png data: {}", e)))?;
        Ok(())
    }

    /// Number of FFT columns accumulated so far.
    pub fn columns_recorded(&self) -> usize {
        self.columns.len()
    }
}

fn build_window(kind: Window, n: usize) -> Vec<f32> {
    let mut w = vec![0.0f32; n];
    let denom = (n - 1) as f32;
    for (i, slot) in w.iter_mut().enumerate().take(n) {
        let phase = 2.0 * std::f32::consts::PI * i as f32 / denom;
        *slot = match kind {
            Window::Hann => 0.5 * (1.0 - phase.cos()),
            Window::Hamming => 0.54 - 0.46 * phase.cos(),
            Window::Blackman => 0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos(),
        };
    }
    w
}

fn colormap_lookup(cm: Colormap, idx: u8) -> (u8, u8, u8) {
    match cm {
        Colormap::Grayscale => (idx, idx, idx),
        Colormap::Viridis => colormaps::VIRIDIS[idx as usize],
        Colormap::Magma => colormaps::MAGMA[idx as usize],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{SampleFormat, TimeBase};

    fn sine_frame(freq: f32, rate: u32, n: usize) -> AudioFrame {
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

    #[test]
    fn opts_default_is_sane() {
        let o = SpectrogramOptions::default();
        assert_eq!(o.fft_size, 1024);
        assert_eq!(o.hop_size, 256);
        assert_eq!(o.width, 800);
        assert_eq!(o.height, 256);
    }

    #[test]
    fn rgb_buffer_has_correct_size() {
        let opts = SpectrogramOptions {
            width: 32,
            height: 16,
            ..Default::default()
        };
        let s = Spectrogram::new(opts).unwrap();
        let rgb = s.finalize_rgb();
        assert_eq!(rgb.len(), 32 * 16 * 3);
    }

    #[test]
    fn sine_produces_band_at_expected_frequency() {
        // 1 second of 440 Hz at 8000 Hz sample rate → bin = 440 / (Fs/N)
        // N = 1024, Fs = 8000 → bin width = 7.8125 Hz → 440 Hz at bin ~56
        let opts = SpectrogramOptions {
            fft_size: 1024,
            hop_size: 256,
            width: 64,
            height: 1024 / 2 + 1, // one row per bin so we can check directly
            db_range: (-90.0, 0.0),
            colormap: Colormap::Grayscale,
            window: Window::Hann,
        };
        let mut s = Spectrogram::new(opts.clone()).unwrap();
        let frame = sine_frame(440.0, 8_000, 8_000);
        s.feed(&frame).unwrap();
        let rgb = s.finalize_rgb();
        let w = opts.width as usize;
        let h = opts.height as usize;
        // Expected bin
        let expected_bin = (440.0_f32 / (8000.0_f32 / 1024.0_f32)).round() as usize;
        // Expected row in image: high freq is at top (y=0), so y for bin b
        // is h - 1 - (b * h / nfreq) ; with h == nfreq this reduces to
        // y = h - 1 - b
        let target_y = h - 1 - expected_bin;
        // Take a sample column near the middle of the image
        let target_x = w / 2;
        let off = (target_y * w + target_x) * 3;
        let intensity = rgb[off] as i32;
        // Sample some other rows that should be much darker
        let mut peak_far = 0i32;
        for y in 0..h {
            if (y as i32 - target_y as i32).abs() < 5 {
                continue;
            }
            let off = (y * w + target_x) * 3;
            peak_far = peak_far.max(rgb[off] as i32);
        }
        assert!(
            intensity > peak_far + 50,
            "expected bright band at y={}, got {} but max-elsewhere = {}",
            target_y,
            intensity,
            peak_far
        );
    }
}
