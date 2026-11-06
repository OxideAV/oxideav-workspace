//! `--vo hash` and `--ao hash` — engines that hash every decoded frame
//! and print the running digest at end-of-stream.
//!
//! Useful for regression testing: run the player twice on the same
//! input, compare the printed hashes, and you have a one-line "did the
//! decoder output change" check without needing a reference file. No
//! device is opened, no SDL2 / sysaudio dependency is touched, so the
//! engines work in any CI environment that can run the binary.
//!
//! A mid-stream shape change (the stream's parameters re-stated after
//! the first frame) is reported as it happens, on its own stderr line —
//! `hash:vo-shape: 96x64 Yuv420P from frame 6`, `hash:ao-shape: 32000 Hz
//! 2 ch S16P from sample 12672` — so a headless run shows where the
//! player re-planned its output. The digest lines are unchanged.
//!
//! The hash is FNV-1a 64-bit. Stable across Rust versions, deterministic,
//! 6 lines of arithmetic, no extra dependency. Collision resistance is
//! a non-goal — the use case is "are these two outputs bit-identical",
//! and FNV is a clean win there.

use std::time::Duration;

use oxideav_core::{AudioFrame, CodecParameters, PixelFormat, Result, SampleFormat, VideoFrame};

use super::audio_convert::reconcile_format;
use super::engine::{AudioEngine, VideoEngine};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// FNV-1a 64-bit. Inline so we don't need a hashing crate.
#[derive(Clone)]
struct Fnv1a64(u64);

