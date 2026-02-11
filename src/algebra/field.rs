use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::iter::Sum;
use std::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};

pub const GOLDILOCKS_MODULUS: u64 = 18_446_744_069_414_584_321;
const EPSILON: u64 = 4_294_967_295;
const MODULUS_U128: u128 = GOLDILOCKS_MODULUS as u128;

#[repr(transparent)]
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
        if value >= GOLDILOCKS_MODULUS {
            Self(value - GOLDILOCKS_MODULUS)
        } else {
            Self(value)
        }
    }

    pub fn from_u128(value: u128) -> Self {
        Self((value % MODULUS_U128) as u64)
    }

    pub fn from_le_bytes_mod_order(bytes: &[u8]) -> Self {
        let mut limbs = Vec::with_capacity(bytes.len().div_ceil(8));
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            limbs.push(u64::from_le_bytes(word));
        }

        let mut acc = Self::zero();
        let base = Self(EPSILON);
        for limb in limbs.into_iter().rev() {
            acc *= base;
            acc += Self::new(limb);
        }
        acc
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
        let (sum, carry) = self.0.overflowing_add(rhs.0);
        if carry {
            let (sum_plus_eps, carry2) = sum.overflowing_add(EPSILON);
            let mut reduced = sum_plus_eps;
            if carry2 || reduced >= GOLDILOCKS_MODULUS {
                reduced = reduced.wrapping_sub(GOLDILOCKS_MODULUS);
            }
            Self(reduced)
        } else if sum >= GOLDILOCKS_MODULUS {
            Self(sum - GOLDILOCKS_MODULUS)
        } else {
            Self(sum)
        }
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
        // Goldilocks reduction for p = 2^64 - 2^32 + 1.
        let a = self.0;
        let b = rhs.0;
        let lo = a.wrapping_mul(b);
        let hi = ((a as u128 * b as u128) >> 64) as u64;

        let x_hi_hi = hi >> 32;
        let x_hi_lo = hi & EPSILON;

        let mut t0 = lo.wrapping_sub(x_hi_hi);
        if lo < x_hi_hi {
            t0 = t0.wrapping_sub(EPSILON);
        }

        let t1 = x_hi_lo.wrapping_mul(EPSILON);
        let mut t2 = t0.wrapping_add(t1);
        if t2 < t1 {
            t2 = t2.wrapping_add(EPSILON);
        }

        if t2 >= GOLDILOCKS_MODULUS {
            t2 -= GOLDILOCKS_MODULUS;
        }
        Self(t2)
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
    use super::{Fp, GOLDILOCKS_MODULUS};
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    #[test]
    fn field_arithmetic_round_trip() {
        let a = Fp::from(17);
        let b = Fp::from(9);
        let c = a * b;
        assert_eq!(c, Fp::from(153));
        assert_eq!(c * b.inv(), a);
    }

    #[test]
    fn arithmetic_matches_reference_modulus() {
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..50_000 {
            let a = rng.gen::<u64>() % GOLDILOCKS_MODULUS;
            let b = rng.gen::<u64>() % GOLDILOCKS_MODULUS;

            let a_fp = Fp::new(a);
            let b_fp = Fp::new(b);

            let add_ref = ((a as u128 + b as u128) % (GOLDILOCKS_MODULUS as u128)) as u64;
            let mul_ref = ((a as u128 * b as u128) % (GOLDILOCKS_MODULUS as u128)) as u64;

            assert_eq!((a_fp + b_fp).as_u64(), add_ref);
            assert_eq!((a_fp * b_fp).as_u64(), mul_ref);
        }
    }
}
