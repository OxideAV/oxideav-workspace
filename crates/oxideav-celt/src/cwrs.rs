//! PVQ codebook decoder (libopus `cwrs.c`, RFC 6716 §4.3.4.5).
//!
//! Decodes the integer-codeword PVQ pulse vector for a band of size `n` with
//! `k` pulses. We use the SMALL_FOOTPRINT recurrence-based decoder (no large
//! lookup table), which is asymptotically O(N*K) but fits in pure Rust code
//! and never needs `unsafe`.

use crate::range_decoder::RangeDecoder;

/// Compute the next row/column of any recurrence that obeys
/// `u[i][j] = u[i-1][j] + u[i][j-1] + u[i-1][j-1]`.
fn unext(u: &mut [u32], len: usize, mut ui0: u32) {
    let mut j = 1;
    while j < len {
        let ui1 = u[j].wrapping_add(u[j - 1]).wrapping_add(ui0);
        u[j - 1] = ui0;
        ui0 = ui1;
        j += 1;
    }
    u[j - 1] = ui0;
}

/// Compute the previous row/column of the same recurrence.
fn uprev(u: &mut [u32], n: usize, mut ui0: u32) {
    let mut j = 1;
    while j < n {
        let ui1 = u[j].wrapping_sub(u[j - 1]).wrapping_sub(ui0);
        u[j - 1] = ui0;
        ui0 = ui1;
        j += 1;
    }
    u[j - 1] = ui0;
}

/// Compute V(_n,_k) and fill `_u[0.._k+1]` with U(_n, 0.._k+1).
fn ncwrs_urow(n: u32, k: u32, u: &mut [u32]) -> u32 {
    let len = (k + 2) as usize;
    debug_assert!(len >= 3);
    debug_assert!(n >= 2);
    debug_assert!(k > 0);
    u[0] = 0;
    u[1] = 1;
    let mut kk = 2;
    while kk < len {
        u[kk] = ((kk as u32) << 1) - 1;
        kk += 1;
    }
    for _ in 2..n {
        unext(&mut u[1..], (k + 1) as usize, 1);
    }
    u[k as usize].wrapping_add(u[(k + 1) as usize])
}

/// Decode `n*k` PVQ index into pulse vector `y`. Returns `yy = sum(y[i]^2)`.
fn cwrsi(n: usize, mut k: u32, mut idx: u32, y: &mut [i32], u: &mut [u32]) -> i32 {
    let mut yy: i32 = 0;
    let mut j = 0;
    while j < n {
        let mut p = u[(k + 1) as usize];
        let s = if idx >= p {
            idx = idx.wrapping_sub(p);
            -1i32
        } else {
            0i32
        };
        let mut yj = k as i32;
        p = u[k as usize];
        while p > idx {
            k -= 1;
            p = u[k as usize];
        }
        idx = idx.wrapping_sub(p);
        yj -= k as i32;
        let val = (yj + s) ^ s;
        y[j] = val;
        yy = yy.wrapping_add(val * val);
        uprev(u, (k + 2) as usize, 0);
        j += 1;
    }
    yy
}

/// Decode pulses from the range coder. Returns `||y||²`.
pub fn decode_pulses(y: &mut [i32], n: usize, k: u32, rc: &mut RangeDecoder<'_>) -> i32 {
    debug_assert!(k > 0);
    debug_assert!(n >= 2);
    let mut u = vec![0u32; (k + 2) as usize];
    let total = ncwrs_urow(n as u32, k, &mut u);
    let i = if total > 1 { rc.decode_uint(total) } else { 0 };
    cwrsi(n, k, i, y, &mut u)
}

/// Inverse of `cwrsi`. The clean way: simulate the decoder step-by-step,
/// figuring out at each position `j` what `idx` contribution corresponds to
/// the known `y[j]`. Working forwards: at iteration `j` we know `|y[j]|` and
/// its sign, so we can recover `idx_at_j = idx_at_{j+1} + u[k_after] + (s==-1 ? u[k_before+1] : 0)`.
/// But to chain, we'd need the "remaining idx" to the right.
///
/// Simpler approach (matches libopus): walk forward, maintaining the u-row
/// state exactly as cwrsi does, and ACCUMULATE the contributions.
///
/// Key observation: the decoder sets `idx_initial = sum over j of
/// (contribution_j)`. Each contribution is independent given the u-row at
/// step j. Since `u` only depends on `n - j` and `k` at step j, we can
/// compute them deterministically. The total idx is:
///
/// `idx = sum_j [ u_row_j[k_after_j] + (s_j==-1 ? u_row_j[k_before_j+1] : 0) ]`
///
/// …provided we keep `k` properly updated.
fn icwrs(n: usize, y: &[i32]) -> u32 {
    let k = y.iter().map(|v| v.unsigned_abs()).sum::<u32>();
    debug_assert!(k > 0);
    let mut u = vec![0u32; (k + 2) as usize];
    let _ = ncwrs_urow(n as u32, k, &mut u);
    let mut idx: u32 = 0;
    let mut kk = k;
    for j in 0..n {
        let yj = y[j];
        let m = yj.unsigned_abs();
        let s_neg = yj < 0;
        // Decoder's steps (at state `u`, `kk`):
        //   p = u[kk+1]; if s_neg: idx -= p  (so encoder: idx += u[kk+1])
        //   ... loop sets k_after = kk - m and does idx -= u[kk - m]
        if s_neg {
            idx = idx.wrapping_add(u[(kk + 1) as usize]);
        }
        idx = idx.wrapping_add(u[(kk - m) as usize]);
        kk -= m;
        uprev(&mut u, (kk + 2) as usize, 0);
    }
    idx
}

