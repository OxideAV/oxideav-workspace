//! Uncompressed audio and video frames.

use crate::format::{PixelFormat, SampleFormat};
use crate::subtitle::SubtitleCue;
use crate::time::TimeBase;

/// A decoded chunk of uncompressed data: either audio samples, a video
/// picture, or (for subtitle streams) a single styled cue.
///
/// Marked `#[non_exhaustive]` — consumers that match on variants must
/// include a wildcard arm. This lets the crate add new frame kinds (data
/// tracks, hap rops, …) without breaking downstream code.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Frame {
    Audio(AudioFrame),
    Video(VideoFrame),
    /// A single subtitle cue. Timing is carried inside the cue itself
    /// (`start_us`/`end_us`) so it's independent of container time bases,
    /// but the enclosing pipeline/muxer can still rescale via `pts` at
    /// the packet layer.
    Subtitle(SubtitleCue),
}

impl Frame {
    pub fn pts(&self) -> Option<i64> {
        match self {
            Self::Audio(a) => a.pts,
            Self::Video(v) => v.pts,
            Self::Subtitle(s) => Some(s.start_us),
        }
    }

    pub fn time_base(&self) -> TimeBase {
        match self {
            Self::Audio(a) => a.time_base,
            Self::Video(v) => v.time_base,
            // Subtitle cues carry raw microseconds. Expose a 1/1_000_000
            // base so the value lines up with the pts() result above.
            Self::Subtitle(_) => TimeBase::new(1, 1_000_000),
        }
    }
}

/// Uncompressed audio frame.
///
/// Sample layout is determined by `format`:
/// - Interleaved formats: `data` has one plane; samples are stored as
///   `ch0 ch1 ... chN ch0 ch1 ... chN ...`.
/// - Planar formats: `data` has one plane per channel.
#[derive(Clone, Debug)]
pub struct AudioFrame {
    pub format: SampleFormat,
    pub channels: u16,
    pub sample_rate: u32,
    /// Number of samples *per channel*.
    pub samples: u32,
    pub pts: Option<i64>,
    pub time_base: TimeBase,
    /// Raw sample bytes. `.len() == planes()` — i.e. one element per plane.
    pub data: Vec<Vec<u8>>,
}

impl AudioFrame {
    pub fn planes(&self) -> usize {
        if self.format.is_planar() {
            self.channels as usize
        } else {
            1
        }
    }
}

/// Uncompressed video frame.
#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub pts: Option<i64>,
    pub time_base: TimeBase,
    /// One entry per plane (e.g., 3 for Yuv420P). Each entry is `(stride, bytes)`.
    pub planes: Vec<VideoPlane>,
}

#[derive(Clone, Debug)]
pub struct VideoPlane {
    /// Bytes per row in `data`.
    pub stride: usize,
    pub data: Vec<u8>,
}
