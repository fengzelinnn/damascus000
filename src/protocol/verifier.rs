use crate::commitment::sis::ModuleCommitment;
use crate::protocol::prover::MicroBlock;
use crate::protocol::transcript::Transcript;
use crate::utils::config::SystemParams;
use anyhow::{ensure, Context, Result};

#[derive(Clone, Debug)]
pub struct DamascusVerifier {
    params: SystemParams,
    transcript: Transcript,
    current_commitment: ModuleCommitment,
    round: usize,
}

impl DamascusVerifier {
    pub fn new(params: SystemParams, initial_commitment: ModuleCommitment) -> Self {
        let transcript = Transcript::new(&params, &initial_commitment);
        Self {
            params,
            transcript,
            current_commitment: initial_commitment,
            round: 0,
        }
    }

    pub fn current_commitment(&self) -> &ModuleCommitment {
        &self.current_commitment
    }

    pub fn round(&self) -> usize {
        self.round
    }

    pub fn update_commitment(&mut self, micro_block: &MicroBlock) -> Result<()> {
        ensure!(
            micro_block.round == self.round,
            "round mismatch in micro-block"
        );

        self.transcript.absorb_stage1_header(
            self.round,
            &self.current_commitment,
            &micro_block.left_vector_commitment,
            &micro_block.right_vector_commitment,
        );

        let alpha = self.transcript.challenge_alpha();
        ensure!(alpha == micro_block.alpha, "alpha challenge mismatch");

        let vector_fold_commitment = micro_block
            .left_vector_commitment
            .add_scaled(&micro_block.right_vector_commitment, alpha)
            .context("failed to combine vector commitments")?;
        ensure!(
            vector_fold_commitment == micro_block.vector_fold_commitment,
            "vector folded commitment mismatch"
        );

        self.transcript
            .absorb_stage1_result(alpha, &vector_fold_commitment);

        self.transcript.absorb_stage2_header(
            &vector_fold_commitment,
            &micro_block.even_poly_commitment,
            &micro_block.odd_poly_commitment,
        );

        let beta = self.transcript.challenge_beta();
        ensure!(beta == micro_block.beta, "beta challenge mismatch");

        let next_commitment = micro_block
            .even_poly_commitment
            .add_scaled(&micro_block.odd_poly_commitment, beta)
            .context("failed to combine polynomial commitments")?;
        ensure!(
            next_commitment == micro_block.next_commitment,
            "next commitment mismatch"
        );

        self.transcript.absorb_stage2_result(beta, &next_commitment);
        self.current_commitment = next_commitment;
        self.round += 1;
        Ok(())
    }

    pub fn params(&self) -> &SystemParams {
        &self.params
    }
}
