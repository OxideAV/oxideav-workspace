//! Video-packet resync-marker detection (ISO/IEC 14496-2 §6.3.5.2).
//!
//! When a VOL has `resync_marker_disable == 0` the encoder is free to splice
//! a "video packet" header after any complete macroblock. The packet header
//! consists of:
//!
//! 1. **stuffing bits** — `0` followed by 0..=7 ones, just enough to byte-
//!    align the bitstream (always at least one bit; if already aligned, a
//!    full `0_1111111` byte is emitted).
//! 2. **resync_marker** — `N` zero bits followed by `1`, where
//!    `N == get_video_packet_prefix_length(pict_type, f_code, b_code)`
//!    (16 for I-VOPs).
//! 3. **macroblock_number** — `ceil(log2(mb_num)) + 1` bits naming the next
//!    MB to decode (zero-indexed, scan order).
//! 4. **quant_scale** — `quant_precision` bits (default 5).
//! 5. **header_extension_code (HEC)** — 1 bit. If set, additional fields
//!    follow (modulo_time_base, marker, vop_time_increment, marker, type,
//!    intra_dc_vlc_thr, [f_code/b_code if not I]).
//!
//! After consumption the decoder resumes at the macroblock indicated by
//! `mb_num`, with the new `quant_scale` in effect. AC/DC predictors are
//! reset across packet boundaries (§7.4.3 — neighbour blocks not in the
//! same packet are unavailable).
//!
//! Detection without consumption is keyed off the encoder's stuffing rule:
//! at any decode position `bits_count` (zero-indexed, MSB-first), the next
//! 16 bits of a valid resync marker are uniquely determined by
//! `bits_count & 7`. The `RESYNC_PREFIX_BY_BIT_ALIGN` table mirrors
//! FFmpeg's `mpeg4_resync_prefix`.

use oxideav_core::{Error, Result};

use crate::bitreader::BitReader;
use crate::headers::vol::VideoObjectLayer;
use crate::headers::vop::{VideoObjectPlane, VopCodingType};

/// First 16 bits of a stuffed resync-marker, indexed by `bits_count & 7`
/// of the decoder's position before the stuffing.
///
/// For the `align == 0` (byte-aligned) case the stuffing fills the byte:
/// `01111111` (1 zero + 7 ones), then the marker zeros begin. So the next
/// 16 bits are `01111111_00000000` = `0x7F00`. Other alignments shift the
/// stuffing-zero into the trailing window. See FFmpeg
/// `mpeg4_resync_prefix` for the matching encoder/decoder constants.
const RESYNC_PREFIX_BY_BIT_ALIGN: [u16; 8] = [
    0x7F00, 0x7E00, 0x7C00, 0x7800, 0x7000, 0x6000, 0x4000, 0x0000,
];

/// `ff_mpeg4_get_video_packet_prefix_length` — number of zero bits in the
/// resync_marker proper (excluding the trailing `1` and the stuffing).
///
/// Source: spec §6.3.5.2; FFmpeg `mpeg4video.c`.
pub fn video_packet_prefix_length(coding_type: VopCodingType, f_code: u8, b_code: u8) -> u32 {
    match coding_type {
        VopCodingType::I => 16,
        VopCodingType::P | VopCodingType::S => (f_code as u32) + 15,
        VopCodingType::B => f_code.max(b_code).max(2) as u32 + 15,
    }
}

/// Number of bits used to encode a macroblock number in a video-packet
/// header — `ceil(log2(mb_num - 1)) + 1` per spec.
pub fn mb_num_bits(mb_count: u32) -> u32 {
    if mb_count <= 1 {
        return 1;
    }
    let v = mb_count - 1;
    32 - v.leading_zeros()
}

/// Outcome of a `try_consume_resync_marker` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResyncResult {
    /// No marker detected; the bit position is unchanged.
    None,
    /// Marker consumed; decoding should resume at the indicated MB number
    /// (flat scan order, zero-indexed) using `new_quant`.
    Resync { mb_num: u32, new_quant: u32 },
}

