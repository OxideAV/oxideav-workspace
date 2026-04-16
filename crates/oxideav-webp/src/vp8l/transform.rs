//! VP8L transforms — predictor, colour, subtract-green, colour-indexing.
//!
//! Each transform is parsed once from the bitstream and later applied in
//! reverse order during the final image assembly. The predictor and
//! colour transforms carry their own sub-image (a small tiled image of
//! transform parameters); colour-indexing carries a 1D palette; subtract-
//! green has no parameters.

use oxideav_core::{Error, Result};

use super::bit_reader::BitReader;
use super::decode_image_stream;

#[derive(Debug)]
pub enum Transform {
    Predictor {
        tile_bits: u32,
        sub_image: Vec<u32>,
        sub_w: u32,
        #[allow(dead_code)]
        sub_h: u32,
        xsize: u32,
    },
    Color {
        tile_bits: u32,
        sub_image: Vec<u32>,
        sub_w: u32,
        #[allow(dead_code)]
        sub_h: u32,
        xsize: u32,
    },
    SubtractGreen,
    ColorIndex {
        colors: Vec<u32>,
        bits_per_pixel: u32,
        orig_xsize: u32,
    },
}

impl Transform {
    pub fn read(br: &mut BitReader<'_>, xsize: u32, ysize: u32) -> Result<Self> {
        let ty = br.read_bits(2)?;
        match ty {
            0 => {
                // Predictor.
                let tile_bits = br.read_bits(3)? + 2;
                let sub_w = subsampled_size(xsize, tile_bits);
                let sub_h = subsampled_size(ysize, tile_bits);
                let sub = decode_image_stream(br, sub_w, sub_h, false)?;
                Ok(Transform::Predictor {
                    tile_bits,
                    sub_image: sub,
                    sub_w,
                    sub_h,
                    xsize,
                })
            }
            1 => {
                // Colour.
                let tile_bits = br.read_bits(3)? + 2;
                let sub_w = subsampled_size(xsize, tile_bits);
                let sub_h = subsampled_size(ysize, tile_bits);
                let sub = decode_image_stream(br, sub_w, sub_h, false)?;
                Ok(Transform::Color {
                    tile_bits,
                    sub_image: sub,
                    sub_w,
                    sub_h,
                    xsize,
                })
            }
            2 => Ok(Transform::SubtractGreen),
            3 => {
                // Colour indexing.
                let num_colors = br.read_bits(8)? + 1;
                let mut colors_raw = decode_image_stream(br, num_colors, 1, false)?;
                // Colour table is delta-coded along the row (each entry
                // differs from the previous by a per-channel value in
                // modulo 256 arithmetic).
                for i in 1..colors_raw.len() {
                    colors_raw[i] = add_argb(colors_raw[i], colors_raw[i - 1]);
                }
                let bits_per_pixel = if num_colors <= 2 {
                    1
                } else if num_colors <= 4 {
                    2
                } else if num_colors <= 16 {
                    4
                } else {
                    8
                };
                Ok(Transform::ColorIndex {
                    colors: colors_raw,
                    bits_per_pixel,
                    orig_xsize: xsize,
                })
            }
            _ => Err(Error::invalid("VP8L: invalid transform type")),
        }
    }

    /// Width of the image stream produced *after* this transform's parse
    /// step. Used while parsing subsequent transforms. For colour-
    /// indexing the pixel stream is packed: its width shrinks by the
    /// packing factor. Other transforms keep `default_w` unchanged —
    /// the caller passes the current xsize as the default.
    pub fn image_width_or_default(&self, default_w: u32) -> u32 {
        match self {
            Transform::ColorIndex {
                bits_per_pixel,
                orig_xsize,
                ..
            } => {
                let pack = 8 / *bits_per_pixel;
                (orig_xsize + pack - 1) / pack
            }
            _ => default_w,
        }
    }

    /// Width of the image after this transform is *applied* in the
    /// reverse pass. For colour-indexing it expands back to `orig_xsize`;
    /// every other transform is width-neutral.
    pub fn output_width(&self, input_w: u32) -> u32 {
        match self {
            Transform::ColorIndex { orig_xsize, .. } => *orig_xsize,
            _ => input_w,
        }
    }

