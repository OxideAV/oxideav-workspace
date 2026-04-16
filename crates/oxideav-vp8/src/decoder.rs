//! VP8 keyframe decoder.
//!
//! P-frames intentionally return `Error::Unsupported` — only the
//! keyframe (intra-only) path is implemented in this milestone.

use std::collections::VecDeque;

use oxideav_codec::Decoder;
use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, Packet, PixelFormat, Result, TimeBase, VideoFrame,
    VideoPlane,
};

use crate::bool_decoder::BoolDecoder;
use crate::frame_header::{parse_keyframe_header, FrameHeader};
use crate::frame_tag::{parse_header, FrameType};
use crate::intra::{predict_16x16, predict_4x4, predict_8x8, B4x4Neighbours};
use crate::loopfilter::{
    filter_normal_horizontal, filter_normal_vertical, filter_simple_horizontal,
    filter_simple_vertical, FilterParams,
};
use crate::tables::quant::{
    clamp_qindex, uv_ac_step, uv_dc_step, y2_ac_step, y2_dc_step, y_ac_step, y_dc_step,
};
use crate::tables::trees::{
    decode_tree, BMODE_TREE, B_PRED, KF_BMODE_PROB, KF_UV_MODE_PROBS, KF_UV_MODE_TREE,
    KF_YMODE_PROBS, KF_YMODE_TREE,
};
use crate::tokens::{decode_block, BlockType};
use crate::transform::{idct4x4, iwht4x4};

/// Public factory used by the registry.
pub fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    Ok(Box::new(Vp8Decoder::new(params.codec_id.clone())))
}

pub struct Vp8Decoder {
    codec_id: CodecId,
    queued: VecDeque<VideoFrame>,
    pending_pts: Option<i64>,
    pending_tb: TimeBase,
}

impl Vp8Decoder {
    pub fn new(codec_id: CodecId) -> Self {
        Self {
            codec_id,
            queued: VecDeque::new(),
            pending_pts: None,
            pending_tb: TimeBase::new(1, 1000),
        }
    }
}

