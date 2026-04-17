//! Pal8 encode / decode.
//!
//! [`expand_to_rgb24`] and [`expand_to_rgba`] turn a Pal8 plane into
//! a packed RGB/RGBA buffer using a provided [`Palette`]. The encode
//! side, [`quantise_rgb24_to_pal8`] and [`quantise_rgba_to_pal8`],
//! uses a nearest-colour search (Euclidean distance in RGB) with
//! optional Floyd-Steinberg or Bayer dithering.

use crate::convert::Dither;
use crate::dither::{bayer8x8_offset, FloydSteinbergError};
use crate::palette::Palette;

/// Decode a Pal8 row into Rgb24. `indices` is one scanline of palette
/// indices; `rgb_out` must be `w * 3` bytes.
pub fn expand_row_to_rgb24(indices: &[u8], rgb_out: &mut [u8], palette: &Palette, w: usize) {
    for i in 0..w {
        let c = palette
            .colors
            .get(indices[i] as usize)
            .copied()
            .unwrap_or([0, 0, 0, 255]);
        rgb_out[i * 3] = c[0];
        rgb_out[i * 3 + 1] = c[1];
        rgb_out[i * 3 + 2] = c[2];
    }
}

/// Decode a Pal8 row into Rgba. `indices` is one scanline of palette
/// indices; `rgba_out` must be `w * 4` bytes.
pub fn expand_row_to_rgba(indices: &[u8], rgba_out: &mut [u8], palette: &Palette, w: usize) {
    for i in 0..w {
        let c = palette
            .colors
            .get(indices[i] as usize)
            .copied()
            .unwrap_or([0, 0, 0, 255]);
        rgba_out[i * 4] = c[0];
        rgba_out[i * 4 + 1] = c[1];
        rgba_out[i * 4 + 2] = c[2];
        rgba_out[i * 4 + 3] = c[3];
    }
}

/// Nearest palette entry to `(r, g, b)` by Euclidean distance.
pub fn nearest_index(palette: &Palette, r: u8, g: u8, b: u8) -> u8 {
    let mut best = 0u8;
    let mut best_d = i64::MAX;
    for (i, c) in palette.colors.iter().enumerate() {
        let dr = r as i64 - c[0] as i64;
        let dg = g as i64 - c[1] as i64;
        let db = b as i64 - c[2] as i64;
        let d = dr * dr + dg * dg + db * db;
        if d < best_d {
            best_d = d;
            best = i as u8;
        }
    }
    best
}

/// Quantise a tightly packed Rgb24 buffer into Pal8. `out.len()` must
/// be at least `w * h`.
pub fn quantise_rgb24_to_pal8(
    src: &[u8],
    out: &mut [u8],
    w: usize,
    h: usize,
    palette: &Palette,
    dither: Dither,
) {
    match dither {
        Dither::None => {
            for i in 0..w * h {
                out[i] = nearest_index(palette, src[i * 3], src[i * 3 + 1], src[i * 3 + 2]);
            }
        }
        Dither::Bayer8x8 => {
            for y in 0..h {
                for x in 0..w {
                    let off = (y * w + x) * 3;
                    let dr = bayer8x8_offset(x, y, 32.0);
                    let r = clamp((src[off] as f32 + dr) as i32);
                    let g = clamp((src[off + 1] as f32 + dr) as i32);
                    let b = clamp((src[off + 2] as f32 + dr) as i32);
                    out[y * w + x] = nearest_index(palette, r, g, b);
                }
            }
        }
        Dither::FloydSteinberg => {
            let mut err = FloydSteinbergError::new(w);
            for y in 0..h {
                for x in 0..w {
                    let off = (y * w + x) * 3;
                    let e = err.take(x);
                    let r = clamp((src[off] as f32 + e[0]) as i32);
                    let g = clamp((src[off + 1] as f32 + e[1]) as i32);
                    let b = clamp((src[off + 2] as f32 + e[2]) as i32);
                    let idx = nearest_index(palette, r, g, b);
                    out[y * w + x] = idx;
                    let chosen = palette.colors[idx as usize];
                    let residual = [
                        r as f32 - chosen[0] as f32,
                        g as f32 - chosen[1] as f32,
                        b as f32 - chosen[2] as f32,
                    ];
                    err.diffuse(x, residual);
                }
                err.advance_row();
            }
        }
    }
}

/// Quantise a tightly packed Rgba buffer into Pal8 (alpha is ignored
/// for the nearest-colour search).
pub fn quantise_rgba_to_pal8(
    src: &[u8],
    out: &mut [u8],
    w: usize,
    h: usize,
    palette: &Palette,
    dither: Dither,
) {
    // Strip alpha and reuse the rgb24 path. To avoid an extra buffer
    // allocation for small images we inline the loop here.
    match dither {
        Dither::None => {
            for i in 0..w * h {
                out[i] = nearest_index(palette, src[i * 4], src[i * 4 + 1], src[i * 4 + 2]);
            }
        }
        Dither::Bayer8x8 => {
            for y in 0..h {
                for x in 0..w {
                    let off = (y * w + x) * 4;
                    let dr = bayer8x8_offset(x, y, 32.0);
                    let r = clamp((src[off] as f32 + dr) as i32);
                    let g = clamp((src[off + 1] as f32 + dr) as i32);
                    let b = clamp((src[off + 2] as f32 + dr) as i32);
                    out[y * w + x] = nearest_index(palette, r, g, b);
                }
            }
        }
        Dither::FloydSteinberg => {
            let mut err = FloydSteinbergError::new(w);
            for y in 0..h {
                for x in 0..w {
                    let off = (y * w + x) * 4;
                    let e = err.take(x);
                    let r = clamp((src[off] as f32 + e[0]) as i32);
                    let g = clamp((src[off + 1] as f32 + e[1]) as i32);
                    let b = clamp((src[off + 2] as f32 + e[2]) as i32);
                    let idx = nearest_index(palette, r, g, b);
                    out[y * w + x] = idx;
                    let chosen = palette.colors[idx as usize];
                    let residual = [
                        r as f32 - chosen[0] as f32,
                        g as f32 - chosen[1] as f32,
                        b as f32 - chosen[2] as f32,
                    ];
                    err.diffuse(x, residual);
                }
                err.advance_row();
            }
        }
    }
}

#[inline]
fn clamp(v: i32) -> u8 {
    if v <= 0 {
        0
    } else if v >= 255 {
        255
    } else {
        v as u8
    }
}
