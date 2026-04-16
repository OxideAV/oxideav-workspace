//! Block-level (8×8) decode — DC prediction + AC VLC walk + dequant + IDCT.
//!
//! Covers the intra path used by I-VOPs:
//! * intra DC VLC + value decode (Table B-13 / B-14, spec §6.3.8)
//! * intra AC tcoef walk (Table B-16, with 3 escape modes)
//! * AC/DC prediction (§7.4.3)
//! * dequantisation (H.263 path and MPEG-4 matrix path)
//! * IDCT (float, §7.4.4.2 post-processing to [0, 255])

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::headers::vol::{
    VideoObjectLayer, ALTERNATE_HORIZONTAL_SCAN, ALTERNATE_VERTICAL_SCAN, ZIGZAG,
};
use crate::iq::{dc_scaler, dequantise_intra_h263, dequantise_intra_mpeg4};
use crate::tables::{dc_size, tcoef, vlc};

/// The direction that the DC predictor used — `Left` picks from the left
/// neighbour, `Top` from the top neighbour (§7.4.3.1). The AC predictor reuses
/// the same direction (§7.4.3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredDir {
    Left,
    Top,
}

/// Decode the intra DC coefficient differential from the bitstream (§6.3.8).
/// Returns the signed residual; the caller adds the predicted DC.
///
/// `block_idx` picks luma or chroma VLC table (0..=3 = luma, 4..=5 = chroma).
pub fn decode_intra_dc_diff(br: &mut BitReader<'_>, block_idx: usize) -> Result<i32> {
    let table = if block_idx < 4 {
        dc_size::luma()
    } else {
        dc_size::chroma()
    };
    let size = vlc::decode(br, table)? as u32;
    if size == 0 {
        return Ok(0);
    }
    // `size` unsigned bits of DC value, followed by (if size > 8) a marker bit.
    // The MSB of the bit group is the sign: 1 = positive, 0 = negative.
    // On negative: value = raw - (2^size - 1) (as in FFmpeg's get_xbits).
    let raw = br.read_u32(size)? as i32;
    let msb_set = raw & (1 << (size - 1)) != 0;
    let value = if msb_set {
        raw
    } else {
        raw - ((1 << size) - 1)
    };
    if size > 8 {
        // Marker bit; spec §6.3.8 requires it to be 1 but permissive parsers
        // accept either.
        let _marker = br.read_u1()?;
    }
    Ok(value)
}

