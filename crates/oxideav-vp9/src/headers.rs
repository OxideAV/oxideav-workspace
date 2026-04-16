//! VP9 uncompressed header parser.
//!
//! Reference: VP9 Bitstream & Decoding Process Specification, version 0.7
//! (2017).
//!
//! Parses §6.2 (`uncompressed_header()`) into a [`UncompressedHeader`]
//! struct, plus the trailing `header_size` (§6.2 last 16 bits) used to
//! locate the compressed header / first tile partition.
//!
//! Subsections covered:
//! * §6.2 frame_marker (must be 2), profile (0..3), show_existing_frame.
//! * §6.2 frame_type, show_frame, error_resilient_mode.
//! * §6.2.1 color_config — bit_depth, color_space, color_range,
//!   subsampling.
//! * §6.2.2 frame_size, render_size, frame_size_with_refs.
//! * §6.2.3 loop_filter_params.
//! * §6.2.4 quantization_params (§7.2.4 dequant tables — values stored
//!   raw, not yet expanded to dequant coefficients).
//! * §6.2.5 segmentation_params.
//! * §6.2.6 tile_info.
//! * §6.2 trailing `header_size` (16-bit length of the compressed header).
//!
//! Decoded but not yet acted upon: ref_frame slots, interpolation_filter,
//! delta probabilities. The struct is public and stable; downstream code
//! can build on it.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;

/// VP9 frame type — §6.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameType {
    /// `KEY_FRAME` — independently decodable.
    Key,
    /// `NON_KEY_FRAME` — depends on prior frames.
    NonKey,
}

/// VP9 reference frame indices — §3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefFrame {
    Intra = 0,
    Last = 1,
    Golden = 2,
    Altref = 3,
}

/// VP9 color space — §6.2.1 (`color_space` 3-bit field).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Unknown = 0,
    Bt601 = 1,
    Bt709 = 2,
    Smpte170 = 3,
    Smpte240 = 4,
    Bt2020 = 5,
    Reserved2 = 6,
    Srgb = 7,
}

impl ColorSpace {
    fn from_bits(v: u32) -> Self {
        match v {
            0 => ColorSpace::Unknown,
            1 => ColorSpace::Bt601,
            2 => ColorSpace::Bt709,
            3 => ColorSpace::Smpte170,
            4 => ColorSpace::Smpte240,
            5 => ColorSpace::Bt2020,
            6 => ColorSpace::Reserved2,
            _ => ColorSpace::Srgb,
        }
    }
}

/// `color_config` from §6.2.1.
#[derive(Clone, Copy, Debug)]
pub struct ColorConfig {
    /// Bit depth — 8, 10, or 12.
    pub bit_depth: u8,
    pub color_space: ColorSpace,
    pub color_range: bool,
    /// Horizontal chroma subsampling (true = 1, false = 0).
    pub subsampling_x: bool,
    /// Vertical chroma subsampling.
    pub subsampling_y: bool,
}

/// `loop_filter_params` from §6.2.3 — raw bitstream fields.
#[derive(Clone, Copy, Debug, Default)]
pub struct LoopFilterParams {
    pub level: u8,
    pub sharpness: u8,
    pub mode_ref_delta_enabled: bool,
    pub mode_ref_delta_update: bool,
    pub ref_deltas: [i8; 4],
    pub mode_deltas: [i8; 2],
}

/// `quantization_params` from §6.2.4.
#[derive(Clone, Copy, Debug, Default)]
pub struct QuantizationParams {
    pub base_q_idx: u8,
    pub delta_q_y_dc: i8,
    pub delta_q_uv_dc: i8,
    pub delta_q_uv_ac: i8,
    /// Lossless mode — derived: `base_q_idx == 0` and all deltas zero.
    pub lossless: bool,
}

/// `segmentation_params` from §6.2.5 — raw fields.
#[derive(Clone, Copy, Debug, Default)]
pub struct SegmentationParams {
    pub enabled: bool,
    pub update_map: bool,
    pub temporal_update: bool,
    pub update_data: bool,
    pub abs_delta: bool,
    pub feature_data: [[i16; 4]; 8],
    pub feature_enabled: [[bool; 4]; 8],
}

/// `tile_info` from §6.2.6.
#[derive(Clone, Copy, Debug, Default)]
pub struct TileInfo {
    pub log2_tile_cols: u8,
    pub log2_tile_rows: u8,
}

