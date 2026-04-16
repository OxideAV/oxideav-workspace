//! VP8 loop filter — RFC 6386 §15.
//!
//! Only key-frame relevant pieces are wired up: the simple and normal
//! modes are both supported for MB- and sub-block-edge filtering. The
//! filter operates in place on the reconstructed YUV planes after
//! intra prediction + IDCT residue has been added.
//!
//! For an I-frame all macroblocks share `loop_filter_level` because
//! ref-frame deltas only matter for predictions involving inter / golden
//! / altref references; on keyframes they default to zero.

#[inline]
fn clamp(v: i32) -> i32 {
    v.clamp(-128, 127)
}

#[inline]
fn u_to_i(v: u8) -> i32 {
    v as i32 - 128
}

#[inline]
fn i_to_u(v: i32) -> u8 {
    (v + 128).clamp(0, 255) as u8
}

#[inline]
fn abs_diff(a: u8, b: u8) -> i32 {
    (a as i32 - b as i32).abs()
}

#[inline]
fn simple_threshold(p1: u8, p0: u8, q0: u8, q1: u8, edge_limit: i32) -> bool {
    let mask = (abs_diff(p0, q0) * 2 + abs_diff(p1, q1) / 2) <= edge_limit;
    mask
}

#[inline]
fn normal_threshold(
    p3: u8,
    p2: u8,
    p1: u8,
    p0: u8,
    q0: u8,
    q1: u8,
    q2: u8,
    q3: u8,
    edge_limit: i32,
    interior_limit: i32,
) -> bool {
    if !simple_threshold(p1, p0, q0, q1, edge_limit) {
        return false;
    }
    abs_diff(p3, p2) <= interior_limit
        && abs_diff(p2, p1) <= interior_limit
        && abs_diff(p1, p0) <= interior_limit
        && abs_diff(q1, q0) <= interior_limit
        && abs_diff(q2, q1) <= interior_limit
        && abs_diff(q3, q2) <= interior_limit
}

#[inline]
fn high_edge_variance(p1: u8, p0: u8, q0: u8, q1: u8, hev: i32) -> bool {
    abs_diff(p1, p0) > hev || abs_diff(q1, q0) > hev
}

/// Simple-mode 4-tap filter on a single edge crossing (`p1 p0 | q0 q1`).
fn simple_filter(p1: u8, p0: u8, q0: u8, q1: u8) -> (u8, u8) {
    let p0i = u_to_i(p0);
    let q0i = u_to_i(q0);
    let p1i = u_to_i(p1);
    let q1i = u_to_i(q1);
    let mut a = 3 * (q0i - p0i);
    a += clamp(p1i - q1i);
    a = clamp(a);
    let b = clamp(a + 3) >> 3;
    let a = clamp(a + 4) >> 3;
    let new_q0 = i_to_u(q0i - a);
    let new_p0 = i_to_u(p0i + b);
    (new_p0, new_q0)
}

/// Normal-mode filter (RFC §15.4) — adjusts up to 3 px on each side
/// depending on HEV / interior masks.
fn normal_filter(
    p2: u8,
    p1: u8,
    p0: u8,
    q0: u8,
    q1: u8,
    q2: u8,
    is_mb_edge: bool,
    hev_threshold: i32,
) -> (u8, u8, u8, u8, u8, u8) {
    let hev = high_edge_variance(p1, p0, q0, q1, hev_threshold);
    let p0i = u_to_i(p0);
    let q0i = u_to_i(q0);
    let p1i = u_to_i(p1);
    let q1i = u_to_i(q1);
    let p2i = u_to_i(p2);
    let q2i = u_to_i(q2);

    let mut a = clamp(p1i - q1i);
    if !hev {
        // No HEV: use the smoothing branch.
        a = 0;
    }
    let mut a = clamp(3 * (q0i - p0i) + a);

    if is_mb_edge && !hev {
        // Subblock-edge full-mb-edge smoothing per §15.4 (b)+ (c)
        let w = clamp(p1i - q1i + 3 * (q0i - p0i));
        let a3 = clamp(27 * w + 63) >> 7;
        let a2 = clamp(18 * w + 63) >> 7;
        let a1 = clamp(9 * w + 63) >> 7;
        let new_p0 = i_to_u(p0i + a1);
        let new_q0 = i_to_u(q0i - a1);
        let new_p1 = i_to_u(p1i + a2);
        let new_q1 = i_to_u(q1i - a2);
        let new_p2 = i_to_u(p2i + a3);
        let new_q2 = i_to_u(q2i - a3);
        return (new_p2, new_p1, new_p0, new_q0, new_q1, new_q2);
    }

    let b = clamp(a + 3) >> 3;
    a = clamp(a + 4) >> 3;
    let new_q0 = i_to_u(q0i - a);
    let new_p0 = i_to_u(p0i + b);

    let (new_p1, new_q1) = if !hev {
        let a2 = (a + 1) >> 1;
        (i_to_u(p1i + a2), i_to_u(q1i - a2))
    } else {
        (p1, q1)
    };

    (p2, new_p1, new_p0, new_q0, new_q1, q2)
}

/// Filter `level` parameters helper — derives the three thresholds
/// needed by §15.2.
#[derive(Clone, Copy, Debug)]
pub struct FilterParams {
    pub edge_limit: i32,
    pub interior_limit: i32,
    pub hev_threshold: i32,
}

