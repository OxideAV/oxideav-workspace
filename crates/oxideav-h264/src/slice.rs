//! Slice header parsing — ITU-T H.264 §7.3.3.
//!
//! Covers the fields needed to identify a slice and reach the start of
//! `slice_data()` for a baseline-profile decoder. Reference picture list
//! modification, prediction weight tables, and decoded reference picture
//! marking are parsed enough to skip past their syntax — a baseline I-slice
//! decoder doesn't need their values.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::nal::{NalHeader, NalUnitType};
use crate::pps::Pps;
use crate::sps::Sps;

/// H.264 slice type (§7.4.3 Table 7-6). Includes the `% 5` and the
/// `>= 5` (single-slice-type-per-picture) variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliceType {
    P = 0,
    B = 1,
    I = 2,
    SP = 3,
    SI = 4,
}

impl SliceType {
    pub fn from_raw(t: u32) -> Result<Self> {
        match t % 5 {
            0 => Ok(SliceType::P),
            1 => Ok(SliceType::B),
            2 => Ok(SliceType::I),
            3 => Ok(SliceType::SP),
            4 => Ok(SliceType::SI),
            _ => unreachable!(),
        }
    }
}

/// Parsed slice header (subset).
#[derive(Clone, Debug)]
pub struct SliceHeader {
    pub first_mb_in_slice: u32,
    pub slice_type_raw: u32,
    pub slice_type: SliceType,
    pub pic_parameter_set_id: u32,
    pub colour_plane_id: u8,
    pub frame_num: u32,
    pub field_pic_flag: bool,
    pub bottom_field_flag: bool,
    pub idr_pic_id: u32,
    pub pic_order_cnt_lsb: u32,
    pub delta_pic_order_cnt_bottom: i32,
    pub delta_pic_order_cnt: [i32; 2],
    pub redundant_pic_cnt: u32,
    pub direct_spatial_mv_pred_flag: bool,
    pub num_ref_idx_active_override_flag: bool,
    pub num_ref_idx_l0_active_minus1: u32,
    pub num_ref_idx_l1_active_minus1: u32,
    pub cabac_init_idc: u32,
    pub slice_qp_delta: i32,
    pub sp_for_switch_flag: bool,
    pub slice_qs_delta: i32,
    pub disable_deblocking_filter_idc: u32,
    pub slice_alpha_c0_offset_div2: i32,
    pub slice_beta_offset_div2: i32,
    /// Total bit position at which `slice_data()` begins, relative to the
    /// start of the RBSP.
    pub slice_data_bit_offset: u64,
    /// True if this slice belongs to an IDR picture.
    pub is_idr: bool,
}