/// One full uncompressed header.
#[derive(Clone, Debug)]
pub struct UncompressedHeader {
    pub profile: u8,
    pub show_existing_frame: bool,
    pub existing_frame_to_show: u8,
    pub frame_type: FrameType,
    pub show_frame: bool,
    pub error_resilient_mode: bool,
    /// Set when an intra_only frame appears mid-stream.
    pub intra_only: bool,
    pub reset_frame_context: u8,
    pub color_config: ColorConfig,
    pub width: u32,
    pub height: u32,
    pub render_width: Option<u32>,
    pub render_height: Option<u32>,
    /// 0..=2, indexes the ref-frame buffer to refresh per slot.
    pub refresh_frame_flags: u8,
    /// Each entry is the index into the ref buffer for LAST/GOLDEN/ALTREF.
    pub ref_frame_idx: [u8; 3],
    /// Sign-bias bits for the three references.
    pub ref_frame_sign_bias: [bool; 4],
    pub allow_high_precision_mv: bool,
    /// 0..=4 — one of {EIGHTTAP, EIGHTTAP_SMOOTH, EIGHTTAP_SHARP, BILINEAR,
    /// SWITCHABLE} per §7.3.7.
    pub interpolation_filter: u8,
    pub refresh_frame_context: bool,
    pub frame_parallel_decoding_mode: bool,
    pub frame_context_idx: u8,
    pub loop_filter: LoopFilterParams,
    pub quantization: QuantizationParams,
    pub segmentation: SegmentationParams,
    pub tile_info: TileInfo,
    /// `header_size` — length of the compressed header in bytes (§6.2).
    pub header_size: u16,
    /// Byte offset (within the frame data) at which the compressed header
    /// starts. Equal to the byte-aligned position right after the
    /// uncompressed header.
    pub uncompressed_header_size: usize,
}

const VP9_SYNC_CODE: u32 = 0x49_8342;

