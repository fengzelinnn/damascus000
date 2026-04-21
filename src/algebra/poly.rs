use crate::algebra::field::Fp;
use crate::algebra::ntt;
use crate::utils::config::POLY_DEGREE;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct Poly {
    pub coeffs: Vec<Fp>,
}

pub type RingElement = Poly;

impl Poly {
    pub fn new(coeffs: Vec<Fp>) -> Self {
        assert_valid_ring_len(coeffs.len());
        Self { coeffs }
    }

    pub fn zero(len: usize) -> Self {
        assert_valid_ring_len(len);
        Self {
            coeffs: vec![Fp::zero(); len],
        }
    }

    pub fn one(len: usize) -> Self {
        let mut coeffs = vec![Fp::zero(); len];
        coeffs[0] = Fp::one();
        Self { coeffs }
    }

    pub fn from_scalar(scalar: Fp, len: usize) -> Self {
        let mut coeffs = vec![Fp::zero(); len];
        coeffs[0] = scalar;
        Self { coeffs }
    }

    pub fn x(len: usize) -> Self {
        assert_valid_ring_len(len);
        let mut coeffs = vec![Fp::zero(); len];
        if len == 1 {
            coeffs[0] = -Fp::one();
        } else {
            coeffs[1] = Fp::one();
        }
        Self { coeffs }
    }

    pub fn len(&self) -> usize {
        self.coeffs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.coeffs.is_empty()
    }

    pub fn add(&self, rhs: &Self) -> Result<Self> {
        ensure!(self.len() == rhs.len(), "ring length mismatch");
        Ok(Self {
            coeffs: self
                .coeffs
                .iter()
                .zip(&rhs.coeffs)
                .map(|(a, b)| *a + *b)
                .collect(),
        })
    }

    pub fn sub(&self, rhs: &Self) -> Result<Self> {
        ensure!(self.len() == rhs.len(), "ring length mismatch");
        Ok(Self {
            coeffs: self
                .coeffs
                .iter()
                .zip(&rhs.coeffs)
                .map(|(a, b)| *a - *b)
                .collect(),
        })
    }

    pub fn scale(&self, scalar: Fp) -> Self {
        Self {
            coeffs: self.coeffs.iter().map(|c| *c * scalar).collect(),
        }
    }

    pub fn odd_even_decomposition(&self) -> (Self, Self) {
        assert!(self.len() > 1, "cannot split scalar ring element");
        let mut even = Vec::with_capacity(self.len() / 2);
        let mut odd = Vec::with_capacity(self.len() / 2);
        for idx in 0..(self.len() / 2) {
            even.push(self.coeffs[2 * idx]);
            odd.push(self.coeffs[2 * idx + 1]);
        }
        (Self::new(even), Self::new(odd))
    }

    pub fn into_odd_even_decomposition(self) -> (Self, Self) {
        assert!(self.len() > 1, "cannot split scalar ring element");
        let mut even = Vec::with_capacity(self.len() / 2);
        let mut odd = Vec::with_capacity(self.len() / 2);
        for idx in 0..(self.coeffs.len() / 2) {
            even.push(self.coeffs[2 * idx]);
            odd.push(self.coeffs[2 * idx + 1]);
        }
        (Self::new(even), Self::new(odd))
    }

    pub fn fold_odd_even(&self, challenge: Fp) -> Self {
        let (even, odd) = self.odd_even_decomposition();
        let coeffs = even
            .coeffs
            .iter()
            .zip(odd.coeffs.iter())
            .map(|(a, b)| *a + (*b * challenge))
            .collect();
        Self::new(coeffs)
    }

    pub fn inner_product(&self, rhs: &Self) -> Result<Fp> {
        ensure!(self.len() == rhs.len(), "ring length mismatch");
        Ok(self
            .coeffs
            .iter()
            .zip(&rhs.coeffs)
            .map(|(a, b)| *a * *b)
            .sum())
    }

    pub fn coeff_sum(&self) -> Fp {
        self.coeffs.iter().copied().sum()
    }

    pub fn mul(&self, rhs: &Self, _ntt_enabled: bool) -> Result<Self> {
        ensure!(self.len() == rhs.len(), "ring length mismatch");
        Ok(Self::new(ntt::negacyclic_multiply(
            &self.coeffs,
            &rhs.coeffs,
        )?))
    }

    pub fn mul_by_x(&self) -> Result<Self> {
        ensure!(!self.is_empty(), "cannot shift empty ring element");
        let len = self.len();
        let mut out = vec![Fp::zero(); len];
        let last = self.coeffs[len - 1];
        out[0] = -last;
        for idx in 1..len {
            out[idx] = self.coeffs[idx - 1];
        }
        Ok(Self::new(out))
    }

    pub fn scalar_value(&self) -> Option<Fp> {
        self.coeffs
            .iter()
            .skip(1)
            .all(|coeff| coeff.is_zero())
            .then_some(self.coeffs[0])
    }
}

fn assert_valid_ring_len(len: usize) {
    assert!(
        len > 0 && len <= POLY_DEGREE && len.is_power_of_two(),
        "ring degree must be a power of two in 1..={POLY_DEGREE}, got {len}"
    );
}

#[cfg(test)]
mod tests {
    use super::Poly;
    use crate::algebra::field::Fp;
    use crate::utils::config::POLY_DEGREE;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    fn random_ring(rng: &mut StdRng, len: usize) -> Poly {
        Poly::new((0..len).map(|_| Fp::from(rng.gen::<u128>())).collect())
    }

    #[test]
    fn odd_even_split_and_fold() {
        let p = Poly::new((1u64..=8).map(Fp::from).collect());
        let (even, odd) = p.odd_even_decomposition();
        assert_eq!(
            even.coeffs,
            vec![
                Fp::from(1u64),
                Fp::from(3u64),
                Fp::from(5u64),
                Fp::from(7u64),
            ]
        );
        assert_eq!(
            odd.coeffs,
            vec![
                Fp::from(2u64),
                Fp::from(4u64),
                Fp::from(6u64),
                Fp::from(8u64),
            ]
        );

        let folded = p.fold_odd_even(Fp::from(2u64));
        assert_eq!(folded.coeffs[0], Fp::from(5u64));
    }

    #[test]
    fn x_n_plus_one_vanishes() {
        let mut acc = Poly::one(POLY_DEGREE);
        for _ in 0..POLY_DEGREE {
            acc = acc.mul_by_x().expect("shift");
        }
        let zero = acc.add(&Poly::one(POLY_DEGREE)).expect("add");
        assert!(zero.coeffs.iter().all(|coeff| coeff.is_zero()));
    }

    #[test]
    fn scalar_unit_inverse_restores_ring_element() {
        let mut rng = StdRng::seed_from_u64(9);
        for _ in 0..32 {
            let a = random_ring(&mut rng, POLY_DEGREE);
            let scalar = loop {
                let cand = Fp::from(rng.gen::<u128>());
                if !cand.is_zero() {
                    break cand;
                }
            };
            let b = Poly::from_scalar(scalar, POLY_DEGREE);
            let b_inv = Poly::from_scalar(scalar.inv(), POLY_DEGREE);
            let recovered = a
                .mul(&b, true)
                .expect("ab")
                .mul(&b_inv, true)
                .expect("abb_inv");
            assert_eq!(recovered, a);
        }
    }
}