/// Decode AC coefficients for an 8×8 intra block, placing them at their
/// zigzag positions (excluding DC). On return, `block[1..64]` holds the raw
/// signed AC levels (not yet dequantised).
///
/// `scan` is the scan order selected by AC prediction direction (or the
/// default zigzag if no AC prediction). `block[ZIGZAG[i]]` is the coefficient
/// for scan index `i`.
///
/// Returns the index of the last non-zero AC coefficient in scan order, or
/// `None` if the block contains no coded AC coefficients.
pub fn decode_intra_ac(
    br: &mut BitReader<'_>,
    block: &mut [i32; 64],
    scan: &[usize; 64],
) -> Result<Option<usize>> {
    let table = tcoef::intra_table();
    let mut i: usize = 1; // current scan position; AC starts at 1 (DC is 0).
    let mut last_idx: Option<usize> = None;
    let _ = last_idx.take();
    loop {
        if i > 63 {
            return Err(Error::invalid("mpeg4 block: AC overrun"));
        }
        let sym = vlc::decode(br, table)?;
        let (last, run, level_abs, esc_level) = match sym {
            tcoef::TcoefSym::RunLevel {
                last,
                run,
                level_abs,
            } => {
                let sign = br.read_u1()? as i32;
                let level = if sign == 1 {
                    -(level_abs as i32)
                } else {
                    level_abs as i32
                };
                (last, run, level_abs, level)
            }
            tcoef::TcoefSym::Escape => {
                // Three escape modes, distinguished by leading bits.
                let marker1 = br.read_u1()?;
                if marker1 == 0 {
                    // First escape: level = decoded + max_level[last][run]; then sign.
                    let sym2 = vlc::decode(br, table)?;
                    let (last2, run2, level_abs2) = match sym2 {
                        tcoef::TcoefSym::RunLevel {
                            last,
                            run,
                            level_abs,
                        } => (last, run, level_abs),
                        tcoef::TcoefSym::Escape => {
                            return Err(Error::invalid(
                                "mpeg4 block: double escape in 1st escape mode",
                            ));
                        }
                    };
                    let max_lvl = tcoef::intra_max_level(last2, run2);
                    let abs_l = (level_abs2 as i32) + (max_lvl as i32);
                    let sign = br.read_u1()? as i32;
                    let level = if sign == 1 { -abs_l } else { abs_l };
                    (last2, run2, abs_l as u8, level)
                } else {
                    let marker2 = br.read_u1()?;
                    if marker2 == 0 {
                        // Second escape: run = decoded + max_run[last][level] + 1; then sign.
                        let sym2 = vlc::decode(br, table)?;
                        let (last2, run2, level_abs2) = match sym2 {
                            tcoef::TcoefSym::RunLevel {
                                last,
                                run,
                                level_abs,
                            } => (last, run, level_abs),
                            tcoef::TcoefSym::Escape => {
                                return Err(Error::invalid(
                                    "mpeg4 block: double escape in 2nd escape mode",
                                ));
                            }
                        };
                        let max_rn = tcoef::intra_max_run(last2, level_abs2);
                        let new_run = (run2 as i32) + (max_rn as i32) + 1;
                        let sign = br.read_u1()? as i32;
                        let level = if sign == 1 {
                            -(level_abs2 as i32)
                        } else {
                            level_abs2 as i32
                        };
                        (last2, new_run as u8, level_abs2, level)
                    } else {
                        // Third escape: marker + last(1) + run(6) + marker + level(12) + marker
                        let last = br.read_u1()? == 1;
                        let run = br.read_u32(6)? as u8;
                        br.read_marker()?;
                        let level = br.read_i32(12)?;
                        br.read_marker()?;
                        if level == 0 {
                            return Err(Error::invalid(
                                "mpeg4 block: 3rd-escape level must be non-zero",
                            ));
                        }
                        let abs = level.unsigned_abs().min(255) as u8;
                        (last, run, abs, level)
                    }
                }
            }
        };
        // Place run zeros + the coefficient.
        i = i.saturating_add(run as usize);
        if i > 63 {
            return Err(Error::invalid("mpeg4 block: AC run overflow"));
        }
        block[scan[i]] = esc_level;
        last_idx = Some(i);
        if last {
            return Ok(last_idx);
        }
        let _ = level_abs; // keep shape of tuple; absolute value unused below
        i += 1;
        if i > 63 {
            // No more room and `last` wasn't set — mildly malformed bitstream;
            // accept end-of-block anyway.
            return Ok(last_idx);
        }
    }
}

/// Per-block neighbour state used for AC/DC prediction. One slot per 8×8
/// block in the picture. Records:
/// * `dc` — the reconstructed DC coefficient value in pel domain (post
///   DC-scaler multiply — i.e. a "DC prediction" value, not the raw level).
/// * `ac_top_row` — the first row (natural positions 1..=7) of dequantised
///   AC coefficients (used by the next block below for top-pred AC).
/// * `ac_left_col` — the first column (natural positions 8,16,…,56).
/// * `quant` — the quantiser in effect when this block was decoded (used to
///   rescale AC predictions across quant changes).
/// * `is_intra` — whether prediction from this block is valid (non-intra
///   neighbours predict `1024` for DC and `0` for AC).
#[derive(Clone, Debug)]
pub struct BlockNeighbour {
    pub dc: i32,
    pub ac_top_row: [i32; 7],
    pub ac_left_col: [i32; 7],
    pub quant: u8,
    pub is_intra: bool,
}

impl Default for BlockNeighbour {
    fn default() -> Self {
        Self {
            dc: 1024,
            ac_top_row: [0; 7],
            ac_left_col: [0; 7],
            quant: 1,
            is_intra: false,
        }
    }
}

