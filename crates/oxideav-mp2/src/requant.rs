//! MPEG-1 Audio Layer II sample-group unpack + requantisation.
//!
//! Layer II splits each subband's 36 samples into three 12-sample groups
//! ("sbgroups" in the spec); each 12-sample group is further split into
//! four 3-sample "triples". For each triple the decoder reads either one
//! codeword per sample (ungrouped quantisers) or a single codeword
//! encoding the triple (the 3-, 5-, or 9-level grouped quantisers —
//! §2.4.3.2, Table 3-B.4).
//!
//! # Requantisation (ISO/IEC 11172-3 §2.4.3.4 eqns 2-9..2-11)
//!
//! For an *ungrouped* `b`-bit codeword `v`:
//!   s' = (v + D) * C
//!   s  = s' * scalefactor
//! with `C = 2 / (2^b - 1)` and `D = -(2^(b-1) - 1)`.
//!
//! For *grouped* quantisers (L ∈ {3, 5, 9}) the triple codeword is first
//! unpacked into three per-sample indices `i ∈ {0..L-1}`; each index
//! maps to a fractional amplitude
//!   s' = (2*i - (L-1)) / L
//! which is then multiplied by the scalefactor as usual.

use crate::bitreader::BitReader;
use crate::tables::{scalefactor_magnitude, AllocEntry, AllocTable};
use oxideav_core::{Error, Result};

/// Decoded subband-sample buffer: `samples[ch][sb][i]`, `i = 0..36`.
pub type SubbandSamples = Vec<Vec<[f32; 36]>>;

/// Reader state for one frame's sample-payload parsing.
pub struct ReadState<'a> {
    pub table: &'a AllocTable,
    pub allocation: &'a [[u8; 32]; 2],
    pub scalefactor: &'a [[[u8; 3]; 32]; 2],
    pub channels: usize,
    pub sblimit: usize,
    /// Joint-stereo bound — subbands at-or-above use one shared set of
    /// sample codewords, requantised into each channel with that
    /// channel's own scalefactor.
    pub bound: usize,
}

/// Read the sample payload from the bitstream, ungroup/requantise, and
/// return `samples[ch][sb][0..36]`.
pub fn read_samples(br: &mut BitReader<'_>, st: &ReadState<'_>) -> Result<SubbandSamples> {
    let mut samples: SubbandSamples = (0..st.channels).map(|_| vec![[0.0f32; 36]; 32]).collect();

    for gr in 0..3 {
        for tr in 0..4 {
            let base_idx = gr * 12 + tr * 3;
            // Independent-allocation subbands (ch 0 + ch 1 each transmit).
            for sb in 0..st.bound.min(st.sblimit) {
                for ch in 0..st.channels {
                    read_triple(br, st, ch, sb, gr, base_idx, &mut samples[ch][sb])?;
                }
            }
            // Shared-allocation subbands (joint stereo upper band).
            for sb in st.bound..st.sblimit {
                read_triple_shared(br, st, sb, gr, base_idx, &mut samples)?;
            }
        }
    }

    Ok(samples)
}

/// Read one 3-sample triple into `out_row[base_idx..base_idx+3]`.
fn read_triple(
    br: &mut BitReader<'_>,
    st: &ReadState<'_>,
    ch: usize,
    sb: usize,
    gr: usize,
    base_idx: usize,
    out_row: &mut [f32; 36],
) -> Result<()> {
    let alloc = st.allocation[ch][sb];
    if alloc == 0 {
        return Ok(());
    }
    let entry = class_entry(st.table, sb, alloc);
    let q = decode_entry(entry);
    let sf_mag = scalefactor_magnitude(st.scalefactor[ch][sb][gr]);

    match q {
        QuantCase::Grouped { levels, bits } => {
            let code = br.read_u32(bits)?;
            let triple = ungroup(code, levels)?;
            for i in 0..3 {
                out_row[base_idx + i] = grouped_fraction(triple[i], levels) * sf_mag;
            }
        }
        QuantCase::Ungrouped { bits, c, d } => {
            for i in 0..3 {
                let v = br.read_u32(bits)? as i32;
                out_row[base_idx + i] = ((v + d) as f32) * c * sf_mag;
            }
        }
    }
    Ok(())
}