    pub fn apply(&self, pixels: &[u32], width: u32, height: u32) -> Result<Vec<u32>> {
        match self {
            Transform::Predictor {
                tile_bits,
                sub_image,
                sub_w,
                ..
            } => Ok(apply_predictor(
                pixels, width, height, *tile_bits, sub_image, *sub_w,
            )),
            Transform::Color {
                tile_bits,
                sub_image,
                sub_w,
                ..
            } => Ok(apply_color_transform(
                pixels, width, height, *tile_bits, sub_image, *sub_w,
            )),
            Transform::SubtractGreen => Ok(apply_subtract_green(pixels)),
            Transform::ColorIndex {
                colors,
                bits_per_pixel,
                orig_xsize,
            } => apply_color_index(pixels, width, height, colors, *bits_per_pixel, *orig_xsize),
        }
    }
}

fn subsampled_size(size: u32, bits: u32) -> u32 {
    (size + (1 << bits) - 1) >> bits
}

/// ARGB addition per-component (modulo 256). Used by transforms that
/// encode residuals.
fn add_argb(a: u32, b: u32) -> u32 {
    let aa = (a >> 24) & 0xff;
    let ar = (a >> 16) & 0xff;
    let ag = (a >> 8) & 0xff;
    let ab = a & 0xff;
    let ba = (b >> 24) & 0xff;
    let br_ = (b >> 16) & 0xff;
    let bg = (b >> 8) & 0xff;
    let bb = b & 0xff;
    (((aa + ba) & 0xff) << 24)
        | (((ar + br_) & 0xff) << 16)
        | (((ag + bg) & 0xff) << 8)
        | ((ab + bb) & 0xff)
}

// ── Predictor transform ───────────────────────────────────────────────
//
// Each tile gets a predictor mode 0..13 from the sub-image's green
// channel. The decoded pixel is `pred + residual` per-component mod 256,
// where `pred` is computed from the already-decoded neighbourhood.

fn apply_predictor(
    residual: &[u32],
    width: u32,
    height: u32,
    tile_bits: u32,
    sub_image: &[u32],
    sub_w: u32,
) -> Vec<u32> {
    let mut out = residual.to_vec();
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let pred = if x == 0 && y == 0 {
                // Top-left: special-case to opaque black + implicit
                // alpha 0xff (per spec).
                0xff00_0000
            } else if y == 0 {
                // First row → use left neighbour.
                out[idx - 1]
            } else if x == 0 {
                // First column → use top neighbour.
                out[idx - width as usize]
            } else {
                let tx = (x >> tile_bits) as usize;
                let ty = (y >> tile_bits) as usize;
                let mode = (sub_image[ty * sub_w as usize + tx] >> 8) & 0x0f;
                predict_argb(&out, width as usize, x as usize, y as usize, mode)
            };
            out[idx] = add_argb(residual[idx], pred);
        }
    }
    out
}

fn predict_argb(out: &[u32], w: usize, x: usize, y: usize, mode: u32) -> u32 {
    let l = out[y * w + x - 1];
    let t = out[(y - 1) * w + x];
    let tl = out[(y - 1) * w + x - 1];
    let tr = if x + 1 < w {
        out[(y - 1) * w + x + 1]
    } else {
        // Spec: TR defaults to the left neighbour at image edge.
        out[y * w + x - 1]
    };
    match mode {
        0 => 0xff00_0000, // opaque black
        1 => l,
        2 => t,
        3 => tr,
        4 => tl,
        5 => avg3(l, tr, t),
        6 => avg2(l, tl),
        7 => avg2(l, t),
        8 => avg2(tl, t),
        9 => avg2(t, tr),
        10 => avg2(avg2(l, tl), avg2(t, tr)),
        11 => select_argb(l, t, tl),
        12 => clamp_add_sub_argb(l, t, tl),
        13 => clamp_add_sub_half_argb(avg2(l, t), tl),
        _ => 0xff00_0000,
    }
}

fn avg2(a: u32, b: u32) -> u32 {
    let mut out = 0u32;
    for c in 0..4 {
        let sh = c * 8;
        let av = (a >> sh) & 0xff;
        let bv = (b >> sh) & 0xff;
        out |= ((av + bv) >> 1) << sh;
    }
    out
}

fn avg3(a: u32, b: u32, c: u32) -> u32 {
    avg2(a, avg2(b, c))
}

fn select_argb(l: u32, t: u32, tl: u32) -> u32 {
    let mut out = 0u32;
    let mut dl = 0i32;
    let mut dt = 0i32;
    for c in 0..4 {
        let sh = c * 8;
        let lv = ((l >> sh) & 0xff) as i32;
        let tv = ((t >> sh) & 0xff) as i32;
        let tlv = ((tl >> sh) & 0xff) as i32;
        dl += (tv - tlv).abs();
        dt += (lv - tlv).abs();
    }
    for c in 0..4 {
        let sh = c * 8;
        let lv = (l >> sh) & 0xff;
        let tv = (t >> sh) & 0xff;
        let v = if dl < dt { lv } else { tv };
        out |= v << sh;
    }
    out
}

