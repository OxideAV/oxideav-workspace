//! Sample-body extraction for MOD files.
//!
//! After the header + pattern data block, the remainder of the file is a
//! concatenation of raw signed-8-bit sample bodies, in the order samples
//! appear in the header. The header tells us each body's length in bytes.
//! Some files are truncated (the last sample's declared length exceeds
//! the file) — we clamp rather than error.

use crate::header::ModHeader;

/// Per-sample decoded body plus the loop metadata needed by the mixer.
#[derive(Clone, Debug, Default)]
pub struct SampleBody {
    /// Raw signed 8-bit PCM. Empty if the header declared zero length.
    pub pcm: Vec<i8>,
    /// Loop start in samples (0 if sample does not loop).
    pub loop_start: u32,
    /// Loop length in samples (0 if sample does not loop — spec says
    /// repeat length of 2 also means "no loop").
    pub loop_length: u32,
    /// Default volume 0..=64.
    pub volume: u8,
    /// Finetune -8..=7.
    pub finetune: i8,
}

impl SampleBody {
    /// True if this sample has a valid loop region.
    pub fn is_looped(&self) -> bool {
        self.loop_length > 2
    }
}

/// Extract all 31 sample bodies from the module bytes.
///
/// Samples declared longer than the remaining file are clamped to what's
/// actually there (many real-world rips are slightly truncated).
pub fn extract_samples(header: &ModHeader, bytes: &[u8]) -> Vec<SampleBody> {
    let mut out = Vec::with_capacity(header.samples.len());
    let mut cursor = header.sample_data_offset();
    let end = bytes.len();

    for sample in &header.samples {
        let declared = sample.length as usize;
        let available = end.saturating_sub(cursor);
        let take = declared.min(available);

        let pcm: Vec<i8> = if take == 0 {
            Vec::new()
        } else {
            // Reinterpret u8 as i8 (MOD samples are signed 8-bit).
            bytes[cursor..cursor + take]
                .iter()
                .map(|&b| b as i8)
                .collect()
        };

        cursor += take;

        // A loop_length of 0 or 2 means "no loop" per the ProTracker spec.
        let (loop_start, loop_length) = if sample.repeat_length > 2 {
            (sample.repeat_start, sample.repeat_length)
        } else {
            (0, 0)
        };

        out.push(SampleBody {
            pcm,
            loop_start,
            loop_length,
            volume: sample.volume,
            finetune: sample.finetune,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::parse_header;

    fn build_minimal_mod_with_sample(pcm: &[i8]) -> Vec<u8> {
        let mut out = vec![0u8; crate::header::HEADER_FIXED_SIZE];
        // Title
        out[0..4].copy_from_slice(b"test");
        // Sample 0: length-in-words at offset 20 + 22..24.
        let len_words = (pcm.len() / 2) as u16;
        out[20 + 22..20 + 24].copy_from_slice(&len_words.to_be_bytes());
        // Volume.
        out[20 + 25] = 64;
        // Repeat start.
        out[20 + 26..20 + 28].copy_from_slice(&0u16.to_be_bytes());
        // Repeat length.
        out[20 + 28..20 + 30].copy_from_slice(&0u16.to_be_bytes());
        // Song length 1 pattern.
        out[950] = 1;
        out[951] = 0x7F;
        out[952] = 0; // order: pattern 0
        out[1080..1084].copy_from_slice(b"M.K.");
        // Pattern 0 — 64 rows × 4 channels × 4 bytes = 1024 bytes of zeros.
        out.extend(std::iter::repeat_n(0u8, 64 * 4 * 4));
        // Sample body.
        out.extend(pcm.iter().map(|&s| s as u8));
        out
    }

    #[test]
    fn extracts_signed_bytes() {
        let pcm = [10i8, -10, 40, -40, 127, -128];
        let bytes = build_minimal_mod_with_sample(&pcm);
        let header = parse_header(&bytes).unwrap();
        let samples = extract_samples(&header, &bytes);
        assert_eq!(samples.len(), 31);
        assert_eq!(samples[0].pcm, pcm);
        // Remaining samples empty.
        for s in &samples[1..] {
            assert!(s.pcm.is_empty());
        }
    }

    #[test]
    fn handles_truncated_body() {
        let pcm = [1i8, 2, 3, 4];
        let mut bytes = build_minimal_mod_with_sample(&pcm);
        // Truncate by 2 bytes.
        bytes.truncate(bytes.len() - 2);
        let header = parse_header(&bytes).unwrap();
        let samples = extract_samples(&header, &bytes);
        assert_eq!(samples[0].pcm, [1, 2]);
    }
}
