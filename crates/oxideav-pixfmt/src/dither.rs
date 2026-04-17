//! Dithering helpers used when down-quantising to a palette.
//!
//! Two strategies are offered:
//! - [`bayer8x8_offset`] returns the threshold offset for a given pixel
//!   coordinate in the classic 8×8 Bayer matrix, normalised to [−0.5,
//!   0.5] scaled by a configurable strength.
//! - [`FloydSteinbergError`] is a lightweight helper that distributes
//!   quantisation error across a 2-row sliding window, the way
//!   Floyd-Steinberg canonically does.

/// Normalised 8×8 Bayer matrix. Values are in [0, 63].
pub const BAYER8: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// Per-pixel Bayer threshold offset, in the range `[-strength/2,
/// strength/2]`. `strength` is the intensity span in 0..255 that the
/// dither noise should occupy; 32 is a good default for 6-bit quantised
/// output.
pub fn bayer8x8_offset(x: usize, y: usize, strength: f32) -> f32 {
    let m = BAYER8[y & 7][x & 7] as f32;
    // Normalise to (-0.5, +0.5), then scale.
    ((m + 0.5) / 64.0 - 0.5) * strength
}

/// Floyd-Steinberg error buffer. Carries the per-channel residuals for
/// the current row and the row beneath so that quantisation errors
/// propagate forwards and downwards.
///
/// The caller invokes [`FloydSteinbergError::take`] at `(x, y)` to
/// collect the error currently assigned to that pixel, quantises using
/// the sum, then [`FloydSteinbergError::diffuse`]s the resulting
/// residual back into the buffer.
pub struct FloydSteinbergError {
    w: usize,
    /// Current row — indexed by column. f32 for headroom.
    row_curr: Vec<[f32; 3]>,
    /// Next row.
    row_next: Vec<[f32; 3]>,
}

impl FloydSteinbergError {
    pub fn new(w: usize) -> Self {
        Self {
            w,
            row_curr: vec![[0.0; 3]; w],
            row_next: vec![[0.0; 3]; w],
        }
    }

    /// Call at the end of each row to slide the window down.
    pub fn advance_row(&mut self) {
        std::mem::swap(&mut self.row_curr, &mut self.row_next);
        for c in self.row_next.iter_mut() {
            *c = [0.0; 3];
        }
    }

    pub fn take(&self, x: usize) -> [f32; 3] {
        self.row_curr[x]
    }

    /// Distribute `err` (quantisation residual for pixel `x`) to the
    /// four Floyd-Steinberg neighbours:
    ///
    /// ```text
    ///          *   7/16
    ///  3/16  5/16  1/16
    /// ```
    pub fn diffuse(&mut self, x: usize, err: [f32; 3]) {
        let w = self.w;
        if x + 1 < w {
            for c in 0..3 {
                self.row_curr[x + 1][c] += err[c] * 7.0 / 16.0;
            }
        }
        if x > 0 {
            for c in 0..3 {
                self.row_next[x - 1][c] += err[c] * 3.0 / 16.0;
            }
        }
        for c in 0..3 {
            self.row_next[x][c] += err[c] * 5.0 / 16.0;
        }
        if x + 1 < w {
            for c in 0..3 {
                self.row_next[x + 1][c] += err[c] * 1.0 / 16.0;
            }
        }
    }
}
