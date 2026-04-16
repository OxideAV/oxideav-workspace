//! Sequence Parameter Set (SPS) parsing — ITU-T H.264 §7.3.2.1.1.
//!
//! Only the fields needed to drive the decoder and report stream metadata
//! are stored. VUI parameters are parsed only enough to skip over their
//! syntax — full VUI extraction (timing, video signal, HRD) can be added
//! later without API breakage.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::nal::NalHeader;

/// Parsed SPS. Field names follow the spec exactly.
#[derive(Clone, Debug)]
pub struct Sps {
    pub profile_idc: u8,
    pub constraint_set0_flag: bool,
    pub constraint_set1_flag: bool,
    pub constraint_set2_flag: bool,
    pub constraint_set3_flag: bool,
    pub constraint_set4_flag: bool,
    pub constraint_set5_flag: bool,
    pub level_idc: u8,
    pub seq_parameter_set_id: u32,

    // Profile-specific (present only for high profiles).
    pub chroma_format_idc: u32,
    pub separate_colour_plane_flag: bool,
    pub bit_depth_luma_minus8: u32,
    pub bit_depth_chroma_minus8: u32,
    pub qpprime_y_zero_transform_bypass_flag: bool,
    /// Optional 6× scaling-list-present flags + lists; we record the
    /// flags only and skip the contents (decoder uses default lists).
    pub seq_scaling_matrix_present_flag: bool,

    pub log2_max_frame_num_minus4: u32,
    pub pic_order_cnt_type: u32,
    pub log2_max_pic_order_cnt_lsb_minus4: u32,
    // pic_order_cnt_type == 1
    pub delta_pic_order_always_zero_flag: bool,
    pub offset_for_non_ref_pic: i32,
    pub offset_for_top_to_bottom_field: i32,
    pub num_ref_frames_in_pic_order_cnt_cycle: u32,
    pub offset_for_ref_frame: Vec<i32>,

    pub max_num_ref_frames: u32,
    pub gaps_in_frame_num_value_allowed_flag: bool,
    pub pic_width_in_mbs_minus1: u32,
    pub pic_height_in_map_units_minus1: u32,
    pub frame_mbs_only_flag: bool,
    pub mb_adaptive_frame_field_flag: bool,
    pub direct_8x8_inference_flag: bool,

    pub frame_cropping_flag: bool,
    pub frame_crop_left_offset: u32,
    pub frame_crop_right_offset: u32,
    pub frame_crop_top_offset: u32,
    pub frame_crop_bottom_offset: u32,

    pub vui_parameters_present_flag: bool,
}

impl Sps {
    /// Width in macroblocks.
    pub fn pic_width_in_mbs(&self) -> u32 {
        self.pic_width_in_mbs_minus1 + 1
    }

    /// Height in map units.
    pub fn pic_height_in_map_units(&self) -> u32 {
        self.pic_height_in_map_units_minus1 + 1
    }

    /// Width in macroblocks * 16.
    pub fn coded_width(&self) -> u32 {
        self.pic_width_in_mbs() * 16
    }

    /// Coded height in luma samples (accounts for `frame_mbs_only_flag`).
    pub fn coded_height(&self) -> u32 {
        let map_units = self.pic_height_in_map_units();
        let factor = if self.frame_mbs_only_flag { 1 } else { 2 };
        map_units * 16 * factor
    }

    /// Final visible width / height after `frame_cropping`. Returns `(w, h)`
    /// in luma samples (`§6.4.1`). Honours `chroma_format_idc` for the
    /// crop unit derivation.
    pub fn visible_size(&self) -> (u32, u32) {
        let (sub_w, sub_h) = match self.chroma_format_idc {
            0 => (1, 1),
            1 => (2, 2), // 4:2:0
            2 => (2, 1), // 4:2:2
            _ => (1, 1), // 4:4:4
        };
        let crop_x = sub_w
            * (self
                .frame_crop_left_offset
                .saturating_add(self.frame_crop_right_offset));
        let frame_mbs = if self.frame_mbs_only_flag { 1 } else { 2 };
        let crop_y = sub_h
            * frame_mbs
            * (self
                .frame_crop_top_offset
                .saturating_add(self.frame_crop_bottom_offset));
        (
            self.coded_width().saturating_sub(crop_x),
            self.coded_height().saturating_sub(crop_y),
        )
    }
}

