//! Pure-Rust audio filters for the oxideav framework.
//!
//! Each filter implements the [`AudioFilter`] trait and operates on
//! [`AudioFrame`](oxideav_core::AudioFrame) values. All filters convert input
//! samples to `f32` internally via [`sample_convert`] and convert back to the
//! input format on output. The exception is [`Resample`](resample::Resample),
//! whose output frame's `sample_rate` differs from its input but whose sample
//! format is preserved.
//!
//! # Streaming model
//!
//! Filters maintain internal state between calls to [`AudioFilter::process`].
//! A single input frame may produce zero, one, or many output frames depending
//! on the filter's buffering behaviour. After the last input frame, callers
//! should invoke [`AudioFilter::flush`] to drain any retained samples.
//!
//! Filters declare themselves `Send` so they can be moved between threads, but
//! they are not required to be `Sync`.
//!
//! # Available filters
//!
//! - [`Volume`](volume::Volume) — gain (linear or dB) with hard clipping.
//! - [`NoiseGate`](noise_gate::NoiseGate) — threshold-based gate with attack,
//!   release, and hold.
//! - [`Echo`](echo::Echo) — single-tap circular delay line with feedback and
//!   wet/dry mix.
//! - [`Resample`](resample::Resample) — polyphase windowed-sinc rate
//!   conversion.
//! - [`Spectrogram`](spectrogram::Spectrogram) — STFT-based image renderer
//!   with PNG output.

pub mod echo;
pub mod fft;
pub mod noise_gate;
pub mod resample;
pub mod sample_convert;
pub mod spectrogram;
pub mod volume;

pub use echo::Echo;
pub use noise_gate::NoiseGate;
pub use resample::Resample;
pub use spectrogram::{Colormap, Spectrogram, SpectrogramOptions, Window};
pub use volume::Volume;

use oxideav_core::{AudioFrame, Result};

/// Streaming audio filter.
///
/// Implementors process one input frame at a time and may emit zero or more
/// output frames. Internal state (delay lines, envelopes, sample histories,
/// resampler phase, FFT accumulators, …) lives in `self` and is preserved
/// across calls.
///
/// At end-of-stream callers invoke [`AudioFilter::flush`] to obtain any
/// frames still buffered inside the filter. The default implementation
/// returns an empty `Vec` for filters that do not buffer.
pub trait AudioFilter: Send {
    /// Process one input frame, returning zero or more output frames.
    fn process(&mut self, input: &AudioFrame) -> Result<Vec<AudioFrame>>;

    /// Drain any internally buffered samples at end-of-stream.
    fn flush(&mut self) -> Result<Vec<AudioFrame>> {
        Ok(Vec::new())
    }
}
