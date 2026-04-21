use crate::algebra::field::Fp;
use crate::commitment::sis::DamascusStatement;
use crate::commitment::sis::ModuleCommitment;
use crate::utils::config::SystemParams;

#[derive(Clone, Debug)]
pub struct Transcript {
    epoch_seed: [u8; 32],
    file_id: [u8; 32],
    epoch_index: u64,
}

impl Transcript {
    pub fn new(params: &SystemParams, statement: &DamascusStatement) -> Self {
        Self {
            epoch_seed: params.epoch_seed,
            file_id: statement.file_id,
            epoch_index: 0,
        }
    }

    pub fn challenge_vec(
        &self,
        round: usize,
        current_commitment: &ModuleCommitment,
        l_vec: &ModuleCommitment,
        r_vec: &ModuleCommitment,
    ) -> Fp {
        self.challenge(b"vec", round, current_commitment, l_vec, r_vec)
    }

    pub fn challenge_poly(
        &self,
        round: usize,
        folded_commitment: &ModuleCommitment,
        l_poly: &ModuleCommitment,
        r_poly: &ModuleCommitment,
    ) -> Fp {
        self.challenge(b"poly", round, folded_commitment, l_poly, r_poly)
    }

    fn challenge(
        &self,
        label: &[u8],
        round: usize,
        current_commitment: &ModuleCommitment,
        lhs: &ModuleCommitment,
        rhs: &ModuleCommitment,
    ) -> Fp {
        let mut counter = 0u32;
        loop {
            let mut hasher = blake3::Hasher::new();
            hasher.update(&self.epoch_seed);
            absorb_bytes(&mut hasher, label);
            absorb_bytes(&mut hasher, &self.file_id);
            hasher.update(&self.epoch_index.to_le_bytes());
            hasher.update(&(round as u64).to_le_bytes());
            absorb_commitment(&mut hasher, current_commitment);
            absorb_commitment(&mut hasher, lhs);
            absorb_commitment(&mut hasher, rhs);
            hasher.update(&counter.to_le_bytes());
            let challenge = Fp::from_le_bytes_mod_order(hasher.finalize().as_bytes());
            if !challenge.is_zero() {
                return challenge;
            }
            counter = counter.wrapping_add(1);
        }
    }
}

fn absorb_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn absorb_commitment(hasher: &mut blake3::Hasher, commitment: &ModuleCommitment) {
    hasher.update(&(commitment.coords.len() as u64).to_le_bytes());
    for coord in &commitment.coords {
        hasher.update(&(coord.coeffs.len() as u64).to_le_bytes());
        for coeff in &coord.coeffs {
            hasher.update(&coeff.to_le_bytes());
        }
    }
}
