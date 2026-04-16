//! AV1 frame header OBU parser — §5.9.
//!
//! This is the most syntactically heavy header in AV1. We parse the leading
//! fields exhaustively (enough to dispatch frame type, recover frame size,
//! and emit codec parameters) and then stop at `tile_info()` — the
//! quantizer, segmentation, loop filter, CDEF, restoration and global motion
//! sections are skimmed only as far as needed for downstream tile-group
//! payload framing. The latter sub-syntax is gated on flags from the
//! sequence header and reference state, which we don't carry across frames
//! in this initial parse-only crate.
//!
//! For undecoded sub-sections we deliberately stop at a clearly-named
//! boundary so the caller can tell where the parser gave up.

use oxideav_core::{Error, Result};

use crate::bitreader::{ceil_log2, BitReader};
use crate::sequence_header::{SequenceHeader, SELECT_INTEGER_MV, SELECT_SCREEN_CONTENT_TOOLS};

pub const NUM_REF_FRAMES: usize = 8;
pub const REFS_PER_FRAME: usize = 7;
pub const SUPERRES_DENOM_BITS: u32 = 3;
pub const SUPERRES_DENOM_MIN: u32 = 9;
pub const SUPERRES_NUM: u32 = 8;

/// `frame_type` values §5.9.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameType {
    Key,
    Inter,
    IntraOnly,
    Switch,
}

impl FrameType {
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Key,
            1 => Self::Inter,
            2 => Self::IntraOnly,
            _ => Self::Switch,
        }
    }
}

/// Top-level uncompressed header. We expose the fields the caller is most
/// likely to need (frame type / dimensions / show flag) plus a `parse_depth`
/// marker indicating how far the parser successfully advanced.
#[derive(Clone, Debug)]
pub struct FrameHeader {
    pub show_existing_frame: bool,
    pub frame_to_show_map_idx: u8,
    pub display_frame_id: u32,

    pub frame_type: FrameType,
    pub show_frame: bool,
    pub showable_frame: bool,
    pub error_resilient_mode: bool,

    pub disable_cdf_update: bool,
    pub allow_screen_content_tools: u32,
    pub force_integer_mv: u32,

    pub current_frame_id: u32,
    pub frame_size_override_flag: bool,

    pub order_hint: u32,
    pub primary_ref_frame: u32,

    pub refresh_frame_flags: u8,
    pub ref_order_hint: [u32; NUM_REF_FRAMES],
    pub ref_frame_idx: [u32; REFS_PER_FRAME],

    pub frame_width: u32,
    pub frame_height: u32,
    pub upscaled_width: u32,
    pub use_superres: bool,
    pub superres_denom: u32,

    pub render_and_frame_size_different: bool,
    pub render_width: u32,
    pub render_height: u32,

    pub allow_intrabc: bool,
    pub allow_high_precision_mv: bool,
    pub is_filter_switchable: bool,
    pub interpolation_filter: u32,
    pub is_motion_mode_switchable: bool,
    pub use_ref_frame_mvs: bool,
    pub disable_frame_end_update_cdf: bool,
    pub allow_warped_motion: bool,
    pub reduced_tx_set: bool,

    /// Last successfully-parsed milestone — see `ParseDepth`.
    pub parse_depth: ParseDepth,
}

/// How far the frame_header parser advanced before yielding back to the
/// caller. Useful so a high-level decoder knows whether to treat the
/// remaining bytes as opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseDepth {
    /// Stopped right after `show_existing_frame` short path.
    ShowExistingFrame,
    /// Fully parsed up to (but not including) `tile_info()`.
    UpToTileInfo,
}

/// Parse a frame_header_obu / frame_obu payload. For OBU_FRAME the caller
/// must subsequently parse `tile_group_obu` from the remaining bytes.
pub fn parse_frame_header(seq: &SequenceHeader, payload: &[u8]) -> Result<FrameHeader> {
    let mut br = BitReader::new(payload);
    parse_uncompressed_header(seq, &mut br)
}