/// Parse slice header. The active SPS and PPS must already be available.
pub fn parse_slice_header(
    header: &NalHeader,
    rbsp: &[u8],
    sps: &Sps,
    pps: &Pps,
) -> Result<SliceHeader> {
    if !header.nal_unit_type.is_slice() {
        return Err(Error::invalid("h264 slice: NAL header is not a slice"));
    }
    let is_idr = header.nal_unit_type == NalUnitType::SliceIdr;
    let mut br = BitReader::new(rbsp);
    let first_mb_in_slice = br.read_ue()?;
    let slice_type_raw = br.read_ue()?;
    let slice_type = SliceType::from_raw(slice_type_raw)?;
    let pic_parameter_set_id = br.read_ue()?;
    if pic_parameter_set_id != pps.pic_parameter_set_id {
        return Err(Error::invalid(format!(
            "h264 slice: pic_parameter_set_id={pic_parameter_set_id} doesn't match PPS={}",
            pps.pic_parameter_set_id
        )));
    }
    let colour_plane_id = if sps.separate_colour_plane_flag {
        br.read_u32(2)? as u8
    } else {
        0
    };
    let frame_num_bits = sps.log2_max_frame_num_minus4 + 4;
    let frame_num = br.read_u32(frame_num_bits)?;

    let mut field_pic_flag = false;
    let mut bottom_field_flag = false;
    if !sps.frame_mbs_only_flag {
        field_pic_flag = br.read_flag()?;
        if field_pic_flag {
            bottom_field_flag = br.read_flag()?;
        }
    }

    let idr_pic_id = if is_idr { br.read_ue()? } else { 0 };

    let mut pic_order_cnt_lsb = 0;
    let mut delta_pic_order_cnt_bottom = 0;
    let mut delta_pic_order_cnt = [0i32; 2];
    if sps.pic_order_cnt_type == 0 {
        let bits = sps.log2_max_pic_order_cnt_lsb_minus4 + 4;
        pic_order_cnt_lsb = br.read_u32(bits)?;
        if pps.bottom_field_pic_order_in_frame_present_flag && !field_pic_flag {
            delta_pic_order_cnt_bottom = br.read_se()?;
        }
    } else if sps.pic_order_cnt_type == 1 && !sps.delta_pic_order_always_zero_flag {
        delta_pic_order_cnt[0] = br.read_se()?;
        if pps.bottom_field_pic_order_in_frame_present_flag && !field_pic_flag {
            delta_pic_order_cnt[1] = br.read_se()?;
        }
    }

    let redundant_pic_cnt = if pps.redundant_pic_cnt_present_flag {
        br.read_ue()?
    } else {
        0
    };

    let mut direct_spatial_mv_pred_flag = false;
    let mut num_ref_idx_active_override_flag = false;
    let mut num_ref_idx_l0_active_minus1 = pps.num_ref_idx_l0_default_active_minus1;
    let mut num_ref_idx_l1_active_minus1 = pps.num_ref_idx_l1_default_active_minus1;

    if slice_type == SliceType::B {
        direct_spatial_mv_pred_flag = br.read_flag()?;
    }
    if matches!(slice_type, SliceType::P | SliceType::SP | SliceType::B) {
        num_ref_idx_active_override_flag = br.read_flag()?;
        if num_ref_idx_active_override_flag {
            num_ref_idx_l0_active_minus1 = br.read_ue()?;
            if slice_type == SliceType::B {
                num_ref_idx_l1_active_minus1 = br.read_ue()?;
            }
        }
    }

    // Reference picture list modification (§7.3.3.1) — skip without storing.
    if header.nal_unit_type != NalUnitType::SliceExtension {
        skip_ref_pic_list_modification(&mut br, slice_type)?;
    }

    // Prediction weight table (§7.3.3.2). Only present when weighted prediction
    // is enabled. Baseline rejects this combination, so we just skip.
    let weighted = (matches!(slice_type, SliceType::P | SliceType::SP) && pps.weighted_pred_flag)
        || (slice_type == SliceType::B && pps.weighted_bipred_idc == 1);
    if weighted {
        skip_pred_weight_table(
            &mut br,
            sps.chroma_format_idc,
            num_ref_idx_l0_active_minus1,
            num_ref_idx_l1_active_minus1,
            slice_type == SliceType::B,
        )?;
    }

    // Decoded reference picture marking (§7.3.3.3).
    if header.nal_ref_idc != 0 {
        skip_dec_ref_pic_marking(&mut br, is_idr)?;
    }

    let mut cabac_init_idc = 0;
    if pps.entropy_coding_mode_flag && !matches!(slice_type, SliceType::I | SliceType::SI) {
        cabac_init_idc = br.read_ue()?;
    }

    let slice_qp_delta = br.read_se()?;
    let mut sp_for_switch_flag = false;
    let mut slice_qs_delta = 0;
    if matches!(slice_type, SliceType::SP | SliceType::SI) {
        if slice_type == SliceType::SP {
            sp_for_switch_flag = br.read_flag()?;
        }
        slice_qs_delta = br.read_se()?;
    }

    let mut disable_deblocking_filter_idc = 0;
    let mut slice_alpha_c0_offset_div2 = 0;
    let mut slice_beta_offset_div2 = 0;
    if pps.deblocking_filter_control_present_flag {
        disable_deblocking_filter_idc = br.read_ue()?;
        if disable_deblocking_filter_idc != 1 {
            slice_alpha_c0_offset_div2 = br.read_se()?;
            slice_beta_offset_div2 = br.read_se()?;
        }
    }

    let slice_data_bit_offset = br.bit_position();

    Ok(SliceHeader {
        first_mb_in_slice,
        slice_type_raw,
        slice_type,
        pic_parameter_set_id,
        colour_plane_id,
        frame_num,
        field_pic_flag,
        bottom_field_flag,
        idr_pic_id,
        pic_order_cnt_lsb,
        delta_pic_order_cnt_bottom,
        delta_pic_order_cnt,
        redundant_pic_cnt,
        direct_spatial_mv_pred_flag,
        num_ref_idx_active_override_flag,
        num_ref_idx_l0_active_minus1,
        num_ref_idx_l1_active_minus1,
        cabac_init_idc,
        slice_qp_delta,
        sp_for_switch_flag,
        slice_qs_delta,
        disable_deblocking_filter_idc,
        slice_alpha_c0_offset_div2,
        slice_beta_offset_div2,
        slice_data_bit_offset,
        is_idr,
    })
}