impl FilterParams {
    pub fn for_mb(level: u8, sharpness: u8, mb_edge: bool) -> Self {
        let l = level as i32;
        let mut interior = (l >> 2).max(1);
        if sharpness > 0 {
            interior >>= 1;
            if sharpness > 4 {
                interior >>= 1;
            }
        }
        if interior < 1 {
            interior = 1;
        }
        let mut edge = if mb_edge { 2 * l + interior } else { l };
        let max_edge = 9 - sharpness as i32;
        if edge > max_edge {
            edge = max_edge;
        }
        let hev = if l < 15 {
            0
        } else if l < 36 {
            1
        } else {
            2
        };
        Self {
            edge_limit: edge,
            interior_limit: interior,
            hev_threshold: hev,
        }
    }
}

/// Apply the simple-mode loop filter to a MB-edge column at `(x, y)`
/// with `width × height` boundary. `simple` mode only filters the four
/// pixels closest to the edge.
pub fn filter_simple_vertical(
    plane: &mut [u8],
    stride: usize,
    x: usize,
    width: usize,
    height: usize,
    params: FilterParams,
) {
    if x < 2 || x + 2 > width {
        return;
    }
    for j in 0..height {
        let row = j * stride;
        let p1 = plane[row + x - 2];
        let p0 = plane[row + x - 1];
        let q0 = plane[row + x];
        let q1 = plane[row + x + 1];
        if simple_threshold(p1, p0, q0, q1, params.edge_limit) {
            let (np0, nq0) = simple_filter(p1, p0, q0, q1);
            plane[row + x - 1] = np0;
            plane[row + x] = nq0;
        }
    }
}

/// Apply the simple-mode loop filter to a MB-edge row at `(x, y)`.
pub fn filter_simple_horizontal(
    plane: &mut [u8],
    stride: usize,
    y: usize,
    width: usize,
    height: usize,
    params: FilterParams,
) {
    if y < 2 || y + 2 > height {
        return;
    }
    for i in 0..width {
        let p1 = plane[(y - 2) * stride + i];
        let p0 = plane[(y - 1) * stride + i];
        let q0 = plane[y * stride + i];
        let q1 = plane[(y + 1) * stride + i];
        if simple_threshold(p1, p0, q0, q1, params.edge_limit) {
            let (np0, nq0) = simple_filter(p1, p0, q0, q1);
            plane[(y - 1) * stride + i] = np0;
            plane[y * stride + i] = nq0;
        }
    }
}

/// Apply the normal-mode loop filter to a vertical edge.
pub fn filter_normal_vertical(
    plane: &mut [u8],
    stride: usize,
    x: usize,
    width: usize,
    height: usize,
    params: FilterParams,
    is_mb_edge: bool,
) {
    if x < 4 || x + 4 > width {
        return;
    }
    for j in 0..height {
        let row = j * stride;
        let p3 = plane[row + x - 4];
        let p2 = plane[row + x - 3];
        let p1 = plane[row + x - 2];
        let p0 = plane[row + x - 1];
        let q0 = plane[row + x];
        let q1 = plane[row + x + 1];
        let q2 = plane[row + x + 2];
        let q3 = plane[row + x + 3];
        if !normal_threshold(
            p3,
            p2,
            p1,
            p0,
            q0,
            q1,
            q2,
            q3,
            params.edge_limit,
            params.interior_limit,
        ) {
            continue;
        }
        let (np2, np1, np0, nq0, nq1, nq2) =
            normal_filter(p2, p1, p0, q0, q1, q2, is_mb_edge, params.hev_threshold);
        plane[row + x - 3] = np2;
        plane[row + x - 2] = np1;
        plane[row + x - 1] = np0;
        plane[row + x] = nq0;
        plane[row + x + 1] = nq1;
        plane[row + x + 2] = nq2;
    }
}

/// Apply the normal-mode loop filter to a horizontal edge.
pub fn filter_normal_horizontal(
    plane: &mut [u8],
    stride: usize,
    y: usize,
    width: usize,
    height: usize,
    params: FilterParams,
    is_mb_edge: bool,
) {
    if y < 4 || y + 4 > height {
        return;
    }
    for i in 0..width {
        let p3 = plane[(y - 4) * stride + i];
        let p2 = plane[(y - 3) * stride + i];
        let p1 = plane[(y - 2) * stride + i];
        let p0 = plane[(y - 1) * stride + i];
        let q0 = plane[y * stride + i];
        let q1 = plane[(y + 1) * stride + i];
        let q2 = plane[(y + 2) * stride + i];
        let q3 = plane[(y + 3) * stride + i];
        if !normal_threshold(
            p3,
            p2,
            p1,
            p0,
            q0,
            q1,
            q2,
            q3,
            params.edge_limit,
            params.interior_limit,
        ) {
            continue;
        }
        let (np2, np1, np0, nq0, nq1, nq2) =
            normal_filter(p2, p1, p0, q0, q1, q2, is_mb_edge, params.hev_threshold);
        plane[(y - 3) * stride + i] = np2;
        plane[(y - 2) * stride + i] = np1;
        plane[(y - 1) * stride + i] = np0;
        plane[y * stride + i] = nq0;
        plane[(y + 1) * stride + i] = nq1;
        plane[(y + 2) * stride + i] = nq2;
    }
}