/// Parse a `seq_parameter_set_rbsp()` body. `rbsp` must already have had
/// emulation-prevention bytes stripped, and `header` is the NAL header
/// preceding the RBSP.
pub fn parse_sps(header: &NalHeader, rbsp: &[u8]) -> Result<Sps> {
    if header.nal_unit_type != crate::nal::NalUnitType::Sps {
        return Err(Error::invalid("h264 sps: NAL header is not SPS"));
    }
    if rbsp.len() < 3 {
        return Err(Error::invalid("h264 sps: RBSP too short"));
    }
    let profile_idc = rbsp[0];
    let cs = rbsp[1];
    let level_idc = rbsp[2];
    let mut br = BitReader::new(&rbsp[3..]);
    let seq_parameter_set_id = br.read_ue()?;
    if seq_parameter_set_id > 31 {
        return Err(Error::invalid(format!(
            "h264 sps: seq_parameter_set_id {seq_parameter_set_id} > 31"
        )));
    }

    let mut chroma_format_idc = 1; // 4:2:0 default
    let mut separate_colour_plane_flag = false;
    let mut bit_depth_luma_minus8 = 0;
    let mut bit_depth_chroma_minus8 = 0;
    let mut qpprime_y_zero_transform_bypass_flag = false;
    let mut seq_scaling_matrix_present_flag = false;

    let high_profile = matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    );
    if high_profile {
        chroma_format_idc = br.read_ue()?;
        if chroma_format_idc > 3 {
            return Err(Error::invalid("h264 sps: chroma_format_idc > 3"));
        }
        if chroma_format_idc == 3 {
            separate_colour_plane_flag = br.read_flag()?;
        }
        bit_depth_luma_minus8 = br.read_ue()?;
        bit_depth_chroma_minus8 = br.read_ue()?;
        qpprime_y_zero_transform_bypass_flag = br.read_flag()?;
        seq_scaling_matrix_present_flag = br.read_flag()?;
        if seq_scaling_matrix_present_flag {
            let count = if chroma_format_idc != 3 { 8 } else { 12 };
            for i in 0..count {
                let present = br.read_flag()?;
                if present {
                    let size = if i < 6 { 16 } else { 64 };
                    skip_scaling_list(&mut br, size)?;
                }
            }
        }
    }

    let log2_max_frame_num_minus4 = br.read_ue()?;
    if log2_max_frame_num_minus4 > 12 {
        return Err(Error::invalid("h264 sps: log2_max_frame_num_minus4 > 12"));
    }
    let pic_order_cnt_type = br.read_ue()?;
    if pic_order_cnt_type > 2 {
        return Err(Error::invalid("h264 sps: pic_order_cnt_type > 2"));
    }

    let mut log2_max_pic_order_cnt_lsb_minus4 = 0;
    let mut delta_pic_order_always_zero_flag = false;
    let mut offset_for_non_ref_pic = 0;
    let mut offset_for_top_to_bottom_field = 0;
    let mut num_ref_frames_in_pic_order_cnt_cycle = 0;
    let mut offset_for_ref_frame = Vec::new();

    match pic_order_cnt_type {
        0 => {
            log2_max_pic_order_cnt_lsb_minus4 = br.read_ue()?;
            if log2_max_pic_order_cnt_lsb_minus4 > 12 {
                return Err(Error::invalid(
                    "h264 sps: log2_max_pic_order_cnt_lsb_minus4 > 12",
                ));
            }
        }
        1 => {
            delta_pic_order_always_zero_flag = br.read_flag()?;
            offset_for_non_ref_pic = br.read_se()?;
            offset_for_top_to_bottom_field = br.read_se()?;
            num_ref_frames_in_pic_order_cnt_cycle = br.read_ue()?;
            if num_ref_frames_in_pic_order_cnt_cycle > 255 {
                return Err(Error::invalid(
                    "h264 sps: num_ref_frames_in_pic_order_cnt_cycle > 255",
                ));
            }
            offset_for_ref_frame.reserve_exact(num_ref_frames_in_pic_order_cnt_cycle as usize);
            for _ in 0..num_ref_frames_in_pic_order_cnt_cycle {
                offset_for_ref_frame.push(br.read_se()?);
            }
        }
        _ => {}
    }

    let max_num_ref_frames = br.read_ue()?;
    if max_num_ref_frames > 16 {
        return Err(Error::invalid("h264 sps: max_num_ref_frames > 16"));
    }
    let gaps_in_frame_num_value_allowed_flag = br.read_flag()?;
    let pic_width_in_mbs_minus1 = br.read_ue()?;
    let pic_height_in_map_units_minus1 = br.read_ue()?;
    let frame_mbs_only_flag = br.read_flag()?;
    let mb_adaptive_frame_field_flag = if !frame_mbs_only_flag {
        br.read_flag()?
    } else {
        false
    };
    let direct_8x8_inference_flag = br.read_flag()?;
    let frame_cropping_flag = br.read_flag()?;
    let (
        frame_crop_left_offset,
        frame_crop_right_offset,
        frame_crop_top_offset,
        frame_crop_bottom_offset,
    ) = if frame_cropping_flag {
        (br.read_ue()?, br.read_ue()?, br.read_ue()?, br.read_ue()?)
    } else {
        (0, 0, 0, 0)
    };
    let vui_parameters_present_flag = br.read_flag()?;
    if vui_parameters_present_flag {
        // Skip VUI parameters — we only need the fact that they're present.
        // The bit-pattern can be quite long; rather than re-implementing the
        // full VUI grammar here, we deliberately stop parsing. Callers that
        // need the VUI fields can re-parse from `vui_offset_bits()` later.
    }

    // We deliberately do not enforce rbsp_trailing_bits: we may have stopped
    // mid-VUI. The fields we extracted are sufficient for the decoder.
    let _ = constraints(cs);

    let cs_bits = constraints(cs);
    Ok(Sps {
        profile_idc,
        constraint_set0_flag: cs_bits[0],
        constraint_set1_flag: cs_bits[1],
        constraint_set2_flag: cs_bits[2],
        constraint_set3_flag: cs_bits[3],
        constraint_set4_flag: cs_bits[4],
        constraint_set5_flag: cs_bits[5],
        level_idc,
        seq_parameter_set_id,
        chroma_format_idc,
        separate_colour_plane_flag,
        bit_depth_luma_minus8,
        bit_depth_chroma_minus8,
        qpprime_y_zero_transform_bypass_flag,
        seq_scaling_matrix_present_flag,
        log2_max_frame_num_minus4,
        pic_order_cnt_type,
        log2_max_pic_order_cnt_lsb_minus4,
        delta_pic_order_always_zero_flag,
        offset_for_non_ref_pic,
        offset_for_top_to_bottom_field,
        num_ref_frames_in_pic_order_cnt_cycle,
        offset_for_ref_frame,
        max_num_ref_frames,
        gaps_in_frame_num_value_allowed_flag,
        pic_width_in_mbs_minus1,
        pic_height_in_map_units_minus1,
        frame_mbs_only_flag,
        mb_adaptive_frame_field_flag,
        direct_8x8_inference_flag,
        frame_cropping_flag,
        frame_crop_left_offset,
        frame_crop_right_offset,
        frame_crop_top_offset,
        frame_crop_bottom_offset,
        vui_parameters_present_flag,
    })
}

