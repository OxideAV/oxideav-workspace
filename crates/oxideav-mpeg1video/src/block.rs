//! Block-level decoding: DC + AC coefficient parsing, dequantisation, IDCT.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::dct::idct8x8;
use crate::headers::ZIGZAG;
use crate::tables::dct_coeffs::{self, DctSym};
use crate::tables::dct_dc;
use crate::vlc;

/// Sign-extend an `size`-bit unsigned DC-differential value to i32.
fn extend_dc(value: u32, size: u32) -> i32 {
    if size == 0 {
        return 0;
    }
    let vt = 1u32 << (size - 1);
    if value < vt {
        (value as i32) - ((1i32 << size) - 1)
    } else {
        value as i32
    }
}

/// Decode one intra macroblock block. `is_chroma` picks the DC size table.
/// `prev_dc_pel` carries the running "DC value in pel-space" (i.e. already
/// multiplied by 8 compared to the DC size differential) for the component
/// across blocks of the same slice.
pub fn decode_intra_block(
    br: &mut BitReader<'_>,
    is_chroma: bool,
    prev_dc_pel: &mut i32,
    quant_scale: u8,
    intra_quant: &[u8; 64],
    out_samples: &mut [u8],
    dst_stride: usize,
) -> Result<()> {
    // 1. DC differential.
    let dc_tbl = if is_chroma {
        dct_dc::chroma()
    } else {
        dct_dc::luma()
    };
    let dc_size = vlc::decode(br, dc_tbl)?;
    let dc_diff = if dc_size == 0 {
        0
    } else {
        let bits = br.read_u32(dc_size as u32)?;
        extend_dc(bits, dc_size as u32)
    };
    // Per §2.4.4.1, `dct_dc_*_past` stores the reconstructed DC coefficient
    // in pel-space (already multiplied by 8). Reset value at slice start is
    // 1024. The reconstructed DC for this block is:
    //   dct_recon[0][0] = dct_dc_differential * 8 + dct_dc_past
    // and then `dct_dc_past = dct_recon[0][0]`.
    let dc_rec = prev_dc_pel.wrapping_add(dc_diff * 8);
    *prev_dc_pel = dc_rec;

    // 2. Zig-zag AC coefficients using Table B-14.
    //
    // Per ISO/IEC 11172-2 §2.4.2.9, the AC stream is ALWAYS terminated by an
    // End-Of-Block marker, even when the block holds all 63 AC coefficients.
    // So we loop unconditionally and only exit on EOB (or on a run overflow,
    // which is a bitstream error).
    let mut coeffs = [0i32; 64];
    coeffs[0] = dc_rec;

    let ac_tbl = dct_coeffs::table();
    let mut k: usize = 1;
    loop {
        let sym = vlc::decode(br, ac_tbl)?;
        let (run, level) = match sym {
            DctSym::Eob | DctSym::EobOrFirstOne => break,
            DctSym::RunLevel { run, level_abs } => {
                let sign = br.read_u32(1)?;
                let mut lv = level_abs as i32;
                if sign == 1 {
                    lv = -lv;
                }
                (run as usize, lv)
            }
            DctSym::Escape => {
                let run = br.read_u32(6)? as usize;
                // Short form: 8-bit signed level.
                let first = br.read_u32(8)? as i32;
                let level = if first == 0 {
                    // Long-escape form (MPEG-1): another 8 bits give an
                    // unsigned positive level ∈ 128..=255.
                    let l = br.read_u32(8)? as i32;
                    if l < 128 {
                        return Err(Error::invalid("dct escape: long form level < 128"));
                    }
                    l
                } else if first == 128 {
                    // Long-escape form negative: following 8 bits form
                    // level ∈ -256..=-129.
                    let l = br.read_u32(8)? as i32;
                    if l > 128 {
                        return Err(Error::invalid("dct escape: long form neg level > 128"));
                    }
                    l - 256
                } else if first >= 128 {
                    first - 256
                } else {
                    first
                };
                (run, level)
            }
        };
        k += run;
        if k >= 64 {
            return Err(Error::invalid("intra block: AC run past end"));
        }
        // Intra dequantisation per §2.4.4.1:
        //   coeff' = (2 * level * quantizer_scale * W[i]) / 16
        // followed by "mismatch control" (make odd) and saturation to ±2047.
        let qf = intra_quant[ZIGZAG[k]] as i32;
        let mut rec = (2 * level * quant_scale as i32 * qf) / 16;
        if rec & 1 == 0 && rec != 0 {
            rec = if rec > 0 { rec - 1 } else { rec + 1 };
        }
        rec = rec.clamp(-2048, 2047);
        coeffs[ZIGZAG[k]] = rec;
        k += 1;
    }

    // 3. IDCT.
    let mut fblock = [0.0f32; 64];
    for i in 0..64 {
        fblock[i] = coeffs[i] as f32;
    }
    idct8x8(&mut fblock);

    // Write back, clamped to [0,255].
    for j in 0..8 {
        for i in 0..8 {
            let v = fblock[j * 8 + i];
            let px = if v <= 0.0 {
                0
            } else if v >= 255.0 {
                255
            } else {
                v.round() as u8
            };
            out_samples[j * dst_stride + i] = px;
        }
    }
    Ok(())
}