/// Find the PVQ codevector (integer vector with `sum(|y_i|) = k`) that
/// maximises correlation with `x`. This is the "projection" used to decide
/// which codeword the encoder should emit. Classical CELT algorithm (libopus
/// `alg_quant` → `op_pvq_search`): initialise with rounded projection, add
/// pulses one at a time by greedy correlation, possibly remove then add to
/// reach exactly k pulses.
pub fn pvq_search(x: &[f32], k: i32, y: &mut [i32]) -> f32 {
    let n = x.len();
    debug_assert!(y.len() >= n);
    debug_assert!(k > 0);
    // Step 1: L1-normalise x and do an initial projection.
    let mut sum_abs: f32 = 0.0;
    for &v in x.iter() {
        sum_abs += v.abs();
    }
    if sum_abs < 1e-10 {
        // Degenerate input: concentrate pulses in y[0].
        y[0] = k;
        for v in y.iter_mut().skip(1).take(n - 1) {
            *v = 0;
        }
        return 1.0;
    }
    let rcp = (k as f32) / sum_abs;
    let mut pulses_used: i32 = 0;
    let mut xy: f32 = 0.0; // running sum x·y
    let mut yy: f32 = 0.0; // running ||y||^2
    for i in 0..n {
        let yi = (x[i].abs() * rcp).floor();
        let yi = yi as i32;
        let sign = if x[i] < 0.0 { -1 } else { 1 };
        y[i] = sign * yi;
        pulses_used += yi;
        xy += x[i] * (sign * yi) as f32;
        yy += (yi * yi) as f32;
    }
    // Step 2: add pulses one at a time until we have k.
    while pulses_used < k {
        let mut best_i = 0usize;
        let mut best_sig = f32::NEG_INFINITY;
        // For each position, tentative new value is y[i] + sign (where sign
        // is chosen to agree with x[i]). The marginal gain criterion is
        // `(xy + |x[i]|)^2 / (yy + 2|y[i]|+1)` — standard PVQ greedy step.
        for i in 0..n {
            let abs_xi = x[i].abs();
            let new_xy = xy + abs_xi;
            let abs_yi = y[i].unsigned_abs() as f32;
            let new_yy = yy + 2.0 * abs_yi + 1.0;
            let metric = new_xy * new_xy / new_yy.max(1e-20);
            if metric > best_sig {
                best_sig = metric;
                best_i = i;
            }
        }
        let sign = if x[best_i] < 0.0 { -1 } else { 1 };
        y[best_i] += sign;
        xy += x[best_i].abs();
        yy += 2.0 * y[best_i].unsigned_abs() as f32 - 1.0;
        pulses_used += 1;
    }
    // Step 3: if we overshot (initial rounding added >k, rare for floor()),
    // remove the weakest positions. floor() guarantees we never overshoot.
    let _ = yy;
    yy.sqrt().max(1e-10)
}

/// Encode a pulse vector `y` via the range coder. Inverse of
/// `decode_pulses`. Returns the total enumeration V(n, k) used (caller
/// already knows it via a separate path; returned here for sanity checks).
pub fn encode_pulses(
    y: &[i32],
    n: usize,
    k: u32,
    rc: &mut crate::range_encoder::RangeEncoder,
) -> u32 {
    debug_assert!(k > 0);
    debug_assert!(n >= 2);
    let mut u = vec![0u32; (k + 2) as usize];
    let total = ncwrs_urow(n as u32, k, &mut u);
    let idx = icwrs(n, y);
    if total > 1 {
        rc.encode_uint(idx, total);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range_encoder::RangeEncoder;

    #[test]
    fn ncwrs_urow_n2() {
        let mut u = vec![0u32; 6];
        let v = ncwrs_urow(2, 4, &mut u);
        // V(2,K) = 4*K = 16
        assert_eq!(v, 16);
    }

    #[test]
    fn decode_pulses_n2_k1_smoke() {
        // V(2,1)=4, so codeword is 2 bits.
        let mut buf = [0x80, 0x00, 0x00, 0x00];
        let mut rc = RangeDecoder::new(&buf[..]);
        let mut y = [0i32; 2];
        let yy = decode_pulses(&mut y, 2, 1, &mut rc);
        assert_eq!(yy, 1);
        assert_eq!(y.iter().map(|x| x.abs()).sum::<i32>(), 1);
        let _ = buf;
    }

    #[test]
    fn pulses_roundtrip_small() {
        // Enumerate a few small pulse vectors for (n, k) = (4, 2) and verify
        // encode_pulses → decode_pulses gives back the same y vector.
        let cases: &[(usize, u32, &[i32])] = &[
            (2, 1, &[1, 0]),
            (2, 1, &[0, -1]),
            (3, 2, &[1, 1, 0]),
            (3, 2, &[-2, 0, 0]),
            (4, 3, &[1, 1, 1, 0]),
            (4, 3, &[0, 0, -3, 0]),
            (4, 3, &[0, 1, -1, 1]),
        ];
        for (n, k, y_in) in cases {
            let mut enc = RangeEncoder::new(16);
            encode_pulses(y_in, *n, *k, &mut enc);
            let buf = enc.done().unwrap();
            let mut dec = RangeDecoder::new(&buf);
            let mut y_out = vec![0i32; *n];
            let _ = decode_pulses(&mut y_out, *n, *k, &mut dec);
            assert_eq!(y_out.as_slice(), *y_in, "n={n} k={k}");
        }
    }
}
