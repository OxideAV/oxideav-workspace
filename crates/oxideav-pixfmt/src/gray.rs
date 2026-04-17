//! Grayscale / mono conversions.
//!
//! The luma value is broadcast directly to RGB components when expanding
//! to a colour format — we do *not* apply any colour-space transfer
//! function here. For accurate luma from a colour source, call
//! [`crate::yuv::rgb_to_yuv`] and keep only the Y plane.

/// Gray8 → Rgb24 (broadcast the grey value to R, G, and B).
pub fn gray8_to_rgb24(src: &[u8], dst: &mut [u8], pixels: usize) {
    for i in 0..pixels {
        let v = src[i];
        dst[i * 3] = v;
        dst[i * 3 + 1] = v;
        dst[i * 3 + 2] = v;
    }
}

/// Gray8 → Rgba (broadcast grey; alpha = 255).
pub fn gray8_to_rgba(src: &[u8], dst: &mut [u8], pixels: usize) {
    for i in 0..pixels {
        let v = src[i];
        dst[i * 4] = v;
        dst[i * 4 + 1] = v;
        dst[i * 4 + 2] = v;
        dst[i * 4 + 3] = 255;
    }
}

/// Gray16Le → Gray8 (keep the high byte of each LE u16 — simple
/// truncation; matches what a naïve >> 8 would produce).
pub fn gray16le_to_gray8(src: &[u8], dst: &mut [u8], pixels: usize) {
    for i in 0..pixels {
        dst[i] = src[i * 2 + 1];
    }
}

/// Gray8 → Gray16Le (replicate byte into high and low halves so a
/// subsequent gray16 → gray8 round-trips to the original value).
pub fn gray8_to_gray16le(src: &[u8], dst: &mut [u8], pixels: usize) {
    for i in 0..pixels {
        let b = src[i];
        dst[i * 2] = b;
        dst[i * 2 + 1] = b;
    }
}

/// 1 bit per pixel (MSB-first) → Gray8. `black_is_zero = true` means
/// MonoBlack (0 bit = 0, 1 bit = 255). `false` means MonoWhite (0 bit
/// = 255, 1 bit = 0). The row stride on the source side is the packed
/// byte width (w + 7) / 8.
pub fn mono_to_gray8(src: &[u8], dst: &mut [u8], w: usize, h: usize, black_is_zero: bool) {
    let stride = w.div_ceil(8);
    for row in 0..h {
        for col in 0..w {
            let byte = src[row * stride + col / 8];
            let bit = (byte >> (7 - (col & 7))) & 1;
            let g = if bit == 1 { 255u8 } else { 0u8 };
            dst[row * w + col] = if black_is_zero { g } else { 255 - g };
        }
    }
}

/// Gray8 → 1 bpp (MSB-first). A threshold of 128 decides bit value.
pub fn gray8_to_mono(src: &[u8], dst: &mut [u8], w: usize, h: usize, black_is_zero: bool) {
    let stride = w.div_ceil(8);
    for b in dst.iter_mut() {
        *b = 0;
    }
    for row in 0..h {
        for col in 0..w {
            let g = src[row * w + col];
            let bit_on = if black_is_zero { g >= 128 } else { g < 128 };
            if bit_on {
                let shift = 7 - (col & 7);
                dst[row * stride + col / 8] |= 1u8 << shift;
            }
        }
    }
}
