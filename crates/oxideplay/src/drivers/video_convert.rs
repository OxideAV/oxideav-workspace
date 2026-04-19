//! Video format conversion helpers shared between output drivers.

use oxideav_core::{PixelFormat, VideoFrame, VideoPlane};

/// Subsample any supported planar source to YUV420P plane data.
/// Returns `(Y, U, V)` with tight strides: `Y` = w×h, `U` = `V` = (w/2)×(h/2).
/// Unknown or non-YUV formats fall back to a flat grey image.
pub fn to_yuv420p(frame: &VideoFrame) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let w = frame.width as usize;
    let h = frame.height as usize;
    match frame.format {
        PixelFormat::Yuv420P => {
            let y = plane_tight(&frame.planes[0].data, frame.planes[0].stride, w, h);
            let u = plane_tight(&frame.planes[1].data, frame.planes[1].stride, w / 2, h / 2);
            let v = plane_tight(&frame.planes[2].data, frame.planes[2].stride, w / 2, h / 2);
            (y, u, v)
        }
        PixelFormat::Yuv422P => {
            let y = plane_tight(&frame.planes[0].data, frame.planes[0].stride, w, h);
            let u_src = &frame.planes[1];
            let v_src = &frame.planes[2];
            let u = downsample_vertical(u_src, w / 2, h);
            let v = downsample_vertical(v_src, w / 2, h);
            (y, u, v)
        }
        PixelFormat::Yuv444P => {
            let y = plane_tight(&frame.planes[0].data, frame.planes[0].stride, w, h);
            let u = downsample_2x2(&frame.planes[1], w, h);
            let v = downsample_2x2(&frame.planes[2], w, h);
            (y, u, v)
        }
        PixelFormat::Gray8 => {
            let y = plane_tight(&frame.planes[0].data, frame.planes[0].stride, w, h);
            let chroma = vec![128u8; (w / 2) * (h / 2)];
            (y, chroma.clone(), chroma)
        }
        _ => {
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

fn downsample_vertical(plane: &VideoPlane, out_w: usize, in_h: usize) -> Vec<u8> {
    let out_h = in_h / 2;
    let mut out = Vec::with_capacity(out_w * out_h);
    for row in 0..out_h {
        let src_row = row * 2;
        let off = src_row * plane.stride;
        if off + out_w > plane.data.len() {
            break;
        }
        out.extend_from_slice(&plane.data[off..off + out_w]);
    }
    out
}

fn downsample_2x2(plane: &VideoPlane, in_w: usize, in_h: usize) -> Vec<u8> {
    let out_w = in_w / 2;
    let out_h = in_h / 2;
    let mut out = Vec::with_capacity(out_w * out_h);
    for row in 0..out_h {
        let src_row = row * 2;
        let off = src_row * plane.stride;
        if off + in_w > plane.data.len() {
            break;
        }
        for col in 0..out_w {
            let src_col = col * 2;
            out.push(plane.data[off + src_col]);
        }
    }
    out
}
