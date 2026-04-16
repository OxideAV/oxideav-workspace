//! MPEG-1 Layer III scalefactor decode.
//!
//! For each granule + channel the scalefactors control per-sfb gain.
//! The scalefac_compress index (4 bits in side info) selects slen1 and
//! slen2 from ISO/IEC 11172-3 Table 3-B.32.
//!
//! Scalefactor band partition:
//! - Long blocks: 21 bands (sfb 0..20) arranged in 4 groups —
//!   sfb 0-5 use slen1, sfb 6-10 use slen1, sfb 11-15 use slen2,
//!   sfb 16-20 use slen2.
//!   If the previous granule of the same channel had the same scfsi
//!   bit set, the scalefactors for that group are reused (not re-sent).
//! - Short (non-switched) blocks: 12 bands × 3 windows, all sent each
//!   granule (scfsi is ignored). Sfb 0-5 use slen1, sfb 6-11 use slen2.
//! - Mixed blocks (block_type == 2 && mixed_block_flag): sfb 0-7 long
//!   use slen1, sfb 3-11 (short ×3) use slen1/slen2 split at sfb 5.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::sideinfo::{GranuleChannel, SideInfo};

/// (slen1, slen2) pair by scalefac_compress (MPEG-1, Table 3-B.32).
pub const SLEN_TABLE: [(u8, u8); 16] = [
    (0, 0),
    (0, 1),
    (0, 2),
    (0, 3),
    (3, 0),
    (1, 1),
    (1, 2),
    (1, 3),
    (2, 1),
    (2, 2),
    (2, 3),
    (3, 1),
    (3, 2),
    (3, 3),
    (4, 2),
    (4, 3),
];

/// Decoded scalefactors for a single (granule, channel).
#[derive(Clone, Debug, Default)]
pub struct ScaleFactors {
    /// Long-block scalefactors, sfb 0..21. Index 22 is never used but
    /// we keep 22 entries for safe indexing when reordering.
    pub l: [u8; 22],
    /// Short-block scalefactors — `s[sfb][window]`.
    pub s: [[u8; 3]; 13],
}

/// Decode scalefactors for MPEG-1 one granule + one channel. `prev` holds
/// the previous granule's scalefactors on the same channel, used for
/// scfsi reuse. Consumes bits from `br`.
pub fn decode_mpeg1(
    br: &mut BitReader<'_>,
    gc: &GranuleChannel,
    scfsi: &[bool; 4],
    gr: usize,
    prev: &ScaleFactors,
) -> Result<ScaleFactors> {
    let (slen1, slen2) = SLEN_TABLE[gc.scalefac_compress as usize];
    let mut sf = ScaleFactors::default();

    if gc.window_switching_flag && gc.block_type == 2 {
        // Short-block or mixed-block case — scfsi is ignored; always send
        // fresh scalefactors.
        if gc.mixed_block_flag {
            // Long portion: sfb 0..7 with slen1.
            for sfb in 0..8 {
                sf.l[sfb] = br.read_u32(slen1 as u32)? as u8;
            }
            // Short portion: sfb 3..5 use slen1, sfb 6..11 use slen2.
            for sfb in 3..6 {
                for win in 0..3 {
                    sf.s[sfb][win] = br.read_u32(slen1 as u32)? as u8;
                }
            }
            for sfb in 6..12 {
                for win in 0..3 {
                    sf.s[sfb][win] = br.read_u32(slen2 as u32)? as u8;
                }
            }
        } else {
            // Pure short block.
            for sfb in 0..6 {
                for win in 0..3 {
                    sf.s[sfb][win] = br.read_u32(slen1 as u32)? as u8;
                }
            }
            for sfb in 6..12 {
                for win in 0..3 {
                    sf.s[sfb][win] = br.read_u32(slen2 as u32)? as u8;
                }
            }
        }
    } else {
        // Long-block case. 4 scfsi groups.
        // Group 0: sfb 0..5, slen1.
        if gr == 0 || !scfsi[0] {
            for sfb in 0..6 {
                sf.l[sfb] = br.read_u32(slen1 as u32)? as u8;
            }
        } else {
            for sfb in 0..6 {
                sf.l[sfb] = prev.l[sfb];
            }
        }
        // Group 1: sfb 6..10, slen1.
        if gr == 0 || !scfsi[1] {
            for sfb in 6..11 {
                sf.l[sfb] = br.read_u32(slen1 as u32)? as u8;
            }
        } else {
            for sfb in 6..11 {
                sf.l[sfb] = prev.l[sfb];
            }
        }
        // Group 2: sfb 11..15, slen2.
        if gr == 0 || !scfsi[2] {
            for sfb in 11..16 {
                sf.l[sfb] = br.read_u32(slen2 as u32)? as u8;
            }
        } else {
            for sfb in 11..16 {
                sf.l[sfb] = prev.l[sfb];
            }
        }
        // Group 3: sfb 16..20, slen2.
        if gr == 0 || !scfsi[3] {
            for sfb in 16..21 {
                sf.l[sfb] = br.read_u32(slen2 as u32)? as u8;
            }
        } else {
            for sfb in 16..21 {
                sf.l[sfb] = prev.l[sfb];
            }
        }
    }

    Ok(sf)
}

/// Decode scalefactors for a whole frame (both granules, all channels
/// present). Returns `[gr][ch]` layout. Reads bits from `br` which must
/// already be positioned at the start of a granule/channel's part2 data.
///
/// `part2_3_length` is tracked externally so callers can resume reading
/// Huffman codes from exactly the right place.
pub fn decode_frame(br: &mut BitReader<'_>, si: &SideInfo) -> Result<[[ScaleFactors; 2]; 2]> {
    let _ = (br, si);
    Err(Error::unsupported(
        "decode_frame placeholder — use decode_mpeg1 per-granule-per-channel",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sideinfo::GranuleChannel;

    #[test]
    fn long_block_scalefactors_compress_0() {
        // scalefac_compress = 0 -> (slen1, slen2) = (0, 0). No bits read.
        let gc = GranuleChannel {
            scalefac_compress: 0,
            ..Default::default()
        };
        let scfsi = [false; 4];
        let data = [0u8; 1];
        let mut br = BitReader::new(&data);
        let prev = ScaleFactors::default();
        let sf = decode_mpeg1(&mut br, &gc, &scfsi, 0, &prev).unwrap();
        assert!(sf.l.iter().all(|&v| v == 0));
    }

    #[test]
    fn scfsi_reuses_from_prev() {
        // Gr 1, scfsi bits set -> take all groups from prev (no bits read).
        let gc = GranuleChannel {
            scalefac_compress: 5, // slen1=1, slen2=1
            ..Default::default()
        };
        let scfsi = [true; 4];
        let mut prev = ScaleFactors::default();
        for (i, v) in prev.l.iter_mut().enumerate() {
            *v = i as u8;
        }
        let data = [0xFFu8; 1];
        let mut br = BitReader::new(&data);
        let sf = decode_mpeg1(&mut br, &gc, &scfsi, 1, &prev).unwrap();
        for i in 0..21 {
            assert_eq!(sf.l[i], i as u8);
        }
    }
}
