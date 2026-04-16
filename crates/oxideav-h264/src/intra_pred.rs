//! Intra prediction — ITU-T H.264 §8.3.
//!
//! Three families:
//! * Intra_4×4 (§8.3.1) — 9 modes, per 4×4 luma sub-block.
//! * Intra_16×16 (§8.3.3) — 4 modes (Vertical / Horizontal / DC / Plane).
//! * Intra chroma 8×8 (§8.3.4) — 4 modes (DC / Horizontal / Vertical / Plane).
//!
//! Functions take samples from neighbouring rows / columns and write the
//! predicted block. They never touch the bitstream — entropy / mode decode
//! happens in `mb.rs`.

// Indices into `top` for an Intra_4×4 prediction. The 4×4 block needs
// `top[0..=7]` plus `top_left` plus `left[0..=3]`.

/// Intra_4×4 prediction modes (§8.3.1.1, Table 8-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Intra4x4Mode {
    Vertical = 0,
    Horizontal = 1,
    Dc = 2,
    DiagonalDownLeft = 3,
    DiagonalDownRight = 4,
    VerticalRight = 5,
    HorizontalDown = 6,
    VerticalLeft = 7,
    HorizontalUp = 8,
}

impl Intra4x4Mode {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Vertical),
            1 => Some(Self::Horizontal),
            2 => Some(Self::Dc),
            3 => Some(Self::DiagonalDownLeft),
            4 => Some(Self::DiagonalDownRight),
            5 => Some(Self::VerticalRight),
            6 => Some(Self::HorizontalDown),
            7 => Some(Self::VerticalLeft),
            8 => Some(Self::HorizontalUp),
            _ => None,
        }
    }
}

/// Intra_16×16 prediction modes (§8.3.3, Table 8-4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Intra16x16Mode {
    Vertical = 0,
    Horizontal = 1,
    Dc = 2,
    Plane = 3,
}

impl Intra16x16Mode {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Vertical),
            1 => Some(Self::Horizontal),
            2 => Some(Self::Dc),
            3 => Some(Self::Plane),
            _ => None,
        }
    }
}

/// Chroma intra prediction modes (§8.3.4, Table 8-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum IntraChromaMode {
    Dc = 0,
    Horizontal = 1,
    Vertical = 2,
    Plane = 3,
}

impl IntraChromaMode {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Dc),
            1 => Some(Self::Horizontal),
            2 => Some(Self::Vertical),
            3 => Some(Self::Plane),
            _ => None,
        }
    }
}

/// Neighbour samples for an Intra_4×4 block (§8.3.1.2, Figure 8-7).
///
/// `top[0..=7]` = the row above the block (8 samples, used by diagonal
/// modes that consume the upper-right neighbour). When the block sits at
/// the right edge of a macroblock, the upper-right neighbour might be
/// unavailable — callers should populate `top[4..=7]` by replicating
/// `top[3]` per §8.3.1.2.1.
///
/// `left[0..=3]` = the column to the left.
/// `top_left` = the corner sample at (-1, -1).
#[derive(Clone, Debug)]
pub struct Intra4x4Neighbours {
    pub top: [u8; 8],
    pub left: [u8; 4],
    pub top_left: u8,
    pub top_available: bool,
    pub left_available: bool,
    pub top_left_available: bool,
    /// True if top[4..=7] are real (i.e. the block to the upper-right is
    /// available). When false, callers must replicate top[3] before calling.
    pub top_right_available: bool,
}

