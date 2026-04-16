//! Convert [`AudioFrame`] payloads to and from `f32` for filter processing.
//!
//! All conversions use `f32` as a common interchange. The output side clamps
//! to the destination format's representable range so caller code does not
//! need to worry about wrap-around.
//!
//! Layout returned by [`decode_to_f32`] is always **interleaved-by-channel**:
//! the outer `Vec` has one entry per channel, the inner `Vec` holds samples
//! for that channel in time order. [`encode_from_f32`] consumes the same
//! layout.

use oxideav_core::{AudioFrame, Error, Result, SampleFormat};

/// Maximum positive value of S24.
const S24_MAX: i32 = 0x7F_FFFF;

/// Decode an [`AudioFrame`] into per-channel `f32` sample buffers.
///
/// The returned `Vec` has `frame.channels` entries; each contains
/// `frame.samples` samples in `[-1.0, 1.0]` (approximately, integer formats
/// are normalised by their full-scale value).
pub fn decode_to_f32(frame: &AudioFrame) -> Result<Vec<Vec<f32>>> {
    let channels = frame.channels as usize;
    let samples = frame.samples as usize;
    if channels == 0 {
        return Err(Error::invalid("audio frame with zero channels"));
    }

    let mut out: Vec<Vec<f32>> = (0..channels).map(|_| Vec::with_capacity(samples)).collect();

    if frame.format.is_planar() {
        if frame.data.len() != channels {
            return Err(Error::invalid("planar frame plane count mismatch"));
        }
        for (ch, plane) in frame.data.iter().enumerate().take(channels) {
            decode_plane_to(frame.format, plane, samples, &mut out[ch])?;
        }
    } else {
        let plane = frame
            .data
            .first()
            .ok_or_else(|| Error::invalid("interleaved frame missing data plane"))?;
        decode_interleaved(frame.format, plane, channels, samples, &mut out)?;
    }

    Ok(out)
}

/// Encode per-channel `f32` sample buffers into an [`AudioFrame`] using
/// `template`'s format, channel count, and sample rate. The PTS and time
/// base are copied from `template`. The new frame's `samples` field is set
/// from the channel buffer length.
pub fn encode_from_f32(template: &AudioFrame, channels_data: &[Vec<f32>]) -> Result<AudioFrame> {
    let channels = template.channels as usize;
    if channels_data.len() != channels {
        return Err(Error::invalid("encode_from_f32: channel count mismatch"));
    }
    let samples = channels_data.first().map(|c| c.len()).unwrap_or(0);
    for ch in channels_data {
        if ch.len() != samples {
            return Err(Error::invalid(
                "encode_from_f32: channel buffers have different lengths",
            ));
        }
    }

    let bps = template.format.bytes_per_sample();
    let data: Vec<Vec<u8>> = if template.format.is_planar() {
        channels_data
            .iter()
            .map(|ch| encode_plane(template.format, ch))
            .collect()
    } else {
        let mut buf = vec![0u8; samples * channels * bps];
        for s in 0..samples {
            for (c, ch_buf) in channels_data.iter().enumerate() {
                let off = (s * channels + c) * bps;
                write_sample(template.format, ch_buf[s], &mut buf[off..off + bps]);
            }
        }
        vec![buf]
    };

    Ok(AudioFrame {
        format: template.format,
        channels: template.channels,
        sample_rate: template.sample_rate,
        samples: samples as u32,
        pts: template.pts,
        time_base: template.time_base,
        data,
    })
}

fn decode_plane_to(
    fmt: SampleFormat,
    plane: &[u8],
    samples: usize,
    out: &mut Vec<f32>,
) -> Result<()> {
    let bps = fmt.bytes_per_sample();
    let need = samples * bps;
    if plane.len() < need {
        return Err(Error::invalid("plane shorter than declared samples"));
    }
    out.clear();
    out.reserve(samples);
    for s in 0..samples {
        let off = s * bps;
        out.push(read_sample(fmt, &plane[off..off + bps]));
    }
    Ok(())
}

fn decode_interleaved(
    fmt: SampleFormat,
    plane: &[u8],
    channels: usize,
    samples: usize,
    out: &mut [Vec<f32>],
) -> Result<()> {
    let bps = fmt.bytes_per_sample();
    let need = samples * channels * bps;
    if plane.len() < need {
        return Err(Error::invalid(
            "interleaved plane shorter than declared samples",
        ));
    }
    for ch in out.iter_mut() {
        ch.clear();
        ch.reserve(samples);
    }
    for s in 0..samples {
        for (c, ch) in out.iter_mut().enumerate().take(channels) {
            let off = (s * channels + c) * bps;
            ch.push(read_sample(fmt, &plane[off..off + bps]));
        }
    }
    Ok(())
}

fn encode_plane(fmt: SampleFormat, ch_buf: &[f32]) -> Vec<u8> {
    let bps = fmt.bytes_per_sample();
    let mut buf = vec![0u8; ch_buf.len() * bps];
    for (s, sample) in ch_buf.iter().enumerate() {
        let off = s * bps;
        write_sample(fmt, *sample, &mut buf[off..off + bps]);
    }
    buf
}

