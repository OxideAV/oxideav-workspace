//! Integration test: a 440 Hz sine should yield a bright horizontal band at
//! the expected frequency bin in the rendered spectrogram.

use oxideav_audio_filter::{Colormap, Spectrogram, SpectrogramOptions, Window};
use oxideav_core::{AudioFrame, SampleFormat, TimeBase};

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
fn bright_band_at_440_hz() {
    let opts = SpectrogramOptions {
        fft_size: 1024,
        hop_size: 256,
        width: 64,
        height: 1024 / 2 + 1,
        db_range: (-90.0, 0.0),
        colormap: Colormap::Grayscale,
        window: Window::Hann,
    };
    let mut s = Spectrogram::new(opts.clone()).unwrap();
    // 1 s at 8 kHz
    let frame = sine_frame(440.0, 8_000, 8_000);
    s.feed(&frame).unwrap();

    let rgb = s.finalize_rgb();
    let w = opts.width as usize;
    let h = opts.height as usize;
    let expected_bin = (440.0_f32 / (8000.0_f32 / 1024.0_f32)).round() as usize;
    let target_y = h - 1 - expected_bin;
    let mid_x = w / 2;
    let band_intensity = rgb[(target_y * w + mid_x) * 3] as i32;

    let mut peak_far = 0i32;
    for y in 0..h {
        if (y as i32 - target_y as i32).abs() < 5 {
            continue;
        }
        let off = (y * w + mid_x) * 3;
        peak_far = peak_far.max(rgb[off] as i32);
    }
    assert!(
        band_intensity > peak_far + 50,
        "no clear band: target={}, others_max={}",
        band_intensity,
        peak_far
    );
}