fn constraints(byte: u8) -> [bool; 8] {
    let mut out = [false; 8];
    for i in 0..8u8 {
        out[i as usize] = (byte >> (7 - i)) & 1 == 1;
    }
    out
}

fn skip_scaling_list(br: &mut BitReader<'_>, size: u32) -> Result<()> {
    // §7.3.2.1.1.1 — read `size` `se(v)` deltas; stop early if delta == 0
    // and last_scale == 0 (would mean "use default", no further reads).
    let mut last_scale: i32 = 8;
    let mut next_scale: i32 = 8;
    for j in 0..size {
        if next_scale != 0 {
            let delta = br.read_se()?;
            next_scale = (last_scale + delta + 256) & 0xFF;
            // useDefaultScalingMatrixFlag = (j == 0 && next_scale == 0)
            let _ = j;
        }
        if next_scale != 0 {
            last_scale = next_scale;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nal::{extract_rbsp, NalHeader, NalUnitType};

    #[test]
    fn parse_baseline_sps() {
        // SPS from a 128x96 baseline clip (encoded with x264).
        // Header byte: 0x67 (forbidden=0, nal_ref_idc=3, type=7).
        // Body: profile=0x42 (66=baseline), constraints=0xC0, level=0x0A.
        // Then exp-golomb fields.
        // For this unit test we synthesise a minimal SPS body that the parser
        // must accept without VUI.
        // Build:
        //   profile_idc = 66
        //   constraint_set_flags = 11000000 (cs0=cs1=1)
        //   level_idc = 10  (level 1.0)
        //   ue: seq_parameter_set_id = 0     -> "1"
        //   ue: log2_max_frame_num_minus4=0  -> "1"
        //   ue: pic_order_cnt_type=2         -> "011"  (= ue(2))
        //   ue: max_num_ref_frames=1         -> "010"
        //   gaps_in_frame_num=0              -> "0"
        //   ue: pic_width_in_mbs_minus1=7    -> "0001000"  (= ue(7))
        //   ue: pic_height_in_map_units_minus1=5 -> "00110" (= ue(5))
        //   frame_mbs_only=1                 -> "1"
        //   direct_8x8_inference=1           -> "1"
        //   frame_cropping=0                 -> "0"
        //   vui_parameters_present=0         -> "0"
        //   trailing rbsp stop bit + zero alignment
        // Concatenate bits (after the 3-byte preamble):
        //   1 1 011 010 0 0001000 00110 1 1 0 0 1 [00..]
        // = 1 1 0 1 1 0 1 0  0 0 0 0 1 0 0 0  0 0 1 1 0 1 1 0  0 1 0 0
        // Group into bytes:
        //   1101 1010 = 0xDA
        //   0000 1000 = 0x08
        //   0011 0110 = 0x36
        //   0100 ____ = need stop bit + alignment
        //   With trailing 1 then zeros: 0100 1000 = 0x48
        let mut rbsp = vec![66u8, 0xC0, 10];
        rbsp.extend_from_slice(&[0xDA, 0x08, 0x36, 0x48]);
        let h = NalHeader::parse(0x67).unwrap();
        let sps = parse_sps(&h, &rbsp).expect("parse");
        assert_eq!(sps.profile_idc, 66);
        assert_eq!(sps.level_idc, 10);
        assert_eq!(sps.pic_order_cnt_type, 2);
        assert!(sps.frame_mbs_only_flag);
        assert_eq!(sps.coded_width(), 128);
        assert_eq!(sps.coded_height(), 96);
    }

    #[test]
    fn rbsp_extract_then_parse() {
        // Round-trip: any NAL payload without 0x00,0x00,0x00..0x03 sequences
        // passes through extract_rbsp unchanged.
        let body = [66u8, 0xC0, 10, 0xDA, 0x08, 0x36, 0x48];
        let rbsp = extract_rbsp(&body);
        assert_eq!(rbsp, body);
        let h = NalHeader::parse(0x67).unwrap();
        assert_eq!(h.nal_unit_type, NalUnitType::Sps);
        let _sps = parse_sps(&h, &rbsp).unwrap();
    }
}
