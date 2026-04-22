use crate::algebra::FieldElement;
use crate::commitment::sis::{
    derive_generator_families_from_seed, DamascusStatement, ModuleCommitment,
};
use crate::protocol::prover::RoundRecord;
use crate::protocol::transcript::Transcript;
use crate::utils::config::SystemParams;
use crate::utils::io::square_witness_layout_for_byte_len;
use anyhow::{ensure, Context, Result};

#[derive(Clone, Debug)]
pub struct DamascusVerifier {
    params: SystemParams,
    statement: DamascusStatement,
    transcript: Transcript,
    current_commitment: ModuleCommitment,
    g: Vec<ModuleCommitment>,
    round: usize,
}

impl DamascusVerifier {
    pub fn new(mut params: SystemParams, statement: DamascusStatement) -> Result<Self> {
        params.validate()?;
        let layout = square_witness_layout_for_byte_len(statement.original_len_bytes)
            .context("derive initial witness layout from statement")?;
        let vector_len = layout.dimension;
        let ring_len = statement.com_0.ring_len();
        params.vector_len = vector_len;
        params.poly_len = ring_len;
        params.rounds = statement.d;
        let families =
            derive_generator_families_from_seed(statement.g_0_seed, vector_len, ring_len)
                .context("derive initial generator families")?;
        let transcript = Transcript::new(&params, &statement);
        Ok(Self {
            params,
            transcript,
            current_commitment: statement.com_0.clone(),
            statement,
            g: families.g,
            round: 0,
        })
    }

    pub fn current_commitment(&self) -> &ModuleCommitment {
        &self.current_commitment
    }

    pub fn round(&self) -> usize {
        self.round
    }

    pub fn update_commitment(&mut self, record: &RoundRecord) -> Result<()> {
        ensure!(record.round == self.round, "round mismatch in round record");
        let (vector_fold_commitment, folded_g) = if self.g.len() > 1 {
            let mid = self.g.len() / 2;
            let g_left = self.g[..mid].to_vec();
            let g_right = self.g[mid..].to_vec();

            let x = self.transcript.challenge_vec(
                self.round,
                &self.current_commitment,
                &record.l_vec,
                &record.r_vec,
            );
            let x_inv = x.inv();
            let vector_fold_commitment = self
                .current_commitment
                .add(&record.l_vec.scale(x_inv)?)?
                .add(&record.r_vec.scale(x)?)?;
            let folded_g = fold_vec_module(&g_left, &g_right, x_inv)?;
            (vector_fold_commitment, folded_g)
        } else {
            let zero = ModuleCommitment::zero(self.current_commitment.ring_len());
            let _ =
                self.transcript
                    .challenge_vec(self.round, &self.current_commitment, &zero, &zero);
            ensure!(
                record.l_vec == zero && record.r_vec == zero,
                "scalar vector stage must emit zero vec cross-terms"
            );
            (self.current_commitment.clone(), self.g.clone())
        };

        let (next_commitment, next_g) = if folded_g[0].ring_len() > 1 {
            let (g_even, g_odd_scaled) = odd_even_vec_module_scaled(&folded_g)?;
            let y = self.transcript.challenge_poly(
                self.round,
                &vector_fold_commitment,
                &record.l_poly,
                &record.r_poly,
            );
            let y_inv = y.inv();
            let (c_even, _) = vector_fold_commitment.odd_even_decomposition()?;
            let next_commitment = c_even
                .add(&record.l_poly.scale(y_inv)?)?
                .add(&record.r_poly.scale(y)?)?;
            let next_g = fold_poly_module(&g_even, &g_odd_scaled, y_inv)?;
            (next_commitment, next_g)
        } else {
            let zero = ModuleCommitment::zero(1);
            let _ =
                self.transcript
                    .challenge_poly(self.round, &vector_fold_commitment, &zero, &zero);
            ensure!(
                record.l_poly == zero && record.r_poly == zero,
                "scalar stage must emit zero poly cross-terms"
            );
            (vector_fold_commitment, folded_g)
        };

        self.current_commitment = next_commitment;
        self.g = next_g;
        self.round += 1;
        Ok(())
    }

    pub fn verify_final_opening(&self, opening: &FieldElement) -> Result<()> {
        ensure!(
            self.round == self.params.rounds,
            "cannot verify final opening before all rounds replay"
        );
        ensure!(
            self.g.len() == 1,
            "terminal generator family must have length one"
        );
        let rhs = self.g[0].scale(*opening)?;
        ensure!(rhs == self.current_commitment, "final opening mismatch");
        Ok(())
    }

    pub fn statement(&self) -> &DamascusStatement {
        &self.statement
    }
}

fn fold_vec_module(
    left: &[ModuleCommitment],
    right: &[ModuleCommitment],
    challenge_inv: crate::algebra::field::Fp,
) -> Result<Vec<ModuleCommitment>> {
    ensure!(left.len() == right.len(), "generator fold length mismatch");
    left.iter()
        .zip(right.iter())
        .map(|(l, r)| l.add_scaled(r, challenge_inv))
        .collect()
}

fn odd_even_vec_module_scaled(
    input: &[ModuleCommitment],
) -> Result<(Vec<ModuleCommitment>, Vec<ModuleCommitment>)> {
    let mut even = Vec::with_capacity(input.len());
    let mut odd = Vec::with_capacity(input.len());
    for module in input {
        let (e, o) = module.odd_even_decomposition()?;
        even.push(e);
        odd.push(o.mul_by_x()?);
    }
    Ok((even, odd))
}

fn fold_poly_module(
    even: &[ModuleCommitment],
    odd_scaled: &[ModuleCommitment],
    challenge_inv: crate::algebra::field::Fp,
) -> Result<Vec<ModuleCommitment>> {
    ensure!(
        even.len() == odd_scaled.len(),
        "generator poly fold length mismatch"
    );
    even.iter()
        .zip(odd_scaled.iter())
        .map(|(e, o)| e.add_scaled(o, challenge_inv))
        .collect()
}