/// Parse a VP9 uncompressed header from `frame`. Returns the header plus
/// the offset (in bytes) to the start of the compressed header.
///
/// `prev_color_config` is required for non-key, non-intra-only frames: VP9
/// inherits the color config from the most recent key/intra-only frame.
pub fn parse_uncompressed_header(
    frame: &[u8],
    prev_color_config: Option<ColorConfig>,
) -> Result<UncompressedHeader> {
    let mut br = BitReader::new(frame);
    let frame_marker = br.f(2)?;
    if frame_marker != 2 {
        return Err(Error::invalid(format!(
            "vp9 §6.2: frame_marker must be 2, got {frame_marker}"
        )));
    }
    let profile_low = br.f(1)?;
    let profile_high = br.f(1)?;
    let mut profile = (profile_high << 1) | profile_low;
    if profile == 3 {
        // §6.2: profile==3 has an extra reserved zero bit.
        let reserved = br.f(1)?;
        if reserved != 0 {
            return Err(Error::invalid("vp9 §6.2: profile-3 reserved bit must be 0"));
        }
        profile = 3;
    }

    let show_existing_frame = br.bit()?;
    let mut existing_frame_to_show = 0u8;
    if show_existing_frame {
        existing_frame_to_show = br.f(3)? as u8;
        // The remainder of the bitstream for this frame is the existing-frame
        // dispatch — there is no further parse work.
        return Ok(UncompressedHeader {
            profile: profile as u8,
            show_existing_frame: true,
            existing_frame_to_show,
            frame_type: FrameType::NonKey,
            show_frame: true,
            error_resilient_mode: false,
            intra_only: false,
            reset_frame_context: 0,
            color_config: prev_color_config.unwrap_or(ColorConfig {
                bit_depth: 8,
                color_space: ColorSpace::Unknown,
                color_range: false,
                subsampling_x: true,
                subsampling_y: true,
            }),
            width: 0,
            height: 0,
            render_width: None,
            render_height: None,
            refresh_frame_flags: 0,
            ref_frame_idx: [0; 3],
            ref_frame_sign_bias: [false; 4],
            allow_high_precision_mv: false,
            interpolation_filter: 0,
            refresh_frame_context: false,
            frame_parallel_decoding_mode: false,
            frame_context_idx: 0,
            loop_filter: LoopFilterParams::default(),
            quantization: QuantizationParams::default(),
            segmentation: SegmentationParams::default(),
            tile_info: TileInfo::default(),
            header_size: 0,
            uncompressed_header_size: br.byte_aligned_position(),
        });
    }

    let frame_type_bit = br.f(1)?;
    let frame_type = if frame_type_bit == 0 {
        FrameType::Key
    } else {
        FrameType::NonKey
    };
    let show_frame = br.bit()?;
    let error_resilient_mode = br.bit()?;

    let mut intra_only = false;
    let mut reset_frame_context = 0u8;
    let refresh_frame_flags: u8;
    let mut ref_frame_idx = [0u8; 3];
    let mut ref_frame_sign_bias = [false; 4];
    let mut allow_high_precision_mv = false;
    let mut interpolation_filter = 0u8;
    let width: u32;
    let height: u32;
    let render_width: Option<u32>;
    let render_height: Option<u32>;
    let color_config: ColorConfig;

    if frame_type == FrameType::Key {
        let sc = br.f(24)?;
        if sc != VP9_SYNC_CODE {
            return Err(Error::invalid(format!(
                "vp9 §6.2: bad sync code 0x{sc:06x} (want 0x498342)"
            )));
        }
        color_config = parse_color_config(&mut br, profile as u8)?;
        let (w, h, rw, rh) = parse_frame_size_and_render(&mut br)?;
        width = w;
        height = h;
        render_width = rw;
        render_height = rh;
        refresh_frame_flags = 0xFF;
    } else {
        intra_only = if show_frame { false } else { br.bit()? };
        if !error_resilient_mode {
            reset_frame_context = br.f(2)? as u8;
        }
        if intra_only {
            let sc = br.f(24)?;
            if sc != VP9_SYNC_CODE {
                return Err(Error::invalid(format!(
                    "vp9 §6.2 intra_only: bad sync code 0x{sc:06x}"
                )));
            }
            color_config = if profile > 0 {
                parse_color_config(&mut br, profile as u8)?
            } else {
                // Profile 0: spec §6.2.1 says color_space etc. are inferred.
                ColorConfig {
                    bit_depth: 8,
                    color_space: ColorSpace::Bt601,
                    color_range: false,
                    subsampling_x: true,
                    subsampling_y: true,
                }
            };
            refresh_frame_flags = br.f(8)? as u8;
            let (w, h, rw, rh) = parse_frame_size_and_render(&mut br)?;
            width = w;
            height = h;
            render_width = rw;
            render_height = rh;
        } else {
            color_config = prev_color_config.ok_or_else(|| {
                Error::invalid("vp9 §6.2: non-intra non-key frame requires prior color_config")
            })?;
            refresh_frame_flags = br.f(8)? as u8;
            // §6.2: for i in 0..3: ref_frame_idx[i] = f(3);
            //                       ref_frame_sign_bias[LAST_FRAME + i] = f(1).
            // ref_frame_sign_bias is indexed by RefFrame; slot i maps to
            // RefFrame::Last + i (i.e. indices 1, 2, 3).
            for i in 0..3 {
                ref_frame_idx[i] = br.f(3)? as u8;
                ref_frame_sign_bias[1 + i] = br.bit()?;
            }
            let (w, h, rw, rh) = parse_frame_size_with_refs(&mut br)?;
            width = w;
            height = h;
            render_width = rw;
            render_height = rh;
            allow_high_precision_mv = br.bit()?;
            interpolation_filter = read_interpolation_filter(&mut br)?;
        }
    }

    let (refresh_frame_context, frame_parallel_decoding_mode) = if !error_resilient_mode {
        let rfc = br.bit()?;
        let fpd = br.bit()?;
        (rfc, fpd)
    } else {
        (false, true)
    };
    let frame_context_idx = br.f(2)? as u8;

    let loop_filter = parse_loop_filter(&mut br)?;
    let mut quantization = parse_quantization(&mut br)?;
    quantization.lossless = quantization.base_q_idx == 0
        && quantization.delta_q_y_dc == 0
        && quantization.delta_q_uv_dc == 0
        && quantization.delta_q_uv_ac == 0;
    let segmentation = parse_segmentation(&mut br)?;
    let tile_info = parse_tile_info(&mut br, width)?;
    let header_size = br.f(16)? as u16;

    let uncompressed_header_size = br.byte_aligned_position();
    Ok(UncompressedHeader {
        profile: profile as u8,
        show_existing_frame: false,
        existing_frame_to_show,
        frame_type,
        show_frame,
        error_resilient_mode,
        intra_only,
        reset_frame_context,
        color_config,
        width,
        height,
        render_width,
        render_height,
        refresh_frame_flags,
        ref_frame_idx,
        ref_frame_sign_bias,
        allow_high_precision_mv,
        interpolation_filter,
        refresh_frame_context,
        frame_parallel_decoding_mode,
        frame_context_idx,
        loop_filter,
        quantization,
        segmentation,
        tile_info,
        header_size,
        uncompressed_header_size,
    })
}

