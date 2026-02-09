use crate::algebra::field::Fp;
use crate::algebra::ntt;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poly {
    pub coeffs: Vec<Fp>,
}

impl Poly {
    pub fn new(coeffs: Vec<Fp>) -> Self {
        Self { coeffs }
    }

    pub fn zero(len: usize) -> Self {
        Self {
            coeffs: vec![Fp::zero(); len],
        }
    }

    pub fn len(&self) -> usize {
        self.coeffs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.coeffs.is_empty()
    }

    pub fn add(&self, rhs: &Self) -> Result<Self> {
        ensure!(self.len() == rhs.len(), "polynomial length mismatch");
        Ok(Self::new(
            self.coeffs
                .iter()
                .zip(&rhs.coeffs)
                .map(|(a, b)| *a + *b)
                .collect(),
        ))
    }

    pub fn sub(&self, rhs: &Self) -> Result<Self> {
        ensure!(self.len() == rhs.len(), "polynomial length mismatch");
        Ok(Self::new(
            self.coeffs
                .iter()
                .zip(&rhs.coeffs)
                .map(|(a, b)| *a - *b)
                .collect(),
        ))
    }

    pub fn scale(&self, scalar: Fp) -> Self {
        Self::new(self.coeffs.iter().map(|c| *c * scalar).collect())
    }

    pub fn odd_even_decomposition(&self) -> (Self, Self) {
        let mut even = Vec::with_capacity(self.len().div_ceil(2));
        let mut odd = Vec::with_capacity(self.len() / 2);
        for (idx, coeff) in self.coeffs.iter().enumerate() {
            if idx % 2 == 0 {
                even.push(*coeff);
            } else {
                odd.push(*coeff);
            }
        }
        (Self::new(even), Self::new(odd))
    }

    pub fn fold_odd_even(&self, challenge: Fp) -> Self {
        let (even, odd) = self.odd_even_decomposition();
        let mut coeffs = even.coeffs;
        let min_len = coeffs.len().min(odd.coeffs.len());
        for i in 0..min_len {
            coeffs[i] += odd.coeffs[i] * challenge;
        }
        Self::new(coeffs)
    }

    pub fn inner_product(&self, rhs: &Self) -> Result<Fp> {
        ensure!(self.len() == rhs.len(), "polynomial length mismatch");
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

    pub fn mul(&self, rhs: &Self, ntt_enabled: bool) -> Result<Self> {
        ensure!(
            !self.is_empty() && !rhs.is_empty(),
            "cannot multiply empty polynomial"
        );
        if ntt_enabled && (self.len() + rhs.len() >= 64) {
            let coeffs = ntt::convolution(&self.coeffs, &rhs.coeffs)?;
            return Ok(Self::new(coeffs));
        }
        Ok(Self::new(naive_mul(&self.coeffs, &rhs.coeffs)))
    }
}

fn naive_mul(lhs: &[Fp], rhs: &[Fp]) -> Vec<Fp> {
    let mut out = vec![Fp::zero(); lhs.len() + rhs.len() - 1];
    for (i, a) in lhs.iter().enumerate() {
        for (j, b) in rhs.iter().enumerate() {
            out[i + j] += *a * *b;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Poly;
    use crate::algebra::field::Fp;

    #[test]
    fn odd_even_split_and_fold() {
        let p = Poly::new((1u64..=8).map(Fp::from).collect());
        let (even, odd) = p.odd_even_decomposition();
        assert_eq!(
            even.coeffs,
            vec![Fp::from(1), Fp::from(3), Fp::from(5), Fp::from(7)]
        );
        assert_eq!(
            odd.coeffs,
            vec![Fp::from(2), Fp::from(4), Fp::from(6), Fp::from(8)]
        );

        let folded = p.fold_odd_even(Fp::from(2));
        assert_eq!(folded.coeffs[0], Fp::from(5));
    }

    #[test]
    fn multiply_matches_naive() {
        let a = Poly::new(vec![Fp::from(1), Fp::from(2), Fp::from(3)]);
        let b = Poly::new(vec![Fp::from(4), Fp::from(5)]);
        let c = a.mul(&b, false).expect("mul");
        assert_eq!(
            c.coeffs,
            vec![Fp::from(4), Fp::from(13), Fp::from(22), Fp::from(15)]
        );
    }
}
