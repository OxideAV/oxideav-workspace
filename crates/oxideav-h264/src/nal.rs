//! NAL (Network Abstraction Layer) unit parsing for H.264.
//!
//! References:
//! * ITU-T H.264 §7.3.1 — NAL unit syntax
//! * ITU-T H.264 §B.1 — Annex B byte stream format
//! * ISO/IEC 14496-15 §5 — AVCC (length-prefixed) format used in MP4
//!
//! Two carriage formats are supported:
//!
//! * **Annex B** (typical for `.h264`/`.264` elementary streams and MPEG-TS):
//!   NAL units separated by start-code prefixes — either the 4-byte
//!   `00 00 00 01` or the 3-byte `00 00 01`. Use [`split_annex_b`] to extract
//!   per-NALU byte slices.
//!
//! * **AVCC / MP4** (the `avcC`/`AVCDecoderConfigurationRecord` extradata
//!   format and the per-sample wire format inside `mdat`): each NALU is
//!   prefixed by a fixed-width big-endian length field whose width
//!   (1, 2, or 4 bytes — almost always 4) is encoded in the configuration
//!   record. Use [`split_length_prefixed`] to extract per-NALU byte slices.
//!
//! The RBSP (raw byte sequence payload) is the NAL unit payload after
//! emulation-prevention byte removal — see [`extract_rbsp`].

use oxideav_core::{Error, Result};

/// NAL unit type, §7.4.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NalUnitType {
    /// 0 — unspecified.
    Unspecified,
    /// 1 — coded slice of a non-IDR picture.
    SliceNonIdr,
    /// 2 — coded slice data partition A.
    DataPartitionA,
    /// 3 — coded slice data partition B.
    DataPartitionB,
    /// 4 — coded slice data partition C.
    DataPartitionC,
    /// 5 — coded slice of an IDR picture.
    SliceIdr,
    /// 6 — Supplemental Enhancement Information.
    Sei,
    /// 7 — Sequence Parameter Set.
    Sps,
    /// 8 — Picture Parameter Set.
    Pps,
    /// 9 — Access Unit Delimiter.
    Aud,
    /// 10 — End of sequence.
    EndOfSeq,
    /// 11 — End of stream.
    EndOfStream,
    /// 12 — Filler data.
    Filler,
    /// 13 — Sequence parameter set extension.
    SpsExt,
    /// 14 — Prefix NAL unit.
    Prefix,
    /// 15 — Subset SPS.
    SubsetSps,
    /// 16-18 — reserved.
    Reserved(u8),
    /// 19 — Coded slice of an auxiliary coded picture.
    AuxiliarySlice,
    /// 20 — Coded slice extension (SVC/MVC).
    SliceExtension,
    /// 21-23 — reserved (Depth view, etc).
    ReservedHigh(u8),
    /// 24-31 — unspecified.
    UnspecifiedHigh(u8),
}

impl NalUnitType {
    pub fn from_u5(t: u8) -> Self {
        use NalUnitType::*;
        match t {
            0 => Unspecified,
            1 => SliceNonIdr,
            2 => DataPartitionA,
            3 => DataPartitionB,
            4 => DataPartitionC,
            5 => SliceIdr,
            6 => Sei,
            7 => Sps,
            8 => Pps,
            9 => Aud,
            10 => EndOfSeq,
            11 => EndOfStream,
            12 => Filler,
            13 => SpsExt,
            14 => Prefix,
            15 => SubsetSps,
            16..=18 => Reserved(t),
            19 => AuxiliarySlice,
            20 => SliceExtension,
            21..=23 => ReservedHigh(t),
            _ => UnspecifiedHigh(t),
        }
    }

    pub fn raw(self) -> u8 {
        use NalUnitType::*;
        match self {
            Unspecified => 0,
            SliceNonIdr => 1,
            DataPartitionA => 2,
            DataPartitionB => 3,
            DataPartitionC => 4,
            SliceIdr => 5,
            Sei => 6,
            Sps => 7,
            Pps => 8,
            Aud => 9,
            EndOfSeq => 10,
            EndOfStream => 11,
            Filler => 12,
            SpsExt => 13,
            Prefix => 14,
            SubsetSps => 15,
            Reserved(t) | ReservedHigh(t) | UnspecifiedHigh(t) => t,
            AuxiliarySlice => 19,
            SliceExtension => 20,
        }
    }

    /// True for any slice payload (IDR, non-IDR, partitioned, auxiliary).
    pub fn is_slice(self) -> bool {
        matches!(
            self,
            NalUnitType::SliceNonIdr
                | NalUnitType::SliceIdr
                | NalUnitType::DataPartitionA
                | NalUnitType::DataPartitionB
                | NalUnitType::DataPartitionC
                | NalUnitType::AuxiliarySlice
        )
    }
}

/// Decoded NAL header (§7.3.1).
#[derive(Clone, Copy, Debug)]
pub struct NalHeader {
    pub forbidden_zero_bit: u8,
    pub nal_ref_idc: u8,
    pub nal_unit_type: NalUnitType,
}

