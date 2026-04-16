//! H.263 picture header parser — Annex C §5.1 of ITU-T Rec. H.263 (02/98).
//!
//! Layout (baseline, no PLUSPTYPE):
//!
//! | Field       | Bits  | Notes                                            |
//! |-------------|-------|--------------------------------------------------|
//! | PSC         | 22    | `0000 0000 0000 0000 1 00000`                    |
//! | TR          | 8     | Temporal reference                               |
//! | PTYPE bit 1 | 1     | Always `1` (start-code emulation prevention)     |
//! | PTYPE bit 2 | 1     | Always `0` (distinguishes from H.261)            |
//! | PTYPE bit 3 | 1     | Split-screen indicator                           |
//! | PTYPE bit 4 | 1     | Document-camera indicator                        |
//! | PTYPE bit 5 | 1     | Freeze-picture release                           |
//! | Source fmt  | 3     | 1=sub-QCIF .. 5=16CIF; 7 = PLUSPTYPE follows     |
//! | PType       | 1     | 0 = I-picture, 1 = P-picture                     |
//! | Annex flags | 4     | UMV (D), SAC (E), AP (F), PB-frames (G)          |
//! | PQUANT      | 5     | Quantiser 1..=31                                 |
//! | CPM         | 1     | Continuous presence multipoint mode              |
//! | PSBI        | 2     | Present iff CPM == 1                             |
//! | TRB         | 3     | Present iff PB-frames mode                       |
//! | DBQUANT     | 2     | Present iff PB-frames mode                       |
//! | PEI/PSPARE  | n     | 1-bit PEI, then 8-bit PSPARE if PEI==1, repeat   |
//!
//! GOB data immediately follows the header (no further alignment required).

use oxideav_core::{Error, Result};
use oxideav_mpeg4video::bitreader::BitReader;

/// H.263 source-format codes (PTYPE bits 6-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceFormat {
    /// Forbidden value `000`.
    Forbidden,
    /// `001` — sub-QCIF, 128 × 96 luma.
    SubQcif,
    /// `010` — QCIF, 176 × 144 luma.
    Qcif,
    /// `011` — CIF, 352 × 288 luma.
    Cif,
    /// `100` — 4CIF, 704 × 576 luma.
    FourCif,
    /// `101` — 16CIF, 1408 × 1152 luma.
    SixteenCif,
    /// `110` — reserved.
    Reserved,
    /// `111` — extended PTYPE (H.263+); not supported.
    Extended,
}

impl SourceFormat {
    pub fn from_code(c: u8) -> Self {
        match c & 0x7 {
            0 => SourceFormat::Forbidden,
            1 => SourceFormat::SubQcif,
            2 => SourceFormat::Qcif,
            3 => SourceFormat::Cif,
            4 => SourceFormat::FourCif,
            5 => SourceFormat::SixteenCif,
            6 => SourceFormat::Reserved,
            7 => SourceFormat::Extended,
            _ => unreachable!(),
        }
    }

    /// Picture dimensions `(width, height)` in luma samples. Returns `None`
    /// for forbidden / reserved / extended formats.
    pub fn dimensions(self) -> Option<(u32, u32)> {
        match self {
            SourceFormat::SubQcif => Some((128, 96)),
            SourceFormat::Qcif => Some((176, 144)),
            SourceFormat::Cif => Some((352, 288)),
            SourceFormat::FourCif => Some((704, 576)),
            SourceFormat::SixteenCif => Some((1408, 1152)),
            _ => None,
        }
    }

    /// Pick the source-format code that exactly matches `(w, h)`. Returns
    /// `None` for non-standard dimensions (H.263 baseline cannot signal
    /// arbitrary sizes — that requires PLUSPTYPE).
    pub fn for_dimensions(w: u32, h: u32) -> Option<Self> {
        match (w, h) {
            (128, 96) => Some(SourceFormat::SubQcif),
            (176, 144) => Some(SourceFormat::Qcif),
            (352, 288) => Some(SourceFormat::Cif),
            (704, 576) => Some(SourceFormat::FourCif),
            (1408, 1152) => Some(SourceFormat::SixteenCif),
            _ => None,
        }
    }

