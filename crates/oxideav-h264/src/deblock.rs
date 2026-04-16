//! In-loop deblocking filter — ITU-T H.264 §8.7.
//!
//! This is a minimal implementation suitable for I-slice only output:
//! every edge between adjacent coded macroblocks (or 4×4 sub-blocks) is
//! checked, and when both sides carry coded residual data, a soft 4-tap
//! filter is applied. We honour `disable_deblocking_filter_idc` (skip the
//! whole pass when == 1) and the alpha/beta offsets carried by the slice
//! header, but skip the per-edge `bS == 0` case to keep the implementation
//! simple.
//!
//! Tables 8-16 and 8-17 from the spec are pre-computed below.

use crate::picture::Picture;
use crate::pps::Pps;
use crate::slice::SliceHeader;

// --- Table 8-16 — alpha/beta look-up indexed by IndexA / IndexB ---
const ALPHA: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 5, 6, 7, 8, 9, 10, 12, 13, 15, 17, 20,
    22, 25, 28, 32, 36, 40, 45, 50, 56, 63, 71, 80, 90, 101, 113, 127, 144, 162, 182, 203, 226,
    255, 255,
];
const BETA: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 6, 6, 7, 7, 8, 8,
    9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 18, 18,
];

// --- Table 8-17 — tC0[bS][IndexA] for bS in {1,2,3} ---
const TC0: [[u8; 52]; 3] = [
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 5, 6, 7, 8,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 5, 5, 6, 7, 8, 10,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1,
        1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 11, 13, 14, 16,
    ],
];

/// Apply deblocking to all edges in `pic`.
pub fn deblock_picture(pic: &mut Picture, _pps: &Pps, sh: &SliceHeader) {
    if sh.disable_deblocking_filter_idc == 1 {
        return;
    }
    let alpha_off = sh.slice_alpha_c0_offset_div2 * 2;
    let beta_off = sh.slice_beta_offset_div2 * 2;

    // Process each macroblock in raster order. For every 4×4 boundary that
    // lies on an edge between coded blocks (and isn't at a picture edge), we
    // pick a boundary strength and apply the 4-tap filter.
    let mb_w = pic.mb_width;
    let mb_h = pic.mb_height;
    for mb_y in 0..mb_h {
        for mb_x in 0..mb_w {
            // Vertical edges within / on the left side of the MB.
            for edge in 0..4 {
                let edge_x = (mb_x * 16) as usize + edge as usize * 4;
                if edge_x == 0 {
                    continue; // picture boundary
                }
                let bs = pick_bs(pic, mb_x, mb_y, edge, /*vertical=*/ true);
                if bs == 0 {
                    continue;
                }
                let qp_p = qp_at(pic, mb_x - if edge == 0 { 1 } else { 0 }, mb_y);
                let qp_q = qp_at(pic, mb_x, mb_y);
                let qp_avg = (qp_p + qp_q + 1) >> 1;
                filter_vertical_4(
                    pic,
                    edge_x,
                    (mb_y * 16) as usize,
                    bs,
                    qp_avg,
                    alpha_off,
                    beta_off,
                );
            }
            // Horizontal edges within / on the top side of the MB.
            for edge in 0..4 {
                let edge_y = (mb_y * 16) as usize + edge as usize * 4;
                if edge_y == 0 {
                    continue;
                }
                let bs = pick_bs(pic, mb_x, mb_y, edge, /*vertical=*/ false);
                if bs == 0 {
                    continue;
                }
                let qp_p = qp_at(pic, mb_x, mb_y - if edge == 0 { 1 } else { 0 });
                let qp_q = qp_at(pic, mb_x, mb_y);
                let qp_avg = (qp_p + qp_q + 1) >> 1;
                filter_horizontal_4(
                    pic,
                    (mb_x * 16) as usize,
                    edge_y,
                    bs,
                    qp_avg,
                    alpha_off,
                    beta_off,
                );
            }
        }
    }
}

fn qp_at(pic: &Picture, mb_x: u32, mb_y: u32) -> i32 {
    let info = pic.mb_info_at(mb_x, mb_y);
    info.qp_y
}

fn pick_bs(pic: &Picture, mb_x: u32, mb_y: u32, edge: u32, vertical: bool) -> u32 {
    // For an I-slice, edges between two macroblocks (edge == 0 here) have
    // bS = 4 (strong); internal edges (edge > 0) have bS = 3 when both
    // sides are intra. Per §8.7.2.
    if edge == 0 {
        return 4;
    }
    // Within-MB edge: nC > 0 on either side → bS = 2; else bS = 1 when both
    // intra (always true for I-slice); else bS = 0.
    let info = pic.mb_info_at(mb_x, mb_y);
    let (a_idx, b_idx) = if vertical {
        // Edge between sub-block columns (edge-1, edge): both at row r.
        // We test all 4 rows; use the maximum nC across them.
        let mut max_nc = 0u8;
        for r in 0..4 {
            max_nc = max_nc.max(info.luma_nc[r * 4 + edge as usize - 1]);
            max_nc = max_nc.max(info.luma_nc[r * 4 + edge as usize]);
        }
        (max_nc, 0u8)
    } else {
        let mut max_nc = 0u8;
        for c in 0..4 {
            max_nc = max_nc.max(info.luma_nc[(edge as usize - 1) * 4 + c]);
            max_nc = max_nc.max(info.luma_nc[edge as usize * 4 + c]);
        }
        (max_nc, 0u8)
    };
    let _ = b_idx;
    if a_idx > 0 {
        2
    } else {
        1
    }
}