impl Fnv1a64 {
    fn new() -> Self {
        Self(FNV_OFFSET)
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut h = self.0;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
        self.0 = h;
    }
    fn write_u64(&mut self, v: u64) {
        self.write(&v.to_le_bytes());
    }
    fn write_i64(&mut self, v: i64) {
        self.write(&v.to_le_bytes());
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

// ─────────────────────────── video ───────────────────────────

pub struct HashVideoEngine {
    state: Fnv1a64,
    frames: u64,
    width: Option<u32>,
    height: Option<u32>,
    pixel_format: Option<PixelFormat>,
}

impl HashVideoEngine {
    pub fn new() -> Self {
        Self {
            state: Fnv1a64::new(),
            frames: 0,
            width: None,
            height: None,
            pixel_format: None,
        }
    }
}

impl Default for HashVideoEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoEngine for HashVideoEngine {
    fn present(&mut self, frame: &VideoFrame) -> Result<()> {
        // Hash a stable serialisation of every frame: pts, plane count,
        // and (stride, len, raw bytes) per plane. The decoded pixels
        // ARE what we want to detect change in.
        self.state.write_i64(frame.pts.unwrap_or(0));
        self.state.write_u64(frame.planes.len() as u64);
        for plane in &frame.planes {
            self.state.write_u64(plane.stride as u64);
            self.state.write_u64(plane.data.len() as u64);
            self.state.write(&plane.data);
        }
        self.frames += 1;
        Ok(())
    }

    fn info(&self) -> String {
        match (self.width, self.height) {
            (Some(w), Some(h)) => format!("hash (FNV-1a 64-bit) — {w}x{h}, no device opened"),
            _ => "hash (FNV-1a 64-bit) — no device opened".into(),
        }
    }

    fn set_source_video_params(&mut self, params: &CodecParameters) {
        let shape = (
            params.width.or(self.width),
            params.height.or(self.height),
            params.pixel_format.or(self.pixel_format),
        );
        let changed = shape != (self.width, self.height, self.pixel_format);
        (self.width, self.height, self.pixel_format) = shape;
        if changed && self.frames > 0 {
            eprintln!(
                "hash:vo-shape: {}x{} {} from frame {}",
                opt(self.width),
                opt(self.height),
                opt(self.pixel_format),
                self.frames
            );
        }
    }

    fn drains_immediately(&self) -> bool {
        true
    }
}

impl Drop for HashVideoEngine {
    fn drop(&mut self) {
        // Print to stderr so it doesn't compete with the TUI on stdout.
        eprintln!(
            "hash:vo: {:016x}  ({} frame{})",
            self.state.finish(),
            self.frames,
            if self.frames == 1 { "" } else { "s" }
        );
    }
}

// ─────────────────────────── audio ───────────────────────────

pub struct HashAudioEngine {
    state: Fnv1a64,
    samples_queued: u64,
    /// Play time of every sample queued so far, each frame at the rate
    /// in force when it was queued (the rate can change mid-stream).
    queued_nanos: u128,
    sample_rate: u32,
    src_format: SampleFormat,
    src_channels: u16,
    paused: bool,
}

impl HashAudioEngine {
    pub fn new() -> Self {
        Self {
            state: Fnv1a64::new(),
            samples_queued: 0,
            queued_nanos: 0,
            sample_rate: 48_000,
            src_format: SampleFormat::F32,
            src_channels: 2,
            paused: false,
        }
    }
}

impl Default for HashAudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine for HashAudioEngine {
    fn queue(&mut self, frame: &AudioFrame) -> Result<()> {
        // Hash the per-frame metadata + raw plane bytes. Use the
        // reconciled format so the digest survives a stale F32 default
        // (see audio_convert::reconcile_format) — the hash should
        // describe what the decoder actually emitted.
        let _format = reconcile_format(frame, self.src_format, self.src_channels);
        self.state.write_i64(frame.pts.unwrap_or(0));
        self.state.write_u64(frame.samples as u64);
        self.state.write_u64(frame.data.len() as u64);
        for plane in &frame.data {
            self.state.write_u64(plane.len() as u64);
            self.state.write(plane);
        }
        self.samples_queued = self.samples_queued.saturating_add(frame.samples as u64);
        if self.sample_rate > 0 {
            self.queued_nanos += frame.samples as u128 * 1_000_000_000 / self.sample_rate as u128;
        }
        Ok(())
    }

    fn master_clock_pos(&self) -> Duration {
        // Advance the master clock as if every queued sample has played
        // — the player needs a forward-going clock to pace presentation.
        Duration::from_nanos(self.queued_nanos.min(u64::MAX as u128) as u64)
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    fn set_volume(&mut self, _vol: f32) {}

    fn info(&self) -> String {
        format!(
            "hash (FNV-1a 64-bit) @ {} Hz {}ch — no device opened",
            self.sample_rate, self.src_channels
        )
    }

    fn set_source_audio_params(&mut self, params: &CodecParameters) {
        let before = (self.sample_rate, self.src_channels, self.src_format);
        self.apply_audio_params(params);
        if self.samples_queued > 0
            && before != (self.sample_rate, self.src_channels, self.src_format)
        {
            eprintln!(
                "hash:ao-shape: {} Hz {} ch {:?} from sample {}",
                self.sample_rate, self.src_channels, self.src_format, self.samples_queued
            );
        }
    }
}

impl HashAudioEngine {
    fn apply_audio_params(&mut self, params: &CodecParameters) {
        if let Some(r) = params.sample_rate
            && r > 0
        {
            self.sample_rate = r;
        }
        if let Some(c) = params.resolved_channels()
            && c > 0
        {
            self.src_channels = c;
        }
        if let Some(f) = params.sample_format {
            self.src_format = f;
        }
    }
}

fn opt<T: std::fmt::Debug>(v: Option<T>) -> String {
    v.map_or_else(|| "?".to_owned(), |v| format!("{v:?}"))
}

impl Drop for HashAudioEngine {
    fn drop(&mut self) {
        eprintln!(
            "hash:ao: {:016x}  ({} sample{} per channel)",
            self.state.finish(),
            self.samples_queued,
            if self.samples_queued == 1 { "" } else { "s" }
        );
    }
}

// ─────────────────────────── tests ───────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::VideoPlane;

    fn vframe(pts: i64, bytes: &[u8]) -> VideoFrame {
        VideoFrame {
            pts: Some(pts),
            planes: vec![VideoPlane {
                stride: bytes.len(),
                data: bytes.to_vec(),
            }],
        }
    }

    fn aframe(pts: i64, samples: u32, bytes: &[u8]) -> AudioFrame {
        AudioFrame {
            samples,
            pts: Some(pts),
            data: vec![bytes.to_vec()],
        }
    }

    #[test]
    fn audio_clock_follows_a_mid_stream_rate_change() {
        let mut a = HashAudioEngine::new();
        let mut p = CodecParameters::audio(oxideav_core::CodecId::new("pcm_s16le"));
        p.sample_rate = Some(1_000);
        p.channels = Some(1);
        p.sample_format = Some(SampleFormat::S16);
        a.set_source_audio_params(&p);
        a.queue(&aframe(0, 1_000, &[0; 2_000])).unwrap();
        assert_eq!(a.master_clock_pos(), Duration::from_secs(1));
        // The same number of samples at twice the rate plays half as long.
        p.sample_rate = Some(2_000);
        a.set_source_audio_params(&p);
        a.queue(&aframe(1_000, 1_000, &[0; 2_000])).unwrap();
        assert_eq!(a.master_clock_pos(), Duration::from_millis(1_500));
    }

    #[test]
    fn video_shape_tracks_partial_updates() {
        let mut v = HashVideoEngine::new();
        let mut p = CodecParameters::video(oxideav_core::CodecId::new("rawvideo"));
        p.width = Some(64);
        p.height = Some(48);
        p.pixel_format = Some(PixelFormat::Yuv420P);
        v.set_source_video_params(&p);
        v.present(&vframe(0, &[0; 64])).unwrap();
        // A re-statement that leaves the layout out keeps the known one.
        let mut q = CodecParameters::video(oxideav_core::CodecId::new("rawvideo"));
        q.width = Some(96);
        q.height = Some(64);
        v.set_source_video_params(&q);
        assert_eq!(
            (v.width, v.height, v.pixel_format),
            (Some(96), Some(64), Some(PixelFormat::Yuv420P))
        );
    }

    #[test]
    fn fnv_initial_state_is_offset_basis() {
        assert_eq!(Fnv1a64::new().finish(), FNV_OFFSET);
    }

    #[test]
    fn fnv_known_vector_for_empty_string() {
        // Standard FNV-1a-64 vector: the empty input keeps the offset basis.
        let mut h = Fnv1a64::new();
        h.write(b"");
        assert_eq!(h.finish(), 0xcbf29ce484222325);
    }

    #[test]
    fn fnv_known_vector_for_a() {
        // Standard FNV-1a-64 test vector for "a".
        let mut h = Fnv1a64::new();
        h.write(b"a");
        assert_eq!(h.finish(), 0xaf63dc4c8601ec8c);
    }

    #[test]
    fn video_hash_stable_across_runs() {
        let mut a = HashVideoEngine::new();
        let mut b = HashVideoEngine::new();
        a.present(&vframe(0, &[1, 2, 3, 4])).unwrap();
        a.present(&vframe(40, &[5, 6, 7, 8])).unwrap();
        b.present(&vframe(0, &[1, 2, 3, 4])).unwrap();
        b.present(&vframe(40, &[5, 6, 7, 8])).unwrap();
        assert_eq!(a.state.finish(), b.state.finish());
        assert_eq!(a.frames, 2);
    }

    #[test]
    fn video_hash_changes_when_pixels_change() {
        let mut a = HashVideoEngine::new();
        let mut b = HashVideoEngine::new();
        a.present(&vframe(0, &[1, 2, 3, 4])).unwrap();
        b.present(&vframe(0, &[1, 2, 3, 5])).unwrap(); // last byte differs
        assert_ne!(a.state.finish(), b.state.finish());
    }

    #[test]
    fn audio_hash_clock_advances_with_samples() {
        let mut e = HashAudioEngine::new();
        e.sample_rate = 48_000;
        e.queue(&aframe(0, 1024, &[0; 4096])).unwrap();
        e.queue(&aframe(0, 1024, &[0; 4096])).unwrap();
        // 2048 samples / 48_000 Hz ≈ 42.667 ms.
        let pos = e.master_clock_pos();
        assert!(pos.as_micros() >= 42_600 && pos.as_micros() <= 42_700);
    }

    #[test]
    fn audio_hash_changes_when_samples_change() {
        let mut a = HashAudioEngine::new();
        let mut b = HashAudioEngine::new();
        a.queue(&aframe(0, 4, &[1, 2, 3, 4, 5, 6, 7, 8])).unwrap();
        b.queue(&aframe(0, 4, &[1, 2, 3, 4, 5, 6, 7, 9])).unwrap();
        assert_ne!(a.state.finish(), b.state.finish());
    }
}