impl Decoder for Vp8Decoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        self.pending_pts = packet.pts;
        self.pending_tb = packet.time_base;
        let frame = decode_frame(&packet.data)?;
        let mut vf = frame;
        vf.pts = self.pending_pts;
        vf.time_base = self.pending_tb;
        self.queued.push_back(vf);
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        match self.queued.pop_front() {
            Some(v) => Ok(Frame::Video(v)),
            None => Err(Error::NeedMore),
        }
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Decode a single VP8 frame and return the resulting VideoFrame
/// (without pts/time_base — caller fills those in).
pub fn decode_frame(buf: &[u8]) -> Result<VideoFrame> {
    let parsed = parse_header(buf)?;
    if !matches!(parsed.tag.frame_type, FrameType::Key) {
        return Err(Error::unsupported(
            "VP8: P-frame decoding not implemented (keyframes only)",
        ));
    }
    let kf = parsed
        .keyframe
        .ok_or_else(|| Error::invalid("VP8: keyframe header missing"))?;
    let width = kf.width as usize;
    let height = kf.height as usize;

    // --- bool-coded header ---
    let header_buf = &buf[parsed.compressed_offset..];
    let mut hdr_dec = BoolDecoder::new(header_buf)?;
    let header = parse_keyframe_header(&mut hdr_dec)?;

    // --- token partitions ---
    let first_part_size = parsed.tag.first_partition_size as usize;
    let after_header_off = parsed.compressed_offset + first_part_size;
    if after_header_off > buf.len() {
        return Err(Error::invalid("VP8: first partition extends past end"));
    }
    let nb_parts = 1usize << header.log2_nb_partitions;
    let mut parts: Vec<&[u8]> = Vec::with_capacity(nb_parts);

    let mut cursor = after_header_off;
    if nb_parts > 1 {
        // Partition sizes occupy 3 bytes each, except the last (rest-of-file).
        let sizes_bytes = (nb_parts - 1) * 3;
        if cursor + sizes_bytes > buf.len() {
            return Err(Error::invalid("VP8: partition size table truncated"));
        }
        let mut sizes = Vec::with_capacity(nb_parts - 1);
        for i in 0..nb_parts - 1 {
            let off = cursor + i * 3;
            let sz = (buf[off] as usize)
                | ((buf[off + 1] as usize) << 8)
                | ((buf[off + 2] as usize) << 16);
            sizes.push(sz);
        }
        cursor += sizes_bytes;
        for sz in sizes {
            if cursor + sz > buf.len() {
                return Err(Error::invalid("VP8: partition extends past end"));
            }
            parts.push(&buf[cursor..cursor + sz]);
            cursor += sz;
        }
        parts.push(&buf[cursor..]);
    } else {
        parts.push(&buf[cursor..]);
    }

    let mb_w = (width + 15) / 16;
    let mb_h = (height + 15) / 16;

    // --- per-MB decode state ---
    // Y plane: stride aligned to MB grid (16 px boundaries).
    let y_stride = mb_w * 16;
    let uv_stride = mb_w * 8;
    let y_buf_h = mb_h * 16;
    let uv_buf_h = mb_h * 8;
    let mut y_plane = vec![0u8; y_stride * y_buf_h];
    let mut u_plane = vec![0u8; uv_stride * uv_buf_h];
    let mut v_plane = vec![0u8; uv_stride * uv_buf_h];

    // Per-MB context for residuals.
    let mut nz_y_above = vec![[0u8; 4]; mb_w];
    let mut nz_uv_above = vec![[0u8; 2]; mb_w]; // U above
    let mut nz_v_above = vec![[0u8; 2]; mb_w]; // V above
    let mut nz_y2_above = vec![0u8; mb_w];
    // Above-mode tracking for keyframe per-subblock prediction.
    // For each MB column we keep a 4-entry "bmode along the bottom edge"
    // array if the MB above was B_PRED, otherwise a single intra mode
    // expanded to all four.
    let mut bmode_above = vec![[0i32; 4]; mb_w];

    // Decode all MBs by decoding header bits from partition 0 (mode info)
    // and residues from the appropriate token partition.
    let mut mb_dec = hdr_dec; // re-use hdr_dec — already advanced to correct point
    let _ = &mb_dec;

    // The mode-info portion lives in the first partition (the one we used
    // for the frame header). We continue reading from `mb_dec`.
    let mut mb_info = vec![MbInfo::default(); mb_w * mb_h];

    for mb_y in 0..mb_h {
        for mb_x in 0..mb_w {
            let info = decode_mb_mode_info_keyframe(
                &mut mb_dec,
                &mb_info,
                mb_x,
                mb_y,
                mb_w,
                &header,
                &mut bmode_above,
            )?;
            mb_info[mb_y * mb_w + mb_x] = info;
        }
    }

    // Token decode + reconstruction. We use one boolean decoder per token
    // partition, walking MBs in raster order and assigning each MB to
    // partition `mb_y mod nb_parts`.
    let mut token_decs: Vec<BoolDecoder> = parts
        .iter()
        .map(|p| BoolDecoder::new(p))
        .collect::<Result<_>>()?;

    // Reset above-context arrays for residual decoding.
    for c in &mut nz_y_above {
        *c = [0; 4];
    }
    for c in &mut nz_uv_above {
        *c = [0; 2];
    }
    for c in &mut nz_v_above {
        *c = [0; 2];
    }
    for c in &mut nz_y2_above {
        *c = 0;
    }

    // Per-row left contexts.
    for mb_y in 0..mb_h {
        let mut nz_y_left = [0u8; 4];
        let mut nz_u_left = [0u8; 2];
        let mut nz_v_left = [0u8; 2];
        let mut nz_y2_left = 0u8;
        let part_idx = mb_y % nb_parts;
        let dec = &mut token_decs[part_idx];

        for mb_x in 0..mb_w {
            let info = mb_info[mb_y * mb_w + mb_x].clone();
            let skip = info.skip;

            // Decide block type.
            let has_y2 = info.y_mode != B_PRED;
            let mut y2_coeffs = [0i16; 16];
            let mut y_coeffs = [[0i16; 16]; 16];
            let mut u_coeffs = [[0i16; 16]; 4];
            let mut v_coeffs = [[0i16; 16]; 4];

            if !skip {
                // Decode Y2 if needed.
                if has_y2 {
                    let nctx = nz_y2_above[mb_x] + nz_y2_left;
                    let nz = decode_block(
                        dec,
                        &header.coef_probs,
                        BlockType::Y2,
                        nctx,
                        &mut y2_coeffs,
                        0,
                    );
                    let nz_flag = if nz > 0 { 1 } else { 0 };
                    nz_y2_above[mb_x] = nz_flag;
                    nz_y2_left = nz_flag;
                }

                // Decode 16 Y blocks.
                let block_type = if has_y2 {
                    BlockType::YAfterY2
                } else {
                    BlockType::YNoY2
                };
                let start = if has_y2 { 1 } else { 0 };
                for by in 0..4 {
                    for bx in 0..4 {
                        let idx = by * 4 + bx;
                        let above_nz = nz_y_above[mb_x][bx];
                        let left_nz = nz_y_left[by];
                        let nctx = above_nz + left_nz;
                        let nz = decode_block(
                            dec,
                            &header.coef_probs,
                            block_type,
                            nctx,
                            &mut y_coeffs[idx],
                            start,
                        );
                        let nz_flag = if nz > 0 { 1 } else { 0 };
                        nz_y_above[mb_x][bx] = nz_flag;
                        nz_y_left[by] = nz_flag;
                    }
                }

                // Decode 4 U + 4 V blocks.
                for by in 0..2 {
                    for bx in 0..2 {
                        let idx = by * 2 + bx;
                        // U
                        let above_nz = nz_uv_above[mb_x][bx];
                        let left_nz = nz_u_left[by];
                        let nctx = above_nz + left_nz;
                        let nz = decode_block(
                            dec,
                            &header.coef_probs,
                            BlockType::UV,
                            nctx,
                            &mut u_coeffs[idx],
                            0,
                        );
                        let nz_flag = if nz > 0 { 1 } else { 0 };
                        nz_uv_above[mb_x][bx] = nz_flag;
                        nz_u_left[by] = nz_flag;
                        // V
                        let above_nz = nz_v_above[mb_x][bx];
                        let left_nz = nz_v_left[by];
                        let nctx = above_nz + left_nz;
                        let nz = decode_block(
                            dec,
                            &header.coef_probs,
                            BlockType::UV,
                            nctx,
                            &mut v_coeffs[idx],
                            0,
                        );
                        let nz_flag = if nz > 0 { 1 } else { 0 };
                        nz_v_above[mb_x][bx] = nz_flag;
                        nz_v_left[by] = nz_flag;
                    }
                }
            } else {
                // skip: clear contexts.
                if has_y2 {
                    nz_y2_above[mb_x] = 0;
                    nz_y2_left = 0;
                }
                for bx in 0..4 {
                    nz_y_above[mb_x][bx] = 0;
                    nz_y_left[bx] = 0;
                }
                for bx in 0..2 {
                    nz_uv_above[mb_x][bx] = 0;
                    nz_v_above[mb_x][bx] = 0;
                    nz_u_left[bx] = 0;
                    nz_v_left[bx] = 0;
                }
            }

            // Dequantize + inverse transform + intra predict.
            reconstruct_mb(
                &header,
                &info,
                has_y2,
                &y2_coeffs,
                &y_coeffs,
                &u_coeffs,
                &v_coeffs,
                mb_x,
                mb_y,
                mb_w,
                mb_h,
                &mut y_plane,
                &mut u_plane,
                &mut v_plane,
                y_stride,
                uv_stride,
            );
        }
    }

    // Loop filter (RFC §15) — applied across MB and sub-block edges.
    if header.loop_filter.level > 0 {
        let lf = &header.loop_filter;
        let params_mb = FilterParams::for_mb(lf.level, lf.sharpness, true);
        let params_sb = FilterParams::for_mb(lf.level, lf.sharpness, false);
        let simple = lf.filter_type == 1;
        // Vertical edges (between MB columns).
        for mb_y in 0..mb_h {
            for mb_x in 1..mb_w {
                let x = mb_x * 16;
                let y0 = mb_y * 16;
                if simple {
                    filter_simple_vertical(&mut y_plane, y_stride, x, y_stride, y0 + 16, params_mb);
                } else {
                    filter_normal_vertical(
                        &mut y_plane,
                        y_stride,
                        x,
                        y_stride,
                        y0 + 16,
                        params_mb,
                        true,
                    );
                }
            }
        }
        // Horizontal edges (between MB rows).
        for mb_y in 1..mb_h {
            let y = mb_y * 16;
            if simple {
                filter_simple_horizontal(&mut y_plane, y_stride, y, y_stride, y_buf_h, params_mb);
            } else {
                filter_normal_horizontal(
                    &mut y_plane,
                    y_stride,
                    y,
                    y_stride,
                    y_buf_h,
                    params_mb,
                    true,
                );
            }
        }
        // Subblock edges (only in normal mode, every 4 px inside each MB).
        if !simple {
            for mb_y in 0..mb_h {
                for mb_x in 0..mb_w {
                    let bx0 = mb_x * 16;
                    let by0 = mb_y * 16;
                    for k in 1..4 {
                        let xv = bx0 + k * 4;
                        filter_normal_vertical(
                            &mut y_plane,
                            y_stride,
                            xv,
                            y_stride,
                            by0 + 16,
                            params_sb,
                            false,
                        );
                        let yh = by0 + k * 4;
                        filter_normal_horizontal(
                            &mut y_plane,
                            y_stride,
                            yh,
                            y_stride,
                            y_buf_h,
                            params_sb,
                            false,
                        );
                    }
                }
            }
        }
        // Chroma planes — same idea but on 8×8 grid.
        for mb_y in 0..mb_h {
            for mb_x in 1..mb_w {
                let x = mb_x * 8;
                let y0 = mb_y * 8;
                if simple {
                    filter_simple_vertical(
                        &mut u_plane,
                        uv_stride,
                        x,
                        uv_stride,
                        y0 + 8,
                        params_mb,
                    );
                    filter_simple_vertical(
                        &mut v_plane,
                        uv_stride,
                        x,
                        uv_stride,
                        y0 + 8,
                        params_mb,
                    );
                } else {
                    filter_normal_vertical(
                        &mut u_plane,
                        uv_stride,
                        x,
                        uv_stride,
                        y0 + 8,
                        params_mb,
                        true,
                    );
                    filter_normal_vertical(
                        &mut v_plane,
                        uv_stride,
                        x,
                        uv_stride,
                        y0 + 8,
                        params_mb,
                        true,
                    );
                }
            }
        }
        for mb_y in 1..mb_h {
            let y = mb_y * 8;
            if simple {
                filter_simple_horizontal(
                    &mut u_plane,
                    uv_stride,
                    y,
                    uv_stride,
                    uv_buf_h,
                    params_mb,
                );
                filter_simple_horizontal(
                    &mut v_plane,
                    uv_stride,
                    y,
                    uv_stride,
                    uv_buf_h,
                    params_mb,
                );
            } else {
                filter_normal_horizontal(
                    &mut u_plane,
                    uv_stride,
                    y,
                    uv_stride,
                    uv_buf_h,
                    params_mb,
                    true,
                );
                filter_normal_horizontal(
                    &mut v_plane,
                    uv_stride,
                    y,
                    uv_stride,
                    uv_buf_h,
                    params_mb,
                    true,
                );
            }
        }
    }

    // Crop to actual width/height when MB-aligned size differs.
    let mut y_out = vec![0u8; width * height];
    for j in 0..height {
        let src = &y_plane[j * y_stride..j * y_stride + width];
        y_out[j * width..j * width + width].copy_from_slice(src);
    }
    let cw = (width + 1) / 2;
    let ch = (height + 1) / 2;
    let mut u_out = vec![0u8; cw * ch];
    let mut v_out = vec![0u8; cw * ch];
    for j in 0..ch {
        let src_u = &u_plane[j * uv_stride..j * uv_stride + cw];
        u_out[j * cw..j * cw + cw].copy_from_slice(src_u);
        let src_v = &v_plane[j * uv_stride..j * uv_stride + cw];
        v_out[j * cw..j * cw + cw].copy_from_slice(src_v);
    }

    Ok(VideoFrame {
        format: PixelFormat::Yuv420P,
        width: width as u32,
        height: height as u32,
        pts: None,
        time_base: TimeBase::new(1, 1000),
        planes: vec![
            VideoPlane {
                stride: width,
                data: y_out,
            },
            VideoPlane {
                stride: cw,
                data: u_out,
            },
            VideoPlane {
                stride: cw,
                data: v_out,
            },
        ],
    })
}