fn filter_vertical_4(
    pic: &mut Picture,
    edge_x: usize,
    mb_y_pix: usize,
    bs: u32,
    qp_avg: i32,
    alpha_off: i32,
    beta_off: i32,
) {
    let stride = pic.luma_stride();
    let index_a = (qp_avg + alpha_off).clamp(0, 51) as usize;
    let index_b = (qp_avg + beta_off).clamp(0, 51) as usize;
    let alpha = ALPHA[index_a] as i32;
    let beta = BETA[index_b] as i32;

    for row_off in 0..4 {
        let row = mb_y_pix + row_off;
        if row >= pic.height as usize {
            break;
        }
        let p0_idx = row * stride + edge_x - 1;
        let p1_idx = row * stride + edge_x - 2;
        let p2_idx = row * stride + edge_x - 3;
        let p3_idx = row * stride + edge_x - 4;
        let q0_idx = row * stride + edge_x;
        let q1_idx = row * stride + edge_x + 1;
        let q2_idx = row * stride + edge_x + 2;
        let q3_idx = row * stride + edge_x + 3;
        // Bounds check on left/right.
        if p3_idx >= pic.y.len() || q3_idx >= pic.y.len() {
            continue;
        }
        let mut p = [
            pic.y[p0_idx] as i32,
            pic.y[p1_idx] as i32,
            pic.y[p2_idx] as i32,
            pic.y[p3_idx] as i32,
        ];
        let mut q = [
            pic.y[q0_idx] as i32,
            pic.y[q1_idx] as i32,
            pic.y[q2_idx] as i32,
            pic.y[q3_idx] as i32,
        ];

        if (p[0] - q[0]).abs() >= alpha
            || (p[1] - p[0]).abs() >= beta
            || (q[1] - q[0]).abs() >= beta
        {
            continue;
        }

        if bs == 4 {
            // Strong filter for bS=4 (§8.7.2.2).
            let ap = (p[2] - p[0]).abs() < beta;
            let aq = (q[2] - q[0]).abs() < beta;
            // p side
            if ap && (p[0] - q[0]).abs() < ((alpha >> 2) + 2) {
                let new_p0 = (p[2] + 2 * p[1] + 2 * p[0] + 2 * q[0] + q[1] + 4) >> 3;
                let new_p1 = (p[2] + p[1] + p[0] + q[0] + 2) >> 2;
                let new_p2 = (2 * p[3] + 3 * p[2] + p[1] + p[0] + q[0] + 4) >> 3;
                p[0] = new_p0;
                p[1] = new_p1;
                p[2] = new_p2;
            } else {
                p[0] = (2 * p[1] + p[0] + q[1] + 2) >> 2;
            }
            if aq && (p[0] - q[0]).abs() < ((alpha >> 2) + 2) {
                let new_q0 = (p[1] + 2 * p[0] + 2 * q[0] + 2 * q[1] + q[2] + 4) >> 3;
                let new_q1 = (p[0] + q[0] + q[1] + q[2] + 2) >> 2;
                let new_q2 = (2 * q[3] + 3 * q[2] + q[1] + q[0] + p[0] + 4) >> 3;
                q[0] = new_q0;
                q[1] = new_q1;
                q[2] = new_q2;
            } else {
                q[0] = (2 * q[1] + q[0] + p[1] + 2) >> 2;
            }
        } else {
            // bS in 1..=3 — normal filter.
            let tc0 = TC0[bs as usize - 1][index_a] as i32;
            let ap = (p[2] - p[0]).abs() < beta;
            let aq = (q[2] - q[0]).abs() < beta;
            let mut tc = tc0;
            if ap {
                tc += 1;
            }
            if aq {
                tc += 1;
            }
            let delta = (((q[0] - p[0]) << 2) + (p[1] - q[1]) + 4) >> 3;
            let delta = delta.clamp(-tc, tc);
            p[0] = (p[0] + delta).clamp(0, 255);
            q[0] = (q[0] - delta).clamp(0, 255);
            if ap {
                let dp = (p[2] + ((p[0] + q[0] + 1) >> 1) - 2 * p[1]) >> 1;
                p[1] = (p[1] + dp.clamp(-tc0, tc0)).clamp(0, 255);
            }
            if aq {
                let dq = (q[2] + ((p[0] + q[0] + 1) >> 1) - 2 * q[1]) >> 1;
                q[1] = (q[1] + dq.clamp(-tc0, tc0)).clamp(0, 255);
            }
        }
        pic.y[p0_idx] = p[0].clamp(0, 255) as u8;
        pic.y[p1_idx] = p[1].clamp(0, 255) as u8;
        pic.y[p2_idx] = p[2].clamp(0, 255) as u8;
        pic.y[q0_idx] = q[0].clamp(0, 255) as u8;
        pic.y[q1_idx] = q[1].clamp(0, 255) as u8;
        pic.y[q2_idx] = q[2].clamp(0, 255) as u8;
    }
}