    /// Number of GOBs in a picture of this source format. Sub-QCIF, QCIF use
    /// 6 GOBs of one MB row each (and one GOB at half height for sub-QCIF).
    /// CIF / 4CIF / 16CIF have GOBs spanning multiple MB rows.
    ///
    /// Returns `(num_gobs, mb_rows_per_gob)`.
    pub fn gob_layout(self) -> Option<(u32, u32)> {
        match self {
            SourceFormat::SubQcif => Some((6, 1)),
            SourceFormat::Qcif => Some((9, 1)),
            SourceFormat::Cif => Some((18, 1)),
            SourceFormat::FourCif => Some((18, 2)),
            SourceFormat::SixteenCif => Some((18, 4)),
            _ => None,
        }
    }
}

/// Picture coding type (PTYPE bit 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureCodingType {
    Intra,
    Predicted,
}

/// Parsed H.263 picture header.
#[derive(Clone, Debug)]
pub struct PictureHeader {
    pub temporal_reference: u8,
    pub split_screen: bool,
    pub document_camera: bool,
    pub freeze_release: bool,
    pub source_format: SourceFormat,
    pub coding_type: PictureCodingType,
    pub umv_mode: bool,
    pub sac_mode: bool,
    pub advanced_prediction: bool,
    pub pb_frames: bool,
    pub pquant: u8,
    pub cpm: bool,
    pub psbi: u8,
    pub trb: u8,
    pub dbquant: u8,
    pub width: u32,
    pub height: u32,
}

