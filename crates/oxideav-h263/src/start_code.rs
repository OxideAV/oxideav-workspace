//! Start-code constants and scanning helpers for ITU-T H.263.
//!
//! H.263 uses two byte-aligned start codes for the hierarchical bitstream
//! (Annex C §5.1):
//! * **PSC** — Picture Start Code, 22 bits `0000 0000 0000 0000 1 00000` —
//!   appears at the head of every coded picture.
//! * **GBSC** — Group of Block Start Code, 17 bits `0000 0000 0000 0000 1`
//!   followed by a 5-bit `GN` field. `GN == 0` indicates a PSC at this
//!   position, `GN == 31` indicates the End-Of-Sequence marker.
//!
//! Both markers begin with the same 17-bit zero-prefix `0x00 0x00 0x80...`.
//! After the 17-bit prefix, the next 5 bits are GN. Byte-aligned PSC has its
//! 22-bit body (including 5 zero suffix bits) and so produces marker-byte
//! values in `0x80..=0x83` (the trailing 2 bits being the top of TR). GBSC
//! with `GN > 0` produces marker-byte values `0x84..=0xFF` depending on GN.

/// Number of zero bytes preceding the marker byte that opens a start code.
const ZERO_PREFIX_LEN: usize = 2;

/// `GN == 0` — picture-level start code (PSC).
pub const GN_PICTURE: u8 = 0;
/// `GN == 31` — End-Of-Sequence code (EOS).
pub const GN_EOS: u8 = 31;

/// One detected start-code event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartCode {
    /// Byte offset of the first of the two zero bytes that lead the marker.
    pub byte_pos: usize,
    /// 5-bit `GN` field. `0` means PSC, `1..=17` is a GOB number, `31` is EOS.
    pub gn: u8,
}

/// Find the next byte-aligned H.263 start code in `data`, beginning at byte
/// `pos`. Returns `Some(StartCode)` on success.
pub fn find_next_start_code(data: &[u8], mut pos: usize) -> Option<StartCode> {
    while pos + 3 <= data.len() {
        if data[pos] != 0 {
            pos += 1;
            continue;
        }
        // Walk run of zeros.
        let mut p = pos;
        while p < data.len() && data[p] == 0 {
            p += 1;
        }
        // Need at least 2 zeros, then a marker byte with the top bit set.
        if p - pos >= ZERO_PREFIX_LEN && p < data.len() {
            let marker = data[p];
            if marker & 0x80 != 0 {
                // Bits 1..=5 of (marker << 1) give GN; equivalently
                // `(marker >> 2) & 0x1F`. PSC has GN==0 (marker in 0x80..=0x83).
                let gn = (marker >> 2) & 0x1F;
                let sc = StartCode {
                    byte_pos: p - ZERO_PREFIX_LEN,
                    gn,
                };
                return Some(sc);
            }
        }
        pos = p.max(pos + 1);
    }
    None
}

/// Iterator over all `StartCode` markers in `data`.
pub fn iter_start_codes(data: &[u8]) -> impl Iterator<Item = StartCode> + '_ {
    let mut pos = 0;
    std::iter::from_fn(move || {
        let sc = find_next_start_code(data, pos)?;
        pos = sc.byte_pos + 3;
        Some(sc)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_psc() {
        // Two zeros + 0x80 + arbitrary trailing bytes -> PSC (GN=0).
        let data = [0x00, 0x00, 0x80, 0x02, 0x04];
        let v: Vec<_> = iter_start_codes(&data).collect();
        assert_eq!(
            v,
            vec![StartCode {
                byte_pos: 0,
                gn: GN_PICTURE
            }]
        );
    }

    #[test]
    fn finds_gbsc_gn_1() {
        // GN=1 -> marker = `1 00001 XX` = 0x84..=0x87.
        let data = [0x00, 0x00, 0x84, 0x00];
        let v: Vec<_> = iter_start_codes(&data).collect();
        assert_eq!(v, vec![StartCode { byte_pos: 0, gn: 1 }]);
    }

    #[test]
    fn finds_eos() {
        // GN=31 -> marker = 0xFC..=0xFF.
        let data = [0x00, 0x00, 0xFC];
        let v: Vec<_> = iter_start_codes(&data).collect();
        assert_eq!(
            v,
            vec![StartCode {
                byte_pos: 0,
                gn: GN_EOS
            }]
        );
    }

    #[test]
    fn skips_non_marker_bytes() {
        let data = [0xAB, 0xCD, 0x00, 0x00, 0x80, 0x01];
        let v: Vec<_> = iter_start_codes(&data).collect();
        assert_eq!(
            v,
            vec![StartCode {
                byte_pos: 2,
                gn: GN_PICTURE
            }]
        );
    }
}