fn parse_color_config(br: &mut BitReader<'_>, profile: u8) -> Result<ColorConfig> {
    // §6.2.1.
    let bit_depth = if profile >= 2 {
        let ten_or_twelve = br.bit()?;
        if ten_or_twelve {
            12
        } else {
            10
        }
    } else {
        8
    };
    let color_space = ColorSpace::from_bits(br.f(3)?);
    let color_range: bool;
    let mut subsampling_x = true;
    let mut subsampling_y = true;
    if !matches!(color_space, ColorSpace::Srgb) {
        color_range = br.bit()?;
        if profile == 1 || profile == 3 {
            subsampling_x = br.bit()?;
            subsampling_y = br.bit()?;
            let reserved = br.bit()?;
            if reserved {
                return Err(Error::invalid(
                    "vp9 §6.2.1: profile 1/3 reserved color bit must be 0",
                ));
            }
        }
    } else {
        color_range = true;
        if profile == 1 || profile == 3 {
            subsampling_x = false;
            subsampling_y = false;
            let reserved = br.bit()?;
            if reserved {
                return Err(Error::invalid(
                    "vp9 §6.2.1: profile 1/3 sRGB reserved bit must be 0",
                ));
            }
        }
    }
    Ok(ColorConfig {
        bit_depth,
        color_space,
        color_range,
        subsampling_x,
        subsampling_y,
    })
}

fn parse_frame_size_and_render(
    br: &mut BitReader<'_>,
) -> Result<(u32, u32, Option<u32>, Option<u32>)> {
    // §6.2.2 frame_size + render_size.
    let width = br.f(16)? + 1;
    let height = br.f(16)? + 1;
    let render_and_frame_size_different = br.bit()?;
    let (rw, rh) = if render_and_frame_size_different {
        (Some(br.f(16)? + 1), Some(br.f(16)? + 1))
    } else {
        (None, None)
    };
    Ok((width, height, rw, rh))
}

fn parse_frame_size_with_refs(
    br: &mut BitReader<'_>,
) -> Result<(u32, u32, Option<u32>, Option<u32>)> {
    // §6.2.2.1 frame_size_with_refs: 3 ref-found bits followed by either
    // "use that ref's frame size" or an inline frame_size + render_size.
    //
    // We don't carry ref-frame dimensions in the parser yet, so we
    // approximate: any "found_ref" is treated as 0×0 (downstream decode is
    // Unsupported anyway). When all three flags are clear, we parse the
    // inline frame_size like a key frame.
    let mut found_ref = false;
    for _ in 0..3 {
        if br.bit()? {
            found_ref = true;
        }
    }
    if found_ref {
        // Sizes inherited from refs — return zeros (parser scaffold only).
        let render_and_frame_size_different = br.bit()?;
        let (rw, rh) = if render_and_frame_size_different {
            (Some(br.f(16)? + 1), Some(br.f(16)? + 1))
        } else {
            (None, None)
        };
        Ok((0, 0, rw, rh))
    } else {
        parse_frame_size_and_render(br)
    }
}

fn read_interpolation_filter(br: &mut BitReader<'_>) -> Result<u8> {
    // §6.2 read_interpolation_filter.
    let is_filter_switchable = br.bit()?;
    if is_filter_switchable {
        Ok(4) // SWITCHABLE
    } else {
        Ok(br.f(2)? as u8)
    }
}

fn parse_loop_filter(br: &mut BitReader<'_>) -> Result<LoopFilterParams> {
    // §6.2.3 loop_filter_params.
    let level = br.f(6)? as u8;
    let sharpness = br.f(3)? as u8;
    let mode_ref_delta_enabled = br.bit()?;
    let mut mode_ref_delta_update = false;
    let mut ref_deltas = [0i8; 4];
    let mut mode_deltas = [0i8; 2];
    if mode_ref_delta_enabled {
        mode_ref_delta_update = br.bit()?;
        if mode_ref_delta_update {
            for d in ref_deltas.iter_mut() {
                if br.bit()? {
                    let v = br.f(6)? as i32;
                    let sign = br.bit()?;
                    *d = if sign { -(v as i8) } else { v as i8 };
                }
            }
            for d in mode_deltas.iter_mut() {
                if br.bit()? {
                    let v = br.f(6)? as i32;
                    let sign = br.bit()?;
                    *d = if sign { -(v as i8) } else { v as i8 };
                }
            }
        }
    }
    Ok(LoopFilterParams {
        level,
        sharpness,
        mode_ref_delta_enabled,
        mode_ref_delta_update,
        ref_deltas,
        mode_deltas,
    })
}