impl NalHeader {
    /// Parse the 1-byte NAL unit header.
    pub fn parse(byte: u8) -> Result<Self> {
        let f = (byte >> 7) & 0x1;
        if f != 0 {
            return Err(Error::invalid("h264: forbidden_zero_bit must be 0"));
        }
        let r = (byte >> 5) & 0x3;
        let t = byte & 0x1F;
        Ok(Self {
            forbidden_zero_bit: f,
            nal_ref_idc: r,
            nal_unit_type: NalUnitType::from_u5(t),
        })
    }
}

/// One NAL unit slice in its source carriage.
#[derive(Clone, Debug)]
pub struct NalUnit<'a> {
    pub header: NalHeader,
    /// NAL payload **with emulation prevention bytes still present**. Pass
    /// through [`extract_rbsp`] for parsing.
    pub raw_payload: &'a [u8],
}

/// Split an Annex B byte stream into NAL unit byte slices, **including** the
/// 1-byte NAL header. Start-code prefixes are stripped.
///
/// Accepts both the 4-byte `00 00 00 01` and the 3-byte `00 00 01` variants.
/// Trailing `00` bytes after the last NALU are tolerated.
pub fn split_annex_b(stream: &[u8]) -> Vec<&[u8]> {
    let starts = find_start_codes(stream);
    let mut out = Vec::with_capacity(starts.len());
    for i in 0..starts.len() {
        let (sc_pos, sc_len) = starts[i];
        let nalu_start = sc_pos + sc_len;
        let nalu_end = if i + 1 < starts.len() {
            starts[i + 1].0
        } else {
            // Trim trailing zero stuffing.
            let mut end = stream.len();
            while end > nalu_start && stream[end - 1] == 0 {
                end -= 1;
            }
            end
        };
        if nalu_end > nalu_start {
            out.push(&stream[nalu_start..nalu_end]);
        }
    }
    out
}

/// Find all Annex B start codes in `data`. Returns `(position, length)`
/// pairs where `length` is 3 or 4.
fn find_start_codes(data: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                out.push((i, 3));
                i += 3;
                continue;
            }
            if i + 3 < data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                out.push((i, 4));
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Split AVCC length-prefixed NAL units. `length_size` must be 1, 2, or 4
/// (per ISO/IEC 14496-15, encoded in `lengthSizeMinusOne` of the
/// AVCDecoderConfigurationRecord — see [`AvcConfig`]).
pub fn split_length_prefixed(stream: &[u8], length_size: u8) -> Result<Vec<&[u8]>> {
    if !matches!(length_size, 1 | 2 | 4) {
        return Err(Error::invalid(format!(
            "h264 avcc: invalid length size {length_size} (must be 1/2/4)"
        )));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < stream.len() {
        if i + length_size as usize > stream.len() {
            return Err(Error::invalid("h264 avcc: truncated length-prefix header"));
        }
        let mut len: u32 = 0;
        for k in 0..length_size {
            len = (len << 8) | stream[i + k as usize] as u32;
        }
        i += length_size as usize;
        let end = i + len as usize;
        if end > stream.len() {
            return Err(Error::invalid(format!(
                "h264 avcc: NALU length {len} exceeds buffer (have {})",
                stream.len() - i
            )));
        }
        out.push(&stream[i..end]);
        i = end;
    }
    Ok(out)
}

/// Strip H.264 emulation-prevention bytes (§7.4.1.1).
///
/// Inside a NAL unit payload, any sequence of two `0x00` bytes followed by
/// a byte ≤ `0x03` is encoded with an extra `0x03` inserted before the
/// third byte to prevent accidental Annex B start codes inside the payload.
/// On parse, those `0x03` bytes are removed.
pub fn extract_rbsp(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len());
    let mut i = 0;
    while i < payload.len() {
        if i + 2 < payload.len() && payload[i] == 0 && payload[i + 1] == 0 && payload[i + 2] == 0x03
        {
            out.push(0);
            out.push(0);
            i += 3;
            continue;
        }
        out.push(payload[i]);
        i += 1;
    }
    out
}

/// Parsed AVCDecoderConfigurationRecord (`avcC`, ISO/IEC 14496-15 §5.2.4.1).
///
/// Layout:
/// ```text
/// configurationVersion       u(8)   = 1
/// AVCProfileIndication       u(8)   (sps[1])
/// profile_compatibility      u(8)   (sps[2])
/// AVCLevelIndication         u(8)   (sps[3])
/// reserved                   u(6)   = '111111'
/// lengthSizeMinusOne         u(2)
/// reserved                   u(3)   = '111'
/// numOfSequenceParameterSets u(5)
///   for each SPS:
///     sequenceParameterSetLength      u(16)
///     sequenceParameterSetNALUnit     (length bytes)
/// numOfPictureParameterSets  u(8)
///   for each PPS:
///     pictureParameterSetLength       u(16)
///     pictureParameterSetNALUnit      (length bytes)
/// ```
#[derive(Clone, Debug)]
pub struct AvcConfig {
    pub configuration_version: u8,
    pub profile_indication: u8,
    pub profile_compatibility: u8,
    pub level_indication: u8,
    /// 1, 2, or 4 — the per-NALU length-prefix width.
    pub length_size: u8,
    /// SPS NAL units (each starts with the 1-byte NAL header).
    pub sps: Vec<Vec<u8>>,
    /// PPS NAL units (each starts with the 1-byte NAL header).
    pub pps: Vec<Vec<u8>>,
}

