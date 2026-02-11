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
    seed_words: [u64; 4],
}

impl ModuleSisCommitter {
    pub fn new(params: SisParams) -> Result<Self> {
        params.validate()?;
        let mut seed_words = [0u64; 4];
        for (idx, chunk) in params.seed.chunks_exact(8).enumerate() {
            let mut limb = [0u8; 8];
            limb.copy_from_slice(chunk);
            seed_words[idx] = u64::from_le_bytes(limb);
        }
        Ok(Self { params, seed_words })
    }

    pub fn params(&self) -> &SisParams {
        &self.params
    }

    pub fn commit(&self, witness: &[Poly], blinding: &[Poly]) -> Result<ModuleCommitment> {
        self.commit_impl(witness, blinding, true)
    }

    pub fn commit_serial(&self, witness: &[Poly], blinding: &[Poly]) -> Result<ModuleCommitment> {
        self.commit_impl(witness, blinding, false)
    }

    fn commit_impl(
        &self,
        witness: &[Poly],
        blinding: &[Poly],
        parallel: bool,
    ) -> Result<ModuleCommitment> {
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

        let row_acc = |row: usize| {
            let mut acc = Fp::zero();
            for (vec_idx, (w_poly, r_poly)) in witness.iter().zip(blinding).enumerate() {
                let mut g_state = self.generator_seed(0, row, vec_idx);
                let mut h_state = self.generator_seed(1, row, vec_idx);

                for coeff_idx in 0..poly_len {
                    let w = w_poly.coeffs[coeff_idx];
                    let r = r_poly.coeffs[coeff_idx];
                    let g = Fp::new(splitmix64_next(&mut g_state));
                    let h = Fp::new(splitmix64_next(&mut h_state));
                    acc += g * w;
                    acc += h * r;
                }
            }
            acc
        };

        let elems = if parallel {
            (0..self.params.module_rank)
                .into_par_iter()
                .map(row_acc)
                .collect()
        } else {
            (0..self.params.module_rank).map(row_acc).collect()
        };

        Ok(ModuleCommitment { elems })
    }

    fn generator_seed(&self, domain: u8, row: usize, vec_idx: usize) -> u64 {
        let mut x = self.seed_words[0]
            ^ self.seed_words[1].rotate_left(17)
            ^ self.seed_words[2].rotate_left(33)
            ^ self.seed_words[3].rotate_left(49)
            ^ ((domain as u64) << 56)
            ^ (row as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (vec_idx as u64).wrapping_mul(0xD134_2543_DE82_EF95);
        x = mix64(x);
        x ^ 0xA076_1D64_78BD_642F
    }
}

#[inline]
fn splitmix64_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    mix64(*state)
}

#[inline]
fn mix64(mut z: u64) -> u64 {
    z ^= z >> 30;
    z = z.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
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
