//! Macroblock-type bookkeeping for I-slices — ITU-T H.264 §7.4.5 Table 7-11.
//!
//! For an I-slice the parser decodes a `mb_type` ue(v). The encoded value
//! `mbType` distinguishes:
//!
//! * `0` → `I_NxN`: macroblock is partitioned into sixteen 4×4 luma blocks
//!   each with its own intra mode (or eight 8×8 if `transform_8x8_mode_flag`
//!   — not handled here).
//! * `1..=24` → `I_16x16`: full-block luma prediction, parameterised by
//!   `(intra16x16_mode, cbp_luma, cbp_chroma)`.
//! * `25` → `I_PCM`: raw uncompressed bytes follow.
//!
//! Table 7-11 maps mb_type → (Intra16x16PredMode, CodedBlockPatternLuma,
//! CodedBlockPatternChroma).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IMbType {
    /// `I_NxN` — 16 separate 4×4 luma intra-mode blocks.
    INxN,
    /// `I_16x16` with parameters baked in.
    I16x16 {
        /// 0..=3 (Vertical / Horizontal / DC / Plane).
        intra16x16_pred_mode: u8,
        /// 0 = no luma AC, 15 = all four 8×8 blocks have AC.
        cbp_luma: u8,
        /// 0 = no chroma, 1 = chroma DC only, 2 = chroma DC + AC.
        cbp_chroma: u8,
    },
    IPcm,
}

/// Decode an I-slice mb_type into the table 7-11 components.
///
/// The mapping table is taken from FFmpeg's `i_mb_type_info` (libavcodec
/// h264data.h, originally derived from H.264 §7.4.5 Table 7-11). The
/// `intra16x16_pred_mode` values use the FFmpeg numbering where
/// `0=Vertical`, `1=Horizontal`, `2=DC`, `3=Plane` — same as ours.
pub fn decode_i_slice_mb_type(mb_type: u32) -> Option<IMbType> {
    match mb_type {
        0 => Some(IMbType::INxN),
        25 => Some(IMbType::IPcm),
        n if (1..=24).contains(&n) => {
            // Per FFmpeg's i_mb_type_info[1..=24] table.
            // (pred_mode, cbp_chroma, cbp_luma) for each entry:
            const TABLE: [(u8, u8, u8); 24] = [
                (2, 0, 0), // mb_type 1: DC, cbp_chroma=0, cbp_luma=0
                (1, 0, 0), // 2: Horizontal
                (0, 0, 0), // 3: Vertical
                (3, 0, 0), // 4: Plane
                (2, 1, 0), // 5: DC, cbp_chroma=1, cbp_luma=0
                (1, 1, 0),
                (0, 1, 0),
                (3, 1, 0),
                (2, 2, 0), // 9: DC, cbp_chroma=2, cbp_luma=0
                (1, 2, 0),
                (0, 2, 0),
                (3, 2, 0),
                (2, 0, 15), // 13: DC, cbp_chroma=0, cbp_luma=15
                (1, 0, 15),
                (0, 0, 15),
                (3, 0, 15),
                (2, 1, 15), // 17: DC, cbp_chroma=1, cbp_luma=15
                (1, 1, 15),
                (0, 1, 15),
                (3, 1, 15),
                (2, 2, 15), // 21: DC, cbp_chroma=2, cbp_luma=15
                (1, 2, 15),
                (0, 2, 15),
                (3, 2, 15),
            ];
            let (pred, cbp_chroma, cbp_luma) = TABLE[(n - 1) as usize];
            Some(IMbType::I16x16 {
                intra16x16_pred_mode: pred,
                cbp_luma,
                cbp_chroma,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mb_type_zero_is_inxn() {
        assert!(matches!(decode_i_slice_mb_type(0), Some(IMbType::INxN)));
    }

    #[test]
    fn mb_type_one() {
        // mb_type 1 -> I_16x16, DC mode (per FFmpeg's table), cbp=0.
        // (FFmpeg's "pred_mode" numbering treats 0/1/2/3 as Vert/Hor/DC/Plane;
        // the spec mb_type 1 maps to internal pred_mode = 2 = DC.)
        match decode_i_slice_mb_type(1).unwrap() {
            IMbType::I16x16 {
                intra16x16_pred_mode,
                cbp_luma,
                cbp_chroma,
            } => {
                assert_eq!(intra16x16_pred_mode, 2);
                assert_eq!(cbp_luma, 0);
                assert_eq!(cbp_chroma, 0);
            }
            _ => panic!("wrong"),
        }
    }
}