/// Predict a 4×4 luma block. Writes the prediction into `out` (raster).
pub fn predict_intra_4x4(out: &mut [u8; 16], mode: Intra4x4Mode, n: &Intra4x4Neighbours) {
    use Intra4x4Mode::*;
    match mode {
        Vertical => {
            // p[x,y] = top[x]
            for y in 0..4 {
                for x in 0..4 {
                    out[y * 4 + x] = n.top[x];
                }
            }
        }
        Horizontal => {
            for y in 0..4 {
                for x in 0..4 {
                    out[y * 4 + x] = n.left[y];
                }
            }
        }
        Dc => {
            let dc: u32 = match (n.top_available, n.left_available) {
                (true, true) => {
                    (n.top[0] as u32
                        + n.top[1] as u32
                        + n.top[2] as u32
                        + n.top[3] as u32
                        + n.left[0] as u32
                        + n.left[1] as u32
                        + n.left[2] as u32
                        + n.left[3] as u32
                        + 4)
                        >> 3
                }
                (true, false) => {
                    (n.top[0] as u32 + n.top[1] as u32 + n.top[2] as u32 + n.top[3] as u32 + 2) >> 2
                }
                (false, true) => {
                    (n.left[0] as u32 + n.left[1] as u32 + n.left[2] as u32 + n.left[3] as u32 + 2)
                        >> 2
                }
                (false, false) => 128,
            };
            let v = dc.min(255) as u8;
            for px in out.iter_mut() {
                *px = v;
            }
        }
        DiagonalDownLeft => {
            // §8.3.1.2.4
            let t = &n.top;
            // Pre-compute zHat ranks 0..=10 (only need 0..=6 effectively).
            let mut z = [0u8; 8];
            // z[0] = (top[0]+2*top[1]+top[2]+2)>>2
            for i in 0..7 {
                let l = if i == 0 { t[0] as u32 } else { t[i - 1] as u32 };
                let m = t[i] as u32;
                let r = t[i + 1] as u32;
                z[i] = (((l + 2 * m + r + 2) >> 2).min(255)) as u8;
            }
            // Last position uses (top[6] + 3*top[7] + 2) >> 2 per spec.
            z[7] = (((t[6] as u32 + 3 * t[7] as u32 + 2) >> 2).min(255)) as u8;
            // pred[x,y] = z[x+y] for (x,y) != (3,3); pred[3,3] = special.
            for y in 0..4 {
                for x in 0..4 {
                    let zi = if x == 3 && y == 3 { 7 } else { x + y };
                    out[y * 4 + x] = z[zi];
                }
            }
        }
        DiagonalDownRight => {
            // §8.3.1.2.5: needs top + top_left + left.
            // p[x,y] = filtered samples on a diagonal.
            // Build a unified "edge" array indexed -3..=3 around the corner.
            let mut e = [0u8; 9]; // -4..=4 for safety
                                  // e index 4 = top_left.
            e[4] = n.top_left;
            for i in 0..4 {
                e[5 + i] = n.top[i]; // e[5..=8] = top[0..=3]
                e[3 - i] = n.left[i]; // e[3..=0] = left[0..=3] reversed
            }
            // Filter triplets.
            let mut f = [0u8; 7]; // f[0..=6]
            for i in 0..7 {
                let a = e[i] as u32;
                let b = e[i + 1] as u32;
                let c = e[i + 2] as u32;
                f[i] = (((a + 2 * b + c + 2) >> 2).min(255)) as u8;
            }
            // p[x,y] = f[3 + x - y]
            for y in 0..4 {
                for x in 0..4 {
                    out[y * 4 + x] = f[(3 + x as i32 - y as i32) as usize];
                }
            }
        }
        VerticalRight => {
            // §8.3.1.2.6
            let mut e = [0u8; 9];
            e[4] = n.top_left;
            for i in 0..4 {
                e[5 + i] = n.top[i];
                e[3 - i] = n.left[i];
            }
            // For each (x,y), zVR = 2x - y.
            // If zVR is even and >=0: p = (e[4 + (x - (y>>1))] + e[5 + (x - (y>>1))] + 1) >> 1
            // If zVR is odd and >=0: p = (e[3 + (x - (y>>1))] + 2*e[4 + (x - (y>>1))] + e[5 + (x - (y>>1))] + 2) >> 2
            // If zVR == -1: p = (e[3] + 2*e[4] + e[5] + 2) >> 2 = (left[0] + 2*top_left + top[0] + 2) >> 2
            // If zVR < -1: p = (e[?] + 2*e[?] + e[?] + 2) >> 2 with e indexed by left
            for y in 0..4 {
                for x in 0..4 {
                    let zvr = 2 * x as i32 - y as i32;
                    let v;
                    if zvr == 0 || zvr == 2 || zvr == 4 || zvr == 6 {
                        // even ≥ 0: avg of two samples
                        let i0 = (4 + x as i32 - (y as i32 >> 1)) as usize;
                        v = ((e[i0] as u32 + e[i0 + 1] as u32 + 1) >> 1) as u8;
                    } else if zvr == 1 || zvr == 3 || zvr == 5 {
                        let i0 = (3 + x as i32 - (y as i32 >> 1)) as usize;
                        v = ((e[i0] as u32 + 2 * e[i0 + 1] as u32 + e[i0 + 2] as u32 + 2) >> 2)
                            as u8;
                    } else if zvr == -1 {
                        v = ((e[3] as u32 + 2 * e[4] as u32 + e[5] as u32 + 2) >> 2) as u8;
                    } else {
                        // zvr < -1 => -2 or -3 (when y=2,x=0 → -2; y=3,x=0 → -3)
                        // Per spec, p = (e[2-y] + 2*e[3-y] + e[4-y] + 2) >> 2 for x=0
                        // Equivalently i0 = 3 + zvr/? Use formula: for zvr=-2: i0=2; zvr=-3: i0=1
                        let off = (-zvr - 1) as usize;
                        let i = 3 - off;
                        v = ((e[i - 1] as u32 + 2 * e[i] as u32 + e[i + 1] as u32 + 2) >> 2) as u8;
                    }
                    out[y * 4 + x] = v;
                }
            }
        }
        HorizontalDown => {
            // §8.3.1.2.7 — like VerticalRight but rotated.
            let mut e = [0u8; 9];
            e[4] = n.top_left;
            for i in 0..4 {
                e[5 + i] = n.top[i];
                e[3 - i] = n.left[i];
            }
            for y in 0..4 {
                for x in 0..4 {
                    let zhd = 2 * y as i32 - x as i32;
                    let v;
                    if zhd == 0 || zhd == 2 || zhd == 4 || zhd == 6 {
                        // even ≥ 0: take two from "left" side mirror
                        let i0 = (3 - (y as i32 - (x as i32 >> 1))) as usize;
                        v = ((e[i0] as u32 + e[i0 + 1] as u32 + 1) >> 1) as u8;
                    } else if zhd == 1 || zhd == 3 || zhd == 5 {
                        let i0 = (3 - (y as i32 - (x as i32 >> 1))) as usize;
                        v = ((e[i0 - 1] as u32 + 2 * e[i0] as u32 + e[i0 + 1] as u32 + 2) >> 2)
                            as u8;
                    } else if zhd == -1 {
                        v = ((e[3] as u32 + 2 * e[4] as u32 + e[5] as u32 + 2) >> 2) as u8;
                    } else {
                        // zhd = -2 or -3.
                        let off = (-zhd - 1) as usize;
                        let i = 4 + off;
                        v = ((e[i - 1] as u32 + 2 * e[i] as u32 + e[i + 1] as u32 + 2) >> 2) as u8;
                    }
                    out[y * 4 + x] = v;
                }
            }
        }
        VerticalLeft => {
            // §8.3.1.2.8 — needs top and (often) top right.
            let t = &n.top;
            for y in 0..4 {
                for x in 0..4 {
                    let i = x + (y >> 1);
                    let v = if y % 2 == 0 {
                        ((t[i] as u32 + t[i + 1] as u32 + 1) >> 1) as u8
                    } else {
                        ((t[i] as u32 + 2 * t[i + 1] as u32 + t[i + 2] as u32 + 2) >> 2) as u8
                    };
                    out[y * 4 + x] = v;
                }
            }
        }
        HorizontalUp => {
            // §8.3.1.2.9 — needs left only.
            let l = &n.left;
            // zHU = x + 2y
            for y in 0..4 {
                for x in 0..4 {
                    let zhu = x + 2 * y;
                    let v = match zhu {
                        0 | 2 | 4 => {
                            let yy = y + (x >> 1);
                            ((l[yy] as u32 + l[yy + 1] as u32 + 1) >> 1) as u8
                        }
                        1 | 3 => {
                            let yy = y + (x >> 1);
                            ((l[yy] as u32 + 2 * l[yy + 1] as u32 + l[yy + 2] as u32 + 2) >> 2)
                                as u8
                        }
                        5 => ((l[2] as u32 + 2 * l[3] as u32 + l[3] as u32 + 2) >> 2) as u8,
                        6 => ((l[2] as u32 + l[3] as u32 + 1) >> 1) as u8,
                        7 => ((l[2] as u32 + 2 * l[3] as u32 + l[3] as u32 + 2) >> 2) as u8,
                        _ => l[3],
                    };
                    out[y * 4 + x] = v;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Intra_16×16 (§8.3.3)
// ---------------------------------------------------------------------------

/// Neighbours for a 16×16 block (luma macroblock).
#[derive(Clone, Debug)]
pub struct Intra16x16Neighbours {
    pub top: [u8; 16],
    pub left: [u8; 16],
    pub top_left: u8,
    pub top_available: bool,
    pub left_available: bool,
    pub top_left_available: bool,
}

pub fn predict_intra_16x16(out: &mut [u8; 256], mode: Intra16x16Mode, n: &Intra16x16Neighbours) {
    use Intra16x16Mode::*;
    // Substitute mode if required neighbour is unavailable (§8.7.2.2 chroma
    // analogue). Vertical without top → DC; Horizontal without left → DC.
    // Plane requires both — fallback to DC if either missing.
    let mode = match mode {
        Vertical if !n.top_available => Dc,
        Horizontal if !n.left_available => Dc,
        Plane if !n.top_available || !n.left_available || !n.top_left_available => Dc,
        m => m,
    };
    match mode {
        Vertical => {
            for y in 0..16 {
                for x in 0..16 {
                    out[y * 16 + x] = n.top[x];
                }
            }
        }
        Horizontal => {
            for y in 0..16 {
                for x in 0..16 {
                    out[y * 16 + x] = n.left[y];
                }
            }
        }
        Dc => {
            let dc: u32 = match (n.top_available, n.left_available) {
                (true, true) => {
                    let s: u32 = n.top.iter().map(|&v| v as u32).sum::<u32>()
                        + n.left.iter().map(|&v| v as u32).sum::<u32>();
                    (s + 16) >> 5
                }
                (true, false) => {
                    let s: u32 = n.top.iter().map(|&v| v as u32).sum();
                    (s + 8) >> 4
                }
                (false, true) => {
                    let s: u32 = n.left.iter().map(|&v| v as u32).sum();
                    (s + 8) >> 4
                }
                (false, false) => 128,
            };
            let v = dc.min(255) as u8;
            for px in out.iter_mut() {
                *px = v;
            }
        }
        Plane => {
            // §8.3.3.4
            // H = sum_{i=0..7} (i+1) * (top[8+i] - top[6-i])
            // V = sum_{i=0..7} (i+1) * (left[8+i] - left[6-i])
            let mut h: i32 = 0;
            for i in 0..8 {
                h += (i as i32 + 1) * (n.top[8 + i] as i32 - n.top[6 - i] as i32);
            }
            let mut v: i32 = 0;
            for i in 0..8 {
                v += (i as i32 + 1) * (n.left[8 + i] as i32 - n.left[6 - i] as i32);
            }
            let b = (5 * h + 32) >> 6;
            let c = (5 * v + 32) >> 6;
            let a = 16 * (n.left[15] as i32 + n.top[15] as i32);
            for y in 0..16 {
                for x in 0..16 {
                    let p = (a + b * (x as i32 - 7) + c * (y as i32 - 7) + 16) >> 5;
                    out[y * 16 + x] = p.clamp(0, 255) as u8;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Intra chroma 8×8 (§8.3.4)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct IntraChromaNeighbours {
    pub top: [u8; 8],
    pub left: [u8; 8],
    pub top_left: u8,
    pub top_available: bool,
    pub left_available: bool,
    pub top_left_available: bool,
}

pub fn predict_intra_chroma(out: &mut [u8; 64], mode: IntraChromaMode, n: &IntraChromaNeighbours) {
    use IntraChromaMode::*;
    let mode = match mode {
        Vertical if !n.top_available => Dc,
        Horizontal if !n.left_available => Dc,
        Plane if !n.top_available || !n.left_available || !n.top_left_available => Dc,
        m => m,
    };
    match mode {
        Vertical => {
            for y in 0..8 {
                for x in 0..8 {
                    out[y * 8 + x] = n.top[x];
                }
            }
        }
        Horizontal => {
            for y in 0..8 {
                for x in 0..8 {
                    out[y * 8 + x] = n.left[y];
                }
            }
        }
        Dc => {
            // Each 4x4 chroma quadrant gets its own DC value (§8.3.4.2).
            // Quadrant (0,0): use top[0..=3] AND left[0..=3] when both available;
            //                 else fall back to whichever is available (or 128).
            // Quadrant (1,0): right-top -> use top[4..=7] only (not left!).
            // Quadrant (0,1): left-bottom -> use left[4..=7] only.
            // Quadrant (1,1): right-bottom -> top[4..=7] AND left[4..=7] when both available.
            let avg = |samples: &[u8]| -> u32 {
                let s: u32 = samples.iter().map(|&v| v as u32).sum();
                (s + (samples.len() as u32 / 2)) / samples.len() as u32
            };
            let dc = |topa: bool, lefta: bool, tslice: &[u8], lslice: &[u8]| -> u8 {
                let val = match (topa, lefta) {
                    (true, true) => {
                        let s: u32 = tslice.iter().map(|&v| v as u32).sum::<u32>()
                            + lslice.iter().map(|&v| v as u32).sum::<u32>();
                        (s + 4) >> 3
                    }
                    (true, false) => avg(tslice),
                    (false, true) => avg(lslice),
                    (false, false) => 128,
                };
                val.min(255) as u8
            };
            // Quadrant 0 (top-left 4x4)
            let q00 = dc(
                n.top_available,
                n.left_available,
                &n.top[0..4],
                &n.left[0..4],
            );
            // Quadrant 1 (top-right 4x4): only top[4..=7]
            let q10 = if n.top_available {
                let s: u32 = n.top[4..8].iter().map(|&v| v as u32).sum();
                ((s + 2) >> 2).min(255) as u8
            } else if n.left_available {
                let s: u32 = n.left[0..4].iter().map(|&v| v as u32).sum();
                ((s + 2) >> 2).min(255) as u8
            } else {
                128
            };
            // Quadrant 2 (bottom-left 4x4): only left[4..=7]
            let q01 = if n.left_available {
                let s: u32 = n.left[4..8].iter().map(|&v| v as u32).sum();
                ((s + 2) >> 2).min(255) as u8
            } else if n.top_available {
                let s: u32 = n.top[0..4].iter().map(|&v| v as u32).sum();
                ((s + 2) >> 2).min(255) as u8
            } else {
                128
            };
            // Quadrant 3 (bottom-right 4x4)
            let q11 = dc(
                n.top_available,
                n.left_available,
                &n.top[4..8],
                &n.left[4..8],
            );
            // Fill quadrants.
            for y in 0..8 {
                for x in 0..8 {
                    let v = match (x / 4, y / 4) {
                        (0, 0) => q00,
                        (1, 0) => q10,
                        (0, 1) => q01,
                        (1, 1) => q11,
                        _ => unreachable!(),
                    };
                    out[y * 8 + x] = v;
                }
            }
        }
        Plane => {
            // §8.3.4.4
            let mut h: i32 = 0;
            for i in 0..4 {
                h += (i as i32 + 1) * (n.top[4 + i] as i32 - n.top[2 - i] as i32);
            }
            let mut v: i32 = 0;
            for i in 0..4 {
                v += (i as i32 + 1) * (n.left[4 + i] as i32 - n.left[2 - i] as i32);
            }
            let b = (34 * h + 32) >> 6;
            let c = (34 * v + 32) >> 6;
            let a = 16 * (n.left[7] as i32 + n.top[7] as i32);
            for y in 0..8 {
                for x in 0..8 {
                    let p = (a + b * (x as i32 - 3) + c * (y as i32 - 3) + 16) >> 5;
                    out[y * 8 + x] = p.clamp(0, 255) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intra4x4_vertical() {
        let n = Intra4x4Neighbours {
            top: [10, 20, 30, 40, 0, 0, 0, 0],
            left: [0; 4],
            top_left: 0,
            top_available: true,
            left_available: false,
            top_left_available: false,
            top_right_available: false,
        };
        let mut out = [0u8; 16];
        predict_intra_4x4(&mut out, Intra4x4Mode::Vertical, &n);
        for y in 0..4 {
            assert_eq!(&out[y * 4..y * 4 + 4], &[10u8, 20, 30, 40]);
        }
    }

    #[test]
    fn intra16x16_dc_neither_available() {
        let n = Intra16x16Neighbours {
            top: [0; 16],
            left: [0; 16],
            top_left: 0,
            top_available: false,
            left_available: false,
            top_left_available: false,
        };
        let mut out = [0u8; 256];
        predict_intra_16x16(&mut out, Intra16x16Mode::Dc, &n);
        assert!(out.iter().all(|&v| v == 128));
    }
}
