//! Top-level WebP decoder that coordinates VP8 lossy, VP8L lossless, and
//! the extended format's alpha/animation paths.
//!
//! The demuxer hands the decoder per-frame packets in an internal `OWEB`
//! envelope carrying the VP8/VP8L bytes + optional ALPH payload + a
//! render bbox against the canvas. The decoder:
//!
//! 1. Decodes the image chunk into a tile-sized RGBA buffer.
//!    * VP8 → run the keyframe through `oxideav-vp8`, then YUV→RGB convert.
//!    * VP8L → run [`crate::vp8l::decode`] directly (native RGBA).
//! 2. If an ALPH chunk is present, decodes it and overlays its alpha
//!    plane on the RGB tile.
//! 3. Composites the tile onto an internal RGBA canvas following the
//!    ANMF blend/disposal rules.
//! 4. Emits one `VideoFrame` per packet with the canvas state at that
//!    point in the animation.

use std::collections::VecDeque;

use oxideav_codec::Decoder;
use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, Packet, PixelFormat, Result, TimeBase, VideoFrame,
    VideoPlane,
};
use oxideav_vp8::decode_frame as decode_vp8_frame;

use crate::demux::{decode_frame_payload, DecodedAlph};
use crate::vp8l;

/// Public helper — decode an entire `.webp` file sitting in `buf` and
/// return all frames as RGBA `WebpFrame`s.
pub fn decode_webp(buf: &[u8]) -> Result<WebpImage> {
    let cursor = std::io::Cursor::new(buf.to_vec());
    let mut demuxer = crate::demux::open_boxed(Box::new(cursor))?;
    let mut frames = Vec::new();
    let streams = demuxer.streams().to_vec();
    let params = &streams[0].params;
    let w = params.width.unwrap_or(0);
    let h = params.height.unwrap_or(0);
    let mut dec = WebpDecoder::new(w, h);
    loop {
        match demuxer.next_packet() {
            Ok(pkt) => {
                let dur = pkt.duration.unwrap_or(0) as u32;
                dec.send_packet(&pkt)?;
                loop {
                    match dec.receive_frame() {
                        Ok(Frame::Video(vf)) => frames.push(WebpFrame {
                            width: vf.width,
                            height: vf.height,
                            duration_ms: dur,
                            rgba: vf.planes[0].data.clone(),
                        }),
                        Ok(_) => {}
                        Err(Error::NeedMore) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            Err(Error::Eof) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(WebpImage {
        width: w,
        height: h,
        frames,
    })
}

/// Convenience struct returned by [`decode_webp`].
#[derive(Debug, Clone)]
pub struct WebpImage {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<WebpFrame>,
}

#[derive(Debug, Clone)]
pub struct WebpFrame {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u32,
    pub rgba: Vec<u8>,
}

/// Factory used for the `webp_vp8l` codec id — decodes a bare VP8L
/// bitstream (no RIFF/WebP wrapper). Useful for consumers that have
/// already stripped the container and want the codec registry to give
/// them a standalone decoder.
pub fn make_vp8l_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    let w = params.width.unwrap_or(0);
    let h = params.height.unwrap_or(0);
    Ok(Box::new(Vp8lStandalone {
        codec_id: params.codec_id.clone(),
        width: w,
        height: h,
        queued: VecDeque::new(),
        pending_pts: None,
        pending_tb: TimeBase::new(1, 1000),
    }))
}

struct Vp8lStandalone {
    codec_id: CodecId,
    width: u32,
    height: u32,
    queued: VecDeque<VideoFrame>,
    pending_pts: Option<i64>,
    pending_tb: TimeBase,
}

impl Decoder for Vp8lStandalone {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        self.pending_pts = packet.pts;
        self.pending_tb = packet.time_base;
        let img = vp8l::decode(&packet.data)?;
        if self.width == 0 {
            self.width = img.width;
        }
        if self.height == 0 {
            self.height = img.height;
        }
        let rgba = img.to_rgba();
        let vf = VideoFrame {
            format: PixelFormat::Rgba,
            width: img.width,
            height: img.height,
            pts: self.pending_pts,
            time_base: self.pending_tb,
            planes: vec![VideoPlane {
                stride: (img.width as usize) * 4,
                data: rgba,
            }],
        };
        self.queued.push_back(vf);
        Ok(())
    }
    fn receive_frame(&mut self) -> Result<Frame> {
        self.queued
            .pop_front()
            .map(Frame::Video)
            .ok_or(Error::NeedMore)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

pub struct WebpDecoder {
    codec_id: CodecId,
    canvas_w: u32,
    canvas_h: u32,
    canvas: Vec<u8>, // RGBA
    queued: VecDeque<VideoFrame>,
    pending_pts: Option<i64>,
    pending_tb: TimeBase,
    first_frame: bool,
}

impl WebpDecoder {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            codec_id: CodecId::new(crate::demux::WEBP_CODEC_ID),
            canvas_w: w,
            canvas_h: h,
            canvas: vec![0; (w as usize) * (h as usize) * 4],
            queued: VecDeque::new(),
            pending_pts: None,
            pending_tb: TimeBase::new(1, 1000),
            first_frame: true,
        }
    }
}

impl Decoder for WebpDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        self.pending_pts = packet.pts;
        self.pending_tb = packet.time_base;
        let payload = decode_frame_payload(&packet.data)?;
        if self.canvas_w == 0 || self.canvas_h == 0 {
            self.canvas_w = payload.canvas.0;
            self.canvas_h = payload.canvas.1;
            self.canvas = vec![0; (self.canvas_w as usize) * (self.canvas_h as usize) * 4];
        }

        let _ = self.first_frame;

        // Decode the image chunk.
        let tile_rgba = if payload.is_vp8l {
            let img = vp8l::decode(payload.image)?;
            img.to_rgba()
        } else {
            decode_vp8_to_rgba(payload.image, payload.width, payload.height)?
        };

        // Apply alpha chunk, if present. VP8L already carries alpha in
        // its RGBA output. VP8 is RGB only, so ALPH (if any) overwrites
        // the alpha channel here.
        let tile_rgba = if let Some(alph) = &payload.alph {
            overlay_alpha(tile_rgba, payload.width, payload.height, alph)?
        } else if !payload.is_vp8l {
            // VP8 without ALPH: opaque image.
            set_alpha_opaque(tile_rgba)
        } else {
            tile_rgba
        };

        // Composite onto the canvas.
        composite(
            &mut self.canvas,
            self.canvas_w,
            self.canvas_h,
            &tile_rgba,
            payload.x_offset,
            payload.y_offset,
            payload.width,
            payload.height,
            payload.blend_with_previous,
        );

        let vf = VideoFrame {
            format: PixelFormat::Rgba,
            width: self.canvas_w,
            height: self.canvas_h,
            pts: self.pending_pts,
            time_base: self.pending_tb,
            planes: vec![VideoPlane {
                stride: (self.canvas_w as usize) * 4,
                data: self.canvas.clone(),
            }],
        };
        self.queued.push_back(vf);

        // Post-frame disposal.
        if payload.dispose_to_background {
            let x0 = payload.x_offset as usize;
            let y0 = payload.y_offset as usize;
            let x1 = (x0 + payload.width as usize).min(self.canvas_w as usize);
            let y1 = (y0 + payload.height as usize).min(self.canvas_h as usize);
            let w = self.canvas_w as usize;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 4;
                    self.canvas[i] = 0;
                    self.canvas[i + 1] = 0;
                    self.canvas[i + 2] = 0;
                    self.canvas[i + 3] = 0;
                }
            }
        }

        self.first_frame = false;
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        self.queued
            .pop_front()
            .map(Frame::Video)
            .ok_or(Error::NeedMore)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        // WebP animation composites each ANMF frame onto a persistent RGBA
        // canvas — that canvas IS the cross-frame state. Wiping it forces
        // the next frame to composite onto a clean background, matching
        // the semantics of starting playback from the seek target.
        // `first_frame` returns true so a post-seek frame with
        // blend_with_previous doesn't mix into the pre-seek canvas.
        if self.canvas_w > 0 && self.canvas_h > 0 {
            self.canvas = vec![0; (self.canvas_w as usize) * (self.canvas_h as usize) * 4];
        } else {
            self.canvas.clear();
        }
        self.queued.clear();
        self.pending_pts = None;
        self.first_frame = true;
        Ok(())
    }
}

