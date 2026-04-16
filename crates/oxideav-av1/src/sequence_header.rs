//! AV1 sequence header OBU parser — §5.5.
//!
//! The sequence header OBU describes properties that hold for the entire
//! coded video sequence: profile / level, frame dimensions, color
//! configuration, and a long list of feature-enable flags consumed by the
//! frame header parser.
//!
//! Spec references in this file follow the published AV1 Bitstream &
//! Decoding Process Specification (2019-01-08): §5.5.x for the syntax
//! tables and §6.4 for the decoding process.

use oxideav_core::{Error, Result};

use crate::bitreader::{ceil_log2, BitReader};

pub const SELECT_SCREEN_CONTENT_TOOLS: u32 = 2;
pub const SELECT_INTEGER_MV: u32 = 2;

/// Color primaries, transfer characteristics, matrix coefficients per CICP.
/// We only mirror the integer codes — the spec defers semantics to ITU-T H.273.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorConfig {
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub bit_depth: u32,
    pub mono_chrome: bool,
    pub num_planes: u8,
    pub color_range: bool,
    pub subsampling_x: bool,
    pub subsampling_y: bool,
    pub chroma_sample_position: u8,
    pub color_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub separate_uv_deltas: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct TimingInfo {
    pub num_units_in_display_tick: u32,
    pub time_scale: u32,
    pub equal_picture_interval: bool,
    pub num_ticks_per_picture_minus_1: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct DecoderModelInfo {
    pub buffer_delay_length_minus_1: u8,
    pub num_units_in_decoding_tick: u32,
    pub buffer_removal_time_length_minus_1: u8,
    pub frame_presentation_time_length_minus_1: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct OperatingPoint {
    pub idc: u16,
    pub seq_level_idx: u8,
    pub seq_tier: u8,
    pub decoder_model_present: bool,
    pub operating_parameters_info_index: u8,
    pub initial_display_delay_present: bool,
    pub initial_display_delay_minus_1: u8,
}

/// Parsed sequence header.
#[derive(Clone, Debug)]
pub struct SequenceHeader {
    pub seq_profile: u8,
    pub still_picture: bool,
    pub reduced_still_picture_header: bool,

    pub timing_info_present: bool,
    pub timing_info: Option<TimingInfo>,
    pub decoder_model_info_present: bool,
    pub decoder_model_info: Option<DecoderModelInfo>,
    pub initial_display_delay_present: bool,
    pub operating_points_cnt: u8,
    pub operating_points: Vec<OperatingPoint>,

    pub frame_width_bits: u8,
    pub frame_height_bits: u8,
    pub max_frame_width_minus_1: u32,
    pub max_frame_height_minus_1: u32,
    pub max_frame_width: u32,
    pub max_frame_height: u32,

    pub frame_id_numbers_present: bool,
    pub delta_frame_id_length_minus_2: u8,
    pub additional_frame_id_length_minus_1: u8,

    pub use_128x128_superblock: bool,
    pub enable_filter_intra: bool,
    pub enable_intra_edge_filter: bool,
    pub enable_interintra_compound: bool,
    pub enable_masked_compound: bool,
    pub enable_warped_motion: bool,
    pub enable_dual_filter: bool,
    pub enable_order_hint: bool,
    pub enable_jnt_comp: bool,
    pub enable_ref_frame_mvs: bool,

    pub seq_force_screen_content_tools: u32,
    pub seq_force_integer_mv: u32,
    pub order_hint_bits: u32,

    pub enable_superres: bool,
    pub enable_cdef: bool,
    pub enable_restoration: bool,

    pub color_config: ColorConfig,
    pub film_grain_params_present: bool,
}

/// Parse a Sequence Header OBU payload (the bytes after the OBU header /
/// size). Implements §5.5.1.
pub fn parse_sequence_header(payload: &[u8]) -> Result<SequenceHeader> {
    let mut br = BitReader::new(payload);

    let seq_profile = br.f(3)? as u8;
    if seq_profile > 2 {
        return Err(Error::invalid(format!(
            "av1: invalid seq_profile={seq_profile}"
        )));
    }
    let still_picture = br.bit()?;
    let reduced_still_picture_header = br.bit()?;
    if reduced_still_picture_header && !still_picture {
        return Err(Error::invalid(
            "av1: reduced_still_picture_header=1 requires still_picture=1",
        ));
    }

    let mut timing_info_present = false;
    let mut timing_info = None;
    let mut decoder_model_info_present = false;
    let mut decoder_model_info: Option<DecoderModelInfo> = None;
    let mut initial_display_delay_present = false;
    let mut operating_points_cnt = 1u8;
    let mut operating_points: Vec<OperatingPoint> = Vec::new();

    if reduced_still_picture_header {
        // §5.5.1: seq_level_idx_0 (5 bits) only.
        let seq_level_idx = br.f(5)? as u8;
        operating_points.push(OperatingPoint {
            idc: 0,
            seq_level_idx,
            seq_tier: 0,
            decoder_model_present: false,
            operating_parameters_info_index: 0,
            initial_display_delay_present: false,
            initial_display_delay_minus_1: 0,
        });
    } else {
        timing_info_present = br.bit()?;
        if timing_info_present {
            timing_info = Some(parse_timing_info(&mut br)?);
            decoder_model_info_present = br.bit()?;
            if decoder_model_info_present {
                decoder_model_info = Some(parse_decoder_model_info(&mut br)?);
            }
        }
        initial_display_delay_present = br.bit()?;
        let cnt_minus_1 = br.f(5)? as u8;
        operating_points_cnt = cnt_minus_1 + 1;
        for _ in 0..operating_points_cnt {
            let idc = br.f(12)? as u16;
            let seq_level_idx = br.f(5)? as u8;
            let seq_tier = if seq_level_idx > 7 { br.f(1)? as u8 } else { 0 };
            let mut decoder_model_present = false;
            let mut operating_parameters_info_index = 0u8;
            if decoder_model_info_present {
                decoder_model_present = br.bit()?;
                if decoder_model_present {
                    let info = decoder_model_info.expect("decoder_model_info");
                    let n = info.buffer_delay_length_minus_1 as u32 + 1;
                    // operating_parameters_info(): bitrate, buffer_size, low_delay_mode flag
                    let _bitrate_minus_1 = br.f(n)?;
                    let _buffer_size_minus_1 = br.f(n)?;
                    let _low_delay_mode_flag = br.bit()?;
                    operating_parameters_info_index = 0; // not exposed; we just consume
                }
            }
            let mut initial_display_delay_present_for_this_op = false;
            let mut initial_display_delay_minus_1 = 0u8;
            if initial_display_delay_present {
                initial_display_delay_present_for_this_op = br.bit()?;
                if initial_display_delay_present_for_this_op {
                    initial_display_delay_minus_1 = br.f(4)? as u8;
                }
            }
            operating_points.push(OperatingPoint {
                idc,
                seq_level_idx,
                seq_tier,
                decoder_model_present,
                operating_parameters_info_index,
                initial_display_delay_present: initial_display_delay_present_for_this_op,
                initial_display_delay_minus_1,
            });
        }
    }

    let frame_width_bits = (br.f(4)? + 1) as u8;
    let frame_height_bits = (br.f(4)? + 1) as u8;
    let max_frame_width_minus_1 = br.f(frame_width_bits as u32)?;
    let max_frame_height_minus_1 = br.f(frame_height_bits as u32)?;
    let max_frame_width = max_frame_width_minus_1 + 1;
    let max_frame_height = max_frame_height_minus_1 + 1;

    let mut frame_id_numbers_present = false;
    let mut delta_frame_id_length_minus_2 = 0u8;
    let mut additional_frame_id_length_minus_1 = 0u8;
    if !reduced_still_picture_header {
        frame_id_numbers_present = br.bit()?;
        if frame_id_numbers_present {
            delta_frame_id_length_minus_2 = br.f(4)? as u8;
            additional_frame_id_length_minus_1 = br.f(3)? as u8;
        }
    }
    let use_128x128_superblock = br.bit()?;
    let enable_filter_intra = br.bit()?;
    let enable_intra_edge_filter = br.bit()?;

    let mut enable_interintra_compound = false;
    let mut enable_masked_compound = false;
    let mut enable_warped_motion = false;
    let mut enable_dual_filter = false;
    let mut enable_order_hint = false;
    let mut enable_jnt_comp = false;
    let mut enable_ref_frame_mvs = false;
    let mut seq_force_screen_content_tools = SELECT_SCREEN_CONTENT_TOOLS;
    let mut seq_force_integer_mv = SELECT_INTEGER_MV;
    let mut order_hint_bits = 0u32;

    if !reduced_still_picture_header {
        enable_interintra_compound = br.bit()?;
        enable_masked_compound = br.bit()?;
        enable_warped_motion = br.bit()?;
        enable_dual_filter = br.bit()?;
        enable_order_hint = br.bit()?;
        if enable_order_hint {
            enable_jnt_comp = br.bit()?;
            enable_ref_frame_mvs = br.bit()?;
        }
        let seq_choose_screen_content_tools = br.bit()?;
        seq_force_screen_content_tools = if seq_choose_screen_content_tools {
            SELECT_SCREEN_CONTENT_TOOLS
        } else {
            br.f(1)?
        };
        if seq_force_screen_content_tools > 0 {
            let seq_choose_integer_mv = br.bit()?;
            seq_force_integer_mv = if seq_choose_integer_mv {
                SELECT_INTEGER_MV
            } else {
                br.f(1)?
            };
        }
        if enable_order_hint {
            let order_hint_bits_minus_1 = br.f(3)?;
            order_hint_bits = order_hint_bits_minus_1 + 1;
        }
    }

    let enable_superres = br.bit()?;
    let enable_cdef = br.bit()?;
    let enable_restoration = br.bit()?;

    let color_config = parse_color_config(&mut br, seq_profile)?;
    let film_grain_params_present = br.bit()?;

    // Trailing bits — required by spec but not strictly needed for further
    // dispatch. Validate with byte_alignment, ignoring spec-mandated zero
    // padding to be lenient with mux quirks.
    let _ = br.byte_alignment();

    let _ = ceil_log2; // referenced in tile dims later; suppress unused warning
    Ok(SequenceHeader {
        seq_profile,
        still_picture,
        reduced_still_picture_header,
        timing_info_present,
        timing_info,
        decoder_model_info_present,
        decoder_model_info,
        initial_display_delay_present,
        operating_points_cnt,
        operating_points,
        frame_width_bits,
        frame_height_bits,
        max_frame_width_minus_1,
        max_frame_height_minus_1,
        max_frame_width,
        max_frame_height,
        frame_id_numbers_present,
        delta_frame_id_length_minus_2,
        additional_frame_id_length_minus_1,
        use_128x128_superblock,
        enable_filter_intra,
        enable_intra_edge_filter,
        enable_interintra_compound,
        enable_masked_compound,
        enable_warped_motion,
        enable_dual_filter,
        enable_order_hint,
        enable_jnt_comp,
        enable_ref_frame_mvs,
        seq_force_screen_content_tools,
        seq_force_integer_mv,
        order_hint_bits,
        enable_superres,
        enable_cdef,
        enable_restoration,
        color_config,
        film_grain_params_present,
    })
}

fn parse_timing_info(br: &mut BitReader<'_>) -> Result<TimingInfo> {
    let num_units_in_display_tick = br.f(32)?;
    let time_scale = br.f(32)?;
    let equal_picture_interval = br.bit()?;
    let num_ticks_per_picture_minus_1 = if equal_picture_interval {
        br.uvlc()?
    } else {
        0
    };
    Ok(TimingInfo {
        num_units_in_display_tick,
        time_scale,
        equal_picture_interval,
        num_ticks_per_picture_minus_1,
    })
}

fn parse_decoder_model_info(br: &mut BitReader<'_>) -> Result<DecoderModelInfo> {
    let buffer_delay_length_minus_1 = br.f(5)? as u8;
    let num_units_in_decoding_tick = br.f(32)?;
    let buffer_removal_time_length_minus_1 = br.f(5)? as u8;
    let frame_presentation_time_length_minus_1 = br.f(5)? as u8;
    Ok(DecoderModelInfo {
        buffer_delay_length_minus_1,
        num_units_in_decoding_tick,
        buffer_removal_time_length_minus_1,
        frame_presentation_time_length_minus_1,
    })
}

fn parse_color_config(br: &mut BitReader<'_>, seq_profile: u8) -> Result<ColorConfig> {
    let high_bitdepth = br.bit()?;
    let mut twelve_bit = false;
    let bit_depth = if seq_profile == 2 && high_bitdepth {
        twelve_bit = br.bit()?;
        if twelve_bit {
            12
        } else {
            10
        }
    } else if high_bitdepth {
        10
    } else {
        8
    };
    let mono_chrome = if seq_profile == 1 { false } else { br.bit()? };
    let num_planes = if mono_chrome { 1 } else { 3 };
    // color_description_present_flag
    let color_description_present = br.bit()?;
    let (color_primaries, transfer_characteristics, matrix_coefficients) =
        if color_description_present {
            (br.f(8)? as u8, br.f(8)? as u8, br.f(8)? as u8)
        } else {
            (2u8, 2u8, 2u8) // unspecified
        };
    let color_range;
    let mut subsampling_x = false;
    let mut subsampling_y = false;
    let mut chroma_sample_position = 0u8;
    if mono_chrome {
        color_range = br.bit()?;
        subsampling_x = true;
        subsampling_y = true;
    } else if color_primaries == 1 && transfer_characteristics == 13 && matrix_coefficients == 0 {
        // sRGB path: bitdepth must be 8 in profile 1, 10/12 in profile 2.
        color_range = true;
        subsampling_x = false;
        subsampling_y = false;
    } else {
        color_range = br.bit()?;
        match seq_profile {
            0 => {
                subsampling_x = true;
                subsampling_y = true;
            }
            1 => {
                subsampling_x = false;
                subsampling_y = false;
            }
            2 => {
                if bit_depth == 12 {
                    subsampling_x = br.bit()?;
                    subsampling_y = if subsampling_x { br.bit()? } else { false };
                } else {
                    subsampling_x = true;
                    subsampling_y = false;
                }
            }
            _ => {}
        }
        if subsampling_x && subsampling_y {
            chroma_sample_position = br.f(2)? as u8;
        }
    }
    let separate_uv_deltas = if mono_chrome { false } else { br.bit()? };
    Ok(ColorConfig {
        high_bitdepth,
        twelve_bit,
        bit_depth,
        mono_chrome,
        num_planes,
        color_range,
        subsampling_x,
        subsampling_y,
        chroma_sample_position,
        color_primaries,
        transfer_characteristics,
        matrix_coefficients,
        separate_uv_deltas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sequence header from a /tmp/av1.mp4 produced by aomenc at 64x64,
    /// profile 0, 8-bit 4:2:0.
    #[test]
    fn parse_sample_seq_header() {
        // Bytes after the OBU size byte from /tmp/av1.mp4's av1C: 10 bytes.
        let payload: [u8; 10] = [0x00, 0x00, 0x00, 0x02, 0xAF, 0xFF, 0x9B, 0x5F, 0x30, 0x08];
        let sh = parse_sequence_header(&payload).unwrap();
        assert_eq!(sh.seq_profile, 0);
        // 64×64 → max_frame_width=64, max_frame_height=64
        assert_eq!(sh.max_frame_width, 64);
        assert_eq!(sh.max_frame_height, 64);
        assert!(!sh.color_config.mono_chrome);
        assert_eq!(sh.color_config.bit_depth, 8);
        assert_eq!(sh.color_config.num_planes, 3);
    }
}
