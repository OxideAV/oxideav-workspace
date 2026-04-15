//! Rational number used for time bases and frame rates.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const fn new(num: i64, den: i64) -> Self {
        Self { num, den }
    }

    pub const fn zero() -> Self {
        Self { num: 0, den: 1 }
    }

    pub fn is_zero(&self) -> bool {
        self.num == 0
    }

    pub fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Reduce the fraction to lowest terms. Sign is normalized onto the numerator.
    pub fn reduced(mut self) -> Self {
        if self.den < 0 {
            self.num = -self.num;
            self.den = -self.den;
        }
        let g = gcd(self.num.unsigned_abs(), self.den.unsigned_abs()) as i64;
        if g > 1 {
            self.num /= g;
            self.den /= g;
        }
        self
    }

    /// Invert the fraction (num/den → den/num).
    pub fn invert(self) -> Self {
        Self {
            num: self.den,
            den: self.num,
        }
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce() {
        assert_eq!(Rational::new(10, 20).reduced(), Rational::new(1, 2));
        assert_eq!(Rational::new(-6, 9).reduced(), Rational::new(-2, 3));
        assert_eq!(Rational::new(6, -9).reduced(), Rational::new(-2, 3));
    }

    #[test]
    fn invert() {
        assert_eq!(Rational::new(1, 2).invert(), Rational::new(2, 1));
    }
}
