use crate::algebra::field::Fp;
use crate::commitment::sis::ModuleCommitment;

#[derive(Clone, Debug)]
pub struct RandomOracle {
    state: blake3::Hasher,
}

impl RandomOracle {
    pub fn new(domain_sep: &str) -> Self {
        let mut state = blake3::Hasher::new();
        state.update(domain_sep.as_bytes());
        Self { state }
    }

    pub fn absorb_bytes(&mut self, bytes: &[u8]) {
        self.state.update(bytes);
    }

    pub fn absorb_u64(&mut self, value: u64) {
        self.absorb_bytes(&value.to_le_bytes());
    }

    pub fn absorb_usize(&mut self, value: usize) {
        self.absorb_u64(value as u64);
    }

    pub fn absorb_field(&mut self, value: Fp) {
        self.absorb_bytes(&value.to_le_bytes());
    }

    pub fn absorb_commitment(&mut self, commitment: &ModuleCommitment) {
        for coord in &commitment.coords {
            for coeff in &coord.coeffs {
                self.absorb_field(*coeff);
            }
        }
    }

    pub fn challenge_field(&self, label: &str) -> Fp {
        let mut fork = self.state.clone();
        fork.update(label.as_bytes());
        let hash = fork.finalize();
        Fp::from_le_bytes_mod_order(hash.as_bytes())
    }
}