fn skip_ref_pic_list_modification(br: &mut BitReader<'_>, st: SliceType) -> Result<()> {
    let do_l0 = !matches!(st, SliceType::I | SliceType::SI);
    let do_l1 = matches!(st, SliceType::B);
    if do_l0 {
        let flag = br.read_flag()?;
        if flag {
            loop {
                let mod_op = br.read_ue()?;
                if mod_op == 3 {
                    break;
                }
                let _val = br.read_ue()?;
            }
        }
    }
    if do_l1 {
        let flag = br.read_flag()?;
        if flag {
            loop {
                let mod_op = br.read_ue()?;
                if mod_op == 3 {
                    break;
                }
                let _val = br.read_ue()?;
            }
        }
    }
    Ok(())
}

fn skip_pred_weight_table(
    br: &mut BitReader<'_>,
    chroma_format_idc: u32,
    num_l0: u32,
    num_l1: u32,
    has_l1: bool,
) -> Result<()> {
    let _luma_log2_weight_denom = br.read_ue()?;
    let chroma_present = chroma_format_idc != 0;
    if chroma_present {
        let _chroma_log2_weight_denom = br.read_ue()?;
    }
    skip_weight_list(br, num_l0, chroma_present)?;
    if has_l1 {
        skip_weight_list(br, num_l1, chroma_present)?;
    }
    Ok(())
}

fn skip_weight_list(br: &mut BitReader<'_>, num: u32, chroma: bool) -> Result<()> {
    for _ in 0..=num {
        let l = br.read_flag()?;
        if l {
            let _w = br.read_se()?;
            let _o = br.read_se()?;
        }
        if chroma {
            let c = br.read_flag()?;
            if c {
                for _ in 0..2 {
                    let _w = br.read_se()?;
                    let _o = br.read_se()?;
                }
            }
        }
    }
    Ok(())
}

fn skip_dec_ref_pic_marking(br: &mut BitReader<'_>, is_idr: bool) -> Result<()> {
    if is_idr {
        let _no_output_of_prior_pics_flag = br.read_flag()?;
        let _long_term_reference_flag = br.read_flag()?;
    } else {
        let adaptive = br.read_flag()?;
        if adaptive {
            loop {
                let op = br.read_ue()?;
                if op == 0 {
                    break;
                }
                match op {
                    1 | 3 => {
                        let _diff_pic_num_minus1 = br.read_ue()?;
                        if op == 3 {
                            let _long_term_frame_idx = br.read_ue()?;
                        }
                    }
                    2 => {
                        let _long_term_pic_num = br.read_ue()?;
                    }
                    4 => {
                        let _max_long_term_frame_idx_plus1 = br.read_ue()?;
                    }
                    5 => {} // mark all reference pictures as unused
                    6 => {
                        let _long_term_frame_idx = br.read_ue()?;
                    }
                    _ => {
                        return Err(Error::invalid(format!(
                            "h264 slice: bad memory_management_control_operation {op}"
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}