/// Try to detect-and-consume a resync marker at the current bit position.
/// If detected, the packet header is fully consumed (including any HEC
/// payload) and `ResyncResult::Resync { mb_num, new_quant }` is returned.
/// Otherwise the bit position is unchanged and `ResyncResult::None` is
/// returned.
///
/// `vol`/`vop` are needed to compute the marker prefix length and to know
/// the quant precision and HEC payload format.
pub fn try_consume_resync_marker(
    br: &mut BitReader<'_>,
    vol: &VideoObjectLayer,
    vop: &VideoObjectPlane,
    mb_count: u32,
) -> Result<ResyncResult> {
    if vol.resync_marker_disable {
        return Ok(ResyncResult::None);
    }

    // Need at least 16 bits to even probe.
    let remaining = br.bits_remaining();
    if remaining < 16 {
        return Ok(ResyncResult::None);
    }

    let bit_pos = br.bit_position();
    let align = (bit_pos & 7) as usize;
    let next16 = br.peek_u32(16)? as u16;
    if next16 != RESYNC_PREFIX_BY_BIT_ALIGN[align] {
        return Ok(ResyncResult::None);
    }

    // Confirm the rest of the marker before committing. For I-VOPs the
    // total marker length (stuffing + 16 zeros + 1) is at most 25 bits.
    // For P/S/B with f_code up to 7 it's still ≤ 32. If the prefix is
    // longer (e.g. f_code=7 with align=0: 8 + 22 + 1 = 31), we still fit.
    let expected_zeros = video_packet_prefix_length(
        vop.vop_coding_type,
        vop.vop_fcode_forward,
        vop.vop_fcode_backward,
    );
    let stuffing_bits = if align == 0 { 8 } else { 8 - align };
    let total = stuffing_bits + (expected_zeros as usize) + 1;
    if total > 32 {
        // Out of scope for the formats we handle (I/P with f_code up to 7).
        return Err(Error::invalid("mpeg4 resync: probe length overflow"));
    }
    if remaining < total as u64 {
        return Ok(ResyncResult::None);
    }

    let probe = br.peek_u32(total as u32)?;
    // Build expected: leading bit `0`, then (stuffing_bits - 1) ones, then
    // expected_zeros zeros, then a `1`.
    let stuffing_pat: u64 = if stuffing_bits == 0 {
        0
    } else {
        // bits: 0_1...1 with width = stuffing_bits → leading 0 then ones.
        (1u64 << (stuffing_bits - 1)) - 1
    };
    let mut expected: u64 = stuffing_pat;
    expected <<= expected_zeros;
    expected = (expected << 1) | 1;
    if (probe as u64) != expected {
        return Ok(ResyncResult::None);
    }
    br.consume(total as u32)?;

    // Now read mb_num, quant_scale, header_extension_code.
    let mb_bits = mb_num_bits(mb_count);
    let mb_num = br.read_u32(mb_bits)?;
    if mb_num >= mb_count || mb_num == 0 {
        // Per spec mb_num must be > 0 (we're not at the very first MB) and
        // must point inside the picture. Many real encoders also forbid 0.
        return Err(Error::invalid("mpeg4 resync: mb_num out of range"));
    }
    let new_quant = br.read_u32(vol.quant_precision as u32)?;
    let hec = br.read_u1()?;
    if hec == 1 {
        // header_extension: modulo_time_base (1s ending in 0), marker,
        // vop_time_increment, marker, vop_coding_type (2), intra_dc_vlc_thr (3),
        // [f_code/b_code if not I]. Drain and discard.
        let mut guard = 0u32;
        loop {
            let b = br.read_u1()?;
            if b == 0 {
                break;
            }
            guard += 1;
            if guard > 60 {
                return Err(Error::invalid("mpeg4 resync HEC: modulo_time_base runaway"));
            }
        }
        br.read_marker()?;
        let _vti = br.read_u32(vol.vop_time_increment_bits)?;
        br.read_marker()?;
        let _ct = br.read_u32(2)?;
        let _ivt = br.read_u32(3)?;
        if vop.vop_coding_type != VopCodingType::I {
            let _fcode = br.read_u32(3)?;
        }
        if vop.vop_coding_type == VopCodingType::B {
            let _bcode = br.read_u32(3)?;
        }
    }

    Ok(ResyncResult::Resync { mb_num, new_quant })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_length_i_vop() {
        assert_eq!(video_packet_prefix_length(VopCodingType::I, 0, 0), 16);
        assert_eq!(video_packet_prefix_length(VopCodingType::P, 1, 0), 16);
        assert_eq!(video_packet_prefix_length(VopCodingType::P, 4, 0), 19);
    }

    #[test]
    fn mb_num_bits_smoke() {
        // 16 MBs need 4 bits.
        assert_eq!(mb_num_bits(16), 4);
        // 17 MBs need 5 bits.
        assert_eq!(mb_num_bits(17), 5);
        // 1 MB needs 1 bit.
        assert_eq!(mb_num_bits(1), 1);
    }

    #[test]
    fn detect_aligned_marker() {
        // Synthetic I-VOP marker at byte boundary:
        //   stuffing 01111111 (1 byte)
        //   16 zeros (2 bytes)
        //   `1` then mb_num=4 (4 bits) then quant=3 (5 bits) then HEC=0
        // Layout (MSB-first):
        //   01111111 00000000 00000000 1 0100 00011 0 ...
        // Pack into bytes:
        //   0111_1111 = 0x7F
        //   0000_0000 = 0x00
        //   0000_0000 = 0x00
        //   1010_0000 = 0xA0
        //   1100_0000 = 0xC0  (ends with don't-care)
        let data = [0x7F, 0x00, 0x00, 0xA0, 0xC0];
        let mut br = BitReader::new(&data);
        let vol = synth_vol();
        let vop = synth_vop_i();
        let r = try_consume_resync_marker(&mut br, &vol, &vop, 16).unwrap();
        match r {
            ResyncResult::Resync { mb_num, new_quant } => {
                assert_eq!(mb_num, 4);
                assert_eq!(new_quant, 3);
            }
            _ => panic!("expected resync to be detected, got {:?}", r),
        }
    }

    #[test]
    fn no_marker_when_disabled() {
        let data = [0x7F, 0x00, 0x00, 0xA0];
        let mut br = BitReader::new(&data);
        let mut vol = synth_vol();
        vol.resync_marker_disable = true;
        let vop = synth_vop_i();
        assert_eq!(
            try_consume_resync_marker(&mut br, &vol, &vop, 16).unwrap(),
            ResyncResult::None
        );
    }

    #[test]
    fn no_marker_on_random_bits() {
        let data = [0xAB, 0xCD, 0xEF, 0x12];
        let mut br = BitReader::new(&data);
        let vol = synth_vol();
        let vop = synth_vop_i();
        assert_eq!(
            try_consume_resync_marker(&mut br, &vol, &vop, 16).unwrap(),
            ResyncResult::None
        );
        // Bit position must be unchanged.
        assert_eq!(br.bit_position(), 0);
    }

    fn synth_vol() -> VideoObjectLayer {
        use crate::headers::vol::{AspectRatioInfo, ChromaFormat, ShapeType};
        VideoObjectLayer {
            random_accessible_vol: false,
            video_object_type_indication: 1,
            is_object_layer_identifier: false,
            verid: 1,
            priority: 0,
            aspect_ratio_info: AspectRatioInfo::Square,
            vol_control_parameters: false,
            chroma_format: ChromaFormat::Yuv420,
            low_delay: false,
            vbv_parameters_present: false,
            shape: ShapeType::Rectangular,
            vop_time_increment_resolution: 10,
            vop_time_increment_bits: 4,
            fixed_vop_rate: false,
            fixed_vop_time_increment: 1,
            width: 64,
            height: 64,
            interlaced: false,
            obmc_disable: true,
            sprite_enable: 0,
            not_8_bit: false,
            quant_precision: 5,
            bits_per_pixel: 8,
            mpeg_quant: false,
            intra_quant_matrix: None,
            non_intra_quant_matrix: None,
            quarter_sample: false,
            complexity_estimation_disable: true,
            resync_marker_disable: false,
            data_partitioned: false,
            reversible_vlc: false,
            newpred_enable: false,
            reduced_resolution_vop_enable: false,
            scalability: false,
        }
    }

    fn synth_vop_i() -> VideoObjectPlane {
        VideoObjectPlane {
            vop_coding_type: VopCodingType::I,
            modulo_time_base: 0,
            vop_time_increment: 0,
            vop_coded: true,
            rounding_type: false,
            intra_dc_vlc_thr: 0,
            vop_quant: 3,
            vop_fcode_forward: 0,
            vop_fcode_backward: 0,
            width: 64,
            height: 64,
        }
    }
}
