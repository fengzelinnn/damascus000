use crate::algebra::field::Fp;
use crate::commitment::hasher::RandomOracle;
use crate::commitment::sis::ModuleCommitment;
use crate::utils::config::SystemParams;

#[derive(Clone, Debug)]
pub struct Transcript {
    oracle: RandomOracle,
}

impl Transcript {
    pub fn new(params: &SystemParams, initial_commitment: &ModuleCommitment) -> Self {
        let mut oracle = RandomOracle::new("damascus-conv-transcript-v1");
        oracle.absorb_usize(params.module_rank);
        oracle.absorb_usize(params.vector_len);
        oracle.absorb_usize(params.poly_len);
        oracle.absorb_usize(params.rounds);
        oracle.absorb_bytes(&params.seed_generators);
        oracle.absorb_commitment(initial_commitment);
        Self { oracle }
    }

    pub fn absorb_stage1_header(
        &mut self,
        round: usize,
        current_commitment: &ModuleCommitment,
        left_commitment: &ModuleCommitment,
        right_commitment: &ModuleCommitment,
    ) {
        self.oracle.absorb_u64(0x5354_4745_315F_4844);
        self.oracle.absorb_usize(round);
        self.oracle.absorb_commitment(current_commitment);
        self.oracle.absorb_commitment(left_commitment);
        self.oracle.absorb_commitment(right_commitment);
    }

    pub fn absorb_stage1_result(&mut self, alpha: Fp, folded_commitment: &ModuleCommitment) {
        self.oracle.absorb_u64(0x5354_4745_315F_5253);
        self.oracle.absorb_field(alpha);
        self.oracle.absorb_commitment(folded_commitment);
    }

    pub fn absorb_stage2_header(
        &mut self,
        vector_fold_commitment: &ModuleCommitment,
        even_commitment: &ModuleCommitment,
        odd_commitment: &ModuleCommitment,
    ) {
        self.oracle.absorb_u64(0x5354_4745_325F_4844);
        self.oracle.absorb_commitment(vector_fold_commitment);
        self.oracle.absorb_commitment(even_commitment);
        self.oracle.absorb_commitment(odd_commitment);
    }

    pub fn absorb_stage2_result(&mut self, beta: Fp, next_commitment: &ModuleCommitment) {
        self.oracle.absorb_u64(0x5354_4745_325F_5253);
        self.oracle.absorb_field(beta);
        self.oracle.absorb_commitment(next_commitment);
    }

    pub fn challenge_alpha(&self) -> Fp {
        self.oracle.challenge_field("alpha")
    }

    pub fn challenge_beta(&self) -> Fp {
        self.oracle.challenge_field("beta")
    }
}