/// Choose the DC predictor and direction for one 8×8 intra block (§7.4.3.1).
///
/// `left`, `top_left`, `top` are the DC values of the three reference
/// neighbours (pel-domain, post-DC-scaler). Returns `(predicted_dc,
/// direction)`.
///
/// If a neighbour is outside the picture or non-intra, the caller passes
/// `1024` for that position.
pub fn choose_dc_predictor(left: i32, top_left: i32, top: i32) -> (i32, PredDir) {
    // (a = left, b = top_left, c = top)
    // if |a - b| < |b - c|: pred = c (top), dir = Top (1)
    // else:                 pred = a (left), dir = Left (0)
    if (left - top_left).abs() < (top_left - top).abs() {
        (top, PredDir::Top)
    } else {
        (left, PredDir::Left)
    }
}

/// Apply AC prediction to `block` (natural order) given a chosen direction.
/// `block` already holds the dequantised AC coefficients from the bitstream
/// (positions 1..63). This function *adds* the predicted row or column.
///
/// When `dir == Top`, the top neighbour's `ac_top_row` is added to positions
/// 1,2,…,7 of this block (the top row). When `dir == Left`, the left
/// neighbour's `ac_left_col` is added to positions 8,16,…,56 (the left column).
///
/// If the two blocks have different `quant` values the neighbour's ACs are
/// rescaled (§7.4.3.2).
pub fn apply_ac_prediction(
    block: &mut [i32; 64],
    dir: PredDir,
    neighbour: &BlockNeighbour,
    my_quant: u8,
) {
    let nbr_q = neighbour.quant as i32;
    let my_q = my_quant as i32;
    let rescale = |v: i32| -> i32 {
        if my_q == 0 || nbr_q == my_q {
            v
        } else {
            // ROUNDED_DIV(v * nbr_q, my_q): (v * nbr_q + my_q/2) / my_q (spec).
            let num = v * nbr_q;
            if num >= 0 {
                (num + my_q / 2) / my_q
            } else {
                -((-num + my_q / 2) / my_q)
            }
        }
    };
    match dir {
        PredDir::Top => {
            for i in 0..7 {
                let nat = i + 1; // positions 1..=7 (top row, excluding DC)
                block[nat] += rescale(neighbour.ac_top_row[i]);
            }
        }
        PredDir::Left => {
            for i in 0..7 {
                let nat = (i + 1) * 8; // positions 8, 16, ..., 56
                block[nat] += rescale(neighbour.ac_left_col[i]);
            }
        }
    }
}

/// Copy the top row and left column of `block` into the neighbour cache (for
/// future blocks to predict from).
pub fn record_ac_prediction_cache(block: &[i32; 64], nbr: &mut BlockNeighbour) {
    for i in 0..7 {
        nbr.ac_top_row[i] = block[i + 1];
        nbr.ac_left_col[i] = block[(i + 1) * 8];
    }
}

/// Pick the coefficient scan order (spec §7.4.3.3) given whether AC
/// prediction is active and the DC-direction that was chosen.
pub fn choose_scan(ac_pred: bool, dir: PredDir) -> &'static [usize; 64] {
    if !ac_pred {
        &ZIGZAG
    } else {
        match dir {
            // AC predicted from left -> the block values tend to be uniform
            // across rows -> use vertical scan (column-major).
            PredDir::Left => &ALTERNATE_VERTICAL_SCAN,
            // AC predicted from top -> use horizontal scan (row-major).
            PredDir::Top => &ALTERNATE_HORIZONTAL_SCAN,
        }
    }
}

/// Textbook 8×8 IDCT (float). Used by the I-VOP path.
pub fn idct8x8(block: &mut [f32; 64]) {
    use std::f32::consts::PI;
    use std::sync::OnceLock;

    static T: OnceLock<[[f32; 8]; 8]> = OnceLock::new();
    let cos = T.get_or_init(|| {
        let mut t = [[0.0f32; 8]; 8];
        for k in 0..8 {
            let c_k = if k == 0 {
                (1.0_f32 / 2.0_f32).sqrt()
            } else {
                1.0
            };
            for n in 0..8 {
                t[k][n] = 0.5 * c_k * ((2 * n + 1) as f32 * k as f32 * PI / 16.0).cos();
            }
        }
        t
    });

    let mut tmp = [0.0f32; 64];
    for y in 0..8 {
        for n in 0..8 {
            let mut s = 0.0f32;
            for k in 0..8 {
                s += cos[k][n] * block[y * 8 + k];
            }
            tmp[y * 8 + n] = s;
        }
    }
    for x in 0..8 {
        for m in 0..8 {
            let mut s = 0.0f32;
            for k in 0..8 {
                s += cos[k][m] * tmp[k * 8 + x];
            }
            block[m * 8 + x] = s;
        }
    }
}

