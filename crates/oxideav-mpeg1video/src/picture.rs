//! Picture-level assembly buffers (Y / Cb / Cr) + display-order reorder.

use oxideav_core::frame::VideoPlane;
use oxideav_core::{PixelFormat, TimeBase, VideoFrame};

use crate::headers::PictureType;

/// Allocate per-picture YUV buffers sized to the macroblock-aligned image.
#[derive(Clone)]
pub struct PictureBuffer {
    pub width: usize,
    pub height: usize,
    pub mb_width: usize,
    pub mb_height: usize,
    pub y: Vec<u8>,
    pub cb: Vec<u8>,
    pub cr: Vec<u8>,
    pub y_stride: usize,
    pub c_stride: usize,
    pub picture_type: PictureType,
    pub temporal_reference: u16,
    /// Display-order PTS computed at decode time (so the value is stable
    /// across GOP anchor roll-overs).
    pub display_pts: Option<i64>,
}

impl PictureBuffer {
    pub fn new(width: usize, height: usize, picture_type: PictureType, tr: u16) -> Self {
        let mb_w = width.div_ceil(16);
        let mb_h = height.div_ceil(16);
        let y_stride = mb_w * 16;
        let c_stride = mb_w * 8;
        let y_h = mb_h * 16;
        let c_h = mb_h * 8;
        Self {
            width,
            height,
            mb_width: mb_w,
            mb_height: mb_h,
            y: vec![0u8; y_stride * y_h],
            cb: vec![0u8; c_stride * c_h],
            cr: vec![0u8; c_stride * c_h],
            y_stride,
            c_stride,
            picture_type,
            temporal_reference: tr,
            display_pts: None,
        }
    }

    /// Copy the MB-aligned luma / chroma buffers into a tight `VideoFrame`
    /// with no padding.
    pub fn to_video_frame(&self, pts: Option<i64>, time_base: TimeBase) -> VideoFrame {
        let w = self.width;
        let h = self.height;
        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let mut y = vec![0u8; w * h];
        for row in 0..h {
            y[row * w..row * w + w]
                .copy_from_slice(&self.y[row * self.y_stride..row * self.y_stride + w]);
        }
        let mut cb = vec![0u8; cw * ch];
        let mut cr = vec![0u8; cw * ch];
        for row in 0..ch {
            cb[row * cw..row * cw + cw]
                .copy_from_slice(&self.cb[row * self.c_stride..row * self.c_stride + cw]);
            cr[row * cw..row * cw + cw]
                .copy_from_slice(&self.cr[row * self.c_stride..row * self.c_stride + cw]);
        }
        VideoFrame {
            format: PixelFormat::Yuv420P,
            width: w as u32,
            height: h as u32,
            pts,
            time_base,
            planes: vec![
                VideoPlane { stride: w, data: y },
                VideoPlane {
                    stride: cw,
                    data: cb,
                },
                VideoPlane {
                    stride: cw,
                    data: cr,
                },
            ],
        }
    }
}

/// Manages the two reference pictures needed for P/B decode and the B-frame
/// reorder buffer.
///
/// MPEG-1 decoding semantics:
///   * I/P pictures are reference pictures. Each new I/P replaces the
///     older of the two references (sliding window of size 2).
///   * B pictures are never used as references. They are decoded after
///     the anchor they depend on (the "future" reference), so display
///     order re-orders them: an I/P "sandwich" holding between them.
///   * On decode, `prev_ref` is the forward anchor and `next_ref` is the
///     backward anchor. A B picture uses both; a P picture uses only
///     `next_ref` (which, for the first P after an I, equals `prev_ref`
///     at the point the P is decoded — conceptually the previous anchor).
#[derive(Default)]
pub struct ReferenceManager {
    /// The reference picture that appeared earliest in decode order and
    /// still has pending B pictures between it and the next anchor.
    pub prev_ref: Option<PictureBuffer>,
    /// The most recently decoded I/P picture (used as backward reference
    /// for B pictures that were decoded before but displayed before it).
    pub next_ref: Option<PictureBuffer>,
}

impl ReferenceManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called after an I or P picture is fully decoded. Rotate the sliding
    /// window: old `next_ref` → `prev_ref`, new picture → `next_ref`. The
    /// previous `prev_ref` is dropped (its display was already emitted when
    /// it was rotated into that slot).
    ///
    /// Returns a clone of the picture that just moved from `next_ref` to
    /// `prev_ref` — the caller emits it now, since by MPEG-1 decode order
    /// no further B-pictures reference it as a backward anchor and all
    /// B-pictures that reference it as a forward anchor have just been
    /// queued (they are decoded between two anchors and emitted immediately).
    pub fn push_anchor(&mut self, pic: PictureBuffer) -> Option<PictureBuffer> {
        let ready_for_display = self.next_ref.clone();
        // Discard the now-unused forward anchor.
        self.prev_ref = self.next_ref.take();
        self.next_ref = Some(pic);
        ready_for_display
    }

    /// Consume the final reference picture on flush (`next_ref` — the
    /// most recently decoded anchor that no subsequent push has moved
    /// into display-ready state). `prev_ref` has already been emitted at
    /// rotation time.
    pub fn drain(&mut self) -> Vec<PictureBuffer> {
        let mut out = Vec::new();
        self.prev_ref.take();
        if let Some(p) = self.next_ref.take() {
            out.push(p);
        }
        out
    }

    pub fn forward(&self) -> Option<&PictureBuffer> {
        self.prev_ref.as_ref()
    }

    pub fn backward(&self) -> Option<&PictureBuffer> {
        self.next_ref.as_ref()
    }
}