impl AvcConfig {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 7 {
            return Err(Error::invalid("h264 avcC: too short"));
        }
        let configuration_version = data[0];
        if configuration_version != 1 {
            return Err(Error::invalid(format!(
                "h264 avcC: unsupported configurationVersion {configuration_version}"
            )));
        }
        let profile_indication = data[1];
        let profile_compatibility = data[2];
        let level_indication = data[3];
        let length_size = (data[4] & 0x03) + 1;
        if !matches!(length_size, 1 | 2 | 4) {
            return Err(Error::invalid(format!(
                "h264 avcC: lengthSizeMinusOne yields {length_size}"
            )));
        }
        let num_sps = (data[5] & 0x1F) as usize;
        let mut off = 6usize;
        let mut sps = Vec::with_capacity(num_sps);
        for _ in 0..num_sps {
            if off + 2 > data.len() {
                return Err(Error::invalid("h264 avcC: SPS length truncated"));
            }
            let len = u16::from_be_bytes([data[off], data[off + 1]]) as usize;
            off += 2;
            if off + len > data.len() {
                return Err(Error::invalid("h264 avcC: SPS body truncated"));
            }
            sps.push(data[off..off + len].to_vec());
            off += len;
        }
        if off >= data.len() {
            return Err(Error::invalid("h264 avcC: missing PPS section"));
        }
        let num_pps = data[off] as usize;
        off += 1;
        let mut pps = Vec::with_capacity(num_pps);
        for _ in 0..num_pps {
            if off + 2 > data.len() {
                return Err(Error::invalid("h264 avcC: PPS length truncated"));
            }
            let len = u16::from_be_bytes([data[off], data[off + 1]]) as usize;
            off += 2;
            if off + len > data.len() {
                return Err(Error::invalid("h264 avcC: PPS body truncated"));
            }
            pps.push(data[off..off + len].to_vec());
            off += len;
        }
        Ok(Self {
            configuration_version,
            profile_indication,
            profile_compatibility,
            level_indication,
            length_size,
            sps,
            pps,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_annex_b_basic() {
        let stream = [
            0, 0, 0, 1, 0x67, 0xAA, 0, 0, 0, 1, 0x68, 0xBB, 0xCC, 0, 0, 1, 0x65, 0xDD,
        ];
        let nalus = split_annex_b(&stream);
        assert_eq!(nalus.len(), 3);
        assert_eq!(nalus[0], &[0x67, 0xAA]);
        assert_eq!(nalus[1], &[0x68, 0xBB, 0xCC]);
        assert_eq!(nalus[2], &[0x65, 0xDD]);
    }

    #[test]
    fn nal_header_parse() {
        let h = NalHeader::parse(0x67).unwrap();
        assert_eq!(h.nal_ref_idc, 3);
        assert_eq!(h.nal_unit_type, NalUnitType::Sps);
        let h = NalHeader::parse(0x65).unwrap();
        assert_eq!(h.nal_unit_type, NalUnitType::SliceIdr);
        assert!(NalHeader::parse(0x80).is_err());
    }

    #[test]
    fn rbsp_unescape() {
        let payload = [0x00, 0x00, 0x03, 0x01, 0x00, 0x00, 0x03, 0x02];
        let rbsp = extract_rbsp(&payload);
        assert_eq!(rbsp, vec![0x00, 0x00, 0x01, 0x00, 0x00, 0x02]);
    }

    #[test]
    fn avcc_parse_minimal() {
        // configurationVersion=1, profile=0x42 (baseline), compat=0x40,
        // level=0x1E, lengthSizeMinusOne=3 (=> length_size=4),
        // numSPS=1, SPS_len=4 bytes [0x67, 0x42, 0xC0, 0x1E],
        // numPPS=1, PPS_len=2 bytes [0x68, 0xCE].
        let data = [
            1, 0x42, 0x40, 0x1E, 0xFF, 0xE1, 0, 4, 0x67, 0x42, 0xC0, 0x1E, 1, 0, 2, 0x68, 0xCE,
        ];
        let cfg = AvcConfig::parse(&data).unwrap();
        assert_eq!(cfg.length_size, 4);
        assert_eq!(cfg.sps.len(), 1);
        assert_eq!(cfg.pps.len(), 1);
        assert_eq!(cfg.sps[0], vec![0x67, 0x42, 0xC0, 0x1E]);
        assert_eq!(cfg.pps[0], vec![0x68, 0xCE]);
    }
}
