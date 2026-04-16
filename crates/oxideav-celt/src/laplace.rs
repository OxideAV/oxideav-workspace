//! Laplace-distribution symbol decoder (libopus `laplace.c`).
//!
//! Used for coarse band-energy quantisation (RFC 6716 §4.3.2.1).

use crate::range_decoder::RangeDecoder;

const LAPLACE_LOG_MINP: u32 = 0;
const LAPLACE_MINP: u32 = 1 << LAPLACE_LOG_MINP;
const LAPLACE_NMIN: u32 = 16;

/// Helper: probability of `|x| > 0` after `decay`-ing past 0.
fn ec_laplace_get_freq1(fs0: u32, decay: i32) -> u32 {
    let ft = 32768u32 - LAPLACE_MINP * (2 * LAPLACE_NMIN) - fs0;
    (ft.wrapping_mul((16384i32 - decay) as u32)) >> 15
}

/// `ec_laplace_decode` (libopus laplace.c).
///
/// `fs` is the initial symbol freq. of "0" (Q15 over 32768) and `decay` is
/// the geometric decay rate of subsequent symbols.
pub fn ec_laplace_decode(rc: &mut RangeDecoder<'_>, fs: u32, decay: i32) -> i32 {
    let mut val: i32 = 0;
    let mut fs = fs;
    let fm = rc.decode_bin(15);
    let mut fl: u32 = 0;
    if fm >= fs {
        val += 1;
        fl = fs;
        fs = ec_laplace_get_freq1(fs, decay) + LAPLACE_MINP;
        // Search the decaying part of the PDF.
        while fs > LAPLACE_MINP && fm >= fl + 2 * fs {
            fs *= 2;
            fl += fs;
            fs = (((fs - 2 * LAPLACE_MINP) as i32 * decay) >> 15) as u32;
            fs += LAPLACE_MINP;
            val += 1;
        }
        // Past the decaying part — uniform tail at LAPLACE_MINP each.
        if fs <= LAPLACE_MINP {
            let di = (fm - fl) >> (LAPLACE_LOG_MINP + 1);
            val += di as i32;
            fl += 2 * di * LAPLACE_MINP;
        }
        if fm < fl + fs {
            val = -val;
        } else {
            fl += fs;
        }
    }
    rc.dec_update(fl, (fl + fs).min(32768), 32768);
    val
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a benign payload the laplace decoder should return small values.
    #[test]
    fn laplace_returns_small_value_on_benign_input() {
        let buf = [0x80u8, 0x00, 0x00, 0x00, 0x00, 0x00];
        let mut rc = RangeDecoder::new(&buf);
        let v = ec_laplace_decode(&mut rc, 100 << 7, 100 << 6);
        assert!(v.abs() < 100, "unexpectedly large laplace value: {v}");
    }
}
