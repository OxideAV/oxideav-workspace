//! Time base and timestamp types.

use crate::rational::Rational;

/// A time base expressed as a rational number of seconds per tick.
///
/// A `TimeBase` of 1/48000 means each timestamp unit is 1/48000 second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimeBase(pub Rational);

impl TimeBase {
    pub const fn new(num: i64, den: i64) -> Self {
        Self(Rational::new(num, den))
    }

    pub fn as_rational(&self) -> Rational {
        self.0
    }

    /// Convert a tick count in this time base to seconds.
    pub fn seconds_of(&self, ticks: i64) -> f64 {
        ticks as f64 * self.0.as_f64()
    }

    /// Rescale a timestamp from this time base to another.
    pub fn rescale(&self, ts: i64, target: TimeBase) -> i64 {
        rescale(ts, self.0, target.0)
    }
}

/// A timestamp in a particular time base.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timestamp {
    pub value: i64,
    pub base: TimeBase,
}

impl Timestamp {
    pub const fn new(value: i64, base: TimeBase) -> Self {
        Self { value, base }
    }

    pub fn seconds(&self) -> f64 {
        self.base.seconds_of(self.value)
    }

    pub fn rescale(&self, target: TimeBase) -> Self {
        Self {
            value: self.base.rescale(self.value, target),
            base: target,
        }
    }
}

/// Rescale a value from one rational time base to another using 128-bit
/// intermediate arithmetic to avoid overflow. Rounding is half-to-even
/// like FFmpeg's `av_rescale_q`.
pub fn rescale(value: i64, from: Rational, to: Rational) -> i64 {
    // value * (from.num/from.den) / (to.num/to.den)
    //   = value * from.num * to.den / (from.den * to.num)
    let num = from.num as i128 * to.den as i128;
    let den = from.den as i128 * to.num as i128;
    if den == 0 {
        return 0;
    }
    let prod = value as i128 * num;
    let half = den.abs() / 2;
    let rounded = if (prod >= 0) == (den > 0) {
        (prod + half) / den
    } else {
        (prod - half) / den
    };
    rounded as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescale_samples_to_pts() {
        // 48000 samples at 1/48000 base → 1 second at 1/1000 base = 1000 ticks
        assert_eq!(
            rescale(48000, Rational::new(1, 48000), Rational::new(1, 1000)),
            1000
        );
    }

    #[test]
    fn timestamp_seconds() {
        let ts = Timestamp::new(48000, TimeBase::new(1, 48000));
        assert!((ts.seconds() - 1.0).abs() < 1e-9);
    }
}