/// Decode one non-intra block. The block does NOT have a DC size/differential
/// prefix — it starts directly with AC coefficients at scan position 0 using
/// the first-coefficient interpretation of Table B-14 (codeword `1s` means
/// `(run=0, level=±1)`, not EOB).
///
/// `prediction` is the motion-compensated prediction samples for this 8x8
/// block; `prediction_stride` gives its row stride. The output is written
/// to `out_samples` as `clamp(prediction + idct(residual), 0, 255)`.
pub fn decode_non_intra_block(
    br: &mut BitReader<'_>,
    quant_scale: u8,
    non_intra_quant: &[u8; 64],
    prediction: &[u8],
    prediction_stride: usize,
    out_samples: &mut [u8],
    dst_stride: usize,
) -> Result<()> {
    let mut coeffs = [0i32; 64];

    // First AC coefficient uses a special table where `1s` decodes to
    // (run=0, level=±1).
    let first_tbl = dct_coeffs::first_coeff_table();
    let ac_tbl = dct_coeffs::table();

    let mut k: usize = 0;
    let mut first = true;
    loop {
        let sym = if first {
            vlc::decode(br, first_tbl)?
        } else {
            vlc::decode(br, ac_tbl)?
        };
        let (run, level) = match sym {
            DctSym::Eob => {
                if first {
                    return Err(Error::invalid("non-intra block: EOB as first symbol"));
                }
                break;
            }
            // Should never fire in these tables — the first-table maps it to
            // RunLevel(0,1) and subsequent uses ac_tbl.
            DctSym::EobOrFirstOne => break,
            DctSym::RunLevel { run, level_abs } => {
                let sign = br.read_u32(1)?;
                let mut lv = level_abs as i32;
                if sign == 1 {
                    lv = -lv;
                }
                (run as usize, lv)
            }
            DctSym::Escape => {
                let run = br.read_u32(6)? as usize;
                let first_byte = br.read_u32(8)? as i32;
                let level = if first_byte == 0 {
                    let l = br.read_u32(8)? as i32;
                    if l < 128 {
                        return Err(Error::invalid("dct escape: long form level < 128"));
                    }
                    l
                } else if first_byte == 128 {
                    let l = br.read_u32(8)? as i32;
                    if l > 128 {
                        return Err(Error::invalid("dct escape: long form neg level > 128"));
                    }
                    l - 256
                } else if first_byte >= 128 {
                    first_byte - 256
                } else {
                    first_byte
                };
                (run, level)
            }
        };
        first = false;
        k += run;
        if k >= 64 {
            return Err(Error::invalid("non-intra block: AC run past end"));
        }
        // Non-intra dequantisation per §2.4.4.2:
        //   rec = ((2 * level + sign(level)) * quantizer_scale * W[i]) / 16
        // followed by mismatch control and ±2047 saturation.
        let qf = non_intra_quant[ZIGZAG[k]] as i32;
        let add = if level > 0 { 1 } else { -1 };
        let mut rec = ((2 * level + add) * quant_scale as i32 * qf) / 16;
        if rec & 1 == 0 && rec != 0 {
            rec = if rec > 0 { rec - 1 } else { rec + 1 };
        }
        rec = rec.clamp(-2048, 2047);
        coeffs[ZIGZAG[k]] = rec;
        k += 1;
    }

    // IDCT residual.
    let mut fblock = [0.0f32; 64];
    for i in 0..64 {
        fblock[i] = coeffs[i] as f32;
    }
    idct8x8(&mut fblock);

    // Add prediction and clamp.
    for j in 0..8 {
        for i in 0..8 {
            let p = prediction[j * prediction_stride + i] as i32;
            let r = fblock[j * 8 + i].round() as i32;
            let v = (p + r).clamp(0, 255);
            out_samples[j * dst_stride + i] = v as u8;
        }
    }
    Ok(())
}

/// Copy a prediction block to the output (no residual). Used for non-coded
/// (no-pattern) or skipped macroblocks.
pub fn copy_prediction(
    prediction: &[u8],
    prediction_stride: usize,
    size: usize,
    out_samples: &mut [u8],
    dst_stride: usize,
) {
    for j in 0..size {
        out_samples[j * dst_stride..j * dst_stride + size]
            .copy_from_slice(&prediction[j * prediction_stride..j * prediction_stride + size]);
    }
}
