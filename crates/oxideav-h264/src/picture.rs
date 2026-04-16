//! Reconstructed picture buffer (YUV420P) with macroblock helpers.
//!
//! Used by the decoder to gather reconstructed luma and chroma samples per
//! macroblock and to feed them to the deblocking pass.

use oxideav_core::{frame::VideoPlane, PixelFormat, TimeBase, VideoFrame};

#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub mb_width: u32,
    pub mb_height: u32,
    /// Luma plane, raw stride == width.
    pub y: Vec<u8>,
    /// Cb plane, raw stride == width / 2.
    pub cb: Vec<u8>,
    /// Cr plane, raw stride == width / 2.
    pub cr: Vec<u8>,
    /// Per-macroblock state needed for predictor neighbour resolution and
    /// deblocking. Stored in raster order.
    pub mb_info: Vec<MbInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct MbInfo {
    /// QP_Y for the macroblock (after slice_qp_delta + per-MB delta).
    pub qp_y: i32,
    /// True if this macroblock decoded successfully (otherwise its samples
    /// are placeholder zeros and shouldn't be used as predictors).
    pub coded: bool,
    /// Per-4×4 luma block coefficient counts (TotalCoeff). 16 entries indexed
    /// in raster order within the MB (`row*4 + col`). Used by §9.2.1.1 to
    /// compute the predicted nC for the block to the right / below.
    pub luma_nc: [u8; 16],
    /// Per-4×4 chroma Cb coefficient counts (4 entries: 2×2 sub-blocks).
    pub cb_nc: [u8; 4],
    /// Per-4×4 chroma Cr coefficient counts (4 entries).
    pub cr_nc: [u8; 4],
    /// Intra4x4 modes per sub-block (16 entries) — used for mode prediction.
    /// For Intra16x16/PCM macroblocks, all entries are `INTRA_DC_FAKE` so
    /// neighbouring 4×4 blocks fall through to the DC fallback (per §8.3.1.1).
    pub intra4x4_pred_mode: [u8; 16],
    /// True when this macroblock is intra. For an I-slice all MBs are intra,
    /// but the field exists to support future P-slice deblocking edges.
    pub intra: bool,
}

/// Sentinel used as "intra mode unavailable for prediction" — see §8.3.1.1.
pub const INTRA_DC_FAKE: u8 = 2;

impl Picture {
    pub fn new(mb_width: u32, mb_height: u32) -> Self {
        let width = mb_width * 16;
        let height = mb_height * 16;
        let cw = width / 2;
        let ch = height / 2;
        Self {
            width,
            height,
            mb_width,
            mb_height,
            y: vec![0u8; (width * height) as usize],
            cb: vec![128u8; (cw * ch) as usize],
            cr: vec![128u8; (cw * ch) as usize],
            mb_info: vec![MbInfo::default(); (mb_width * mb_height) as usize],
        }
    }

    /// Stride (bytes per row) of the luma plane.
    pub fn luma_stride(&self) -> usize {
        self.width as usize
    }
    pub fn chroma_stride(&self) -> usize {
        (self.width / 2) as usize
    }

    /// Linear address of the top-left luma sample of `(mb_x, mb_y)`.
    pub fn luma_off(&self, mb_x: u32, mb_y: u32) -> usize {
        (mb_y as usize * 16) * self.luma_stride() + (mb_x as usize * 16)
    }
    pub fn chroma_off(&self, mb_x: u32, mb_y: u32) -> usize {
        (mb_y as usize * 8) * self.chroma_stride() + (mb_x as usize * 8)
    }

    /// Get a mutable reference to a single MB's bookkeeping.
    pub fn mb_info_mut(&mut self, mb_x: u32, mb_y: u32) -> &mut MbInfo {
        let idx = (mb_y * self.mb_width + mb_x) as usize;
        &mut self.mb_info[idx]
    }
    pub fn mb_info_at(&self, mb_x: u32, mb_y: u32) -> &MbInfo {
        let idx = (mb_y * self.mb_width + mb_x) as usize;
        &self.mb_info[idx]
    }

    /// Crop to (visible_w × visible_h) and emit a `VideoFrame`. The picture's
    /// raw dimensions are MB-aligned; the encoder requested any smaller
    /// visible area via SPS frame cropping.
    pub fn into_video_frame(
        self,
        visible_w: u32,
        visible_h: u32,
        pts: Option<i64>,
        time_base: TimeBase,
    ) -> VideoFrame {
        let cw = visible_w.div_ceil(2);
        let ch = visible_h.div_ceil(2);
        let l_stride = self.luma_stride();
        let c_stride = self.chroma_stride();

        let mut y_out = Vec::with_capacity((visible_w * visible_h) as usize);
        for r in 0..visible_h as usize {
            let off = r * l_stride;
            y_out.extend_from_slice(&self.y[off..off + visible_w as usize]);
        }
        let mut cb_out = Vec::with_capacity((cw * ch) as usize);
        let mut cr_out = Vec::with_capacity((cw * ch) as usize);
        for r in 0..ch as usize {
            let off = r * c_stride;
            cb_out.extend_from_slice(&self.cb[off..off + cw as usize]);
            cr_out.extend_from_slice(&self.cr[off..off + cw as usize]);
        }

        VideoFrame {
            format: PixelFormat::Yuv420P,
            width: visible_w,
            height: visible_h,
            pts,
            time_base,
            planes: vec![
                VideoPlane {
                    stride: visible_w as usize,
                    data: y_out,
                },
                VideoPlane {
                    stride: cw as usize,
                    data: cb_out,
                },
                VideoPlane {
                    stride: cw as usize,
                    data: cr_out,
                },
            ],
        }
    }
}