#[derive(Clone, Default)]
struct MbInfo {
    /// Y intra mode (DC_PRED / V_PRED / H_PRED / TM_PRED / B_PRED).
    y_mode: i32,
    /// Per-subblock 4×4 intra mode (only meaningful when y_mode == B_PRED).
    bmodes: [i32; 16],
    /// UV intra mode.
    uv_mode: i32,
    /// MB skip flag (if mb_skip_enabled).
    skip: bool,
    /// Segment id (0..3) — currently unused for keyframe decode without
    /// segmentation.
    #[allow(dead_code)]
    segment_id: u8,
}

fn decode_mb_mode_info_keyframe(
    dec: &mut BoolDecoder<'_>,
    mb_info: &[MbInfo],
    mb_x: usize,
    mb_y: usize,
    mb_w: usize,
    header: &FrameHeader,
    bmode_above: &mut [[i32; 4]],
) -> Result<MbInfo> {
    let mut info = MbInfo::default();
    // Segment map.
    if header.segmentation.enabled && header.segmentation.update_map {
        // Decode segment id from segment tree.
        let probs = &header.segmentation.tree_probs;
        let s0 = dec.read_bool(probs[0] as u32) as u8;
        let s = if s0 == 0 {
            dec.read_bool(probs[1] as u32) as u8
        } else {
            2 + dec.read_bool(probs[2] as u32) as u8
        };
        info.segment_id = s;
    }
    // mb_skip flag.
    info.skip = if header.mb_skip_enabled {
        dec.read_bool(header.mb_skip_prob as u32)
    } else {
        false
    };
    // Y intra mode.
    info.y_mode = decode_tree(dec, &KF_YMODE_TREE, &KF_YMODE_PROBS);
    if info.y_mode == B_PRED {
        // 16 sub-block modes. Each is context-dependent on the b-mode of
        // the above and left neighbour subblocks.
        let mut left_bmodes = if mb_x > 0 {
            // Right column of the MB to our left.
            let l = &mb_info[mb_y * mb_w + mb_x - 1];
            if l.y_mode == B_PRED {
                [l.bmodes[3], l.bmodes[7], l.bmodes[11], l.bmodes[15]]
            } else {
                [intra_to_b(l.y_mode); 4]
            }
        } else {
            [intra_to_b(0); 4] // DC_PRED equivalent
        };
        let mut new_above = [0i32; 4];
        for i in 0..16 {
            let row = i / 4;
            let col = i % 4;
            let above_mode = if row == 0 {
                bmode_above[mb_x][col]
            } else {
                info.bmodes[(row - 1) * 4 + col]
            };
            let left_mode = if col == 0 {
                left_bmodes[row]
            } else {
                info.bmodes[row * 4 + col - 1]
            };
            let probs = &KF_BMODE_PROB[above_mode as usize][left_mode as usize];
            let m = decode_tree(dec, &BMODE_TREE, probs);
            info.bmodes[i] = m;
            if row == 3 {
                new_above[col] = m;
            }
            if col == 3 {
                left_bmodes[row] = m;
            }
        }
        bmode_above[mb_x] = new_above;
    } else {
        // Expand the intra-16×16 mode to bmodes for neighbour propagation.
        let bm = intra_to_b(info.y_mode);
        for i in 0..16 {
            info.bmodes[i] = bm;
        }
        bmode_above[mb_x] = [bm; 4];
    }
    // UV mode.
    info.uv_mode = decode_tree(dec, &KF_UV_MODE_TREE, &KF_UV_MODE_PROBS);
    Ok(info)
}