fn read_triple_shared(
    br: &mut BitReader<'_>,
    st: &ReadState<'_>,
    sb: usize,
    gr: usize,
    base_idx: usize,
    samples: &mut SubbandSamples,
) -> Result<()> {
    let alloc = st.allocation[0][sb];
    if alloc == 0 {
        return Ok(());
    }
    let entry = class_entry(st.table, sb, alloc);
    let q = decode_entry(entry);
    let sf0 = scalefactor_magnitude(st.scalefactor[0][sb][gr]);
    let sf1 = if st.channels == 2 {
        scalefactor_magnitude(st.scalefactor[1][sb][gr])
    } else {
        0.0
    };

    match q {
        QuantCase::Grouped { levels, bits } => {
            let code = br.read_u32(bits)?;
            let triple = ungroup(code, levels)?;
            for i in 0..3 {
                let f = grouped_fraction(triple[i], levels);
                samples[0][sb][base_idx + i] = f * sf0;
                if st.channels == 2 {
                    samples[1][sb][base_idx + i] = f * sf1;
                }
            }
        }
        QuantCase::Ungrouped { bits, c, d } => {
            for i in 0..3 {
                let v = br.read_u32(bits)? as i32;
                let f = ((v + d) as f32) * c;
                samples[0][sb][base_idx + i] = f * sf0;
                if st.channels == 2 {
                    samples[1][sb][base_idx + i] = f * sf1;
                }
            }
        }
    }
    Ok(())
}

fn class_entry(table: &AllocTable, sb: usize, alloc: u8) -> AllocEntry {
    let base = table.offsets[sb];
    table.entries[base + alloc as usize]
}

enum QuantCase {
    /// Grouped 3-, 5- or 9-level quantiser: one codeword of `bits` bits
    /// encodes a triple.
    Grouped { levels: u32, bits: u32 },
    /// Ungrouped: one codeword of `bits` bits per sample. `c` is the
    /// fractional multiplier `2/(2^bits - 1)` and `d` is the additive
    /// centring offset `-(2^(bits-1) - 1)`.
    Ungrouped { bits: u32, c: f32, d: i32 },
}

fn decode_entry(entry: AllocEntry) -> QuantCase {
    let bits = entry.bits as u32;
    let d = entry.d as i32;
    if d > 0 {
        // Grouped. `d` is the level count (3, 5, or 9).
        QuantCase::Grouped {
            levels: d as u32,
            bits,
        }
    } else {
        // Ungrouped. Number of levels encoded in `bits` bits is `2^bits - 1`
        // (spec §2.4.3.4.2: only the 2^b - 1 even codewords are used, with
        // the sign convention expressed via the centring offset `d`).
        let levels = (1u32 << bits) - 1;
        let c = 2.0f64 / (levels as f64);
        QuantCase::Ungrouped {
            bits,
            c: c as f32,
            d,
        }
    }
}

/// Fractional amplitude for a grouped-quantiser sample index.
/// Returns `(2 * idx - (L - 1)) / L`.
fn grouped_fraction(idx: i32, levels: u32) -> f32 {
    let l = levels as f32;
    ((2 * idx) as f32 - (l - 1.0)) / l
}

/// Unpack a grouped 3-/5-/9-level codeword into three per-sample indices.
/// The codeword is a base-L little-endian integer `v = s0 + L*s1 + L²*s2`.
fn ungroup(code: u32, levels: u32) -> Result<[i32; 3]> {
    let l = levels;
    if l != 3 && l != 5 && l != 9 {
        return Err(Error::invalid(format!(
            "mp2: bad grouped-quantiser level count {l}"
        )));
    }
    let s0 = code % l;
    let r = code / l;
    let s1 = r % l;
    let s2 = r / l;
    if s2 >= l {
        return Err(Error::invalid(format!(
            "mp2: grouped codeword {code} out of range for L={l}"
        )));
    }
    Ok([s0 as i32, s1 as i32, s2 as i32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ungroup_base3() {
        assert_eq!(ungroup(0, 3).unwrap(), [0, 0, 0]);
        assert_eq!(ungroup(7, 3).unwrap(), [1, 2, 0]);
        assert_eq!(ungroup(26, 3).unwrap(), [2, 2, 2]);
    }

    #[test]
    fn grouped_fraction_endpoints() {
        // L=3: idx 0 -> -2/3, idx 1 -> 0, idx 2 -> +2/3.
        assert!((grouped_fraction(0, 3) - (-2.0 / 3.0)).abs() < 1e-6);
        assert!(grouped_fraction(1, 3).abs() < 1e-6);
        assert!((grouped_fraction(2, 3) - (2.0 / 3.0)).abs() < 1e-6);
        // L=5: idx 0 -> -4/5, idx 4 -> +4/5.
        assert!((grouped_fraction(0, 5) - (-4.0 / 5.0)).abs() < 1e-6);
        assert!((grouped_fraction(4, 5) - (4.0 / 5.0)).abs() < 1e-6);
    }

    #[test]
    fn ungrouped_center_is_zero() {
        // Decode an entry with bits=4, d=-7 (i.e. 15-level ungrouped).
        let entry = AllocEntry { bits: 4, d: -7 };
        let q = decode_entry(entry);
        match q {
            QuantCase::Ungrouped { bits, c, d } => {
                assert_eq!(bits, 4);
                assert_eq!(d, -7);
                // Center (v = 7) should map to 0.
                let frac = ((7 + d) as f32) * c;
                assert!(frac.abs() < 1e-5);
            }
            _ => panic!("expected ungrouped case"),
        }
    }
}