/// Dequantise + IDCT a finalised intra block in-place. `coeffs[0]` must
/// already be the reconstructed DC coefficient (in post-prediction,
/// pre-IDCT scale — i.e. `pred_dc + dc_diff * dc_scaler`). `coeffs[1..64]`
/// hold the raw decoded AC levels. On return, `out` holds 64 `i16`-like pel
/// values clipped to [0, 255].
pub fn reconstruct_intra_block(
    coeffs: &mut [i32; 64],
    vol: &VideoObjectLayer,
    quant: u32,
    out: &mut [i32; 64],
) -> Result<()> {
    if vol.mpeg_quant {
        dequantise_intra_mpeg4(coeffs, quant, vol)?;
    } else {
        dequantise_intra_h263(coeffs, quant)?;
    }

    // Saturate the DC coefficient too (the reconstruction path rarely hits
    // these bounds, but the spec clamps to [-2048, 2047]).
    coeffs[0] = coeffs[0].clamp(-2048, 2047);

    // IDCT.
    let mut f = [0.0f32; 64];
    for i in 0..64 {
        f[i] = coeffs[i] as f32;
    }
    idct8x8(&mut f);
    for i in 0..64 {
        let v = f[i].round() as i32;
        out[i] = v.clamp(-256, 255);
    }
    Ok(())
}

/// Clamp a signed sample value to an 8-bit pixel.
pub fn clip_to_u8(v: i32) -> u8 {
    if v <= 0 {
        0
    } else if v >= 255 {
        255
    } else {
        v as u8
    }
}

/// Full intra DC handling (§7.4.3.1): takes the diff decoded from the
/// bitstream, the predicted DC (pel domain), and returns the reconstructed DC
/// coefficient (in pel domain) to be stored in the neighbour cache.
///
/// The block-level coefficient array `coeffs[0]` ends up holding
/// `pred_dc + dc_diff * dc_scaler` (post-IDCT scale) — ready for the IDCT.
pub fn reconstruct_dc(
    coeffs_dc: &mut i32,
    dc_diff: i32,
    predicted_dc_pel: i32,
    block_idx: usize,
    quant: u32,
) -> i32 {
    let scale = dc_scaler(block_idx, quant) as i32;
    // `dc_diff` is in "scaled DC units"; multiply by scale to get pel domain.
    let reconstructed = predicted_dc_pel + dc_diff * scale;
    // Store the pel-domain DC into the coefficient buffer so the IDCT input
    // is "level in DCT domain" (DC component of the DCT).
    // The spec says the DC coefficient fed to IDCT is `reconstructed`
    // directly — our IDCT expects values in DCT domain.
    *coeffs_dc = reconstructed;
    reconstructed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_only_roundtrip() {
        // A flat DC-only block: with our IDCT normalisation DC * 8 ≈ 128 out.
        let mut b = [0.0f32; 64];
        b[0] = 8.0 * 128.0;
        idct8x8(&mut b);
        for v in &b {
            assert!((v - 128.0).abs() < 1.0, "got {v}, want ~128");
        }
    }

    #[test]
    fn choose_predictor_top_wins_when_vertical_smoother() {
        // a=100, b=100, c=90. |a-b|=0 < |b-c|=10 -> top = 90.
        let (p, d) = choose_dc_predictor(100, 100, 90);
        assert_eq!(p, 90);
        assert_eq!(d, PredDir::Top);
    }

    #[test]
    fn choose_predictor_left_wins_when_horizontal_smoother() {
        // a=100, b=90, c=90. |a-b|=10, |b-c|=0 -> left = 100.
        let (p, d) = choose_dc_predictor(100, 90, 90);
        assert_eq!(p, 100);
        assert_eq!(d, PredDir::Left);
    }
}