/// Map an intra-16 mode onto an equivalent b-mode for neighbour-context
/// purposes (RFC §16.2).
fn intra_to_b(intra_mode: i32) -> i32 {
    use crate::tables::trees::{B_DC_PRED, B_HE_PRED, B_TM_PRED, B_VE_PRED};
    use crate::tables::trees::{DC_PRED, H_PRED, TM_PRED, V_PRED};
    match intra_mode {
        DC_PRED => B_DC_PRED,
        V_PRED => B_VE_PRED,
        H_PRED => B_HE_PRED,
        TM_PRED => B_TM_PRED,
        _ => B_DC_PRED,
    }
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_mb(
    header: &FrameHeader,
    info: &MbInfo,
    has_y2: bool,
    y2_coeffs: &[i16; 16],
    y_coeffs: &[[i16; 16]; 16],
    u_coeffs: &[[i16; 16]; 4],
    v_coeffs: &[[i16; 16]; 4],
    mb_x: usize,
    mb_y: usize,
    mb_w: usize,
    mb_h: usize,
    y_plane: &mut [u8],
    u_plane: &mut [u8],
    v_plane: &mut [u8],
    y_stride: usize,
    uv_stride: usize,
) {
    let _ = mb_h;
    // Quantiser steps.
    let qi = clamp_qindex(header.quant.y_ac_qi);
    let y_dc = y_dc_step(qi as i32 + header.quant.y_dc_delta);
    let y_ac = y_ac_step(qi as i32);
    let y2_dc = y2_dc_step(qi as i32 + header.quant.y2_dc_delta);
    let y2_ac = y2_ac_step(qi as i32 + header.quant.y2_ac_delta);
    let uv_dc = uv_dc_step(qi as i32 + header.quant.uv_dc_delta);
    let uv_ac = uv_ac_step(qi as i32 + header.quant.uv_ac_delta);

    // Compute Y2 inverse if needed → 16 DC values for the Y blocks.
    let y2_dc_vals: [i16; 16] = if has_y2 {
        let mut deq = [0i16; 16];
        for i in 0..16 {
            let v = y2_coeffs[i] as i32;
            let q = if i == 0 { y2_dc } else { y2_ac };
            deq[i] = (v * q) as i16;
        }
        iwht4x4(&deq)
    } else {
        [0; 16]
    };

    // Predict + inverse transform Y blocks.
    let mb_x_px = mb_x * 16;
    let mb_y_px = mb_y * 16;
    if info.y_mode == B_PRED {
        // Per-subblock prediction. We need the row-of-pixels above each
        // subblock and the column to its left, plus a top-right extension
        // (sometimes synthesised).
        for i in 0..16 {
            let by = i / 4;
            let bx = i % 4;
            let dst_x = mb_x_px + bx * 4;
            let dst_y = mb_y_px + by * 4;
            let mut neigh = B4x4Neighbours {
                above: [127; 8],
                left: [129; 4],
                tl: 127,
            };
            if dst_y > 0 {
                for k in 0..4 {
                    neigh.above[k] = y_plane[(dst_y - 1) * y_stride + dst_x + k];
                }
                // Extension to the right — only present at top-row right
                // subblocks.
                let max_right = if by == 0 {
                    // Above MB extends fully right within this MB.
                    let right_lim = (mb_x_px + 16).min(mb_w * 16);
                    right_lim
                } else {
                    // For non-top rows in B_PRED, the extension comes from
                    // the just-decoded row above (within the same MB), but
                    // since we go in raster order and reconstructed pixels
                    // are already in the plane, we can fetch them directly.
                    let right_lim = (dst_x + 8).min(mb_w * 16);
                    right_lim
                };
                for k in 4..8 {
                    let xx = dst_x + k;
                    if xx < max_right {
                        neigh.above[k] = y_plane[(dst_y - 1) * y_stride + xx];
                    } else {
                        // Replicate the rightmost available pixel.
                        neigh.above[k] = neigh.above[3];
                    }
                }
            }
            if dst_x > 0 {
                for k in 0..4 {
                    neigh.left[k] = y_plane[(dst_y + k) * y_stride + dst_x - 1];
                }
            }
            if dst_x > 0 && dst_y > 0 {
                neigh.tl = y_plane[(dst_y - 1) * y_stride + dst_x - 1];
            }

            let mut pred = [0u8; 16];
            predict_4x4(info.bmodes[i], &neigh, &mut pred, 4);

            // Dequantise this Y block, inverse-transform, add to pred, write.
            let mut deq = [0i16; 16];
            for k in 0..16 {
                let q = if k == 0 { y_dc } else { y_ac };
                deq[k] = (y_coeffs[i][k] as i32 * q) as i16;
            }
            let res = idct4x4(&deq);
            for r in 0..4 {
                for c in 0..4 {
                    let p = pred[r * 4 + c] as i32;
                    let rr = res[r * 4 + c] as i32;
                    y_plane[(dst_y + r) * y_stride + dst_x + c] = (p + rr).clamp(0, 255) as u8;
                }
            }
        }
    } else {
        // 16×16 intra prediction.
        let mut above = [0u8; 16];
        let mut left = [0u8; 16];
        let above_avail = mb_y_px > 0;
        let left_avail = mb_x_px > 0;
        if above_avail {
            for i in 0..16 {
                above[i] = y_plane[(mb_y_px - 1) * y_stride + mb_x_px + i];
            }
        }
        if left_avail {
            for j in 0..16 {
                left[j] = y_plane[(mb_y_px + j) * y_stride + mb_x_px - 1];
            }
        }
        let tl = if above_avail && left_avail {
            Some(y_plane[(mb_y_px - 1) * y_stride + mb_x_px - 1])
        } else if above_avail {
            Some(127)
        } else if left_avail {
            Some(129)
        } else {
            None
        };
        let mut pred = vec![0u8; 16 * 16];
        predict_16x16(
            info.y_mode,
            if above_avail { Some(&above) } else { None },
            if left_avail { Some(&left) } else { None },
            tl,
            &mut pred,
            16,
        );
        // Now dequantise and add residue 4×4 block by 4×4 block.
        for i in 0..16 {
            let by = i / 4;
            let bx = i % 4;
            let mut deq = [0i16; 16];
            // DC from Y2.
            deq[0] = y2_dc_vals[i];
            for k in 1..16 {
                deq[k] = (y_coeffs[i][k] as i32 * y_ac) as i16;
            }
            // For non-Y2 blocks (e.g. B_PRED) start would be 0, but here
            // has_y2 is true so y_coeffs[i][0] should be unused / zero.
            let res = idct4x4(&deq);
            let dst_x = mb_x_px + bx * 4;
            let dst_y = mb_y_px + by * 4;
            for r in 0..4 {
                for c in 0..4 {
                    let p = pred[(by * 4 + r) * 16 + bx * 4 + c] as i32;
                    let rr = res[r * 4 + c] as i32;
                    y_plane[(dst_y + r) * y_stride + dst_x + c] = (p + rr).clamp(0, 255) as u8;
                }
            }
        }
    }

    // UV prediction (always 8×8 modes).
    let mb_xc = mb_x * 8;
    let mb_yc = mb_y * 8;
    for plane_sel in 0..2 {
        let (plane, coeffs) = if plane_sel == 0 {
            (u_plane.as_mut(), u_coeffs)
        } else {
            (v_plane.as_mut(), v_coeffs)
        };
        let mut above = [0u8; 8];
        let mut left = [0u8; 8];
        let above_avail = mb_yc > 0;
        let left_avail = mb_xc > 0;
        if above_avail {
            for i in 0..8 {
                above[i] = plane[(mb_yc - 1) * uv_stride + mb_xc + i];
            }
        }
        if left_avail {
            for j in 0..8 {
                left[j] = plane[(mb_yc + j) * uv_stride + mb_xc - 1];
            }
        }
        let tl = if above_avail && left_avail {
            Some(plane[(mb_yc - 1) * uv_stride + mb_xc - 1])
        } else if above_avail {
            Some(127)
        } else if left_avail {
            Some(129)
        } else {
            None
        };
        let mut pred = vec![0u8; 8 * 8];
        predict_8x8(
            info.uv_mode,
            if above_avail { Some(&above) } else { None },
            if left_avail { Some(&left) } else { None },
            tl,
            &mut pred,
            8,
        );
        // 4 sub-blocks, each 4×4.
        for i in 0..4 {
            let by = i / 2;
            let bx = i % 2;
            let mut deq = [0i16; 16];
            deq[0] = (coeffs[i][0] as i32 * uv_dc) as i16;
            for k in 1..16 {
                deq[k] = (coeffs[i][k] as i32 * uv_ac) as i16;
            }
            let res = idct4x4(&deq);
            let dst_x = mb_xc + bx * 4;
            let dst_y = mb_yc + by * 4;
            for r in 0..4 {
                for c in 0..4 {
                    let p = pred[(by * 4 + r) * 8 + bx * 4 + c] as i32;
                    let rr = res[r * 4 + c] as i32;
                    plane[(dst_y + r) * uv_stride + dst_x + c] = (p + rr).clamp(0, 255) as u8;
                }
            }
        }
    }
}
