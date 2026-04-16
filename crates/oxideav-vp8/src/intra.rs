//! Intra prediction for VP8 — RFC 6386 §16.
//!
//! This module exposes one function per prediction mode. Inputs are
//! pre-fetched neighbouring samples (the row immediately above and the
//! column immediately to the left of the predicted block, plus the
//! single corner pixel). Outputs are written into a caller-supplied
//! row-major buffer.
//!
//! All modes are clipped only at the I/O boundary — internal arithmetic
//! is i32.

use crate::tables::trees::{
    B_DC_PRED, B_HD_PRED, B_HE_PRED, B_HU_PRED, B_LD_PRED, B_RD_PRED, B_TM_PRED, B_VE_PRED,
    B_VL_PRED, B_VR_PRED, DC_PRED, H_PRED, TM_PRED, V_PRED,
};

#[inline]
fn clip255(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

#[inline]
fn avg2(a: u8, b: u8) -> u8 {
    (((a as i32) + (b as i32) + 1) >> 1) as u8
}

#[inline]
fn avg3(a: u8, b: u8, c: u8) -> u8 {
    (((a as i32) + 2 * (b as i32) + (c as i32) + 2) >> 2) as u8
}

// --- 16×16 luma intra prediction (RFC §16.1) -----------------------------

pub fn predict_16x16(
    mode: i32,
    above: Option<&[u8; 16]>,
    left: Option<&[u8; 16]>,
    tl: Option<u8>,
    out: &mut [u8],
    stride: usize,
) {
    match mode {
        DC_PRED => dc16(above, left, out, stride),
        V_PRED => v16(above, out, stride),
        H_PRED => h16(left, out, stride),
        TM_PRED => tm16(above, left, tl, out, stride),
        _ => dc16(above, left, out, stride), // B_PRED is handled per-block
    }
}

fn dc16(above: Option<&[u8; 16]>, left: Option<&[u8; 16]>, out: &mut [u8], stride: usize) {
    let mut sum = 0i32;
    let mut cnt = 0i32;
    if let Some(a) = above {
        for &p in a.iter() {
            sum += p as i32;
        }
        cnt += 16;
    }
    if let Some(l) = left {
        for &p in l.iter() {
            sum += p as i32;
        }
        cnt += 16;
    }
    let dc = if cnt == 0 {
        128
    } else if cnt == 16 {
        (sum + 8) >> 4
    } else {
        (sum + 16) >> 5
    } as u8;
    for j in 0..16 {
        for i in 0..16 {
            out[j * stride + i] = dc;
        }
    }
}

fn v16(above: Option<&[u8; 16]>, out: &mut [u8], stride: usize) {
    let row = above.copied().unwrap_or([127; 16]);
    for j in 0..16 {
        for i in 0..16 {
            out[j * stride + i] = row[i];
        }
    }
}

fn h16(left: Option<&[u8; 16]>, out: &mut [u8], stride: usize) {
    let col = left.copied().unwrap_or([129; 16]);
    for j in 0..16 {
        for i in 0..16 {
            out[j * stride + i] = col[j];
        }
    }
}

fn tm16(
    above: Option<&[u8; 16]>,
    left: Option<&[u8; 16]>,
    tl: Option<u8>,
    out: &mut [u8],
    stride: usize,
) {
    let row = above.copied().unwrap_or([127; 16]);
    let col = left.copied().unwrap_or([129; 16]);
    let p = tl.unwrap_or(127) as i32;
    for j in 0..16 {
        for i in 0..16 {
            out[j * stride + i] = clip255(col[j] as i32 + row[i] as i32 - p);
        }
    }
}

// --- 8×8 chroma intra prediction (same modes as 16×16, smaller block) ----

pub fn predict_8x8(
    mode: i32,
    above: Option<&[u8; 8]>,
    left: Option<&[u8; 8]>,
    tl: Option<u8>,
    out: &mut [u8],
    stride: usize,
) {
    match mode {
        DC_PRED => dc8(above, left, out, stride),
        V_PRED => v8(above, out, stride),
        H_PRED => h8(left, out, stride),
        TM_PRED => tm8(above, left, tl, out, stride),
        _ => dc8(above, left, out, stride),
    }
}

fn dc8(above: Option<&[u8; 8]>, left: Option<&[u8; 8]>, out: &mut [u8], stride: usize) {
    let mut sum = 0i32;
    let mut cnt = 0i32;
    if let Some(a) = above {
        for &p in a.iter() {
            sum += p as i32;
        }
        cnt += 8;
    }
    if let Some(l) = left {
        for &p in l.iter() {
            sum += p as i32;
        }
        cnt += 8;
    }
    let dc = if cnt == 0 {
        128
    } else if cnt == 8 {
        (sum + 4) >> 3
    } else {
        (sum + 8) >> 4
    } as u8;
    for j in 0..8 {
        for i in 0..8 {
            out[j * stride + i] = dc;
        }
    }
}

fn v8(above: Option<&[u8; 8]>, out: &mut [u8], stride: usize) {
    let row = above.copied().unwrap_or([127; 8]);
    for j in 0..8 {
        for i in 0..8 {
            out[j * stride + i] = row[i];
        }
    }
}

fn h8(left: Option<&[u8; 8]>, out: &mut [u8], stride: usize) {
    let col = left.copied().unwrap_or([129; 8]);
    for j in 0..8 {
        for i in 0..8 {
            out[j * stride + i] = col[j];
        }
    }
}

fn tm8(
    above: Option<&[u8; 8]>,
    left: Option<&[u8; 8]>,
    tl: Option<u8>,
    out: &mut [u8],
    stride: usize,
) {
    let row = above.copied().unwrap_or([127; 8]);
    let col = left.copied().unwrap_or([129; 8]);
    let p = tl.unwrap_or(127) as i32;
    for j in 0..8 {
        for i in 0..8 {
            out[j * stride + i] = clip255(col[j] as i32 + row[i] as i32 - p);
        }
    }
}

// --- 4×4 sub-block intra prediction (RFC §16.2) --------------------------

/// Neighbouring samples for 4×4 sub-block prediction.
///   a = `above[0..4]`     above-row (4 pixels)
///   ar = `above[4..8]`    above-row extension (4 more pixels — used by some modes)
///   l = `left[0..4]`      left-column (4 pixels)
///   tl = top-left corner pixel
#[derive(Clone, Copy)]
pub struct B4x4Neighbours {
    pub above: [u8; 8],
    pub left: [u8; 4],
    pub tl: u8,
}

pub fn predict_4x4(mode: i32, n: &B4x4Neighbours, out: &mut [u8], stride: usize) {
    let above = &n.above;
    let left = &n.left;
    let tl = n.tl;
    // Aliases matching the RFC 6386 §16.2 figures.
    let p0 = above[0];
    let p1 = above[1];
    let p2 = above[2];
    let p3 = above[3];
    let p4 = above[4];
    let p5 = above[5];
    let p6 = above[6];
    let p7 = above[7];
    let l0 = left[0];
    let l1 = left[1];
    let l2 = left[2];
    let l3 = left[3];
    let lp = tl;

    macro_rules! set {
        ($r:expr, $c:expr, $v:expr) => {
            out[$r * stride + $c] = $v;
        };
    }

    match mode {
        B_DC_PRED => {
            let sum = above[0..4].iter().map(|&v| v as i32).sum::<i32>()
                + left.iter().map(|&v| v as i32).sum::<i32>();
            let dc = ((sum + 4) >> 3) as u8;
            for r in 0..4 {
                for c in 0..4 {
                    set!(r, c, dc);
                }
            }
        }
        B_TM_PRED => {
            for r in 0..4 {
                for c in 0..4 {
                    set!(r, c, clip255(left[r] as i32 + above[c] as i32 - lp as i32));
                }
            }
        }
        B_VE_PRED => {
            // a = avg3(L, above[0], above[1]) at column 0
            let row = [
                avg3(lp, p0, p1),
                avg3(p0, p1, p2),
                avg3(p1, p2, p3),
                avg3(p2, p3, p4),
            ];
            for r in 0..4 {
                for c in 0..4 {
                    set!(r, c, row[c]);
                }
            }
        }
        B_HE_PRED => {
            let col = [
                avg3(lp, l0, l1),
                avg3(l0, l1, l2),
                avg3(l1, l2, l3),
                avg3(l2, l3, l3),
            ];
            for r in 0..4 {
                for c in 0..4 {
                    set!(r, c, col[r]);
                }
            }
        }
        B_LD_PRED => {
            // pixels along diagonal
            let pix = [
                avg3(p0, p1, p2),
                avg3(p1, p2, p3),
                avg3(p2, p3, p4),
                avg3(p3, p4, p5),
                avg3(p4, p5, p6),
                avg3(p5, p6, p7),
                avg3(p6, p7, p7),
            ];
            for r in 0..4 {
                for c in 0..4 {
                    set!(r, c, pix[r + c]);
                }
            }
        }
        B_RD_PRED => {
            // pixels along anti-diagonal
            let pix = [
                avg3(l3, l2, l1),
                avg3(l2, l1, l0),
                avg3(l1, l0, lp),
                avg3(l0, lp, p0),
                avg3(lp, p0, p1),
                avg3(p0, p1, p2),
                avg3(p1, p2, p3),
            ];
            for r in 0..4 {
                for c in 0..4 {
                    set!(r, c, pix[3 + c - r]);
                }
            }
        }
        B_VR_PRED => {
            // mode 6
            set!(0, 0, avg2(lp, p0));
            set!(0, 1, avg2(p0, p1));
            set!(0, 2, avg2(p1, p2));
            set!(0, 3, avg2(p2, p3));
            set!(1, 0, avg3(l0, lp, p0));
            set!(1, 1, avg3(lp, p0, p1));
            set!(1, 2, avg3(p0, p1, p2));
            set!(1, 3, avg3(p1, p2, p3));
            set!(2, 0, avg2(l0, lp));
            set!(2, 1, avg2(lp, p0));
            set!(2, 2, avg2(p0, p1));
            set!(2, 3, avg2(p1, p2));
            set!(3, 0, avg3(l1, l0, lp));
            set!(3, 1, avg3(l0, lp, p0));
            set!(3, 2, avg3(lp, p0, p1));
            set!(3, 3, avg3(p0, p1, p2));
        }
        B_VL_PRED => {
            // mode 7
            set!(0, 0, avg2(p0, p1));
            set!(0, 1, avg2(p1, p2));
            set!(0, 2, avg2(p2, p3));
            set!(0, 3, avg2(p3, p4));
            set!(1, 0, avg3(p0, p1, p2));
            set!(1, 1, avg3(p1, p2, p3));
            set!(1, 2, avg3(p2, p3, p4));
            set!(1, 3, avg3(p3, p4, p5));
            set!(2, 0, avg2(p1, p2));
            set!(2, 1, avg2(p2, p3));
            set!(2, 2, avg2(p3, p4));
            set!(2, 3, avg2(p4, p5));
            set!(3, 0, avg3(p1, p2, p3));
            set!(3, 1, avg3(p2, p3, p4));
            set!(3, 2, avg3(p3, p4, p5));
            set!(3, 3, avg3(p4, p5, p6));
        }
        B_HD_PRED => {
            set!(0, 0, avg2(l0, lp));
            set!(0, 1, avg3(l1, l0, lp));
            set!(0, 2, avg3(l0, lp, p0));
            set!(0, 3, avg3(lp, p0, p1));
            set!(1, 0, avg2(l1, l0));
            set!(1, 1, avg3(l2, l1, l0));
            set!(1, 2, avg3(l0, lp, p0));
            set!(1, 3, avg3(lp, p0, p1));
            set!(2, 0, avg2(l2, l1));
            set!(2, 1, avg3(l3, l2, l1));
            set!(2, 2, avg2(l1, l0));
            set!(2, 3, avg3(l1, l0, lp));
            set!(3, 0, avg2(l3, l2));
            set!(3, 1, avg3(l3, l2, l3));
            set!(3, 2, avg2(l2, l1));
            set!(3, 3, avg3(l2, l1, l0));
            // Adjust the duplicates per RFC 6386 §16.2 figure.
        }
        B_HU_PRED => {
            set!(0, 0, avg2(l0, l1));
            set!(0, 1, avg3(l0, l1, l2));
            set!(0, 2, avg2(l1, l2));
            set!(0, 3, avg3(l1, l2, l3));
            set!(1, 0, avg2(l1, l2));
            set!(1, 1, avg3(l1, l2, l3));
            set!(1, 2, avg2(l2, l3));
            set!(1, 3, avg3(l2, l3, l3));
            set!(2, 0, avg2(l2, l3));
            set!(2, 1, avg3(l2, l3, l3));
            set!(2, 2, l3);
            set!(2, 3, l3);
            set!(3, 0, l3);
            set!(3, 1, l3);
            set!(3, 2, l3);
            set!(3, 3, l3);
        }
        _ => {
            // Unknown — leave as DC fallback.
            let sum = above[0..4].iter().map(|&v| v as i32).sum::<i32>()
                + left.iter().map(|&v| v as i32).sum::<i32>();
            let dc = ((sum + 4) >> 3) as u8;
            for r in 0..4 {
                for c in 0..4 {
                    set!(r, c, dc);
                }
            }
        }
    }
}
