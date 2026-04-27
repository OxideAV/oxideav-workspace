//! Audio format conversion helpers shared between output drivers.
//!
//! The `oxideav` core audio model admits many sample formats (U8/S8/
//! S16/S24/S32/F32/F64 in interleaved or planar layouts) and decoders
//! emit whatever their native format is. Both the SDL2 and the
//! winit+sysaudio drivers feed the OS a single normalised f32 interleaved
//! stream, so the conversion + resampling + up/down-mix logic lives
//! here once.
//!
//! Stream-level shape (sample format + channel count) used to live on
//! every `AudioFrame`; the slim moved them onto the stream's
//! `CodecParameters`. Drivers cache them once at stream open (via the
//! engine's `set_source_audio_params` setter) and pass them in
//! explicitly to every helper here.

use oxideav_core::{AudioFrame, SampleFormat};

/// Reconcile the player's cached `SampleFormat` against what the frame's
/// byte buffer actually contains. The slim refactor moved sample format
/// from per-frame onto `CodecParameters`, but most demuxers (MP4 / MKV /
/// …) don't pin `sample_format` in the audio stream params — they just
/// know it's "AAC audio", not "AAC outputting S16". The driver then
/// keeps its constructor default (F32) and wrong-strides into a buffer
/// half the expected size. This helper inspects the actual buffer and
/// returns a SampleFormat whose stride fits, preserving the cached
/// format's planar/interleaved + float/int family when downgrading.
pub fn reconcile_format(frame: &AudioFrame, cached: SampleFormat, channels: u16) -> SampleFormat {
    let in_ch = channels.max(1) as usize;
    let n = frame.samples as usize;
    if n == 0 || frame.data.is_empty() || frame.data[0].is_empty() {
        return cached;
    }
    let cached_bps = cached.bytes_per_sample();
    let planar = cached.is_planar();
    let plane_bytes = if planar {
        frame.data[0].len()
    } else {
        frame.data[0].len() / in_ch.max(1)
    };
    if plane_bytes >= n.saturating_mul(cached_bps) {
        return cached;
    }
    let actual_bps = (plane_bytes / n).max(1);
    pick_format(actual_bps, planar, cached.is_float()).unwrap_or(cached)
}

fn pick_format(bps: usize, planar: bool, prefer_float: bool) -> Option<SampleFormat> {
    Some(match (bps, planar, prefer_float) {
        (1, false, _) => SampleFormat::U8,
        (1, true, _) => SampleFormat::U8P,
        (2, false, _) => SampleFormat::S16,
        (2, true, _) => SampleFormat::S16P,
        (3, false, _) => SampleFormat::S24,
        (4, false, true) => SampleFormat::F32,
        (4, false, false) => SampleFormat::S32,
        (4, true, true) => SampleFormat::F32P,
        (4, true, false) => SampleFormat::S32P,
        (8, false, _) => SampleFormat::F64,
        (8, true, _) => SampleFormat::F64P,
        _ => return None,
    })
}

/// Decode one (channel, sample-index) value from an [`AudioFrame`] as
/// f32 in `[-1.0, 1.0]`. Handles every interleaved + planar variant of
/// [`SampleFormat`].
///
/// `format` and `channels` are stream-level (off `CodecParameters`) —
/// the frame itself no longer carries them. Callers should pass a
/// format that has been run through [`reconcile_format`] first so a
/// stale demuxer-side default (e.g. F32) doesn't over-stride into a
/// shorter S16 buffer.
///
/// Out-of-bounds reads return `0.0` rather than panicking — the audio
/// thread shouldn't take down the whole player on a single malformed
/// frame.
///
/// Pulled out of `to_f32_interleaved` so the surround-aware routing
/// module can reuse the per-sample decoder without going through the
/// implicit channel-count adjustment.
pub fn sample_to_f32(
    frame: &AudioFrame,
    format: SampleFormat,
    channels: u16,
    ch: usize,
    i: usize,
) -> f32 {
    let in_ch = channels.max(1) as usize;
    let buf = match format.is_planar() {
        true => frame.data.get(ch),
        false => frame.data.first(),
    };
    let Some(buf) = buf else { return 0.0 };
    let read = |off: usize, len: usize| -> Option<&[u8]> { buf.get(off..off + len) };
    match format {
        SampleFormat::U8 => {
            let Some(s) = read(i * in_ch + ch, 1) else {
                return 0.0;
            };
            (s[0] as f32 - 128.0) / 128.0
        }
        SampleFormat::S8 => {
            let Some(s) = read(i * in_ch + ch, 1) else {
                return 0.0;
            };
            (s[0] as i8) as f32 / 128.0
        }
        SampleFormat::S16 => {
            let Some(s) = read((i * in_ch + ch) * 2, 2) else {
                return 0.0;
            };
            i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0
        }
        SampleFormat::S24 => {
            let Some(s) = read((i * in_ch + ch) * 3, 3) else {
                return 0.0;
            };
            let mut v = (s[0] as i32) | ((s[1] as i32) << 8) | ((s[2] as i32) << 16);
            if v & 0x80_0000 != 0 {
                v |= !0xFF_FFFF;
            }
            v as f32 / 8_388_608.0
        }
        SampleFormat::S32 => {
            let Some(s) = read((i * in_ch + ch) * 4, 4) else {
                return 0.0;
            };
            i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0
        }
        SampleFormat::F32 => {
            let Some(s) = read((i * in_ch + ch) * 4, 4) else {
                return 0.0;
            };
            f32::from_le_bytes([s[0], s[1], s[2], s[3]])
        }
        SampleFormat::F64 => {
            let Some(s) = read((i * in_ch + ch) * 8, 8) else {
                return 0.0;
            };
            f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]) as f32
        }
        SampleFormat::U8P => {
            let Some(s) = read(i, 1) else { return 0.0 };
            (s[0] as f32 - 128.0) / 128.0
        }
        SampleFormat::S16P => {
            let Some(s) = read(i * 2, 2) else { return 0.0 };
            i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0
        }
        SampleFormat::S32P => {
            let Some(s) = read(i * 4, 4) else { return 0.0 };
            i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0
        }
        SampleFormat::F32P => {
            let Some(s) = read(i * 4, 4) else { return 0.0 };
            f32::from_le_bytes([s[0], s[1], s[2], s[3]])
        }
        SampleFormat::F64P => {
            let Some(s) = read(i * 8, 8) else { return 0.0 };
            f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]) as f32
        }
        // SampleFormat is `#[non_exhaustive]` (oxideav-core); future variants
        // need their own arm. Decode silence rather than panic so an unknown
        // input format degrades gracefully on the realtime audio path.
        _ => 0.0,
    }
}