fn decode_vp8_to_rgba(bytes: &[u8], frame_w: u32, frame_h: u32) -> Result<Vec<u8>> {
    let vf = decode_vp8_frame(bytes)?;
    if vf.format != PixelFormat::Yuv420P {
        return Err(Error::invalid("WebP: VP8 frame is not 4:2:0"));
    }
    let w = frame_w as usize;
    let h = frame_h as usize;
    // VP8 produces MB-aligned strides; take the top-left w×h region.
    let y = &vf.planes[0];
    let u = &vf.planes[1];
    let v = &vf.planes[2];
    let mut out = vec![0u8; w * h * 4];
    for j in 0..h {
        for i in 0..w {
            let y_val = y.data[j * y.stride + i] as i32;
            let u_val = u.data[(j / 2) * u.stride + (i / 2)] as i32 - 128;
            let v_val = v.data[(j / 2) * v.stride + (i / 2)] as i32 - 128;
            // BT.601 limited-range YUV→RGB (VP8 spec uses BT.601 coefs).
            let c = y_val - 16;
            let d = u_val;
            let e = v_val;
            let r = (298 * c + 409 * e + 128) >> 8;
            let g = (298 * c - 100 * d - 208 * e + 128) >> 8;
            let b = (298 * c + 516 * d + 128) >> 8;
            let idx = (j * w + i) * 4;
            out[idx] = r.clamp(0, 255) as u8;
            out[idx + 1] = g.clamp(0, 255) as u8;
            out[idx + 2] = b.clamp(0, 255) as u8;
            out[idx + 3] = 0xff;
        }
    }
    Ok(out)
}

