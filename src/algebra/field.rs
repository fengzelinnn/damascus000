use crate::utils::config::MSIS_Q;
use num_bigint::BigUint;
use num_traits::{ToPrimitive, Zero};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Display, Formatter};
use std::iter::Sum;
use std::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};
use std::sync::OnceLock;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Fp(pub u128);

pub type FieldElement = Fp;

const TWO_POW_112: u128 = 1u128 << 112;
const MASK_112: u128 = TWO_POW_112 - 1;
const LIMB_BITS: u32 = 56;
const LIMB_MASK: u128 = (1u128 << LIMB_BITS) - 1;
const PSEUDO_MERSENNE_C: u128 = 75;

impl Fp {
    pub const SERDE_BYTES: usize = 16;

    pub const fn modulus() -> u128 {
        MSIS_Q
    }

    pub const fn zero() -> Self {
        Self(0)
    }

    pub const fn one() -> Self {
        Self(1)
    }

    pub fn new(value: u64) -> Self {
        Self::from_u128(value as u128)
    }

    pub fn from_u128(value: u128) -> Self {
        Self(value % MSIS_Q)
    }

    pub fn from_le_bytes_mod_order(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self::zero();
        }

        let mut acc = BigUint::zero();
        let mut factor = BigUint::from(1u8);
        let modulus = modulus_biguint();
        for &byte in bytes {
            acc += BigUint::from(byte) * &factor;
            factor <<= 8usize;
        }
        let reduced = acc % modulus;
        Self(
            reduced
                .to_u128()
                .expect("reduced challenge must fit in u128"),
        )
    }

    pub fn from_le_chunk(bytes: &[u8]) -> Self {
        let mut acc = 0u128;
        for (shift, byte) in bytes.iter().enumerate() {
            acc |= (*byte as u128) << (shift * 8);
        }
        Self::from_u128(acc)
    }

    pub fn as_u128(self) -> u128 {
        self.0
    }

    pub fn as_u64(self) -> u64 {
        self.0 as u64
    }

    pub fn to_le_bytes(self) -> [u8; 16] {
        self.0.to_le_bytes()
    }

    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub fn pow(self, exp: u64) -> Self {
        self.pow_u128(exp as u128)
    }

    pub fn pow_u128(self, mut exp: u128) -> Self {
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
        debug_assert!(!self.is_zero(), "attempted inversion of zero");
        self.pow_u128(MSIS_Q - 2)
    }
}

impl Serialize for Fp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_le_bytes().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Fp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = <[u8; 16]>::deserialize(deserializer)?;
        Ok(Self::from_u128(u128::from_le_bytes(bytes)))
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
        let sum = self.0 + rhs.0;
        if sum >= MSIS_Q {
            Self(sum - MSIS_Q)
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
            Self(MSIS_Q - (rhs.0 - self.0))
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
        Self(mul_reduce(self.0, rhs.0))
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
        if self.is_zero() {
            self
        } else {
            Self(MSIS_Q - self.0)
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

impl From<u128> for Fp {
    fn from(value: u128) -> Self {
        Self::from_u128(value)
    }
}

fn modulus_biguint() -> &'static BigUint {
    static MODULUS: OnceLock<BigUint> = OnceLock::new();
    MODULUS.get_or_init(|| BigUint::from(MSIS_Q))
}

fn mul_reduce(lhs: u128, rhs: u128) -> u128 {
    let a0 = lhs & LIMB_MASK;
    let a1 = lhs >> LIMB_BITS;
    let b0 = rhs & LIMB_MASK;
    let b1 = rhs >> LIMB_BITS;

    let c0 = a0 * b0;
    let c1 = a0 * b1 + a1 * b0;
    let c2 = a1 * b1;

    let c1_lo = c1 & LIMB_MASK;
    let c1_hi = c1 >> LIMB_BITS;

    let low_sum = c0 + (c1_lo << LIMB_BITS);
    let low = low_sum & MASK_112;
    let carry = low_sum >> 112;
    let high = c2 + c1_hi + carry;

    reduce_pseudo_mersenne(low, high)
}

fn reduce_pseudo_mersenne(low: u128, high: u128) -> u128 {
    let mut acc = low + high * PSEUDO_MERSENNE_C;
    let hi = acc >> 112;
    acc = (acc & MASK_112) + hi * PSEUDO_MERSENNE_C;
    while acc >= MSIS_Q {
        acc -= MSIS_Q;
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::Fp;
    use crate::utils::config::MSIS_Q;
    use num_traits::ToPrimitive;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    #[test]
    fn field_arithmetic_round_trip() {
        let a = Fp::from(17u64);
        let b = Fp::from(9u64);
        let c = a * b;
        assert_eq!(c, Fp::from(153u64));
        assert_eq!(c * b.inv(), a);
    }

    #[test]
    fn arithmetic_matches_reference_modulus() {
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..20_000 {
            let a = rng.gen::<u128>() % MSIS_Q;
            let b = rng.gen::<u128>() % MSIS_Q;

            let a_fp = Fp::from(a);
            let b_fp = Fp::from(b);

            let add_ref = (a + b) % MSIS_Q;
            let mul_ref = ((num_bigint::BigUint::from(a) * num_bigint::BigUint::from(b))
                % num_bigint::BigUint::from(MSIS_Q))
            .to_u128()
            .unwrap();

            assert_eq!((a_fp + b_fp).as_u128(), add_ref);
            assert_eq!((a_fp * b_fp).as_u128(), mul_ref);
        }
    }

    #[test]
    fn challenge_bytes_reduce_into_field() {
        let elem = Fp::from_le_bytes_mod_order(&[0xFF; 64]);
        assert!(elem.as_u128() < MSIS_Q);
    }

    #[test]
    fn serde_uses_fixed_width_encoding() {
        let elem = Fp::from(42u64);
        let encoded = bincode::serialize(&elem).expect("serialize");
        assert_eq!(encoded.len(), 16);
        let decoded: Fp = bincode::deserialize(&encoded).expect("deserialize");
        assert_eq!(decoded, elem);
    }
}