fn filter_horizontal_4(
    pic: &mut Picture,
    mb_x_pix: usize,
    edge_y: usize,
    bs: u32,
    qp_avg: i32,
    alpha_off: i32,
    beta_off: i32,
) {
    let stride = pic.luma_stride();
    let index_a = (qp_avg + alpha_off).clamp(0, 51) as usize;
    let index_b = (qp_avg + beta_off).clamp(0, 51) as usize;
    let alpha = ALPHA[index_a] as i32;
    let beta = BETA[index_b] as i32;

    for col_off in 0..4 {
        let col = mb_x_pix + col_off;
        if col >= pic.width as usize {
            break;
        }
        let p0_idx = (edge_y - 1) * stride + col;
        let p1_idx = (edge_y - 2) * stride + col;
        let p2_idx = (edge_y - 3) * stride + col;
        let p3_idx = (edge_y - 4) * stride + col;
        let q0_idx = edge_y * stride + col;
        let q1_idx = (edge_y + 1) * stride + col;
        let q2_idx = (edge_y + 2) * stride + col;
        let q3_idx = (edge_y + 3) * stride + col;
        if q3_idx >= pic.y.len() {
            continue;
        }
        let mut p = [
            pic.y[p0_idx] as i32,
            pic.y[p1_idx] as i32,
            pic.y[p2_idx] as i32,
            pic.y[p3_idx] as i32,
        ];
        let mut q = [
            pic.y[q0_idx] as i32,
            pic.y[q1_idx] as i32,
            pic.y[q2_idx] as i32,
            pic.y[q3_idx] as i32,
        ];
        if (p[0] - q[0]).abs() >= alpha
            || (p[1] - p[0]).abs() >= beta
            || (q[1] - q[0]).abs() >= beta
        {
            continue;
        }
        if bs == 4 {
            let ap = (p[2] - p[0]).abs() < beta;
            let aq = (q[2] - q[0]).abs() < beta;
            if ap && (p[0] - q[0]).abs() < ((alpha >> 2) + 2) {
                let new_p0 = (p[2] + 2 * p[1] + 2 * p[0] + 2 * q[0] + q[1] + 4) >> 3;
                let new_p1 = (p[2] + p[1] + p[0] + q[0] + 2) >> 2;
                let new_p2 = (2 * p[3] + 3 * p[2] + p[1] + p[0] + q[0] + 4) >> 3;
                p[0] = new_p0;
                p[1] = new_p1;
                p[2] = new_p2;
            } else {
                p[0] = (2 * p[1] + p[0] + q[1] + 2) >> 2;
            }
            if aq && (p[0] - q[0]).abs() < ((alpha >> 2) + 2) {
                let new_q0 = (p[1] + 2 * p[0] + 2 * q[0] + 2 * q[1] + q[2] + 4) >> 3;
                let new_q1 = (p[0] + q[0] + q[1] + q[2] + 2) >> 2;
                let new_q2 = (2 * q[3] + 3 * q[2] + q[1] + q[0] + p[0] + 4) >> 3;
                q[0] = new_q0;
                q[1] = new_q1;
                q[2] = new_q2;
            } else {
                q[0] = (2 * q[1] + q[0] + p[1] + 2) >> 2;
            }
        } else {
            let tc0 = TC0[bs as usize - 1][index_a] as i32;
            let ap = (p[2] - p[0]).abs() < beta;
            let aq = (q[2] - q[0]).abs() < beta;
            let mut tc = tc0;
            if ap {
                tc += 1;
            }
            if aq {
                tc += 1;
            }
            let delta = (((q[0] - p[0]) << 2) + (p[1] - q[1]) + 4) >> 3;
            let delta = delta.clamp(-tc, tc);
            p[0] = (p[0] + delta).clamp(0, 255);
            q[0] = (q[0] - delta).clamp(0, 255);
            if ap {
                let dp = (p[2] + ((p[0] + q[0] + 1) >> 1) - 2 * p[1]) >> 1;
                p[1] = (p[1] + dp.clamp(-tc0, tc0)).clamp(0, 255);
            }
            if aq {
                let dq = (q[2] + ((p[0] + q[0] + 1) >> 1) - 2 * q[1]) >> 1;
                q[1] = (q[1] + dq.clamp(-tc0, tc0)).clamp(0, 255);
            }
        }
        pic.y[p0_idx] = p[0].clamp(0, 255) as u8;
        pic.y[p1_idx] = p[1].clamp(0, 255) as u8;
        pic.y[p2_idx] = p[2].clamp(0, 255) as u8;
        pic.y[q0_idx] = q[0].clamp(0, 255) as u8;
        pic.y[q1_idx] = q[1].clamp(0, 255) as u8;
        pic.y[q2_idx] = q[2].clamp(0, 255) as u8;
    }
}
