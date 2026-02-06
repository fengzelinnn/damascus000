use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::iter::Sum;
use std::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};

pub const GOLDILOCKS_MODULUS: u64 = 18_446_744_069_414_584_321;
const MODULUS_U128: u128 = GOLDILOCKS_MODULUS as u128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct Fp(pub u64);

impl Fp {
    pub const fn modulus() -> u64 {
        GOLDILOCKS_MODULUS
    }

    pub const fn zero() -> Self {
        Self(0)
    }

    pub const fn one() -> Self {
        Self(1)
    }

    pub fn new(value: u64) -> Self {
        Self((value as u128 % MODULUS_U128) as u64)
    }

    pub fn from_u128(value: u128) -> Self {
        Self((value % MODULUS_U128) as u64)
    }

    pub fn from_le_bytes_mod_order(bytes: &[u8]) -> Self {
        let mut acc = 0u128;
        let mut shift = 0u32;
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            let limb = u64::from_le_bytes(word) as u128;
            acc = (acc + (limb << shift)) % MODULUS_U128;
            shift = (shift + 64) % 128;
        }
        Self::from_u128(acc)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn pow(self, mut exp: u64) -> Self {
        let mut base = self;
        let mut result = Self::one();
        while exp > 0 {
            if exp & 1 == 1 {
                result *= base;
            }
            base *= base;
            exp >>= 1;
        }
        result
    }

    pub fn inv(self) -> Self {
        debug_assert!(self != Self::zero(), "attempted inversion of zero");
        self.pow(GOLDILOCKS_MODULUS - 2)
    }
}

impl Display for Fp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Add for Fp {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::from_u128(self.0 as u128 + rhs.0 as u128)
    }
}

impl AddAssign for Fp {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for Fp {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        if self.0 >= rhs.0 {
            Self(self.0 - rhs.0)
        } else {
            Self(GOLDILOCKS_MODULUS - (rhs.0 - self.0))
        }
    }
}

impl SubAssign for Fp {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl Mul for Fp {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self::from_u128((self.0 as u128) * (rhs.0 as u128))
    }
}

impl MulAssign for Fp {
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

impl Neg for Fp {
    type Output = Self;

    fn neg(self) -> Self::Output {
        if self == Self::zero() {
            self
        } else {
            Self(GOLDILOCKS_MODULUS - self.0)
        }
    }
}

impl Sum for Fp {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::zero(), |acc, x| acc + x)
    }
}

impl<'a> Sum<&'a Fp> for Fp {
    fn sum<I: Iterator<Item = &'a Fp>>(iter: I) -> Self {
        iter.fold(Self::zero(), |acc, x| acc + *x)
    }
}

impl From<u64> for Fp {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::Fp;

    #[test]
    fn field_arithmetic_round_trip() {
        let a = Fp::from(17);
        let b = Fp::from(9);
        let c = a * b;
        assert_eq!(c, Fp::from(153));
        assert_eq!(c * b.inv(), a);
    }
}
