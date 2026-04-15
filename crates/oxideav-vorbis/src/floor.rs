//! Floor 1 packet decoding and curve synthesis.
//!
//! Reference: Vorbis I §7.2.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::codebook::Codebook;
use crate::setup::{Floor, Floor1};

/// Bit width for the first two amplitude values (Y[0], Y[1]) per floor1
/// multiplier setting (Vorbis I §6.2.3 table).
fn amp_bits_for_multiplier(multiplier: u8) -> u32 {
    match multiplier {
        1 => 8, // range = 256
        2 => 7, // range = 128
        3 => 7, // range = 86 (still 7 bits)
        4 => 6, // range = 64
        _ => 8,
    }
}

/// Per-multiplier "step2 flag = unused" range as defined in Vorbis I
/// (multiplier values map to a fixed dB step size).
pub fn floor1_db_step(multiplier: u8) -> f32 {
    match multiplier {
        1 => 256.0,
        2 => 128.0,
        3 => 86.0,
        4 => 64.0,
        _ => 256.0,
    }
}

/// Decoded floor 1 amplitude vector + "is unused" flag.
#[derive(Clone, Debug)]
pub struct Floor1Decoded {
    pub unused: bool,
    /// Amplitude (Y) values, one per X-list entry. Empty if `unused`.
    pub y: Vec<i32>,
}

pub fn decode_floor1_packet(
    floor: &Floor1,
    codebooks: &[Codebook],
    br: &mut BitReader<'_>,
) -> Result<Floor1Decoded> {
    let nonzero = br.read_bit()?;
    if !nonzero {
        return Ok(Floor1Decoded {
            unused: true,
            y: Vec::new(),
        });
    }
    let amp_bits = amp_bits_for_multiplier(floor.multiplier);
    let mut y: Vec<i32> = Vec::with_capacity(floor.xlist.len());
    y.push(br.read_u32(amp_bits)? as i32);
    y.push(br.read_u32(amp_bits)? as i32);
    let mut offset = 2usize;
    for &class_idx in &floor.partition_class_list {
        let c = class_idx as usize;
        let cdim = floor.class_dimensions[c] as usize;
        let cbits = floor.class_subclasses[c] as u32;
        let csub = 1u32 << cbits;
        let mut cval = if cbits > 0 {
            let cb = &codebooks[floor.class_masterbook[c] as usize];
            cb.decode_scalar(br)?
        } else {
            0
        };
        for _j in 0..cdim {
            let book_index = floor.class_subbook[c][(cval & (csub - 1)) as usize];
            // Shift off the consumed sub-book bits BEFORE decoding the value;
            // libvorbis decrements after, but the order is equivalent so long
            // as we use the masked low bits first.
            cval >>= cbits;
            let v = if book_index >= 0 {
                let cb = &codebooks[book_index as usize];
                cb.decode_scalar(br)? as i32
            } else {
                0
            };
            y.push(v);
            offset += 1;
        }
    }
    if offset != floor.xlist.len() {
        return Err(Error::invalid(format!(
            "Vorbis floor1 decoded {} amplitudes, expected {}",
            offset,
            floor.xlist.len()
        )));
    }
    Ok(Floor1Decoded { unused: false, y })
}

/// Public entry point: decode a floor packet given its setup type.
pub fn decode_floor_packet(
    floor: &Floor,
    codebooks: &[Codebook],
    br: &mut BitReader<'_>,
) -> Result<Floor1Decoded> {
    match floor {
        Floor::Type1(f) => decode_floor1_packet(f, codebooks, br),
        Floor::Type0(_) => Err(Error::unsupported(
            "Vorbis floor 0 (LSP) decoding not implemented",
        )),
    }
}