fn parse_quantization(br: &mut BitReader<'_>) -> Result<QuantizationParams> {
    // §6.2.4 quantization_params.
    let base_q_idx = br.f(8)? as u8;
    let delta_q_y_dc = read_delta_q(br)?;
    let delta_q_uv_dc = read_delta_q(br)?;
    let delta_q_uv_ac = read_delta_q(br)?;
    Ok(QuantizationParams {
        base_q_idx,
        delta_q_y_dc,
        delta_q_uv_dc,
        delta_q_uv_ac,
        lossless: false,
    })
}

fn read_delta_q(br: &mut BitReader<'_>) -> Result<i8> {
    if br.bit()? {
        let v = br.f(4)? as i32;
        let sign = br.bit()?;
        Ok(if sign { -(v as i8) } else { v as i8 })
    } else {
        Ok(0)
    }
}

fn parse_segmentation(br: &mut BitReader<'_>) -> Result<SegmentationParams> {
    // §6.2.5 segmentation_params.
    let mut s = SegmentationParams {
        enabled: br.bit()?,
        ..Default::default()
    };
    if !s.enabled {
        return Ok(s);
    }
    s.update_map = br.bit()?;
    if s.update_map {
        // Segmentation tree probabilities: 7 conditional probs.
        for _ in 0..7 {
            if br.bit()? {
                let _ = br.f(8)?;
            }
        }
        s.temporal_update = br.bit()?;
        if s.temporal_update {
            for _ in 0..3 {
                if br.bit()? {
                    let _ = br.f(8)?;
                }
            }
        }
    }
    s.update_data = br.bit()?;
    if s.update_data {
        s.abs_delta = br.bit()?;
        // 8 segments × 4 features (Q, LF, ref_frame, skip).
        const FEATURE_BITS: [u8; 4] = [8, 6, 2, 0];
        const FEATURE_SIGNED: [bool; 4] = [true, true, false, false];
        for seg in 0..8usize {
            for feat in 0..4usize {
                if br.bit()? {
                    s.feature_enabled[seg][feat] = true;
                    let bits = FEATURE_BITS[feat];
                    if bits == 0 {
                        s.feature_data[seg][feat] = 0;
                    } else {
                        let v = br.f(bits as u32)? as i16;
                        let signed = if FEATURE_SIGNED[feat] {
                            if br.bit()? {
                                -v
                            } else {
                                v
                            }
                        } else {
                            v
                        };
                        s.feature_data[seg][feat] = signed;
                    }
                }
            }
        }
    }
    Ok(s)
}

