use crate::algebra::field::Fp;
use crate::algebra::poly::Poly;
use crate::commitment::sis::{ModuleCommitment, ModuleSisCommitter, SisParams};
use crate::protocol::transcript::Transcript;
use crate::utils::config::{RuntimeConfig, SystemParams};
use crate::utils::gpu;
use crate::utils::io;
use anyhow::{anyhow, ensure, Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MicroBlock {
    pub round: usize,
    pub left_vector_commitment: ModuleCommitment,
    pub right_vector_commitment: ModuleCommitment,
    pub vector_cross_term: Fp,
    pub alpha: Fp,
    pub vector_fold_commitment: ModuleCommitment,
    pub even_poly_commitment: ModuleCommitment,
    pub odd_poly_commitment: ModuleCommitment,
    pub poly_cross_term: Fp,
    pub beta: Fp,
    pub next_commitment: ModuleCommitment,
}

#[derive(Clone, Debug)]
pub struct RoundOutput {
    pub micro_block: MicroBlock,
    pub vector_fold_time: Duration,
    pub poly_fold_time: Duration,
    pub total_round_time: Duration,
}

#[derive(Clone, Debug)]
struct WitnessState {
    message: Vec<Poly>,
    blinding: Vec<Poly>,
}

#[derive(Clone, Debug)]
pub struct DamascusProver {
    params: SystemParams,
    config: RuntimeConfig,
    committer: ModuleSisCommitter,
    transcript: Transcript,
    witness: WitnessState,
    current_commitment: ModuleCommitment,
    round: usize,
}

impl DamascusProver {
    pub fn initialize(file_path: &Path, params: SystemParams) -> Result<Self> {
        Self::initialize_with_config(file_path, params, RuntimeConfig::default())
    }

    pub fn initialize_with_config(
        file_path: &Path,
        mut params: SystemParams,
        config: RuntimeConfig,
    ) -> Result<Self> {
        params.validate()?;

        let mmap = io::mmap_file(file_path)?;
        let message = io::mmap_to_fixed_polys(&mmap, params.vector_len, params.poly_len);

        let max_rounds_from_vector = floor_log2(params.vector_len);
        let max_rounds_from_poly = floor_log2(params.poly_len);
        let max_rounds = max_rounds_from_vector.min(max_rounds_from_poly);

        if params.rounds == 0 {
            params.rounds = max_rounds;
        } else {
            params.rounds = params.rounds.min(max_rounds);
        }

        let blinding = io::sample_blinding_polys(
            message.len(),
            params.poly_len,
            params.seed_generators,
            mmap.len() as u64,
        );

        let committer = ModuleSisCommitter::new(SisParams {
            module_rank: params.module_rank,
            seed: params.seed_generators,
        })?;

        let current_commitment = committer
            .commit(&message, &blinding)
            .context("initial commitment failed")?;
        let transcript = Transcript::new(&params, &current_commitment);

        Ok(Self {
            params,
            config,
            committer,
            transcript,
            witness: WitnessState { message, blinding },
            current_commitment,
            round: 0,
        })
    }

    pub fn rounds_total(&self) -> usize {
        self.params.rounds
    }

    pub fn current_round(&self) -> usize {
        self.round
    }

    pub fn current_commitment(&self) -> &ModuleCommitment {
        &self.current_commitment
    }

    pub fn fold_round(&mut self, round_idx: usize) -> Result<RoundOutput> {
        ensure!(round_idx == self.round, "round index mismatch");
        ensure!(self.round < self.params.rounds, "all rounds have finished");
        ensure!(
            self.witness.message.len() >= 2,
            "vector length too small for another fold"
        );

        let total_start = Instant::now();

        let vector_start = Instant::now();
        let mid = self.witness.message.len() / 2;
        let msg_left = &self.witness.message[..mid];
        let msg_right = &self.witness.message[mid..];
        let rnd_left = &self.witness.blinding[..mid];
        let rnd_right = &self.witness.blinding[mid..];

        let left_vector_commitment = self
            .committer
            .commit(msg_left, rnd_left)
            .context("left vector commitment failed")?;
        let right_vector_commitment = self
            .committer
            .commit(msg_right, rnd_right)
            .context("right vector commitment failed")?;

        let vector_cross_term =
            self.compute_vector_cross_term(msg_left, msg_right, rnd_left, rnd_right)?;

        self.transcript.absorb_stage1_header(
            round_idx,
            &self.current_commitment,
            &left_vector_commitment,
            &right_vector_commitment,
        );
        let alpha = self.transcript.challenge_alpha();

        let folded_message = fold_poly_vectors(
            msg_left,
            msg_right,
            alpha,
            self.config.parallel_enabled,
            self.config.gpu_enabled,
            self.config.gpu_min_elements,
        )?;
        let folded_blinding = fold_poly_vectors(
            rnd_left,
            rnd_right,
            alpha,
            self.config.parallel_enabled,
            self.config.gpu_enabled,
            self.config.gpu_min_elements,
        )?;
        let vector_fold_commitment =
            left_vector_commitment.add_scaled(&right_vector_commitment, alpha)?;
        self.transcript
            .absorb_stage1_result(alpha, &vector_fold_commitment);
        let vector_fold_time = vector_start.elapsed();

        let poly_start = Instant::now();
        let (msg_even, msg_odd) =
            split_even_odd_vector(&folded_message, self.config.parallel_enabled);
        let (rnd_even, rnd_odd) =
            split_even_odd_vector(&folded_blinding, self.config.parallel_enabled);

        ensure!(!msg_even.is_empty(), "even polynomial vector is empty");
        ensure!(
            !msg_odd.is_empty() && !rnd_odd.is_empty(),
            "odd polynomial vector is empty; increase poly_len for folding rounds"
        );

        let even_poly_commitment = self
            .committer
            .commit(&msg_even, &rnd_even)
            .context("even poly commitment failed")?;
        let odd_poly_commitment = self
            .committer
            .commit(&msg_odd, &rnd_odd)
            .context("odd poly commitment failed")?;

        let poly_cross_term =
            self.compute_poly_cross_term(&msg_even, &msg_odd, &rnd_even, &rnd_odd)?;

        self.transcript.absorb_stage2_header(
            &vector_fold_commitment,
            &even_poly_commitment,
            &odd_poly_commitment,
        );
        let beta = self.transcript.challenge_beta();

        let next_message = fold_even_odd_pairs(
            msg_even,
            msg_odd,
            beta,
            self.config.parallel_enabled,
            self.config.gpu_enabled,
            self.config.gpu_min_elements,
        )?;
        let next_blinding = fold_even_odd_pairs(
            rnd_even,
            rnd_odd,
            beta,
            self.config.parallel_enabled,
            self.config.gpu_enabled,
            self.config.gpu_min_elements,
        )?;
        let next_commitment = even_poly_commitment.add_scaled(&odd_poly_commitment, beta)?;

        let recomputed = self
            .committer
            .commit(&next_message, &next_blinding)
            .context("recomputed commitment failed")?;
        if recomputed != next_commitment {
            return Err(anyhow!(
                "consistency check failed: folded commitment does not match witness"
            ));
        }

        self.transcript.absorb_stage2_result(beta, &next_commitment);
        let poly_fold_time = poly_start.elapsed();

        self.witness.message = next_message;
        self.witness.blinding = next_blinding;
        self.current_commitment = next_commitment.clone();
        self.round += 1;

        let micro_block = MicroBlock {
            round: round_idx,
            left_vector_commitment,
            right_vector_commitment,
            vector_cross_term,
            alpha,
            vector_fold_commitment,
            even_poly_commitment,
            odd_poly_commitment,
            poly_cross_term,
            beta,
            next_commitment,
        };

        Ok(RoundOutput {
            micro_block,
            vector_fold_time,
            poly_fold_time,
            total_round_time: total_start.elapsed(),
        })
    }

    fn compute_vector_cross_term(
        &self,
        msg_left: &[Poly],
        msg_right: &[Poly],
        rnd_left: &[Poly],
        rnd_right: &[Poly],
    ) -> Result<Fp> {
        if self.config.parallel_enabled {
            let msg_sum: Result<Fp> = msg_left
                .par_iter()
                .zip(msg_right.par_iter())
                .map(|(l, r)| l.inner_product(r))
                .reduce(|| Ok(Fp::zero()), |a, b| Ok(a? + b?));
            let rnd_sum: Result<Fp> = rnd_left
                .par_iter()
                .zip(rnd_right.par_iter())
                .map(|(l, r)| l.inner_product(r))
                .reduce(|| Ok(Fp::zero()), |a, b| Ok(a? + b?));
            Ok(msg_sum? + rnd_sum?)
        } else {
            let msg_sum = msg_left
                .iter()
                .zip(msg_right)
                .try_fold(Fp::zero(), |acc, (l, r)| {
                    Ok::<_, anyhow::Error>(acc + l.inner_product(r)?)
                })?;
            let rnd_sum = rnd_left
                .iter()
                .zip(rnd_right)
                .try_fold(Fp::zero(), |acc, (l, r)| {
                    Ok::<_, anyhow::Error>(acc + l.inner_product(r)?)
                })?;
            Ok(msg_sum + rnd_sum)
        }
    }

    fn compute_poly_cross_term(
        &self,
        msg_even: &[Poly],
        msg_odd: &[Poly],
        rnd_even: &[Poly],
        rnd_odd: &[Poly],
    ) -> Result<Fp> {
        ensure!(
            msg_even.len() == msg_odd.len(),
            "msg even/odd vector length mismatch"
        );
        ensure!(
            rnd_even.len() == rnd_odd.len(),
            "rnd even/odd vector length mismatch"
        );

        let pair_sum = |lhs: &[Poly], rhs: &[Poly]| -> Fp {
            if self.config.parallel_enabled {
                lhs.par_iter()
                    .zip(rhs.par_iter())
                    .map(|(l, r)| l.coeff_sum() * r.coeff_sum())
                    .reduce(|| Fp::zero(), |a, b| a + b)
            } else {
                lhs.iter()
                    .zip(rhs)
                    .map(|(l, r)| l.coeff_sum() * r.coeff_sum())
                    .sum()
            }
        };

        Ok(pair_sum(msg_even, msg_odd) + pair_sum(rnd_even, rnd_odd))
    }
}

fn fold_poly_vectors(
    left: &[Poly],
    right: &[Poly],
    challenge: Fp,
    parallel_enabled: bool,
    gpu_enabled: bool,
    gpu_min_elements: usize,
) -> Result<Vec<Poly>> {
    ensure!(left.len() == right.len(), "vector split length mismatch");

    if left.is_empty() {
        return Ok(Vec::new());
    }

    let poly_len = left[0].len();
    ensure!(poly_len > 0, "empty polynomial is not allowed");
    ensure!(
        left.iter().all(|p| p.len() == poly_len) && right.iter().all(|p| p.len() == poly_len),
        "inconsistent polynomial length in fold"
    );

    let total_elements = left.len().saturating_mul(poly_len);
    let use_gpu = gpu_enabled && total_elements >= gpu_min_elements;
    if use_gpu {
        let mut left_flat = Vec::with_capacity(total_elements);
        let mut right_flat = Vec::with_capacity(total_elements);
        for poly in left {
            left_flat.extend(poly.coeffs.iter().map(|c| c.as_u64()));
        }
        for poly in right {
            right_flat.extend(poly.coeffs.iter().map(|c| c.as_u64()));
        }

        if let Some(out_flat) = gpu::try_fold_pairs_gpu(&left_flat, &right_flat, challenge.as_u64())
        {
            let mut out = Vec::with_capacity(left.len());
            for chunk in out_flat.chunks_exact(poly_len) {
                out.push(Poly::new(chunk.iter().map(|v| Fp(*v)).collect()));
            }
            return Ok(out);
        }
    }

    if parallel_enabled {
        left.par_iter()
            .zip(right.par_iter())
            .map(|(l, r)| l.add(&r.scale(challenge)))
            .collect()
    } else {
        left.iter()
            .zip(right)
            .map(|(l, r)| l.add(&r.scale(challenge)))
            .collect()
    }
}

fn split_even_odd_vector(polys: &[Poly], parallel_enabled: bool) -> (Vec<Poly>, Vec<Poly>) {
    if parallel_enabled {
        polys.par_iter().map(Poly::odd_even_decomposition).unzip()
    } else {
        let mut even = Vec::with_capacity(polys.len());
        let mut odd = Vec::with_capacity(polys.len());
        for poly in polys {
            let (e, o) = poly.odd_even_decomposition();
            even.push(e);
            odd.push(o);
        }
        (even, odd)
    }
}

fn fold_even_odd_pairs(
    even: Vec<Poly>,
    odd: Vec<Poly>,
    beta: Fp,
    parallel_enabled: bool,
    gpu_enabled: bool,
    gpu_min_elements: usize,
) -> Result<Vec<Poly>> {
    ensure!(even.len() == odd.len(), "even/odd vector length mismatch");
    fold_poly_vectors(
        &even,
        &odd,
        beta,
        parallel_enabled,
        gpu_enabled,
        gpu_min_elements,
    )
}

fn floor_log2(x: usize) -> usize {
    if x <= 1 {
        0
    } else {
        (usize::BITS as usize - 1) - (x.leading_zeros() as usize)
    }
}