/// Convert any [`AudioFrame`] payload to f32 interleaved at
/// `out_channels` (1 = mono, 2 = stereo). `src_format` and
/// `src_channels` describe the upstream stream's shape (off
/// `CodecParameters`, no longer per-frame). Mono destination averages
/// input channels; stereo destination duplicates mono or picks the
/// first two channels verbatim.
///
/// **Note:** for surround-aware downmix (LoRo / LtRt / Binaural)
/// callers should go through `audio_routing::apply_routing` instead.
/// This function remains the simple count-adjusting fallback used by
/// the SDL2 audio engine.
pub fn to_f32_interleaved(
    frame: &AudioFrame,
    src_format: SampleFormat,
    src_channels: u16,
    out_channels: u16,
) -> Vec<f32> {
    let in_ch = src_channels.max(1) as usize;
    let n = frame.samples as usize;
    let out_ch = out_channels.max(1) as usize;
    let format = reconcile_format(frame, src_format, src_channels);
    let mut out = Vec::with_capacity(n * out_ch);

    // Up/down-mix by duplicating or averaging channels.
    for i in 0..n {
        for oc in 0..out_ch {
            let src_ch = if in_ch == 1 {
                0
            } else if out_ch == 1 {
                // Mono: average input channels.
                let mut acc = 0.0f32;
                for ic in 0..in_ch {
                    acc += sample_to_f32(frame, format, src_channels, ic, i);
                }
                out.push(acc / in_ch as f32);
                continue;
            } else {
                oc.min(in_ch - 1)
            };
            out.push(sample_to_f32(frame, format, src_channels, src_ch, i));
        }
    }
    out
}

/// Dumb linear-interpolation resampler over f32 interleaved input.
/// Good enough for playback; not used for transcoding (see
/// `oxideav-audio-filter` for that).
pub fn resample_linear(src: &[f32], src_rate: u32, dst_rate: u32, channels: usize) -> Vec<f32> {
    if src.is_empty() || channels == 0 || src_rate == 0 || dst_rate == 0 {
        return Vec::new();
    }
    let in_frames = src.len() / channels;
    if in_frames == 0 {
        return Vec::new();
    }
    let out_frames = (in_frames as u64 * dst_rate as u64 / src_rate as u64) as usize;
    let mut out = Vec::with_capacity(out_frames * channels);
    for i in 0..out_frames {
        let pos = (i as f64) * (src_rate as f64) / (dst_rate as f64);
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let idx_a = idx.min(in_frames - 1);
        let idx_b = (idx + 1).min(in_frames - 1);
        for c in 0..channels {
            let a = src[idx_a * channels + c];
            let b = src[idx_b * channels + c];
            out.push(a + (b - a) * frac);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s16_stereo_frame(samples: usize) -> AudioFrame {
        // 1024 stereo S16 samples = 4096 bytes — the AAC default-mismatch case.
        let mut data = vec![0u8; samples * 2 * 2];
        for i in 0..samples {
            let v = (i as i16).to_le_bytes();
            data[i * 4..i * 4 + 2].copy_from_slice(&v);
            data[i * 4 + 2..i * 4 + 4].copy_from_slice(&v);
        }
        AudioFrame {
            samples: samples as u32,
            pts: Some(0),
            data: vec![data],
        }
    }

    #[test]
    fn reconcile_downgrades_f32_default_to_s16_when_buffer_is_half_size() {
        // Demuxer left sample_format unpinned → driver default F32. AAC
        // emits S16 → buffer is 4096 bytes for 1024 stereo samples.
        let frame = s16_stereo_frame(1024);
        let actual = reconcile_format(&frame, SampleFormat::F32, 2);
        assert_eq!(actual, SampleFormat::S16);
    }

    #[test]
    fn reconcile_keeps_format_when_buffer_already_fits() {
        let frame = s16_stereo_frame(1024);
        // S16 is already correct — must not be downgraded.
        assert_eq!(
            reconcile_format(&frame, SampleFormat::S16, 2),
            SampleFormat::S16
        );
    }

    #[test]
    fn to_f32_interleaved_does_not_panic_on_aac_style_default_mismatch() {
        // Regression test for the panic at audio_convert.rs:74 reading
        // `frame.data[0][4096]` against a 4096-byte buffer when the
        // cached SampleFormat (F32) over-strided into an actual-S16
        // payload.
        let frame = s16_stereo_frame(1024);
        let out = to_f32_interleaved(&frame, SampleFormat::F32, 2, 2);
        assert_eq!(out.len(), 1024 * 2);
    }

    #[test]
    fn sample_to_f32_returns_silence_on_out_of_range_index() {
        let frame = s16_stereo_frame(8);
        // i past end → must not panic; returns 0.0.
        assert_eq!(sample_to_f32(&frame, SampleFormat::S16, 2, 0, 999), 0.0);
    }
}
