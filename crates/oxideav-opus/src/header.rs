//! OpusHead parsing — the identification packet of an Opus-in-Ogg stream
//! (RFC 7845 §5.1).

use oxideav_core::{Error, Result};

#[derive(Clone, Debug)]
pub struct OpusHead {
    pub version: u8,
    pub output_channel_count: u8,
    pub pre_skip: u16,
    /// Nominal sample rate of the encoder input. Opus always decodes at 48 kHz.
    pub input_sample_rate: u32,
    pub output_gain: i16,
    pub channel_mapping_family: u8,
    /// Channel-mapping-family-specific data (streams count, coupled streams,
    /// channel-mapping table). Present when `channel_mapping_family != 0`.
    pub mapping_table: Vec<u8>,
}

pub fn parse_opus_head(packet: &[u8]) -> Result<OpusHead> {
    if packet.len() < 19 {
        return Err(Error::invalid("OpusHead too short"));
    }
    if &packet[0..8] != b"OpusHead" {
        return Err(Error::invalid("not an OpusHead packet"));
    }
    let version = packet[8];
    if version & 0xF0 != 0 && version != 1 {
        // Major version must be 0 per RFC 7845 §5.1. Some tools emit version 1;
        // many decoders tolerate anything with the upper nibble clear.
        return Err(Error::unsupported(format!(
            "unsupported OpusHead version {version}"
        )));
    }
    let output_channel_count = packet[9];
    if output_channel_count == 0 {
        return Err(Error::invalid("OpusHead channel count is zero"));
    }
    let pre_skip = u16::from_le_bytes([packet[10], packet[11]]);
    let input_sample_rate = u32::from_le_bytes([packet[12], packet[13], packet[14], packet[15]]);
    let output_gain = i16::from_le_bytes([packet[16], packet[17]]);
    let channel_mapping_family = packet[18];
    let mapping_table = if channel_mapping_family == 0 {
        Vec::new()
    } else {
        // For families 1 and 2: 1 byte stream count, 1 byte coupled count,
        // then output_channel_count bytes of mapping. Total = 2 + C bytes.
        let expected = 2 + output_channel_count as usize;
        if packet.len() < 19 + expected {
            return Err(Error::invalid("OpusHead channel-mapping table truncated"));
        }
        packet[19..19 + expected].to_vec()
    };
    Ok(OpusHead {
        version,
        output_channel_count,
        pre_skip,
        input_sample_rate,
        output_gain,
        channel_mapping_family,
        mapping_table,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_stereo_head() {
        // OpusHead + version=1 + 2ch + pre_skip=312 + rate=48000 + gain=0 + family=0.
        let mut p = Vec::new();
        p.extend_from_slice(b"OpusHead");
        p.push(1); // version
        p.push(2); // channel count
        p.extend_from_slice(&312u16.to_le_bytes());
        p.extend_from_slice(&48_000u32.to_le_bytes());
        p.extend_from_slice(&0i16.to_le_bytes());
        p.push(0); // family
        let h = parse_opus_head(&p).unwrap();
        assert_eq!(h.output_channel_count, 2);
        assert_eq!(h.pre_skip, 312);
        assert_eq!(h.input_sample_rate, 48_000);
        assert_eq!(h.channel_mapping_family, 0);
        assert!(h.mapping_table.is_empty());
    }

    #[test]
    fn rejects_bad_signature() {
        let p = vec![0u8; 20];
        assert!(parse_opus_head(&p).is_err());
    }

    #[test]
    fn parses_family_1_mapping() {
        let mut p = Vec::new();
        p.extend_from_slice(b"OpusHead");
        p.push(1);
        p.push(2);
        p.extend_from_slice(&312u16.to_le_bytes());
        p.extend_from_slice(&48_000u32.to_le_bytes());
        p.extend_from_slice(&0i16.to_le_bytes());
        p.push(1); // family 1
        p.push(1); // stream count
        p.push(1); // coupled count
        p.push(0); // channel mapping [0]
        p.push(1); // channel mapping [1]
        let h = parse_opus_head(&p).unwrap();
        assert_eq!(h.channel_mapping_family, 1);
        assert_eq!(h.mapping_table, vec![1, 1, 0, 1]);
    }
}
