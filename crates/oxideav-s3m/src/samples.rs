//! Sample data extraction for S3M files.
//!
//! Each PCM instrument's sample body lives at `instrument.sample_parapointer
//! << 4`. S3M samples can be:
//!
//! - **8-bit unsigned** (FFI = 2 in the header, the ST3-standard format).
//! - **8-bit signed** (FFI = 1 — rare, older tools).
//! - **16-bit** (flag bit 2 set) — LE unsigned by convention.
//! - **Stereo** (flag bit 1 set) — interleaved as left-then-right.
//!
//! We convert everything up to signed 16-bit. For mono samples `pcm_right`
//! is `None` and mixing uses the single `pcm` buffer as both L and R. For
//! true stereo samples we decode both sides into `pcm` (left) and
//! `pcm_right` (right) and the mixer routes them through the channel pan.

use crate::header::{Instrument, S3mHeader};

/// Decoded sample body ready for mixing.
#[derive(Clone, Debug, Default)]
pub struct SampleBody {
    /// Signed 16-bit PCM; for stereo samples this is the left channel. Empty
    /// if the instrument had no data.
    pub pcm: Vec<i16>,
    /// Right channel for true-stereo samples. `None` for mono samples —
    /// the mixer then uses `pcm` for both L and R before panning.
    pub pcm_right: Option<Vec<i16>>,
    /// Loop start in samples (0 if not looped).
    pub loop_start: u32,
    /// Loop end in samples (exclusive).
    pub loop_end: u32,
    /// Whether this sample should loop on playback.
    pub looped: bool,
    /// Default volume 0..=64.
    pub volume: u8,
    /// C5 (middle-C) playback rate in Hz.
    pub c5_speed: u32,
}

impl SampleBody {
    pub fn is_looped(&self) -> bool {
        self.looped && self.loop_end > self.loop_start
    }

    pub fn loop_length(&self) -> u32 {
        self.loop_end.saturating_sub(self.loop_start)
    }
}

/// Convert one instrument's raw bytes to a `SampleBody`.
///
/// `signed_samples` selects how to interpret 8-bit PCM (FFI = 1 in the
/// file-format-info field); 16-bit samples follow ST3's convention of
/// "unsigned" regardless of FFI (but in practice, modern players assume
/// signed for 16-bit too — we follow ST3).
pub fn decode_instrument(inst: &Instrument, bytes: &[u8], signed_samples: bool) -> SampleBody {
    if !inst.is_pcm() || inst.length == 0 {
        return SampleBody {
            volume: inst.volume,
            c5_speed: inst.c5_speed.max(1),
            ..Default::default()
        };
    }
    let off = inst.sample_byte_offset();
    let len = inst.length as usize;
    let is_16 = inst.is_16bit();
    let is_stereo = inst.is_stereo();
    let bytes_per_frame = if is_16 { 2 } else { 1 } * if is_stereo { 2 } else { 1 };
    let needed = len.saturating_mul(bytes_per_frame);
    let end = (off + needed).min(bytes.len());
    if off >= end {
        return SampleBody {
            volume: inst.volume,
            c5_speed: inst.c5_speed.max(1),
            ..Default::default()
        };
    }
    let raw = &bytes[off..end];
    let actual_samples = raw.len() / bytes_per_frame;
    let mut pcm: Vec<i16> = Vec::with_capacity(actual_samples);
    let mut pcm_right: Option<Vec<i16>> = if is_stereo {
        Some(Vec::with_capacity(actual_samples))
    } else {
        None
    };

    // ST3 stereo sample layout: the full left block is followed by the
    // full right block (not interleaved per-frame). MemSeg gives the
    // start of the left block; the right block starts `length * bps`
    // bytes later.
    let bps = if is_16 { 2 } else { 1 };
    let frame_bytes_mono = bps;
    let left_end = (actual_samples * frame_bytes_mono).min(raw.len());
    let left_raw = &raw[..left_end];
    let right_raw: &[u8] = if is_stereo {
        let start = actual_samples * frame_bytes_mono;
        let max_end = raw.len();
        if start >= max_end {
            &[]
        } else {
            let len = (actual_samples * frame_bytes_mono).min(max_end - start);
            &raw[start..start + len]
        }
    } else {
        &[]
    };

    let decode_into = |dst: &mut Vec<i16>, src: &[u8]| {
        if is_16 {
            let mut i = 0;
            while i + 2 <= src.len() {
                let lo = src[i];
                let hi = src[i + 1];
                let s16_unsigned = u16::from_le_bytes([lo, hi]);
                // ST3 stores 16-bit as unsigned (bias 0x8000).
                let s = if signed_samples {
                    i16::from_le_bytes([lo, hi])
                } else {
                    (s16_unsigned as i32 - 0x8000) as i16
                };
                dst.push(s);
                i += 2;
            }
        } else {
            for &b in src {
                let s = if signed_samples {
                    (b as i8 as i32) * 256
                } else {
                    (b as i32 - 128) * 256
                };
                dst.push(s.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
            }
        }
    };

    decode_into(&mut pcm, left_raw);
    if let Some(ref mut r) = pcm_right {
        decode_into(r, right_raw);
        // Pad or truncate the right channel to match the left length so
        // a single sample_pos cursor can index both without bounds checks.
        if r.len() < pcm.len() {
            r.resize(pcm.len(), 0);
        } else if r.len() > pcm.len() {
            r.truncate(pcm.len());
        }
    }

    let loop_start = inst.loop_start.min(pcm.len() as u32);
    let loop_end = inst.loop_end.min(pcm.len() as u32);
    let looped = inst.is_looped() && loop_end > loop_start;

    SampleBody {
        pcm,
        pcm_right,
        loop_start,
        loop_end,
        looped,
        volume: inst.volume,
        c5_speed: inst.c5_speed.max(1),
    }
}

/// Decode every instrument's sample body.
pub fn extract_samples(header: &S3mHeader, bytes: &[u8]) -> Vec<SampleBody> {
    // FFI: 1 = signed, 2 = unsigned. Default to unsigned (the common ST3 case).
    let signed_samples = header.ffi == 1;
    header
        .instruments
        .iter()
        .map(|i| decode_instrument(i, bytes, signed_samples))
        .collect()
}