fn clamp_add_sub_argb(l: u32, t: u32, tl: u32) -> u32 {
    let mut out = 0u32;
    for c in 0..4 {
        let sh = c * 8;
        let lv = ((l >> sh) & 0xff) as i32;
        let tv = ((t >> sh) & 0xff) as i32;
        let tlv = ((tl >> sh) & 0xff) as i32;
        let v = (lv + tv - tlv).clamp(0, 255) as u32;
        out |= v << sh;
    }
    out
}

fn clamp_add_sub_half_argb(a: u32, b: u32) -> u32 {
    let mut out = 0u32;
    for c in 0..4 {
        let sh = c * 8;
        let av = ((a >> sh) & 0xff) as i32;
        let bv = ((b >> sh) & 0xff) as i32;
        let v = (av + (av - bv) / 2).clamp(0, 255) as u32;
        out |= v << sh;
    }
    out
}

// ── Colour transform ──────────────────────────────────────────────────
//
// Spec §4.2. Removes correlation between R/B channels by subtracting
// scaled versions of G and of (post-subtract) R.

fn apply_color_transform(
    pixels: &[u32],
    width: u32,
    height: u32,
    tile_bits: u32,
    sub_image: &[u32],
    sub_w: u32,
) -> Vec<u32> {
    let mut out = Vec::with_capacity(pixels.len());
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let p = pixels[idx];
            let tx = (x >> tile_bits) as usize;
            let ty = (y >> tile_bits) as usize;
            let coeffs = sub_image[ty * sub_w as usize + tx];
            // Coeff packing: A=0, R=green_to_red, G=green_to_blue, B=red_to_blue.
            let g2r = ((coeffs >> 16) & 0xff) as i8 as i32;
            let g2b = ((coeffs >> 8) & 0xff) as i8 as i32;
            let r2b = (coeffs & 0xff) as i8 as i32;

            let a = (p >> 24) & 0xff;
            let mut r = ((p >> 16) & 0xff) as i32;
            let g = ((p >> 8) & 0xff) as i32;
            let mut b = (p & 0xff) as i32;

            // g2r / g2b / r2b are sign-extended 8-bit values; per spec the
            // correction is `((coeff * sign_extend(green)) >> 5)`.
            r = (r + ((g2r * (g as i8 as i32)) >> 5)) & 0xff;
            b = (b + ((g2b * (g as i8 as i32)) >> 5)) & 0xff;
            b = (b + ((r2b * (r as i8 as i32)) >> 5)) & 0xff;

            let argb = (a << 24)
                | ((r as u32 & 0xff) << 16)
                | ((g as u32 & 0xff) << 8)
                | (b as u32 & 0xff);
            out.push(argb);
        }
    }
    out
}

// ── Subtract-green transform ──────────────────────────────────────────

fn apply_subtract_green(pixels: &[u32]) -> Vec<u32> {
    pixels
        .iter()
        .map(|&p| {
            let a = (p >> 24) & 0xff;
            let r = (p >> 16) & 0xff;
            let g = (p >> 8) & 0xff;
            let b = p & 0xff;
            (a << 24) | (((r + g) & 0xff) << 16) | (g << 8) | ((b + g) & 0xff)
        })
        .collect()
}

// ── Colour indexing transform ─────────────────────────────────────────
//
// The decoded pixel stream is an "index image": each pixel's green
// channel is an index into `colors`. When there are ≤16 colours the
// stream is bit-packed — `bits_per_pixel` indices per green byte.

fn apply_color_index(
    packed: &[u32],
    width: u32,
    _height: u32,
    colors: &[u32],
    bits_per_pixel: u32,
    orig_xsize: u32,
) -> Result<Vec<u32>> {
    let num_colors = colors.len() as u32;
    let pack = 8 / bits_per_pixel;
    let mask = (1u32 << bits_per_pixel) - 1;
    let rows = packed.len() / width as usize;
    let mut out = Vec::with_capacity((orig_xsize as usize) * rows.max(1));
    for y in 0..rows {
        for xp in 0..width as usize {
            let p = packed[y * width as usize + xp];
            let g = (p >> 8) & 0xff;
            for sub in 0..pack {
                let ox = xp * pack as usize + sub as usize;
                if ox >= orig_xsize as usize {
                    break;
                }
                let idx = (g >> (bits_per_pixel * sub)) & mask;
                let color = if idx < num_colors {
                    colors[idx as usize]
                } else {
                    0
                };
                out.push(color);
            }
        }
    }
    Ok(out)
}