fn read_sample(fmt: SampleFormat, bytes: &[u8]) -> f32 {
    match fmt {
        SampleFormat::U8 | SampleFormat::U8P => (bytes[0] as f32 - 128.0) / 128.0,
        SampleFormat::S8 => (bytes[0] as i8 as f32) / 128.0,
        SampleFormat::S16 | SampleFormat::S16P => {
            let v = i16::from_le_bytes([bytes[0], bytes[1]]) as f32;
            v / 32768.0
        }
        SampleFormat::S24 => {
            // packed signed 24-bit little-endian
            let raw = (bytes[0] as i32) | ((bytes[1] as i32) << 8) | ((bytes[2] as i32) << 16);
            let signed = if raw & 0x80_0000 != 0 {
                raw | !0x00FF_FFFF_i32
            } else {
                raw
            };
            signed as f32 / 8_388_608.0
        }
        SampleFormat::S32 | SampleFormat::S32P => {
            let v = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32;
            v / 2_147_483_648.0
        }
        SampleFormat::F32 | SampleFormat::F32P => {
            f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        }
        SampleFormat::F64 | SampleFormat::F64P => f64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]) as f32,
    }
}

fn write_sample(fmt: SampleFormat, value: f32, out: &mut [u8]) {
    let v = value.clamp(-1.0, 1.0);
    match fmt {
        SampleFormat::U8 | SampleFormat::U8P => {
            let scaled = (v * 128.0 + 128.0).round().clamp(0.0, 255.0) as u8;
            out[0] = scaled;
        }
        SampleFormat::S8 => {
            let scaled = (v * 128.0).round().clamp(-128.0, 127.0) as i8;
            out[0] = scaled as u8;
        }
        SampleFormat::S16 | SampleFormat::S16P => {
            let scaled = (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
            let bytes = scaled.to_le_bytes();
            out[0] = bytes[0];
            out[1] = bytes[1];
        }
        SampleFormat::S24 => {
            let scaled = (v * S24_MAX as f32)
                .round()
                .clamp(-(S24_MAX as f32 + 1.0), S24_MAX as f32) as i32;
            out[0] = (scaled & 0xFF) as u8;
            out[1] = ((scaled >> 8) & 0xFF) as u8;
            out[2] = ((scaled >> 16) & 0xFF) as u8;
        }
        SampleFormat::S32 | SampleFormat::S32P => {
            let scaled = (v as f64 * 2_147_483_647.0)
                .round()
                .clamp(-2_147_483_648.0, 2_147_483_647.0) as i32;
            let bytes = scaled.to_le_bytes();
            out[0] = bytes[0];
            out[1] = bytes[1];
            out[2] = bytes[2];
            out[3] = bytes[3];
        }
        SampleFormat::F32 | SampleFormat::F32P => {
            let bytes = value.to_le_bytes();
            out[..4].copy_from_slice(&bytes);
        }
        SampleFormat::F64 | SampleFormat::F64P => {
            let bytes = (value as f64).to_le_bytes();
            out[..8].copy_from_slice(&bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::TimeBase;

    fn make_frame(
        fmt: SampleFormat,
        channels: u16,
        planes: Vec<Vec<u8>>,
        samples: u32,
    ) -> AudioFrame {
        AudioFrame {
            format: fmt,
            channels,
            sample_rate: 48_000,
            samples,
            pts: None,
            time_base: TimeBase::new(1, 48_000),
            data: planes,
        }
    }

    #[test]
    fn roundtrip_s16_interleaved() {
        let samples: Vec<i16> = vec![0, 16384, -16384, 32767, -32768];
        let mut bytes = Vec::new();
        for s in &samples {
            bytes.extend_from_slice(&s.to_le_bytes());
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let frame = make_frame(SampleFormat::S16, 2, vec![bytes], samples.len() as u32);
        let decoded = decode_to_f32(&frame).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].len(), samples.len());
        let re = encode_from_f32(&frame, &decoded).unwrap();
        // Re-decode and compare
        let again = decode_to_f32(&re).unwrap();
        for ch in 0..2 {
            for i in 0..samples.len() {
                assert!((again[ch][i] - decoded[ch][i]).abs() < 1.0e-4);
            }
        }
    }

    #[test]
    fn roundtrip_f32_planar() {
        let mut left: Vec<u8> = Vec::new();
        let mut right: Vec<u8> = Vec::new();
        for i in 0..16 {
            left.extend_from_slice(&((i as f32 / 32.0).to_le_bytes()));
            right.extend_from_slice(&((-(i as f32) / 32.0).to_le_bytes()));
        }
        let frame = make_frame(SampleFormat::F32P, 2, vec![left, right], 16);
        let decoded = decode_to_f32(&frame).unwrap();
        let re = encode_from_f32(&frame, &decoded).unwrap();
        assert_eq!(re.data.len(), 2);
        assert_eq!(re.data[0].len(), 16 * 4);
    }

    #[test]
    fn s24_roundtrip() {
        // 3-byte LE samples
        let raw: Vec<i32> = vec![0, 1_000_000, -1_000_000, S24_MAX, -(S24_MAX + 1)];
        let mut bytes = Vec::new();
        for v in &raw {
            bytes.push((v & 0xFF) as u8);
            bytes.push(((v >> 8) & 0xFF) as u8);
            bytes.push(((v >> 16) & 0xFF) as u8);
        }
        let frame = make_frame(SampleFormat::S24, 1, vec![bytes], raw.len() as u32);
        let decoded = decode_to_f32(&frame).unwrap();
        let re = encode_from_f32(&frame, &decoded).unwrap();
        let again = decode_to_f32(&re).unwrap();
        for i in 0..raw.len() {
            assert!((decoded[0][i] - again[0][i]).abs() < 1.0e-6);
        }
    }
}
