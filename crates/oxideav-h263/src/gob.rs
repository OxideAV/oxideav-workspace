//! H.263 GOB header parser — §5.2 of ITU-T Rec. H.263 (02/98).
//!
//! GOB header layout (baseline, no CPM):
//!
//! | Field   | Bits | Notes                                                  |
//! |---------|------|--------------------------------------------------------|
//! | GBSC    | 17   | `0000 0000 0000 0000 1`                                |
//! | GN      | 5    | Group Number (1..=17 for normal GOBs)                  |
//! | GSBI    | 2    | Present iff CPM == 1                                   |
//! | GFID    | 2    | GOB Frame ID — stable within a picture                 |
//! | GQUANT  | 5    | Quantiser for this and following GOBs                  |
//!
//! No GOB header is transmitted for the first GOB (`GN == 0`) — the picture
//! header serves as its prologue. GOBs 1..N are optional in the bitstream;
//! the encoder may emit only the start codes it needs (typically none for
//! short clips).

use oxideav_core::{Error, Result};
use oxideav_mpeg4video::bitreader::BitReader;

/// Parsed GOB header.
#[derive(Clone, Debug)]
pub struct GobHeader {
    pub gn: u8,
    pub gsbi: u8,
    pub gfid: u8,
    pub gquant: u8,
}

/// Parse the GOB header that follows the 17-bit GBSC and 5-bit GN.
///
/// `br` must be positioned at the start of the byte that contains the GBSC.
/// On entry the function consumes the 17-bit GBSC, the 5-bit GN (returned in
/// the result), then GFID and GQUANT.
///
/// `cpm` selects whether GSBI is present. The caller propagates this from the
/// picture header.
pub fn parse_gob_header(br: &mut BitReader<'_>, cpm: bool) -> Result<GobHeader> {
    // GBSC: 17 bits = `0000 0000 0000 0000 1`. Read as u32 and check.
    let gbsc = br.read_u32(17)?;
    const GBSC_VALUE: u32 = 0b00_0000_0000_0000_0001;
    if gbsc != GBSC_VALUE {
        return Err(Error::invalid(format!(
            "h263 GOB: bad GBSC 0x{gbsc:05x} (want 0x{GBSC_VALUE:05x})"
        )));
    }
    let gn = br.read_u32(5)? as u8;
    if gn == 0 {
        return Err(Error::invalid(
            "h263 GOB: GN == 0 indicates a PSC, not a GBSC",
        ));
    }
    if gn >= 30 {
        return Err(Error::invalid(format!(
            "h263 GOB: GN={gn} is reserved or EOS"
        )));
    }
    let gsbi = if cpm { br.read_u32(2)? as u8 } else { 0 };
    let gfid = br.read_u32(2)? as u8;
    let gquant = br.read_u32(5)? as u8;
    if gquant == 0 {
        return Err(Error::invalid("h263 GOB: GQUANT == 0"));
    }
    Ok(GobHeader {
        gn,
        gsbi,
        gfid,
        gquant,
    })
}
