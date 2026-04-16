//! 256-entry RGB lookup tables for Viridis and Magma colormaps.
//!
//! The full LUTs are derived at startup by linearly interpolating between 17
//! evenly-spaced control points sampled from Matplotlib's perceptually
//! uniform colormaps (originally released under CC0 by Stefan van der Walt
//! and Nathaniel Smith). At 8-bit precision the linearly interpolated
//! result is visually indistinguishable from the source tables.

use std::sync::OnceLock;

pub static VIRIDIS: ColormapLut = ColormapLut {
    cell: OnceLock::new(),
    points: &VIRIDIS_POINTS,
};
pub static MAGMA: ColormapLut = ColormapLut {
    cell: OnceLock::new(),
    points: &MAGMA_POINTS,
};

pub struct ColormapLut {
    cell: OnceLock<[(u8, u8, u8); 256]>,
    points: &'static [(u8, u8, u8)],
}

impl std::ops::Index<usize> for ColormapLut {
    type Output = (u8, u8, u8);
    fn index(&self, i: usize) -> &Self::Output {
        let lut = self.cell.get_or_init(|| build_lut(self.points));
        &lut[i]
    }
}

fn build_lut(points: &[(u8, u8, u8)]) -> [(u8, u8, u8); 256] {
    let mut out = [(0u8, 0u8, 0u8); 256];
    let n = points.len();
    let last = (n - 1) as f32;
    for (i, slot) in out.iter_mut().enumerate().take(256) {
        let t = (i as f32 / 255.0) * last;
        let lo = (t.floor() as usize).min(n - 1);
        let hi = (lo + 1).min(n - 1);
        let frac = t - lo as f32;
        let (lr, lg, lb) = points[lo];
        let (hr, hg, hb) = points[hi];
        let r = lr as f32 + (hr as f32 - lr as f32) * frac;
        let g = lg as f32 + (hg as f32 - lg as f32) * frac;
        let b = lb as f32 + (hb as f32 - lb as f32) * frac;
        *slot = (r.round() as u8, g.round() as u8, b.round() as u8);
    }
    out
}

/// Viridis control points — 17 evenly spaced samples (indices 0, 16, 32, …,
/// 256) of Matplotlib's `viridis` colormap.
const VIRIDIS_POINTS: [(u8, u8, u8); 17] = [
    (68, 1, 84),
    (72, 26, 108),
    (71, 47, 125),
    (65, 68, 135),
    (57, 86, 140),
    (49, 104, 142),
    (42, 120, 142),
    (35, 137, 141),
    (31, 154, 138),
    (34, 168, 132),
    (53, 183, 121),
    (84, 197, 104),
    (122, 209, 81),
    (165, 219, 54),
    (210, 226, 27),
    (248, 230, 33),
    (253, 231, 37),
];

/// Magma control points — 17 evenly spaced samples of Matplotlib's `magma`
/// colormap.
const MAGMA_POINTS: [(u8, u8, u8); 17] = [
    (0, 0, 4),
    (12, 8, 38),
    (28, 16, 68),
    (57, 15, 109),
    (86, 21, 124),
    (114, 31, 129),
    (140, 41, 129),
    (168, 50, 125),
    (196, 60, 117),
    (222, 73, 104),
    (241, 96, 93),
    (250, 127, 94),
    (254, 159, 109),
    (254, 191, 132),
    (253, 220, 163),
    (252, 246, 191),
    (252, 253, 191),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viridis_endpoints_are_dark_purple_and_yellow() {
        let lo = VIRIDIS[0];
        let hi = VIRIDIS[255];
        assert!(lo.2 > lo.0);
        assert!(hi.0 as u32 + hi.1 as u32 > 2 * hi.2 as u32);
    }

    #[test]
    fn magma_endpoints_are_black_and_pale() {
        let lo = MAGMA[0];
        let hi = MAGMA[255];
        assert!(lo.0 < 30 && lo.1 < 30);
        assert!(hi.0 > 200);
    }

    #[test]
    fn viridis_increases_in_brightness() {
        let lo = VIRIDIS[0];
        let hi = VIRIDIS[255];
        let sum_lo = lo.0 as u32 + lo.1 as u32 + lo.2 as u32;
        let sum_hi = hi.0 as u32 + hi.1 as u32 + hi.2 as u32;
        assert!(sum_hi > sum_lo);
    }

    #[test]
    fn lut_is_monotonic_brightness_for_viridis() {
        // Viridis is roughly monotonically increasing in luminance.
        let mut prev = 0i32;
        let mut decreases = 0;
        for i in 0..256 {
            let (r, g, b) = VIRIDIS[i];
            let lum = r as i32 + g as i32 + b as i32;
            if lum < prev - 5 {
                decreases += 1;
            }
            prev = lum;
        }
        assert!(decreases < 8, "too many luminance decreases: {}", decreases);
    }
}