/// Parse the picture header that follows the 22-bit PSC.
///
/// `br` must be positioned at the start of the byte that contains the PSC.
/// On entry the function consumes the 22-bit PSC and validates it.
pub fn parse_picture_header(br: &mut BitReader<'_>) -> Result<PictureHeader> {
    // PSC: 22 bits = 0x0000_8000 >> 10 in MSB form. Read as 22 bits and
    // compare against the constant.
    let psc = br.read_u32(22)?;
    // 0000 0000 0000 0000 1 00000 = bit 17 is 1, the remaining low 5 bits
    // of the 22-bit value are zero. As an integer this is 0x20.
    #[allow(clippy::unusual_byte_groupings)]
    const PSC_VALUE: u32 = 0b00_0000_0000_0000_0000_1_00000;
    if psc != PSC_VALUE {
        return Err(Error::invalid(format!(
            "h263 picture: bad PSC 0x{psc:06x} (want 0x{PSC_VALUE:06x})"
        )));
    }

    let tr = br.read_u32(8)? as u8;

    // PTYPE bits 1-13.
    let always_one = br.read_u1()?;
    if always_one != 1 {
        return Err(Error::invalid("h263 picture: PTYPE bit 1 must be 1"));
    }
    let always_zero = br.read_u1()?;
    if always_zero != 0 {
        return Err(Error::invalid("h263 picture: PTYPE bit 2 must be 0"));
    }
    let split_screen = br.read_u1()? == 1;
    let document_camera = br.read_u1()? == 1;
    let freeze_release = br.read_u1()? == 1;
    let src_code = br.read_u32(3)? as u8;
    let source_format = SourceFormat::from_code(src_code);
    let coding_bit = br.read_u1()?;
    let coding_type = if coding_bit == 0 {
        PictureCodingType::Intra
    } else {
        PictureCodingType::Predicted
    };
    let umv_mode = br.read_u1()? == 1;
    let sac_mode = br.read_u1()? == 1;
    let advanced_prediction = br.read_u1()? == 1;
    let pb_frames = br.read_u1()? == 1;

    // Reject anything that needs out-of-scope annexes for v1.
    if matches!(
        source_format,
        SourceFormat::Forbidden | SourceFormat::Reserved
    ) {
        return Err(Error::invalid("h263 picture: forbidden source format"));
    }
    if source_format == SourceFormat::Extended {
        return Err(Error::unsupported(
            "h263 PLUSPTYPE / H.263+ extended picture format: follow-up",
        ));
    }
    if umv_mode {
        return Err(Error::unsupported(
            "h263 Annex D unrestricted MV mode: follow-up",
        ));
    }
    if sac_mode {
        return Err(Error::unsupported(
            "h263 Annex E syntax-based arithmetic coding: follow-up",
        ));
    }
    if advanced_prediction {
        return Err(Error::unsupported(
            "h263 Annex F advanced prediction (4MV / OBMC): follow-up",
        ));
    }
    if pb_frames {
        return Err(Error::unsupported(
            "h263 Annex G PB-frames mode: follow-up (B-pictures)",
        ));
    }

    let pquant = br.read_u32(5)? as u8;
    if pquant == 0 {
        return Err(Error::invalid("h263 picture: PQUANT == 0"));
    }

    let cpm = br.read_u1()? == 1;
    let psbi = if cpm { br.read_u32(2)? as u8 } else { 0 };
    if cpm {
        return Err(Error::unsupported(
            "h263 CPM continuous-presence multipoint: follow-up",
        ));
    }

    // PB-frames extras would go here if pb_frames were set — already rejected.
    let trb = 0u8;
    let dbquant = 0u8;

    // PEI / PSPARE loop.
    loop {
        let pei = br.read_u1()?;
        if pei == 0 {
            break;
        }
        let _pspare = br.read_u32(8)?;
    }

    let (width, height) = source_format
        .dimensions()
        .ok_or_else(|| Error::unsupported("h263 picture: source format has no fixed dimensions"))?;

    Ok(PictureHeader {
        temporal_reference: tr,
        split_screen,
        document_camera,
        freeze_release,
        source_format,
        coding_type,
        umv_mode,
        sac_mode,
        advanced_prediction,
        pb_frames,
        pquant,
        cpm,
        psbi,
        trb,
        dbquant,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the byte sequence for a minimal sub-QCIF I-picture header with
    /// PQUANT=5, no CPM, no PEI, no annexes. Mirrors the bit layout produced
    /// by `ffmpeg -c:v h263 -qscale:v 5` for a 128×96 source.
    fn minimal_subqcif_iframe() -> Vec<u8> {
        // Bit stream (50 bits, padded with zeros to byte boundary):
        //   PSC(22)     = 0000 0000 0000 0000 1 00000
        //   TR(8)       = 00000000
        //   PTYPE(13)   = 1 0 0 0 0 001 0 0 0 0 0
        //                 (marker, id, split, cam, freeze, fmt=1, I, all annex 0)
        //   PQUANT(5)   = 00101 (=5)
        //   CPM(1)      = 0
        //   PEI(1)      = 0
        // Concatenated: 0000 0000 0000 0000 1000 0000 0000 0010 0000 0100 0000 0101 0010 0000
        //              = 00 00 80 02 04 05 20 (with 0x20 trailing)
        vec![0x00, 0x00, 0x80, 0x02, 0x04, 0x05, 0x20]
    }

    #[test]
    fn parses_subqcif_iframe() {
        let data = minimal_subqcif_iframe();
        let mut br = BitReader::new(&data);
        let p = parse_picture_header(&mut br).unwrap();
        assert_eq!(p.temporal_reference, 0);
        assert_eq!(p.source_format, SourceFormat::SubQcif);
        assert_eq!(p.coding_type, PictureCodingType::Intra);
        assert_eq!(p.pquant, 5);
        assert!(!p.cpm);
        assert_eq!(p.width, 128);
        assert_eq!(p.height, 96);
    }
}
