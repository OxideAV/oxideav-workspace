//! AV1CodecConfigurationRecord (`av1C`) parser — AV1 ISO/IEC 14496-1
//! mapping. The record is the standard container-level "extradata" for
//! AV1 in MP4 (`av01` sample entry) and Matroska / WebM (`V_AV1` codec
//! private). Layout (4 fixed bytes + variable configOBUs):
//!
//! ```text
//! marker(1)=1, version(7)=1
//! seq_profile(3), seq_level_idx_0(5)
//! seq_tier_0(1), high_bitdepth(1), twelve_bit(1), monochrome(1),
//! chroma_subsampling_x(1), chroma_subsampling_y(1), chroma_sample_position(2)
//! reserved(3), initial_presentation_delay_present(1),
//! initial_presentation_delay_minus_one(4) | reserved(4)
//! configOBUs[]
//! ```

use oxideav_core::{Error, Result};

use crate::obu::{parse_config_obus, Obu, ObuType};
use crate::sequence_header::{parse_sequence_header, SequenceHeader};

/// Decoded `av1C` record. The `seq_header` is conveniently re-parsed from
/// the embedded sequence-header config OBU.
#[derive(Clone, Debug)]
pub struct Av1CodecConfig {
    pub version: u8,
    pub seq_profile: u8,
    pub seq_level_idx_0: u8,
    pub seq_tier_0: bool,
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub monochrome: bool,
    pub chroma_subsampling_x: bool,
    pub chroma_subsampling_y: bool,
    pub chroma_sample_position: u8,
    pub initial_presentation_delay_present: bool,
    pub initial_presentation_delay_minus_one: u8,
    pub config_obus: Vec<u8>,
    pub seq_header: Option<SequenceHeader>,
}

impl Av1CodecConfig {
    /// Parse an `av1C` body (the bytes inside the box, excluding the 8-byte
    /// box header). The record is at least 4 bytes; configOBUs follow.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            return Err(Error::invalid("av1C: too short"));
        }
        let b0 = data[0];
        let marker = (b0 >> 7) & 1;
        let version = b0 & 0x7f;
        if marker != 1 {
            return Err(Error::invalid("av1C: marker bit not set"));
        }
        if version != 1 {
            return Err(Error::invalid(format!(
                "av1C: unsupported version {version}"
            )));
        }
        let b1 = data[1];
        let seq_profile = (b1 >> 5) & 0x07;
        let seq_level_idx_0 = b1 & 0x1f;
        let b2 = data[2];
        let seq_tier_0 = ((b2 >> 7) & 1) != 0;
        let high_bitdepth = ((b2 >> 6) & 1) != 0;
        let twelve_bit = ((b2 >> 5) & 1) != 0;
        let monochrome = ((b2 >> 4) & 1) != 0;
        let chroma_subsampling_x = ((b2 >> 3) & 1) != 0;
        let chroma_subsampling_y = ((b2 >> 2) & 1) != 0;
        let chroma_sample_position = b2 & 0x03;
        let b3 = data[3];
        let initial_presentation_delay_present = ((b3 >> 4) & 1) != 0;
        let initial_presentation_delay_minus_one = if initial_presentation_delay_present {
            b3 & 0x0f
        } else {
            0
        };
        let config_obus = data[4..].to_vec();

        // Locate and decode the embedded sequence header for caller convenience.
        let mut seq_header = None;
        if !config_obus.is_empty() {
            let parsed = parse_config_obus(&config_obus)?;
            for o in &parsed {
                if o.header.obu_type == ObuType::SequenceHeader {
                    seq_header = Some(parse_sequence_header(o.payload)?);
                    break;
                }
            }
        }

        Ok(Self {
            version,
            seq_profile,
            seq_level_idx_0,
            seq_tier_0,
            high_bitdepth,
            twelve_bit,
            monochrome,
            chroma_subsampling_x,
            chroma_subsampling_y,
            chroma_sample_position,
            initial_presentation_delay_present,
            initial_presentation_delay_minus_one,
            config_obus,
            seq_header,
        })
    }

    /// Iterate the config OBUs as `Obu<'_>` views.
    pub fn iter_config_obus(&self) -> Result<Vec<Obu<'_>>> {
        parse_config_obus(&self.config_obus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_real_av1c() {
        // Captured from /tmp/av1.mp4 (aomenc-encoded 64x64 AV1 clip).
        let data = [
            0x81, 0x00, 0x0c, 0x00, 0x0a, 0x0a, 0x00, 0x00, 0x00, 0x02, 0xaf, 0xff, 0x9b, 0x5f,
            0x30, 0x08,
        ];
        let cfg = Av1CodecConfig::parse(&data).unwrap();
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.seq_profile, 0);
        assert!(cfg.chroma_subsampling_x);
        assert!(cfg.chroma_subsampling_y);
        assert!(!cfg.monochrome);
        let sh = cfg.seq_header.expect("seq header");
        assert_eq!(sh.max_frame_width, 64);
        assert_eq!(sh.max_frame_height, 64);
    }
}
