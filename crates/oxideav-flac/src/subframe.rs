//! Subframe decoding: constant, verbatim, fixed-predictor, LPC.
//!
//! Reference: <https://xiph.org/flac/format.html#subframe>

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;

/// Decode one subframe from the bitstream.
///
/// `bps` is the per-sample bit depth for *this* channel — caller must add 1
/// for the side channel of left-side / right-side / mid-side stereo frames.
pub fn decode_subframe(br: &mut BitReader, block_size: u32, bps: u32) -> Result<Vec<i32>> {
    let pad = br.read_u32(1)?;
    if pad != 0 {
        return Err(Error::invalid("subframe header pad bit must be zero"));
    }
    let type_code = br.read_u32(6)? as u8;
    let has_wasted = br.read_bit()?;
    let wasted = if has_wasted {
        // Wasted-bit count is unary-encoded — count of leading zeros, plus 1
        // (because at least one wasted bit was indicated).
        br.read_unary()? + 1
    } else {
        0
    };

    if wasted >= bps {
        return Err(Error::invalid(format!(
            "wasted bits {wasted} >= bps {bps}"
        )));
    }
    let effective_bps = bps - wasted;

    let mut samples = match type_code {
        0b000000 => decode_constant(br, block_size, effective_bps)?,
        0b000001 => decode_verbatim(br, block_size, effective_bps)?,
        0b001000..=0b001100 => {
            let order = (type_code & 0x07) as usize;
            decode_fixed(br, block_size, effective_bps, order)?
        }
        0b100000..=0b111111 => {
            let order = ((type_code & 0x1F) + 1) as usize;
            decode_lpc(br, block_size, effective_bps, order)?
        }
        _ => {
            return Err(Error::invalid(format!(
                "reserved FLAC subframe type 0x{type_code:02x}"
            )));
        }
    };

    if wasted > 0 {
        for s in samples.iter_mut() {
            *s = ((*s as i64) << wasted) as i32;
        }
    }
    Ok(samples)
}

fn decode_constant(br: &mut BitReader, block_size: u32, bps: u32) -> Result<Vec<i32>> {
    let v = br.read_i32(bps)?;
    Ok(vec![v; block_size as usize])
}

fn decode_verbatim(br: &mut BitReader, block_size: u32, bps: u32) -> Result<Vec<i32>> {
    let mut s = Vec::with_capacity(block_size as usize);
    for _ in 0..block_size {
        s.push(br.read_i32(bps)?);
    }
    Ok(s)
}

fn decode_fixed(
    br: &mut BitReader,
    block_size: u32,
    bps: u32,
    order: usize,
) -> Result<Vec<i32>> {
    let mut samples = Vec::with_capacity(block_size as usize);
    for _ in 0..order {
        samples.push(br.read_i32(bps)?);
    }
    let residual = decode_residual(br, block_size, order)?;
    apply_fixed_predictor(&mut samples, &residual, order);
    Ok(samples)
}

fn apply_fixed_predictor(samples: &mut Vec<i32>, residual: &[i32], order: usize) {
    // Fixed-predictor coefficients applied to the previous samples.
    const COEFFS: [&[i32]; 5] = [
        &[],
        &[1],
        &[2, -1],
        &[3, -3, 1],
        &[4, -6, 4, -1],
    ];
    let c = COEFFS[order];
    for &r in residual {
        let mut pred: i64 = 0;
        for (i, &ci) in c.iter().enumerate() {
            let s = samples[samples.len() - 1 - i] as i64;
            pred += (ci as i64) * s;
        }
        samples.push((pred + r as i64) as i32);
    }
}

fn decode_lpc(
    br: &mut BitReader,
    block_size: u32,
    bps: u32,
    order: usize,
) -> Result<Vec<i32>> {
    let mut samples = Vec::with_capacity(block_size as usize);
    for _ in 0..order {
        samples.push(br.read_i32(bps)?);
    }
    let qlp_precision_raw = br.read_u32(4)?;
    if qlp_precision_raw == 0xF {
        return Err(Error::invalid("FLAC LPC: invalid qlp precision (0xF)"));
    }
    let qlp_precision = qlp_precision_raw + 1;
    let qlp_shift = br.read_i32(5)?;
    if qlp_shift < 0 {
        return Err(Error::invalid("FLAC LPC: negative qlp_shift not supported"));
    }
    let mut coeffs = Vec::with_capacity(order);
    for _ in 0..order {
        coeffs.push(br.read_i32(qlp_precision)?);
    }
    let residual = decode_residual(br, block_size, order)?;
    apply_lpc(&mut samples, &residual, &coeffs, qlp_shift as u32);
    Ok(samples)
}

fn apply_lpc(samples: &mut Vec<i32>, residual: &[i32], coeffs: &[i32], qlp_shift: u32) {
    for &r in residual {
        let mut pred: i64 = 0;
        for (i, &c) in coeffs.iter().enumerate() {
            let s = samples[samples.len() - 1 - i] as i64;
            pred += (c as i64) * s;
        }
        let predicted = (pred >> qlp_shift) as i32;
        samples.push(predicted.wrapping_add(r));
    }
}

fn decode_residual(
    br: &mut BitReader,
    block_size: u32,
    predictor_order: usize,
) -> Result<Vec<i32>> {
    let method = br.read_u32(2)?;
    let (param_bits, escape_marker) = match method {
        0 => (4u32, 15u32),
        1 => (5u32, 31u32),
        _ => return Err(Error::invalid("reserved FLAC residual coding method")),
    };
    let partition_order = br.read_u32(4)?;
    let n_partitions = 1u32 << partition_order;
    if block_size % n_partitions != 0 {
        return Err(Error::invalid(
            "FLAC residual: block_size not divisible by partition count",
        ));
    }
    let partition_size = block_size / n_partitions;
    if partition_size as usize <= predictor_order {
        // First partition would have zero or negative samples — invalid.
        return Err(Error::invalid(
            "FLAC residual: first partition smaller than predictor order",
        ));
    }

    let total = block_size as usize - predictor_order;
    let mut residual = Vec::with_capacity(total);
    for p in 0..n_partitions {
        let n_samples = if p == 0 {
            partition_size as usize - predictor_order
        } else {
            partition_size as usize
        };
        let k = br.read_u32(param_bits)?;
        if k == escape_marker {
            // "Escape" partition: 5 bits of raw bps then n_samples raw signed values.
            let raw_bps = br.read_u32(5)?;
            for _ in 0..n_samples {
                residual.push(br.read_i32(raw_bps)?);
            }
        } else {
            for _ in 0..n_samples {
                let q = br.read_unary()?;
                let r = br.read_u32(k)?;
                let unsigned = ((q as u64) << k) | (r as u64);
                let signed: i64 = if unsigned & 1 == 0 {
                    (unsigned >> 1) as i64
                } else {
                    -((unsigned >> 1) as i64) - 1
                };
                residual.push(signed as i32);
            }
        }
    }
    Ok(residual)
}
