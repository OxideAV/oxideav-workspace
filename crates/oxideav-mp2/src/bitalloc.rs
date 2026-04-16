//! Bit-allocation and scalefactor decoding for MPEG-1 Audio Layer II.
//!
//! Reads, in this order:
//!   1. Per-subband bit-allocation indices (§2.4.1.6, §2.4.2.7).
//!   2. SCFSI — scalefactor selection info (§2.4.1.7, §2.4.2.7), 2 bits per
//!      subband whose allocation is non-zero.
//!   3. Scalefactors themselves (§2.4.1.8), 0–3 six-bit values per subband
//!      depending on SCFSI.
//!
//! The result is a `Layer2Side` struct holding, for each channel and
//! subband, the chosen allocation class and three scalefactors (one per
//! 1/3-frame "sbgroup" — Layer II splits its 36-sample subband into three
//! 12-sample groups, §2.4.2.1).

use crate::bitreader::BitReader;
use crate::header::Mode;
use crate::tables::AllocTable;
use oxideav_core::{Error, Result};

/// Per-subband, per-channel bit allocation and scalefactor information.
#[derive(Clone, Debug)]
pub struct Layer2Side {
    /// `allocation[ch][sb]` — allocation index read from the frame. 0 means
    /// "no samples transmitted"; values ≥ 1 index into the chosen
    /// allocation table's quantisation-class list.
    pub allocation: [[u8; 32]; 2],
    /// `scalefactor[ch][sb][part]` — part ∈ {0, 1, 2} is the 1/3-frame
    /// index. Entry is 0 (spec reserves index 63 for invalid and produces
    /// silent output).
    pub scalefactor: [[[u8; 3]; 32]; 2],
    /// Number of subbands that actually carry data (= `sblimit` from the
    /// active allocation table).
    pub sblimit: usize,
    /// Joint-stereo boundary: subbands `[bound..sblimit)` share a single
    /// allocation value between the two channels.
    pub bound: usize,
    /// Total number of channels in the stream.
    pub channels: usize,
}

impl Layer2Side {
    pub fn new(sblimit: usize, bound: usize, channels: usize) -> Self {
        Self {
            allocation: [[0u8; 32]; 2],
            scalefactor: [[[0u8; 3]; 32]; 2],
            sblimit,
            bound,
            channels,
        }
    }
}

/// Read the bit-allocation + SCFSI + scalefactor payload from the
/// bitstream and return the decoded `Layer2Side`.
///
/// `mode` determines channel/stereo layout and the intensity-stereo bound.
/// `sblimit_bound` is `min(bound, sblimit)` — subbands below `bound` have
/// independent allocations per channel; subbands at or above share one
/// allocation value (intensity stereo).
pub fn read_layer2_side(
    br: &mut BitReader<'_>,
    table: &AllocTable,
    mode: Mode,
    bound_sb: usize,
) -> Result<Layer2Side> {
    let channels = match mode {
        Mode::Mono => 1,
        _ => 2,
    };
    let sblimit = table.sblimit;
    let bound = bound_sb.min(sblimit);

    let mut side = Layer2Side::new(sblimit, bound, channels);

    // --- 1. Bit allocation ---
    // Subbands below the bound: per-channel allocation.
    for sb in 0..bound {
        let nbal = table.nbal(sb);
        for ch in 0..channels {
            side.allocation[ch][sb] = br.read_u32(nbal)? as u8;
        }
    }
    // Subbands at-or-above the bound: single allocation shared across
    // channels in stereo/joint-stereo/dual modes; mono reads one.
    for sb in bound..sblimit {
        let nbal = table.nbal(sb);
        let alloc = br.read_u32(nbal)? as u8;
        for ch in 0..channels {
            side.allocation[ch][sb] = alloc;
        }
    }

    // --- 2. SCFSI — two bits per active (allocation != 0) subband*channel ---
    // SCFSI values:
    //   0: three independent scalefactors (no reuse)
    //   1: scalefactors 0 and 1 share; third is separate
    //   2: one scalefactor applies to all three parts
    //   3: scalefactor 0 separate; parts 1 and 2 share
    let mut scfsi = [[0u8; 32]; 2];
    for sb in 0..sblimit {
        for ch in 0..channels {
            if side.allocation[ch][sb] != 0 {
                scfsi[ch][sb] = br.read_u32(2)? as u8;
            }
        }
    }

    // --- 3. Scalefactors — 6 bits per field ---
    for sb in 0..sblimit {
        for ch in 0..channels {
            if side.allocation[ch][sb] == 0 {
                continue;
            }
            let s = scfsi[ch][sb];
            match s {
                0 => {
                    // Three independent scalefactors.
                    side.scalefactor[ch][sb][0] = br.read_u32(6)? as u8;
                    side.scalefactor[ch][sb][1] = br.read_u32(6)? as u8;
                    side.scalefactor[ch][sb][2] = br.read_u32(6)? as u8;
                }
                1 => {
                    // Parts 0 and 1 share; part 2 separate.
                    let a = br.read_u32(6)? as u8;
                    let c = br.read_u32(6)? as u8;
                    side.scalefactor[ch][sb][0] = a;
                    side.scalefactor[ch][sb][1] = a;
                    side.scalefactor[ch][sb][2] = c;
                }
                2 => {
                    // One scalefactor applies to all three parts.
                    let a = br.read_u32(6)? as u8;
                    side.scalefactor[ch][sb][0] = a;
                    side.scalefactor[ch][sb][1] = a;
                    side.scalefactor[ch][sb][2] = a;
                }
                _ => {
                    // SCFSI = 3: part 0 separate, parts 1 and 2 share.
                    let a = br.read_u32(6)? as u8;
                    let c = br.read_u32(6)? as u8;
                    side.scalefactor[ch][sb][0] = a;
                    side.scalefactor[ch][sb][1] = c;
                    side.scalefactor[ch][sb][2] = c;
                }
            }
        }
    }

    let _ = scfsi; // kept around for potential diagnostic dumps.
    Ok(side)
}

/// Sanity-check that any allocation index stays within the subband's class
/// list bounds. This catches malformed bitstreams early.
pub fn validate_allocations(side: &Layer2Side, table: &AllocTable) -> Result<()> {
    for sb in 0..side.sblimit {
        let nbal = table.nbal(sb);
        let max = 1u32 << nbal;
        for ch in 0..side.channels {
            if (side.allocation[ch][sb] as u32) >= max {
                return Err(Error::invalid(format!(
                    "mp2: allocation[{ch}][{sb}] = {} out of range (nbal={nbal})",
                    side.allocation[ch][sb]
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::TABLE_B2A;

    #[test]
    fn zero_stream_produces_zero_side_info() {
        // 400 bytes of zeros: every allocation and scalefactor = 0.
        let buf = vec![0u8; 400];
        let mut br = BitReader::new(&buf);
        let side = read_layer2_side(&mut br, &TABLE_B2A, Mode::Stereo, 32).expect("read side info");
        assert_eq!(side.sblimit, 27);
        for ch in 0..2 {
            for sb in 0..27 {
                assert_eq!(side.allocation[ch][sb], 0);
                assert_eq!(side.scalefactor[ch][sb], [0u8; 3]);
            }
        }
    }

    #[test]
    fn mono_reads_only_one_channel() {
        let buf = vec![0u8; 400];
        let mut br = BitReader::new(&buf);
        let side = read_layer2_side(&mut br, &TABLE_B2A, Mode::Mono, 32).unwrap();
        assert_eq!(side.channels, 1);
        assert_eq!(side.sblimit, 27);
    }
}
