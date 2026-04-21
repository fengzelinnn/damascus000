use crate::algebra::module::ModuleElement;
use crate::algebra::poly::Poly;
use crate::utils::config::MODULE_RANK;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenericSisParams<const K: usize> {
    pub seed: [u8; 32],
}

impl<const K: usize> GenericSisParams<K> {
    pub fn validate(&self) -> Result<()> {
        ensure!(K > 0, "module rank must be > 0");
        Ok(())
    }
}

pub type SisParams = GenericSisParams<MODULE_RANK>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Statement<const K: usize> {
    pub file_id: [u8; 32],
    pub original_len_bytes: u64,
    pub d: usize,
    pub com_0: ModuleElement<K>,
    pub g_0_seed: [u8; 32],
    pub h_0_seed: [u8; 32],
}

pub type DamascusStatement = Statement<MODULE_RANK>;
pub type ModuleCommitment = ModuleElement<MODULE_RANK>;

#[derive(Clone)]
pub struct GeneratorFamilies<const K: usize> {
    pub g: Vec<ModuleElement<K>>,
    pub h: Vec<ModuleElement<K>>,
}

pub struct GenericModuleSisCommitter<const K: usize> {
    params: GenericSisParams<K>,
    g_seed: [u8; 32],
    h_seed: [u8; 32],
    cache: Arc<Mutex<HashMap<(usize, usize), Arc<GeneratorFamilies<K>>>>>,
}

pub type ModuleSisCommitter = GenericModuleSisCommitter<MODULE_RANK>;

impl<const K: usize> Clone for GenericModuleSisCommitter<K> {
    fn clone(&self) -> Self {
        Self {
            params: self.params.clone(),
            g_seed: self.g_seed,
            h_seed: self.h_seed,
            cache: Arc::clone(&self.cache),
        }
    }
}

impl<const K: usize> std::fmt::Debug for GenericModuleSisCommitter<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericModuleSisCommitter")
            .field("params", &self.params)
            .field("g_seed", &self.g_seed)
            .field("h_seed", &self.h_seed)
            .finish_non_exhaustive()
    }
}