fn set_alpha_opaque(mut rgba: Vec<u8>) -> Vec<u8> {
    for i in (3..rgba.len()).step_by(4) {
        rgba[i] = 0xff;
    }
    rgba
}

/// Decode an ALPH payload and overlay its alpha plane onto an RGBA tile.
///
/// Spec §5.2.3: the alpha plane can be raw (`compression=0`), filtered,
/// or VP8L-compressed (`compression=1`, carrying a stripped single-green
/// VP8L bitstream whose green channel holds the alpha values).
fn overlay_alpha(
    mut rgba: Vec<u8>,
    width: u32,
    height: u32,
    alph: &DecodedAlph<'_>,
) -> Result<Vec<u8>> {
    let alpha = decode_alpha_plane(width, height, alph)?;
    if alpha.len() != (width as usize) * (height as usize) {
        return Err(Error::invalid("WebP: alpha plane size mismatch"));
    }
    for (i, &a) in alpha.iter().enumerate() {
        rgba[i * 4 + 3] = a;
    }
    Ok(rgba)
}

fn decode_alpha_plane(width: u32, height: u32, alph: &DecodedAlph<'_>) -> Result<Vec<u8>> {
    let mut plane = match alph.compression {
        0 => alph.data.to_vec(),
        1 => {
            // VP8L wrapper: synthesise a VP8L header in front of the
            // payload so we can reuse `vp8l::decode`. The header has no
            // alpha bit and 0 version, and the signature byte 0x2f.
            let mut synth = Vec::with_capacity(alph.data.len() + 5);
            synth.push(0x2f);
            let w = width.saturating_sub(1) & 0x3fff;
            let h = height.saturating_sub(1) & 0x3fff;
            let packed = w | (h << 14);
            synth.extend_from_slice(&packed.to_le_bytes());
            synth.extend_from_slice(alph.data);
            let img = vp8l::decode(&synth)?;
            img.pixels.iter().map(|p| ((p >> 8) & 0xff) as u8).collect()
        }
        _ => return Err(Error::invalid("WebP: unknown ALPH compression")),
    };
    unfilter_alpha(&mut plane, width as usize, height as usize, alph.filtering);
    Ok(plane)
}

fn unfilter_alpha(plane: &mut [u8], w: usize, h: usize, filt: u8) {
    match filt {
        0 => {}
        1 => {
            // Horizontal.
            for y in 0..h {
                for x in 1..w {
                    let i = y * w + x;
                    let left = plane[i - 1] as u16;
                    plane[i] = ((plane[i] as u16 + left) & 0xff) as u8;
                }
            }
        }
        2 => {
            // Vertical.
            for y in 1..h {
                for x in 0..w {
                    let i = y * w + x;
                    let top = plane[i - w] as u16;
                    plane[i] = ((plane[i] as u16 + top) & 0xff) as u8;
                }
            }
        }
        3 => {
            // Gradient (Paeth-like: predictor = clip(L + T - TL)).
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    let pred = if y == 0 && x == 0 {
                        0
                    } else if y == 0 {
                        plane[i - 1] as i32
                    } else if x == 0 {
                        plane[i - w] as i32
                    } else {
                        let l = plane[i - 1] as i32;
                        let t = plane[i - w] as i32;
                        let tl = plane[i - w - 1] as i32;
                        (l + t - tl).clamp(0, 255)
                    };
                    plane[i] = ((plane[i] as i32 + pred) & 0xff) as u8;
                }
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn composite(
    canvas: &mut [u8],
    canvas_w: u32,
    canvas_h: u32,
    tile: &[u8],
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    blend: bool,
) {
    let cw = canvas_w as usize;
    for j in 0..h as usize {
        let cy = y as usize + j;
        if cy >= canvas_h as usize {
            break;
        }
        for i in 0..w as usize {
            let cx = x as usize + i;
            if cx >= canvas_w as usize {
                break;
            }
            let src = &tile[(j * w as usize + i) * 4..(j * w as usize + i) * 4 + 4];
            let dst_idx = (cy * cw + cx) * 4;
            if blend && src[3] < 0xff {
                let sa = src[3] as u32;
                let ia = 255 - sa;
                for c in 0..3 {
                    let s = src[c] as u32;
                    let d = canvas[dst_idx + c] as u32;
                    canvas[dst_idx + c] = ((s * sa + d * ia + 127) / 255) as u8;
                }
                let da = canvas[dst_idx + 3] as u32;
                canvas[dst_idx + 3] = (sa + ((da * ia) + 127) / 255) as u8;
            } else {
                canvas[dst_idx..dst_idx + 4].copy_from_slice(src);
            }
        }
    }
}
