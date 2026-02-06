use crate::algebra::field::Fp;
use crate::algebra::poly::Poly;
use anyhow::{ensure, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SisParams {
    pub module_rank: usize,
    pub seed: [u8; 32],
}

impl SisParams {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.module_rank > 0, "module_rank must be > 0");
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleCommitment {
    pub elems: Vec<Fp>,
}

impl ModuleCommitment {
    pub fn zero(module_rank: usize) -> Self {
        Self {
            elems: vec![Fp::zero(); module_rank],
        }
    }

    pub fn add_scaled(&self, rhs: &Self, scalar: Fp) -> Result<Self> {
        ensure!(
            self.elems.len() == rhs.elems.len(),
            "commitment rank mismatch in add_scaled"
        );
        Ok(Self {
            elems: self
                .elems
                .iter()
                .zip(&rhs.elems)
                .map(|(a, b)| *a + (*b * scalar))
                .collect(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ModuleSisCommitter {
    params: SisParams,
}

impl ModuleSisCommitter {
    pub fn new(params: SisParams) -> Result<Self> {
        params.validate()?;
        Ok(Self { params })
    }

    pub fn params(&self) -> &SisParams {
        &self.params
    }

    pub fn commit(&self, witness: &[Poly], blinding: &[Poly]) -> Result<ModuleCommitment> {
        ensure!(
            witness.len() == blinding.len(),
            "witness and blinding length mismatch"
        );
        if witness.is_empty() {
            return Ok(ModuleCommitment::zero(self.params.module_rank));
        }
        let poly_len = witness[0].len();
        ensure!(poly_len > 0, "empty polynomial is not allowed");
        for (w, r) in witness.iter().zip(blinding) {
            ensure!(
                w.len() == poly_len,
                "inconsistent witness polynomial length"
            );
            ensure!(
                r.len() == poly_len,
                "inconsistent blinding polynomial length"
            );
        }

        let elems = (0..self.params.module_rank)
            .into_par_iter()
            .map(|row| {
                let mut acc = Fp::zero();
                for (vec_idx, (w_poly, r_poly)) in witness.iter().zip(blinding).enumerate() {
                    for coeff_idx in 0..poly_len {
                        let w = w_poly.coeffs[coeff_idx];
                        let r = r_poly.coeffs[coeff_idx];
                        let g = self.generator(0, row, vec_idx, coeff_idx);
                        let h = self.generator(1, row, vec_idx, coeff_idx);
                        acc += g * w;
                        acc += h * r;
                    }
                }
                acc
            })
            .collect();

        Ok(ModuleCommitment { elems })
    }

    fn generator(&self, domain: u8, row: usize, vec_idx: usize, coeff_idx: usize) -> Fp {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.params.seed);
        hasher.update(&[domain]);
        hasher.update(&(row as u64).to_le_bytes());
        hasher.update(&(vec_idx as u64).to_le_bytes());
        hasher.update(&(coeff_idx as u64).to_le_bytes());
        let digest = hasher.finalize();
        Fp::from_le_bytes_mod_order(digest.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::{ModuleSisCommitter, SisParams};
    use crate::algebra::poly::Poly;

    #[test]
    fn commitment_is_deterministic() {
        let params = SisParams {
            module_rank: 2,
            seed: [42u8; 32],
        };
        let committer = ModuleSisCommitter::new(params).expect("committer");
        let witness = vec![Poly::zero(8), Poly::zero(8)];
        let blinding = vec![Poly::zero(8), Poly::zero(8)];
        let c1 = committer.commit(&witness, &blinding).expect("c1");
        let c2 = committer.commit(&witness, &blinding).expect("c2");
        assert_eq!(c1, c2);
    }
}
