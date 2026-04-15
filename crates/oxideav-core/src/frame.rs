//! Uncompressed audio and video frames.

use crate::format::{PixelFormat, SampleFormat};
use crate::time::TimeBase;

/// A decoded chunk of uncompressed data: either audio samples or a video picture.
#[derive(Clone, Debug)]
pub enum Frame {
    Audio(AudioFrame),
    Video(VideoFrame),
}

impl Frame {
    pub fn pts(&self) -> Option<i64> {
        match self {
            Self::Audio(a) => a.pts,
            Self::Video(v) => v.pts,
        }
    }

    pub fn time_base(&self) -> TimeBase {
        match self {
            Self::Audio(a) => a.time_base,
            Self::Video(v) => v.time_base,
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
