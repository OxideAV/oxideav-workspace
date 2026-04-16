//! VP8 frame header — bool-coded section starting after the uncompressed
//! chunk. Covers RFC 6386 §9.2 through §9.10.

use oxideav_core::Result;

use crate::bool_decoder::BoolDecoder;
use crate::frame_tag::FrameType;
use crate::tables::coeff_probs::{CoeffProbs, COEF_UPDATE_PROBS, DEFAULT_COEF_PROBS};
use crate::tables::token_tree::{NUM_BANDS, NUM_CTX, NUM_PROBS, NUM_TYPES};

#[derive(Clone, Debug)]
pub struct SegmentationHeader {
    pub enabled: bool,
    pub update_map: bool,
    pub update_data: bool,
    pub abs_delta: bool,
    pub quant: [i32; 4],
    pub lf: [i32; 4],
    pub tree_probs: [u8; 3],
}

impl Default for SegmentationHeader {
    fn default() -> Self {
        Self {
            enabled: false,
            update_map: false,
            update_data: false,
            abs_delta: false,
            quant: [0; 4],
            lf: [0; 4],
            tree_probs: [255; 3],
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoopFilterHeader {
    /// 0 = normal, 1 = simple.
    pub filter_type: u8,
    pub level: u8,
    pub sharpness: u8,
    pub mode_ref_delta_enabled: bool,
    pub mode_ref_delta_update: bool,
    pub ref_deltas: [i32; 4],
    pub mode_deltas: [i32; 4],
}

impl Default for LoopFilterHeader {
    fn default() -> Self {
        Self {
            filter_type: 0,
            level: 0,
            sharpness: 0,
            mode_ref_delta_enabled: false,
            mode_ref_delta_update: false,
            ref_deltas: [0; 4],
            mode_deltas: [0; 4],
        }
    }
}

#[derive(Clone, Debug)]
pub struct QuantHeader {
    pub y_ac_qi: i32,
    pub y_dc_delta: i32,
    pub y2_dc_delta: i32,
    pub y2_ac_delta: i32,
    pub uv_dc_delta: i32,
    pub uv_ac_delta: i32,
}

impl Default for QuantHeader {
    fn default() -> Self {
        Self {
            y_ac_qi: 0,
            y_dc_delta: 0,
            y2_dc_delta: 0,
            y2_ac_delta: 0,
            uv_dc_delta: 0,
            uv_ac_delta: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FrameHeader {
    pub color_space: u8,
    pub clamping_type: u8,
    pub segmentation: SegmentationHeader,
    pub loop_filter: LoopFilterHeader,
    pub log2_nb_partitions: u8,
    pub quant: QuantHeader,
    pub refresh_entropy: bool,
    pub coef_probs: CoeffProbs,
    pub mb_skip_enabled: bool,
    pub mb_skip_prob: u8,
}

/// Parse a key-frame compressed header. The `BoolDecoder` is positioned
/// at the first compressed byte (after the 7-byte uncompressed chunk
/// for keyframes). Probabilities are reset to defaults at the start
/// of every keyframe.
pub fn parse_keyframe_header(d: &mut BoolDecoder<'_>) -> Result<FrameHeader> {
    let color_space = d.read_literal(1) as u8;
    let clamping_type = d.read_literal(1) as u8;

    let segmentation = parse_segmentation(d, FrameType::Key)?;
    let loop_filter = parse_loop_filter(d)?;
    let log2_nb_partitions = d.read_literal(2) as u8;
    let quant = parse_quant(d)?;
    // Keyframe — refresh_entropy_probs only: no refresh_last/refresh_golden flags.
    let refresh_entropy = d.read_bool(128);
    let mut coef_probs = DEFAULT_COEF_PROBS;
    update_coef_probs(d, &mut coef_probs);

    let mb_skip_enabled = d.read_bool(128);
    let mb_skip_prob = if mb_skip_enabled {
        d.read_literal(8) as u8
    } else {
        255
    };

    Ok(FrameHeader {
        color_space,
        clamping_type,
        segmentation,
        loop_filter,
        log2_nb_partitions,
        quant,
        refresh_entropy,
        coef_probs,
        mb_skip_enabled,
        mb_skip_prob,
    })
}

fn parse_segmentation(d: &mut BoolDecoder<'_>, _ft: FrameType) -> Result<SegmentationHeader> {
    let mut s = SegmentationHeader::default();
    s.enabled = d.read_bool(128);
    if !s.enabled {
        return Ok(s);
    }
    s.update_map = d.read_bool(128);
    s.update_data = d.read_bool(128);
    if s.update_data {
        s.abs_delta = d.read_bool(128);
        for i in 0..4 {
            if d.read_bool(128) {
                s.quant[i] = d.read_signed_literal(7);
            }
        }
        for i in 0..4 {
            if d.read_bool(128) {
                s.lf[i] = d.read_signed_literal(6);
            }
        }
    }
    if s.update_map {
        for i in 0..3 {
            if d.read_bool(128) {
                s.tree_probs[i] = d.read_literal(8) as u8;
            } else {
                s.tree_probs[i] = 255;
            }
        }
    }
    Ok(s)
}

fn parse_loop_filter(d: &mut BoolDecoder<'_>) -> Result<LoopFilterHeader> {
    let mut lf = LoopFilterHeader::default();
    lf.filter_type = d.read_literal(1) as u8;
    lf.level = d.read_literal(6) as u8;
    lf.sharpness = d.read_literal(3) as u8;
    lf.mode_ref_delta_enabled = d.read_bool(128);
    if lf.mode_ref_delta_enabled {
        lf.mode_ref_delta_update = d.read_bool(128);
        if lf.mode_ref_delta_update {
            for i in 0..4 {
                if d.read_bool(128) {
                    lf.ref_deltas[i] = d.read_signed_literal(6);
                }
            }
            for i in 0..4 {
                if d.read_bool(128) {
                    lf.mode_deltas[i] = d.read_signed_literal(6);
                }
            }
        }
    }
    Ok(lf)
}

fn parse_quant(d: &mut BoolDecoder<'_>) -> Result<QuantHeader> {
    let mut q = QuantHeader::default();
    q.y_ac_qi = d.read_literal(7) as i32;
    q.y_dc_delta = read_delta(d);
    q.y2_dc_delta = read_delta(d);
    q.y2_ac_delta = read_delta(d);
    q.uv_dc_delta = read_delta(d);
    q.uv_ac_delta = read_delta(d);
    Ok(q)
}

fn read_delta(d: &mut BoolDecoder<'_>) -> i32 {
    if d.read_bool(128) {
        d.read_signed_literal(4)
    } else {
        0
    }
}

fn update_coef_probs(d: &mut BoolDecoder<'_>, probs: &mut CoeffProbs) {
    for i in 0..NUM_TYPES {
        for j in 0..NUM_BANDS {
            for k in 0..NUM_CTX {
                for l in 0..NUM_PROBS {
                    let upd = COEF_UPDATE_PROBS[i][j][k][l] as u32;
                    if d.read_bool(upd) {
                        probs[i][j][k][l] = d.read_literal(8) as u8;
                    }
                }
            }
        }
    }
}