/// §5.9.1 uncompressed_header().
fn parse_uncompressed_header(seq: &SequenceHeader, br: &mut BitReader<'_>) -> Result<FrameHeader> {
    let id_len = if seq.frame_id_numbers_present {
        seq.additional_frame_id_length_minus_1 as u32 + seq.delta_frame_id_length_minus_2 as u32 + 3
    } else {
        0
    };
    let all_frames = (1u32 << NUM_REF_FRAMES) - 1;

    if seq.reduced_still_picture_header {
        return finish_minimal(
            false,
            0,
            0,
            FrameType::Key,
            true,
            true,
            true,
            ParseDepth::ShowExistingFrame,
        );
    }

    let show_existing_frame = br.bit()?;
    if show_existing_frame {
        let frame_to_show_map_idx = br.f(3)? as u8;
        if let Some(info) = seq.decoder_model_info {
            if seq.decoder_model_info_present
                && seq
                    .timing_info
                    .map(|t| !t.equal_picture_interval)
                    .unwrap_or(true)
            {
                let _ = br.f(info.frame_presentation_time_length_minus_1 as u32 + 1)?;
            }
        }
        let display_frame_id = if seq.frame_id_numbers_present {
            br.f(id_len)?
        } else {
            0
        };
        // Remaining state is taken from RefFrame[frame_to_show_map_idx] which we
        // don't track in this initial parse-only build. Stop here.
        return finish_minimal(
            true,
            frame_to_show_map_idx,
            display_frame_id,
            FrameType::Key, // unknown; placeholder
            true,
            true,
            false,
            ParseDepth::ShowExistingFrame,
        );
    }

    let frame_type = FrameType::from_u32(br.f(2)?);
    let show_frame = br.bit()?;
    if show_frame
        && seq.decoder_model_info_present
        && seq
            .timing_info
            .map(|t| !t.equal_picture_interval)
            .unwrap_or(true)
    {
        if let Some(info) = seq.decoder_model_info {
            let _ = br.f(info.frame_presentation_time_length_minus_1 as u32 + 1)?;
        }
    }
    let showable_frame = if show_frame {
        frame_type != FrameType::Key
    } else {
        br.bit()?
    };
    let error_resilient_mode =
        if frame_type == FrameType::Switch || (frame_type == FrameType::Key && show_frame) {
            true
        } else {
            br.bit()?
        };

    let disable_cdf_update = br.bit()?;
    let allow_screen_content_tools =
        if seq.seq_force_screen_content_tools == SELECT_SCREEN_CONTENT_TOOLS {
            br.f(1)?
        } else {
            seq.seq_force_screen_content_tools
        };
    let force_integer_mv = if allow_screen_content_tools != 0 {
        if seq.seq_force_integer_mv == SELECT_INTEGER_MV {
            br.f(1)?
        } else {
            seq.seq_force_integer_mv
        }
    } else {
        // Per §5.9.1: if frame_type intra-only/key force_integer_mv is 1
        match frame_type {
            FrameType::Key | FrameType::IntraOnly => 1,
            _ => 0,
        }
    };
    let mut current_frame_id = 0u32;
    if seq.frame_id_numbers_present {
        current_frame_id = br.f(id_len)?;
    }
    let frame_size_override_flag = if frame_type == FrameType::Switch {
        true
    } else if seq.reduced_still_picture_header {
        false
    } else {
        br.bit()?
    };
    let order_hint = if seq.enable_order_hint {
        br.f(seq.order_hint_bits)?
    } else {
        0
    };
    let primary_ref_frame = if frame_type == FrameType::Key
        || frame_type == FrameType::IntraOnly
        || error_resilient_mode
    {
        7 // PRIMARY_REF_NONE
    } else {
        br.f(3)?
    };

    // Decoder model buffer-removal-time (§5.9.4): we skim the structure but
    // do not retain values.
    if seq.decoder_model_info_present {
        let buffer_removal_time_present_flag = br.bit()?;
        if buffer_removal_time_present_flag {
            // For each operating point: if op idc and op_pt_idc test passes...
            for op in &seq.operating_points {
                if op.decoder_model_present {
                    if let Some(info) = seq.decoder_model_info {
                        let _t = br.f(info.buffer_removal_time_length_minus_1 as u32 + 1)?;
                    }
                }
            }
        }
    }

    let refresh_frame_flags = if frame_type == FrameType::Key && show_frame {
        all_frames as u8
    } else {
        br.f(8)? as u8
    };

    let mut ref_order_hint = [0u32; NUM_REF_FRAMES];
    if (frame_type != FrameType::Key || !show_frame)
        && seq.enable_order_hint
        && (error_resilient_mode || frame_type != FrameType::Key)
    {
        for v in ref_order_hint.iter_mut() {
            *v = br.f(seq.order_hint_bits)?;
        }
    }

    // ---- Frame size + render size ----
    let (frame_width, frame_height, upscaled_width, use_superres, superres_denom) = if frame_type
        == FrameType::Key
        || frame_type == FrameType::IntraOnly
    {
        let (fw, fh) = parse_frame_size(seq, br, frame_size_override_flag)?;
        let (sup, denom, upscaled) = parse_superres(seq, br, fw)?;
        (sup, fh, upscaled, denom != SUPERRES_NUM, denom)
    } else if !frame_size_override_flag {
        // Use sequence-level max dims (no per-frame override).
        let (fw, fh) = (seq.max_frame_width, seq.max_frame_height);
        let (sup, denom, upscaled) = parse_superres(seq, br, fw)?;
        (sup, fh, upscaled, denom != SUPERRES_NUM, denom)
    } else {
        // frame_size_with_refs() — uses reference frame state we don't have
        // yet. Stop here, returning what we have so far.
        return Err(Error::unsupported(
                "av1 frame_size_with_refs (§5.9.6) requires reference state — parse-only crate stops here",
            ));
    };
    let (render_and_frame_size_different, render_width, render_height) =
        parse_render_size(br, frame_width, frame_height)?;

    // Align: from §5.9.5: allow_intrabc only when intra_only/key + allow_screen_content_tools
    let allow_intrabc = if (frame_type == FrameType::Key || frame_type == FrameType::IntraOnly)
        && allow_screen_content_tools != 0
        && (frame_width == upscaled_width)
    {
        br.bit()?
    } else {
        false
    };

    // ref_frame stuff for inter frames
    let mut ref_frame_idx = [0u32; REFS_PER_FRAME];
    let mut allow_high_precision_mv = false;
    let mut is_filter_switchable = false;
    let mut interpolation_filter = 0u32;
    let mut is_motion_mode_switchable = false;
    let mut use_ref_frame_mvs = false;
    if frame_type == FrameType::Inter || frame_type == FrameType::Switch {
        // frame_refs_short_signaling
        let frame_refs_short_signaling = if seq.enable_order_hint {
            br.bit()?
        } else {
            false
        };
        if frame_refs_short_signaling {
            let _last = br.f(3)?;
            let _golden = br.f(3)?;
            // Real implementation runs set_frame_refs() to fill the rest;
            // we don't emulate it.
        }
        for v in ref_frame_idx.iter_mut() {
            if !frame_refs_short_signaling {
                *v = br.f(3)?;
            }
            if seq.frame_id_numbers_present {
                let n = seq.delta_frame_id_length_minus_2 as u32 + 2;
                let _delta_frame_id_minus_1 = br.f(n)?;
            }
        }
        // skip allow_high_precision_mv-related branch fields
        allow_high_precision_mv = if force_integer_mv != 0 {
            false
        } else {
            br.bit()?
        };
        is_filter_switchable = br.bit()?;
        interpolation_filter = if is_filter_switchable { 4 } else { br.f(2)? };
        is_motion_mode_switchable = br.bit()?;
        use_ref_frame_mvs = if error_resilient_mode || !seq.enable_ref_frame_mvs {
            false
        } else {
            br.bit()?
        };
    }

    let disable_frame_end_update_cdf = if seq.reduced_still_picture_header || disable_cdf_update {
        true
    } else {
        br.bit()?
    };

    // We deliberately stop here. The remaining sub-sections (tile_info, quant,
    // segmentation, deblock, cdef, lr, tx_mode, frame_reference_mode,
    // skip_mode_params, global_motion_params, film_grain_params) are
    // out of scope for parse-only.
    let allow_warped_motion = false;
    let reduced_tx_set = false;

    Ok(FrameHeader {
        show_existing_frame: false,
        frame_to_show_map_idx: 0,
        display_frame_id: 0,
        frame_type,
        show_frame,
        showable_frame,
        error_resilient_mode,
        disable_cdf_update,
        allow_screen_content_tools,
        force_integer_mv,
        current_frame_id,
        frame_size_override_flag,
        order_hint,
        primary_ref_frame,
        refresh_frame_flags,
        ref_order_hint,
        ref_frame_idx,
        frame_width,
        frame_height,
        upscaled_width,
        use_superres,
        superres_denom,
        render_and_frame_size_different,
        render_width,
        render_height,
        allow_intrabc,
        allow_high_precision_mv,
        is_filter_switchable,
        interpolation_filter,
        is_motion_mode_switchable,
        use_ref_frame_mvs,
        disable_frame_end_update_cdf,
        allow_warped_motion,
        reduced_tx_set,
        parse_depth: ParseDepth::UpToTileInfo,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_minimal(
    show_existing_frame: bool,
    frame_to_show_map_idx: u8,
    display_frame_id: u32,
    frame_type: FrameType,
    show_frame: bool,
    showable_frame: bool,
    error_resilient_mode: bool,
    parse_depth: ParseDepth,
) -> Result<FrameHeader> {
    Ok(FrameHeader {
        show_existing_frame,
        frame_to_show_map_idx,
        display_frame_id,
        frame_type,
        show_frame,
        showable_frame,
        error_resilient_mode,
        disable_cdf_update: false,
        allow_screen_content_tools: 0,
        force_integer_mv: 0,
        current_frame_id: 0,
        frame_size_override_flag: false,
        order_hint: 0,
        primary_ref_frame: 7,
        refresh_frame_flags: 0,
        ref_order_hint: [0; NUM_REF_FRAMES],
        ref_frame_idx: [0; REFS_PER_FRAME],
        frame_width: 0,
        frame_height: 0,
        upscaled_width: 0,
        use_superres: false,
        superres_denom: SUPERRES_NUM,
        render_and_frame_size_different: false,
        render_width: 0,
        render_height: 0,
        allow_intrabc: false,
        allow_high_precision_mv: false,
        is_filter_switchable: false,
        interpolation_filter: 0,
        is_motion_mode_switchable: false,
        use_ref_frame_mvs: false,
        disable_frame_end_update_cdf: true,
        allow_warped_motion: false,
        reduced_tx_set: false,
        parse_depth,
    })
}

fn parse_frame_size(
    seq: &SequenceHeader,
    br: &mut BitReader<'_>,
    frame_size_override_flag: bool,
) -> Result<(u32, u32)> {
    if frame_size_override_flag {
        let w = br.f(seq.frame_width_bits as u32)? + 1;
        let h = br.f(seq.frame_height_bits as u32)? + 1;
        Ok((w, h))
    } else {
        Ok((seq.max_frame_width, seq.max_frame_height))
    }
}

/// Returns (frame_width, superres_denom, upscaled_width). When superres is
/// disabled the frame width is unchanged and the denom is 8 (== SUPERRES_NUM).
fn parse_superres(
    seq: &SequenceHeader,
    br: &mut BitReader<'_>,
    upscaled_width: u32,
) -> Result<(u32, u32, u32)> {
    let use_superres = if seq.enable_superres {
        br.bit()?
    } else {
        false
    };
    let denom = if use_superres {
        let coded = br.f(SUPERRES_DENOM_BITS)?;
        coded + SUPERRES_DENOM_MIN
    } else {
        SUPERRES_NUM
    };
    // FrameWidth = (UpscaledWidth * SUPERRES_NUM + (SuperresDenom/2)) / SuperresDenom
    let frame_width = (upscaled_width * SUPERRES_NUM + denom / 2) / denom;
    Ok((frame_width, denom, upscaled_width))
}

fn parse_render_size(
    br: &mut BitReader<'_>,
    frame_width: u32,
    frame_height: u32,
) -> Result<(bool, u32, u32)> {
    let different = br.bit()?;
    if different {
        let rw = br.f(16)? + 1;
        let rh = br.f(16)? + 1;
        Ok((true, rw, rh))
    } else {
        Ok((false, frame_width, frame_height))
    }
}

#[allow(dead_code)]
pub(crate) fn _bit_field_width_for(seq: &SequenceHeader) -> u32 {
    ceil_log2(seq.max_frame_width)
}
