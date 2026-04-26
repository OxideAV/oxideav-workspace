//! Video format conversion helpers shared between output drivers.
//!
//! The heavy lifting (RGB↔YUV, chroma subsampling, Gray8, etc.) is
//! owned by [`oxideav_pixfmt`]; this module is just a thin wrapper
//! that produces the tight-stride `(Y, U, V)` tuple our wgpu / SDL
//! shaders expect.

use oxideav_core::{PixelFormat, VideoFrame};
use oxideav_pixfmt::{convert, ConvertOptions, FrameInfo};

/// Subsample any supported source to YUV420P plane data. Returns
/// `(Y, U, V)` with tight strides: `Y` = w×h, `U` = `V` = (w/2)×(h/2).
///
/// `src_format` / `src_width` / `src_height` describe the upstream
/// stream's shape (off `CodecParameters`) — the frame itself no
/// longer carries them.
///
/// If the input isn't already `Yuv420P` we delegate to `oxideav_pixfmt`
/// — that crate knows every conversion path the workspace supports.
/// Formats pixfmt can't handle fall back to a flat grey image so the
/// renderer never crashes on an exotic input.
pub fn to_yuv420p(
    frame: &VideoFrame,
    src_format: PixelFormat,
    src_width: u32,
    src_height: u32,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let w = src_width as usize;
    let h = src_height as usize;

    if src_format == PixelFormat::Yuv420P {
        return (
            plane_tight(&frame.planes[0].data, frame.planes[0].stride, w, h),
            plane_tight(&frame.planes[1].data, frame.planes[1].stride, w / 2, h / 2),
            plane_tight(&frame.planes[2].data, frame.planes[2].stride, w / 2, h / 2),
        );
    }

    let src_info = FrameInfo::new(src_format, src_width, src_height);
    match convert(frame, src_info, PixelFormat::Yuv420P, &ConvertOptions::default()) {
        Ok(conv) => (
            plane_tight(&conv.planes[0].data, conv.planes[0].stride, w, h),
            plane_tight(&conv.planes[1].data, conv.planes[1].stride, w / 2, h / 2),
            plane_tight(&conv.planes[2].data, conv.planes[2].stride, w / 2, h / 2),
        ),
        Err(_) => {
            let y = vec![128u8; w * h];
            let chroma = vec![128u8; (w / 2) * (h / 2)];
            (y, chroma.clone(), chroma)
        }
    }
}

pub fn plane_tight(src: &[u8], stride: usize, w: usize, h: usize) -> Vec<u8> {
    if stride == w {
        return src[..w * h.min(src.len() / stride.max(1))].to_vec();
    }
    let mut out = Vec::with_capacity(w * h);
    for row in 0..h {
        let off = row * stride;
        if off + w > src.len() {
            break;
        }
        out.extend_from_slice(&src[off..off + w]);
    }
    out
}