impl<const K: usize> GenericModuleSisCommitter<K> {
    pub fn new(params: GenericSisParams<K>) -> Result<Self> {
        params.validate()?;
        let g_seed = derive_subseed(params.seed, b"g0");
        let h_seed = derive_subseed(params.seed, b"h0");
        Ok(Self {
            params,
            g_seed,
            h_seed,
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn params(&self) -> &GenericSisParams<K> {
        &self.params
    }

    pub fn generator_seeds(&self) -> ([u8; 32], [u8; 32]) {
        (self.g_seed, self.h_seed)
    }

    pub fn generators_for(
        &self,
        vector_len: usize,
        ring_len: usize,
    ) -> Result<Arc<GeneratorFamilies<K>>> {
        ensure!(vector_len > 0, "generator vector length must be > 0");
        ensure!(ring_len > 0, "generator ring length must be > 0");

        {
            let guard = self.cache.lock().expect("generator cache lock poisoned");
            if let Some(entry) = guard.get(&(vector_len, ring_len)) {
                return Ok(Arc::clone(entry));
            }
        }

        let families = Arc::new(GeneratorFamilies {
            g: derive_generators(self.g_seed, b"g", vector_len, ring_len)?,
            h: derive_generators(self.h_seed, b"h", vector_len, ring_len)?,
        });
        let mut guard = self.cache.lock().expect("generator cache lock poisoned");
        let entry = guard
            .entry((vector_len, ring_len))
            .or_insert_with(|| Arc::clone(&families));
        Ok(Arc::clone(entry))
    }

    pub fn commit(&self, witness: &[Poly], blinding: &[Poly]) -> Result<ModuleElement<K>> {
        ensure!(
            witness.len() == blinding.len(),
            "witness and blinding length mismatch"
        );
        if witness.is_empty() {
            return Ok(ModuleElement::<K>::zero(1));
        }

        let ring_len = witness[0].len();
        ensure!(
            witness.iter().all(|poly| poly.len() == ring_len)
                && blinding.iter().all(|poly| poly.len() == ring_len),
            "witness and blinding ring lengths must be uniform"
        );

        let families = self.generators_for(witness.len(), ring_len)?;
        self.commit_with_generators(witness, blinding, &families.g, &families.h)
    }

    pub fn commit_serial(&self, witness: &[Poly], blinding: &[Poly]) -> Result<ModuleElement<K>> {
        self.commit(witness, blinding)
    }

    pub fn commit_with_generators(
        &self,
        witness: &[Poly],
        blinding: &[Poly],
        g: &[ModuleElement<K>],
        h: &[ModuleElement<K>],
    ) -> Result<ModuleElement<K>> {
        ensure!(
            witness.len() == blinding.len() && witness.len() == g.len() && g.len() == h.len(),
            "message/opening/generator length mismatch"
        );
        if witness.is_empty() {
            return Ok(ModuleElement::<K>::zero(1));
        }

        let ring_len = witness[0].len();
        let mut acc = ModuleElement::<K>::zero(ring_len);
        for idx in 0..witness.len() {
            acc = acc.add(&g[idx].ring_mul(&witness[idx])?)?;
        }
        for idx in 0..blinding.len() {
            acc = acc.add(&h[idx].ring_mul(&blinding[idx])?)?;
        }
        Ok(acc)
    }

    pub fn register(
        &self,
        file_id: [u8; 32],
        original_len_bytes: u64,
        d: usize,
        witness: &[Poly],
        blinding: &[Poly],
    ) -> Result<Statement<K>> {
        let com_0 = self.commit(witness, blinding)?;
        Ok(Statement {
            file_id,
            original_len_bytes,
            d,
            com_0,
            g_0_seed: self.g_seed,
            h_0_seed: self.h_seed,
        })
    }
}

pub fn derive_generator_families_from_seeds<const K: usize>(
    g_seed: [u8; 32],
    h_seed: [u8; 32],
    vector_len: usize,
    ring_len: usize,
) -> Result<GeneratorFamilies<K>> {
    Ok(GeneratorFamilies {
        g: derive_generators(g_seed, b"g", vector_len, ring_len)?,
        h: derive_generators(h_seed, b"h", vector_len, ring_len)?,
    })
}

fn derive_subseed(seed: [u8; 32], label: &[u8]) -> [u8; 32] {
    *blake3::keyed_hash(&seed, label).as_bytes()
}

fn derive_generators<const K: usize>(
    seed: [u8; 32],
    domain: &[u8],
    vector_len: usize,
    ring_len: usize,
) -> Result<Vec<ModuleElement<K>>> {
    let mut out = Vec::with_capacity(vector_len);
    for vec_idx in 0..vector_len {
        let coords = std::array::from_fn(|coord| {
            let coeffs = (0..ring_len)
                .map(|coeff_idx| derive_field(seed, domain, vec_idx, coord, coeff_idx))
                .collect();
            Poly::new(coeffs)
        });
        out.push(ModuleElement::from_coords(coords)?);
    }
    Ok(out)
}

fn derive_field(
    seed: [u8; 32],
    domain: &[u8],
    vec_idx: usize,
    coord: usize,
    coeff_idx: usize,
) -> crate::algebra::field::Fp {
    let mut input = Vec::with_capacity(domain.len() + 24);
    input.extend_from_slice(domain);
    input.extend_from_slice(&(vec_idx as u64).to_le_bytes());
    input.extend_from_slice(&(coord as u64).to_le_bytes());
    input.extend_from_slice(&(coeff_idx as u64).to_le_bytes());
    let hash = blake3::keyed_hash(&seed, &input);
    crate::algebra::field::Fp::from_le_bytes_mod_order(hash.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::{GenericModuleSisCommitter, GenericSisParams, ModuleSisCommitter, SisParams};
    use crate::algebra::field::Fp;
    use crate::algebra::module::ModuleElement;
    use crate::algebra::poly::Poly;
    use crate::utils::config::POLY_DEGREE;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    fn random_poly(rng: &mut StdRng) -> Poly {
        Poly::new(
            (0..POLY_DEGREE)
                .map(|_| Fp::from(rng.gen::<u128>()))
                .collect(),
        )
    }

    fn ring_linear_combination(lhs: &[Poly], rhs: &[Poly], a: &Poly, b: &Poly) -> Vec<Poly> {
        lhs.iter()
            .zip(rhs.iter())
            .map(|(l, r)| {
                l.mul(a, true)
                    .and_then(|left| left.add(&r.mul(b, true)?))
                    .expect("linear combination")
            })
            .collect()
    }

    #[test]
    fn commitment_is_deterministic() {
        let params = SisParams { seed: [42u8; 32] };
        let committer = ModuleSisCommitter::new(params).expect("committer");
        let witness = vec![Poly::zero(POLY_DEGREE), Poly::zero(POLY_DEGREE)];
        let blinding = vec![Poly::zero(POLY_DEGREE), Poly::zero(POLY_DEGREE)];
        let c1 = committer.commit(&witness, &blinding).expect("c1");
        let c2 = committer.commit(&witness, &blinding).expect("c2");
        assert_eq!(c1, c2);
    }

    fn commitment_linearity_holds_for_rank<const K: usize>() {
        let mut rng = StdRng::seed_from_u64(12);
        let committer = GenericModuleSisCommitter::<K>::new(GenericSisParams { seed: [9u8; 32] })
            .expect("committer");

        let witness_a = (0..4).map(|_| random_poly(&mut rng)).collect::<Vec<_>>();
        let witness_b = (0..4).map(|_| random_poly(&mut rng)).collect::<Vec<_>>();
        let blinding_a = (0..4).map(|_| random_poly(&mut rng)).collect::<Vec<_>>();
        let blinding_b = (0..4).map(|_| random_poly(&mut rng)).collect::<Vec<_>>();
        let ring_a = random_poly(&mut rng);
        let ring_b = random_poly(&mut rng);

        let c_a = committer.commit(&witness_a, &blinding_a).expect("c_a");
        let c_b = committer.commit(&witness_b, &blinding_b).expect("c_b");

        let witness_lin = ring_linear_combination(&witness_a, &witness_b, &ring_a, &ring_b);
        let blinding_lin = ring_linear_combination(&blinding_a, &blinding_b, &ring_a, &ring_b);
        let c_lin = committer
            .commit(&witness_lin, &blinding_lin)
            .expect("linear commitment");

        let rhs = c_a
            .ring_mul(&ring_a)
            .and_then(|left| left.add(&c_b.ring_mul(&ring_b)?))
            .expect("rhs");
        assert_eq!(c_lin, rhs);
    }

    #[test]
    fn commitment_preserves_ring_linearity_across_supported_module_ranks() {
        commitment_linearity_holds_for_rank::<4>();
        commitment_linearity_holds_for_rank::<8>();
    }

    #[test]
    fn generator_family_is_module_valued() {
        let committer = ModuleSisCommitter::new(SisParams { seed: [1u8; 32] }).expect("committer");
        let families = committer.generators_for(2, POLY_DEGREE).expect("families");
        assert_eq!(families.g.len(), 2);
        assert_eq!(
            families.g[0],
            ModuleElement::<8>::from_coords(families.g[0].coords.clone()).unwrap()
        );
    }
}
