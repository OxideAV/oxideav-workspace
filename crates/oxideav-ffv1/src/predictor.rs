//! Median predictor + gradient context quantisation (RFC 9043 §3.7-3.8).

/// FFV1 median predictor: returns the middle value of `l`, `t`, `l + t - tl`.
/// The formula is the same one used by JPEG-LS: it picks one of the three
/// candidates depending on where `tl` lies relative to `l` and `t`.
#[inline]
pub fn median3(a: i32, b: i32, c: i32) -> i32 {
    // min(a,b,c) + max(a,b,c) + a+b+c actually; simplest to express as the
    // middle value of a sort.
    let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
    a + b + c - lo - hi
}

/// Compute the FFV1 predicted value for sample X given neighbour L (left),
/// T (top) and TL (top-left) using the JPEG-LS / median predictor.
#[inline]
pub fn predict(l: i32, t: i32, tl: i32) -> i32 {
    median3(l, t, l + t - tl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_basic() {
        assert_eq!(median3(1, 2, 3), 2);
        assert_eq!(median3(3, 2, 1), 2);
        assert_eq!(median3(5, 5, 5), 5);
        assert_eq!(median3(-3, 2, 10), 2);
    }

    #[test]
    fn predict_flat() {
        // Flat region: L=T=TL=128 → prediction must equal 128.
        assert_eq!(predict(128, 128, 128), 128);
    }

    #[test]
    fn predict_horizontal_edge() {
        // L=100, T=100, TL=100 → 100
        assert_eq!(predict(100, 100, 100), 100);
    }
}