fn parse_tile_info(br: &mut BitReader<'_>, frame_width: u32) -> Result<TileInfo> {
    // §6.2.6 tile_info.
    let sb_cols = frame_width.max(1).div_ceil(64);
    let mut min_log2 = 0u32;
    while (64u32 << min_log2) < sb_cols {
        min_log2 += 1;
    }
    let max_log2 = {
        let mut m = 1u32;
        while ((sb_cols + (1 << m) - 1) >> m) >= 4 {
            m += 1;
        }
        m.saturating_sub(1)
    };
    let mut log2_tile_cols = min_log2;
    while log2_tile_cols < max_log2 {
        if br.bit()? {
            log2_tile_cols += 1;
        } else {
            break;
        }
    }
    let log2_tile_rows = if br.bit()? {
        let extra = br.bit()?;
        if extra {
            2
        } else {
            1
        }
    } else {
        0
    };
    Ok(TileInfo {
        log2_tile_cols: log2_tile_cols as u8,
        log2_tile_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal hand-rolled VP9 key-frame header for a 64×64 picture, profile 0.
    /// Built bit-by-bit to verify the parser.
    fn synth_key_frame_header() -> Vec<u8> {
        // We construct the byte sequence manually:
        //   frame_marker (2)         = 2     -> bits 10
        //   profile_low (1)          = 0
        //   profile_high (1)         = 0
        //   show_existing_frame (1)  = 0
        //   frame_type (1)           = 0  (KEY_FRAME)
        //   show_frame (1)           = 1
        //   error_resilient_mode (1) = 0
        // -> first byte = 1010_0010 = 0xA2
        //   sync code 24 bits        = 0x49 0x83 0x42
        //   color_space (3)          = 1 (BT601)
        //   color_range (1)          = 0
        //   width-1 (16)             = 63 = 0x003F
        //   height-1 (16)            = 63 = 0x003F
        //   render_and_frame_size_different (1) = 0
        //   refresh_frame_context (1) = 1
        //   frame_parallel_decoding_mode (1) = 0
        //   frame_context_idx (2)    = 0
        //   loop_filter level (6)    = 0
        //   loop_filter sharpness (3)= 0
        //   mode_ref_delta_enabled (1) = 0
        //   base_q_idx (8)           = 60 = 0x3C
        //   delta_q_y_dc (1=0)
        //   delta_q_uv_dc (1=0)
        //   delta_q_uv_ac (1=0)
        //   segmentation enabled (1) = 0
        //   tile_cols inc (sb_cols=1, min=0, max=0 -> no bit)
        //   tile_rows row inc (1=0)
        //   header_size (16) = 0
        //
        // We just emit those bits using a tiny writer.
        let mut bw = BitWriter::new();
        bw.write(2, 2); // frame_marker
        bw.write(0, 1); // profile_low
        bw.write(0, 1); // profile_high
        bw.write(0, 1); // show_existing_frame
        bw.write(0, 1); // frame_type
        bw.write(1, 1); // show_frame
        bw.write(0, 1); // error_resilient_mode
                        // sync code
        bw.write(0x49, 8);
        bw.write(0x83, 8);
        bw.write(0x42, 8);
        // color_config (profile 0)
        bw.write(1, 3); // BT601
        bw.write(0, 1); // color_range
                        // frame_size + render_size
        bw.write(63, 16);
        bw.write(63, 16);
        bw.write(0, 1); // render_and_frame_size_different
                        // refresh_frame_context, frame_parallel_decoding_mode, frame_context_idx
        bw.write(1, 1);
        bw.write(0, 1);
        bw.write(0, 2);
        // loop_filter
        bw.write(0, 6);
        bw.write(0, 3);
        bw.write(0, 1);
        // quantization
        bw.write(60, 8);
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);
        // segmentation
        bw.write(0, 1);
        // tile_info: with width=64, sb_cols=1, min_log2=0, max_log2=0 — no
        // increment bit. row: 1 bit = 0.
        bw.write(0, 1);
        // header_size
        bw.write(0, 16);
        bw.finish()
    }

    /// Tiny MSB-first bit writer used by the unit test.
    struct BitWriter {
        out: Vec<u8>,
        cur: u8,
        bits: u32,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                out: Vec::new(),
                cur: 0,
                bits: 0,
            }
        }

        fn write(&mut self, value: u32, n: u32) {
            assert!(n <= 32);
            for i in (0..n).rev() {
                let b = ((value >> i) & 1) as u8;
                self.cur = (self.cur << 1) | b;
                self.bits += 1;
                if self.bits == 8 {
                    self.out.push(self.cur);
                    self.cur = 0;
                    self.bits = 0;
                }
            }
        }

        fn finish(mut self) -> Vec<u8> {
            if self.bits > 0 {
                self.cur <<= 8 - self.bits;
                self.out.push(self.cur);
            }
            // Append some payload bytes so the parser doesn't EOF on
            // header_size lookups beyond the synthetic header.
            self.out.extend_from_slice(&[0u8; 4]);
            self.out
        }
    }

    #[test]
    fn parse_synthetic_key_frame() {
        let buf = synth_key_frame_header();
        let h = parse_uncompressed_header(&buf, None).expect("parse");
        assert_eq!(h.profile, 0);
        assert_eq!(h.frame_type, FrameType::Key);
        assert!(h.show_frame);
        assert_eq!(h.width, 64);
        assert_eq!(h.height, 64);
        assert_eq!(h.color_config.bit_depth, 8);
        assert_eq!(h.color_config.color_space, ColorSpace::Bt601);
        assert_eq!(h.quantization.base_q_idx, 60);
        assert!(!h.quantization.lossless);
        assert!(!h.segmentation.enabled);
        assert_eq!(h.tile_info.log2_tile_cols, 0);
        assert_eq!(h.tile_info.log2_tile_rows, 0);
        assert_eq!(h.header_size, 0);
    }

    #[test]
    fn rejects_bad_marker() {
        // first 2 bits = 00 -> frame_marker = 0
        let mut buf = vec![0u8; 64];
        buf[0] = 0b0000_0000;
        let err = parse_uncompressed_header(&buf, None).unwrap_err();
        assert!(matches!(err, oxideav_core::Error::InvalidData(_)));
    }
}
