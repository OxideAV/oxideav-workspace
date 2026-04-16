//! H.263 block-level decoding (§5.4 of ITU-T Rec. H.263).
//!
//! For an INTRA macroblock, each of the six 8×8 blocks is encoded as:
//! * **INTRADC** — 8 fixed-length bits. Bitstream values `0x00` and `0x80`
//!   are illegal; `0xFF` decodes to a reconstruction level of 1024 (giving a
//!   gray block of pel value 128). All other values decode to
//!   `intradc << 3` — i.e. the pel-domain DC times 8. This is fed directly
//!   into the IDCT input at position 0.
//! * **TCOEF** — present iff the block's CBP bit is set. Variable-length
//!   `(last, run, level)` triples in zig-zag scan order, terminated by
//!   `last == true`. Codes use Table 16/H.263 (the same table as the
//!   MPEG-4 Part 2 inter TCOEF, Annex B-17). Escape mode is much simpler
//!   than MPEG-4: the 7-bit `0000011` escape prefix is followed by
//!   `last(1) + run(6) + level(8 signed)` — no marker bits, no max-level
//!   trick.
//!
//! Dequantisation uses the H.263 formula (identical to MPEG-4's H.263 mode):
//!   `|F''| = q * (2 * |level| + 1)`, with bit-0 cleared when `q` is even.
//! INTRADC bypasses dequantisation — it's already in the correct domain.

use oxideav_core::{Error, Result};
use oxideav_mpeg4video::bitreader::BitReader;
use oxideav_mpeg4video::headers::vol::ZIGZAG;
use oxideav_mpeg4video::tables::{
    tcoef::{inter_table, TcoefSym},
    vlc,
};

/// Decode the 8-bit INTRADC value and return the reconstruction level for
/// position `[0]` of the IDCT input.
///
/// Returns `Err` for the two illegal bitstream values `0x00` and `0x80`.
pub fn decode_intradc(br: &mut BitReader<'_>) -> Result<i32> {
    let v = br.read_u32(8)? as u8;
    if v == 0x00 || v == 0x80 {
        return Err(Error::invalid(format!(
            "h263 INTRADC: illegal bitstream value 0x{v:02x}"
        )));
    }
    if v == 0xFF {
        Ok(1024)
    } else {
        Ok((v as i32) << 3)
    }
}

/// Decode the AC coefficients of an 8×8 H.263 block, placing them in zig-zag
/// scan positions of `block`. AC starts at scan index 1 for INTRA blocks (DC
/// is in `block[0]` already, set by the caller from INTRADC) and at scan
/// index 0 for INTER blocks. `start` selects which.
///
/// Coefficients are written in their **dequantised** form so the IDCT can be
/// run directly afterwards.
pub fn decode_ac(
    br: &mut BitReader<'_>,
    block: &mut [i32; 64],
    start: usize,
    quant: u32,
) -> Result<()> {
    let table = inter_table();
    let mut i: usize = start;
    let q = quant as i32;
    let q_minus_one_if_even = if q & 1 == 1 { 0 } else { -1 };
    loop {
        if i > 63 {
            return Err(Error::invalid("h263 block: AC overrun"));
        }
        let sym = vlc::decode(br, table)?;
        let (last, run, level_signed) = match sym {
            TcoefSym::RunLevel {
                last,
                run,
                level_abs,
            } => {
                let sign = br.read_u1()? as i32;
                let l = if sign == 1 {
                    -(level_abs as i32)
                } else {
                    level_abs as i32
                };
                (last, run, l)
            }
            TcoefSym::Escape => {
                // H.263 escape: last(1) + run(6) + level(8 signed).
                let last = br.read_u1()? == 1;
                let run = br.read_u32(6)? as u8;
                let raw = br.read_u32(8)?;
                // 8-bit two's complement; reject 0x80 (forbidden).
                let level: i32 = if raw == 0 {
                    return Err(Error::invalid("h263 block: escape level == 0"));
                } else if raw == 0x80 {
                    return Err(Error::invalid("h263 block: escape level == -128 forbidden"));
                } else if raw & 0x80 != 0 {
                    raw as i32 - 256
                } else {
                    raw as i32
                };
                (last, run, level)
            }
        };
        i = i.saturating_add(run as usize);
        if i > 63 {
            return Err(Error::invalid("h263 block: AC run overflow"));
        }
        // Dequantise: |F''| = q * (2*|level| + 1) - (1 if q even else 0).
        let abs = level_signed.unsigned_abs() as i32;
        let mut val = q * (2 * abs + 1) + q_minus_one_if_even;
        if level_signed < 0 {
            val = -val;
        }
        let val = val.clamp(-2048, 2047);
        block[ZIGZAG[i]] = val;
        if last {
            return Ok(());
        }
        i += 1;
        if i > 63 {
            // No more room and `last` wasn't set — accept end-of-block anyway.
            return Ok(());
        }
    }
}

/// Run the float-domain IDCT on `block` and return clipped 8-bit pel samples
/// (0..=255).
pub fn idct_and_clip(block: &mut [i32; 64], out: &mut [u8; 64]) {
    let mut f = [0.0f32; 64];
    for i in 0..64 {
        f[i] = block[i] as f32;
    }
    oxideav_mpeg4video::block::idct8x8(&mut f);
    for i in 0..64 {
        let v = f[i].round() as i32;
        out[i] = if v < 0 {
            0
        } else if v > 255 {
            255
        } else {
            v as u8
        };
    }
}

/// Run the float-domain IDCT on `block` and return signed residual samples
/// clipped to the spec's inter-residual range `[-256, 255]`. Used by the
/// P-picture inter path where the output is added to a motion-compensated
/// predictor before the final 8-bit clip.
pub fn idct_signed(block: &mut [i32; 64], out: &mut [i32; 64]) {
    let mut f = [0.0f32; 64];
    for i in 0..64 {
        f[i] = block[i] as f32;
    }
    oxideav_mpeg4video::block::idct8x8(&mut f);
    for i in 0..64 {
        let v = f[i].round() as i32;
        out[i] = v.clamp(-256, 255);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intradc_basic() {
        let data = [0x10u8];
        let mut br = BitReader::new(&data);
        assert_eq!(decode_intradc(&mut br).unwrap(), 0x10 << 3);
    }

    #[test]
    fn intradc_special_ff() {
        let data = [0xFFu8];
        let mut br = BitReader::new(&data);
        assert_eq!(decode_intradc(&mut br).unwrap(), 1024);
    }

    #[test]
    fn intradc_zero_is_illegal() {
        let data = [0x00u8];
        let mut br = BitReader::new(&data);
        assert!(decode_intradc(&mut br).is_err());
    }

    #[test]
    fn intradc_0x80_is_illegal() {
        let data = [0x80u8];
        let mut br = BitReader::new(&data);
        assert!(decode_intradc(&mut br).is_err());
    }
}
