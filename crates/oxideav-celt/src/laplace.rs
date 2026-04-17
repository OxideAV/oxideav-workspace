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

/// Encoder side of `ec_laplace_decode`. Mirrors libopus `ec_laplace_encode`.
///
/// Encodes integer `val` with the Laplace distribution parameterised by
/// starting freq `fs` and geometric decay `decay`. The resulting range-coder
/// stream decodes back to the same integer via `ec_laplace_decode`.
pub fn ec_laplace_encode(
    rc: &mut crate::range_encoder::RangeEncoder,
    val: i32,
    fs_start: u32,
    decay: i32,
) {
    // Clamp |val| so the Laplace walk can't run fl past 32768 — libopus
    // bounds this implicitly via the 15-bit arithmetic; our wider ints need
    // an explicit guard to avoid subtract-with-overflow in the range coder.
    let val_clamped = val.clamp(-127, 127);
    let mut fl: u32 = 0;
    let mut fs = fs_start;
    if val_clamped != 0 {
        let s = if val_clamped < 0 { 1i32 } else { 0 };
        let v = val_clamped.unsigned_abs();
        fl = fs;
        fs = ec_laplace_get_freq1(fs, decay) + LAPLACE_MINP;
        // Advance through the geometric tail.
        let mut i = 1u32;
        while fs > LAPLACE_MINP && i < v {
            // Abort early if `fl + fs` would exceed the coder's 32768 total.
            if fl + 2 * fs >= 32768 {
                break;
            }
            fs *= 2;
            fl += fs;
            fs = (((fs - 2 * LAPLACE_MINP) as i32 * decay) >> 15) as u32;
            fs += LAPLACE_MINP;
            i += 1;
        }
        if fs <= LAPLACE_MINP {
            let di = v.saturating_sub(i);
            // Bound di so fl + 2*di*MINP stays well under 32768.
            let room = 32768u32.saturating_sub(fl + 2 * LAPLACE_MINP);
            let di_capped = di.min(room / (2 * LAPLACE_MINP).max(1));
            fl += 2 * di_capped * LAPLACE_MINP;
            fs = LAPLACE_MINP;
        }
        if s != 0 {
            // negative: [fl .. fl+fs)
        } else {
            fl += fs;
        }
    }
    if fl >= 32768 {
        // Unreachable under the clamp above, but guard anyway.
        fl = 32767;
        fs = 1;
    }
    let new_fl = fl;
    let new_fh = (fl + fs).min(32768);
    rc.encode(new_fl, new_fh, 32768);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range_encoder::RangeEncoder;

    /// On a benign payload the laplace decoder should return small values.
    #[test]
    fn laplace_returns_small_value_on_benign_input() {
        let buf = [0x80u8, 0x00, 0x00, 0x00, 0x00, 0x00];
        let mut rc = RangeDecoder::new(&buf);
        let v = ec_laplace_decode(&mut rc, 100 << 7, 100 << 6);
        assert!(v.abs() < 100, "unexpectedly large laplace value: {v}");
    }

    #[test]
    fn laplace_roundtrip_small_values() {
        // Use realistic fs/decay from E_PROB_MODEL[3][0] band 0: fs=42<<7, decay=121<<6.
        let fs = 42u32 << 7;
        let decay = 121i32 << 6;
        for v in [0i32, 1, -1, 2, -2, 3, -5, 7] {
            let mut enc = RangeEncoder::new(16);
            ec_laplace_encode(&mut enc, v, fs, decay);
            let buf = enc.done().unwrap();
            let mut dec = RangeDecoder::new(&buf);
            let got = ec_laplace_decode(&mut dec, fs, decay);
            assert_eq!(got, v, "laplace mismatch for v={v}");
        }
    }
}