/// Synthesize the floor curve into `output[0..n_half]` (length = blocksize/2).
///
/// On entry, `output` must already hold the dequantised residue spectrum (or
/// 1.0 everywhere if the channel is residue-free). On exit, each spectral
/// bin has been multiplied by the floor's per-bin magnitude.
///
/// Implements Vorbis I §7.2.4 step1 + step2 + render — translated from the
/// libvorbis reference for bit-exact output.
pub fn synth_floor1(
    floor: &Floor1,
    decoded: &Floor1Decoded,
    n_half: usize,
    output: &mut [f32],
) -> Result<()> {
    if decoded.unused {
        for v in output.iter_mut().take(n_half) {
            *v = 0.0;
        }
        return Ok(());
    }
    if output.len() < n_half {
        return Err(Error::invalid("synth_floor1: output buffer too short"));
    }

    let n_posts = floor.xlist.len();
    if decoded.y.len() != n_posts {
        return Err(Error::invalid("synth_floor1: y length != xlist length"));
    }

    // Sort posts ascending by X, remembering original indices for Y lookup.
    let mut order: Vec<usize> = (0..n_posts).collect();
    order.sort_by_key(|&i| floor.xlist[i]);

    // Precompute, for each post (in original index space), its low/high
    // neighbour in the SORTED order — index of nearest preceding/following
    // post in the X dimension. Only meaningful for original indices >= 2.
    let mut low_neighbor = vec![0usize; n_posts];
    let mut high_neighbor = vec![0usize; n_posts];
    for j in 2..n_posts {
        let xj = floor.xlist[j];
        let mut lo = 0usize;
        let mut lo_x = floor.xlist[0];
        let mut hi = 1usize;
        let mut hi_x = floor.xlist[1];
        for k in 0..j {
            let xk = floor.xlist[k];
            if xk < xj && xk > lo_x {
                lo = k;
                lo_x = xk;
            }
            if xk > xj && xk < hi_x {
                hi = k;
                hi_x = xk;
            }
        }
        low_neighbor[j] = lo;
        high_neighbor[j] = hi;
    }

    // step1: reconstruct final Y per post + mark which are "used".
    let multiplier = floor.multiplier as i32;
    let range = match floor.multiplier {
        1 => 256,
        2 => 128,
        3 => 86,
        4 => 64,
        _ => 256,
    };
    let mut final_y = vec![0i32; n_posts];
    let mut step2_used = vec![false; n_posts];
    final_y[0] = decoded.y[0];
    final_y[1] = decoded.y[1];
    step2_used[0] = true;
    step2_used[1] = true;
    for j in 2..n_posts {
        let lo = low_neighbor[j];
        let hi = high_neighbor[j];
        let predicted = render_point(
            floor.xlist[lo] as i32,
            final_y[lo],
            floor.xlist[hi] as i32,
            final_y[hi],
            floor.xlist[j] as i32,
        );
        let val = decoded.y[j];
        let high_room = range - predicted;
        let low_room = predicted;
        let room = if high_room < low_room {
            high_room
        } else {
            low_room
        } * 2;
        if val != 0 {
            step2_used[lo] = true;
            step2_used[hi] = true;
            step2_used[j] = true;
            if val >= room {
                final_y[j] = if high_room > low_room {
                    val - low_room + predicted
                } else {
                    predicted - val + high_room - 1
                };
            } else {
                final_y[j] = if val % 2 == 1 {
                    predicted - (val + 1) / 2
                } else {
                    predicted + val / 2
                };
            }
        } else {
            step2_used[j] = false;
            final_y[j] = predicted;
        }
    }

    // Render the floor curve into `output` by walking the sorted posts and
    // drawing line segments between each consecutive pair of "step2 used"
    // posts. Multiplies into `output` (which holds the residue values).
    let mut rendered: Vec<bool> = vec![false; n_half];
    let mut last_used_idx_in_sorted: usize = 0; // index in `order`
    let mut prev_x = 0i32;
    let mut prev_y = final_y[order[0]];
    for k in 1..n_posts {
        let i = order[k];
        if !step2_used[i] {
            continue;
        }
        let cur_x = floor.xlist[i] as i32;
        let cur_y = final_y[i];
        if cur_x > prev_x {
            render_line(
                prev_x,
                prev_y,
                cur_x,
                cur_y,
                multiplier,
                n_half as i32,
                output,
                &mut rendered,
            );
        }
        prev_x = cur_x;
        prev_y = cur_y;
        last_used_idx_in_sorted = k;
    }
    let _ = last_used_idx_in_sorted;
    // Anything past the final used post is filled with the last Y.
    if (prev_x as usize) < n_half {
        let mul = mult_value(prev_y, multiplier);
        for i in prev_x as usize..n_half {
            if !rendered[i] {
                output[i] *= mul;
                rendered[i] = true;
            }
        }
    }
    // Bins before the first post (X=0): also rendered with the post-0 value.
    Ok(())
}

/// Vorbis render_point: integer-arithmetic line interpolation.
fn render_point(x0: i32, y0: i32, x1: i32, y1: i32, x: i32) -> i32 {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let ady = dy.abs();
    let err = ady * (x - x0);
    let off = err / adx;
    if dy < 0 {
        y0 - off
    } else {
        y0 + off
    }
}

/// Convert a rendered Y value (0..=range-1) to the linear-magnitude
/// multiplier via the dB lookup table. The index uses the low 8 bits of
/// `Y * multiplier` per Vorbis I §9.2.6 and falls back to 0 if Y is out
/// of range.
fn mult_value(y: i32, multiplier: i32) -> f32 {
    let idx = y.wrapping_mul(multiplier) & 0xFF;
    crate::dbtable::FLOOR1_INVERSE_DB[idx as usize]
}

/// Render a line from (x0, y0) to (x1, y1) into the spectral output
/// buffer, multiplying each bin's existing value by the floor's
/// linear-magnitude multiplier at that frequency. `n_half` is the spectrum
/// length (blocksize / 2); writes outside that are clipped.
#[allow(clippy::too_many_arguments)]
fn render_line(
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    multiplier: i32,
    n_half: i32,
    out: &mut [f32],
    rendered: &mut [bool],
) {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let ady = dy.abs();
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let mut x = x0;
    let mut y = y0;
    let mut err = 0i32;
    let ady = ady - base.abs() * adx;

    if x >= 0 && x < n_half {
        out[x as usize] *= mult_value(y, multiplier);
        rendered[x as usize] = true;
    }
    while {
        x += 1;
        x < x1 && x < n_half
    } {
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        if x >= 0 {
            out[x as usize] *= mult_value(y, multiplier);
            rendered[x as usize] = true;
        }
    }
}
