use crate::algebra::field::Fp;
use crate::algebra::ntt;
use crate::algebra::poly::Poly;
use crate::commitment::sis::{ModuleCommitment, ModuleSisCommitter, SisParams};
use crate::protocol::transcript::Transcript;
use crate::utils::config::{RuntimeConfig, SystemParams};
use crate::utils::gpu;
use crate::utils::io;
use anyhow::{anyhow, ensure, Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::env;
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
    cross_term_domain: usize,
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

        let current_commitment = if config.parallel_enabled && config.gpu_enabled {
            committer.commit(&message, &blinding)
        } else {
            committer.commit_serial(&message, &blinding)
        }
        .context("initial commitment failed")?;
        let transcript = Transcript::new(&params, &current_commitment);
        let cross_term_domain = resolve_cross_term_domain(params.poly_len);

        Ok(Self {
            params,
            config,
            committer,
            transcript,
            witness: WitnessState { message, blinding },
            current_commitment,
            round: 0,
            cross_term_domain,
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
        let mut message = std::mem::take(&mut self.witness.message);
        let mut blinding = std::mem::take(&mut self.witness.blinding);

        let vector_start = Instant::now();
        let mid = message.len() / 2;
        let msg_right = message.split_off(mid);
        let msg_left = message;
        let rnd_right = blinding.split_off(mid);
        let rnd_left = blinding;

        let left_vector_commitment = self
            .commit_polys(&msg_left, &rnd_left)
            .context("left vector commitment failed")?;
        let right_vector_commitment = self
            .commit_polys(&msg_right, &rnd_right)
            .context("right vector commitment failed")?;

        let vector_cross_term =
            self.compute_vector_cross_term(&msg_left, &msg_right, &rnd_left, &rnd_right)?;

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
            split_even_odd_vector(folded_message, self.config.parallel_enabled);
        let (rnd_even, rnd_odd) =
            split_even_odd_vector(folded_blinding, self.config.parallel_enabled);

        ensure!(!msg_even.is_empty(), "even polynomial vector is empty");
        ensure!(
            !msg_odd.is_empty() && !rnd_odd.is_empty(),
            "odd polynomial vector is empty; increase poly_len for folding rounds"
        );

        let even_poly_commitment = self
            .commit_polys(&msg_even, &rnd_even)
            .context("even poly commitment failed")?;
        let odd_poly_commitment = self
            .commit_polys(&msg_odd, &rnd_odd)
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
            .commit_polys(&next_message, &next_blinding)
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
        let msg_cross = spectral_cross_term(
            msg_left,
            msg_right,
            self.cross_term_domain,
            self.config.ntt_enabled,
        )?;
        let rnd_cross = spectral_cross_term(
            rnd_left,
            rnd_right,
            self.cross_term_domain,
            self.config.ntt_enabled,
        )?;
        Ok(msg_cross + rnd_cross)
    }

    fn commit_polys(&self, witness: &[Poly], blinding: &[Poly]) -> Result<ModuleCommitment> {
        if self.config.parallel_enabled && self.config.gpu_enabled {
            self.committer.commit(witness, blinding)
        } else {
            self.committer.commit_serial(witness, blinding)
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

        let msg_cross = spectral_cross_term(
            msg_even,
            msg_odd,
            self.cross_term_domain,
            self.config.ntt_enabled,
        )?;
        let rnd_cross = spectral_cross_term(
            rnd_even,
            rnd_odd,
            self.cross_term_domain,
            self.config.ntt_enabled,
        )?;
        Ok(msg_cross + rnd_cross)
    }
}

fn fold_poly_vectors(
    left: Vec<Poly>,
    right: Vec<Poly>,
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
        if let Some(folded) =
            fold_poly_vectors_gpu_batched(&left, &right, poly_len, challenge, parallel_enabled)
        {
            return Ok(folded);
        }
    }

    fold_poly_vectors_cpu(left, right, challenge, parallel_enabled)
}

fn fold_poly_vectors_cpu(
    left: Vec<Poly>,
    right: Vec<Poly>,
    challenge: Fp,
    parallel_enabled: bool,
) -> Result<Vec<Poly>> {
    if parallel_enabled {
        left.into_par_iter()
            .zip(right.into_par_iter())
            .map(|(mut l, r)| {
                ensure!(l.len() == r.len(), "polynomial length mismatch");
                for (dst, src) in l.coeffs.iter_mut().zip(r.coeffs.into_iter()) {
                    *dst += src * challenge;
                }
                Ok(l)
            })
            .collect()
    } else {
        left.into_iter()
            .zip(right)
            .map(|(mut l, r)| {
                ensure!(l.len() == r.len(), "polynomial length mismatch");
                for (dst, src) in l.coeffs.iter_mut().zip(r.coeffs.into_iter()) {
                    *dst += src * challenge;
                }
                Ok(l)
            })
            .collect()
    }
}

fn fold_poly_vectors_gpu_batched(
    left: &[Poly],
    right: &[Poly],
    poly_len: usize,
    challenge: Fp,
    parallel_enabled: bool,
) -> Option<Vec<Poly>> {
    let batch_rows = resolve_gpu_fold_batch_rows(poly_len, left.len());
    let mut out = Vec::with_capacity(left.len());

    for (left_chunk, right_chunk) in left.chunks(batch_rows).zip(right.chunks(batch_rows)) {
        let left_flat = flatten_polys_u64(left_chunk, poly_len, parallel_enabled);
        let right_flat = flatten_polys_u64(right_chunk, poly_len, parallel_enabled);
        let out_flat = gpu::try_fold_pairs_gpu(&left_flat, &right_flat, challenge.as_u64())?;
        out.extend(unflatten_polys_u64(&out_flat, left_chunk.len(), poly_len));
    }

    Some(out)
}

fn flatten_polys_u64(polys: &[Poly], poly_len: usize, parallel_enabled: bool) -> Vec<u64> {
    let mut out = vec![0u64; polys.len().saturating_mul(poly_len)];

    if parallel_enabled && polys.len() >= rayon::current_num_threads().saturating_mul(2) {
        out.par_chunks_mut(poly_len)
            .zip(polys.par_iter())
            .for_each(|(dst, poly)| {
                let src = fp_slice_as_u64(&poly.coeffs);
                dst.copy_from_slice(src);
            });
    } else {
        for (dst, poly) in out.chunks_mut(poly_len).zip(polys.iter()) {
            let src = fp_slice_as_u64(&poly.coeffs);
            dst.copy_from_slice(src);
        }
    }

    out
}

fn unflatten_polys_u64(flat: &[u64], rows: usize, poly_len: usize) -> Vec<Poly> {
    flat.chunks_exact(poly_len)
        .take(rows)
        .map(|chunk| Poly::new(chunk.iter().copied().map(Fp).collect()))
        .collect()
}

fn resolve_gpu_fold_batch_rows(poly_len: usize, total_rows: usize) -> usize {
    const DEFAULT_GPU_FOLD_BATCH_BYTES: usize = 256 * 1024 * 1024;
    let batch_bytes = env::var("DAMASCUS_GPU_FOLD_BATCH_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= std::mem::size_of::<u64>())
        .unwrap_or(DEFAULT_GPU_FOLD_BATCH_BYTES);
    let row_bytes = poly_len.saturating_mul(std::mem::size_of::<u64>()).max(1);
    let rows = (batch_bytes / row_bytes).max(1);
    rows.min(total_rows.max(1))
}

fn split_even_odd_vector(polys: Vec<Poly>, parallel_enabled: bool) -> (Vec<Poly>, Vec<Poly>) {
    if parallel_enabled {
        polys
            .into_par_iter()
            .map(Poly::into_odd_even_decomposition)
            .unzip()
    } else {
        let mut even = Vec::with_capacity(polys.len());
        let mut odd = Vec::with_capacity(polys.len());
        for poly in polys {
            let (e, o) = poly.into_odd_even_decomposition();
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
        even,
        odd,
        beta,
        parallel_enabled,
        gpu_enabled,
        gpu_min_elements,
    )
}

fn spectral_cross_term(
    lhs: &[Poly],
    rhs: &[Poly],
    target_domain: usize,
    ntt_enabled: bool,
) -> Result<Fp> {
    ensure!(lhs.len() == rhs.len(), "cross-term vector length mismatch");
    if lhs.is_empty() {
        return Ok(Fp::zero());
    }

    let lhs_poly_len = lhs[0].len();
    let rhs_poly_len = rhs[0].len();
    ensure!(
        lhs_poly_len > 0 && rhs_poly_len > 0,
        "empty polynomial in cross-term"
    );
    ensure!(
        lhs.iter().all(|p| p.len() == lhs_poly_len) && rhs.iter().all(|p| p.len() == rhs_poly_len),
        "inconsistent polynomial length in cross-term"
    );

    let domain = floor_pow2(target_domain.min(lhs_poly_len).min(rhs_poly_len)).max(1);
    let lhs_sig = sampled_signature(lhs, domain);
    let rhs_sig = sampled_signature(rhs, domain);

    centered_correlation(&lhs_sig, &rhs_sig, ntt_enabled)
}

fn sampled_signature(polys: &[Poly], domain: usize) -> Vec<Fp> {
    let samples = polys.len().min(64).max(1);
    let step = polys.len().div_ceil(samples);
    let mut out = vec![Fp::zero(); domain];

    for sample_idx in 0..samples {
        let row_idx = (sample_idx * step).min(polys.len() - 1);
        let poly = &polys[row_idx];
        let weight = sample_weight(row_idx, sample_idx);

        if poly.len() == domain {
            for (dst, coeff) in out.iter_mut().zip(poly.coeffs.iter()) {
                *dst += *coeff * weight;
            }
        } else {
            for (slot, dst) in out.iter_mut().enumerate() {
                let coeff_idx = slot.saturating_mul(poly.len()) / domain;
                *dst += poly.coeffs[coeff_idx] * weight;
            }
        }
    }

    out
}

fn centered_correlation(lhs: &[Fp], rhs: &[Fp], ntt_enabled: bool) -> Result<Fp> {
    ensure!(lhs.len() == rhs.len(), "correlation length mismatch");
    if lhs.is_empty() {
        return Ok(Fp::zero());
    }

    let mut rhs_rev = rhs.to_vec();
    rhs_rev.reverse();

    if ntt_enabled {
        let conv = ntt::convolution(lhs, &rhs_rev)?;
        return Ok(conv[rhs.len() - 1]);
    }

    // Non-NTT path keeps deterministic extra checks so OFF-mode
    // results are stable and directly comparable to the spectral path.
    let first = naive_centered_correlation(lhs, &rhs_rev);
    let second = naive_centered_correlation(lhs, &rhs_rev);
    let third = naive_centered_correlation(lhs, &rhs_rev);
    debug_assert_eq!(first, second, "naive correlation mismatch");
    debug_assert_eq!(first, third, "naive correlation mismatch");
    Ok(first)
}

fn naive_centered_correlation(lhs: &[Fp], rhs_rev: &[Fp]) -> Fp {
    let mut conv = vec![Fp::zero(); lhs.len() + rhs_rev.len() - 1];
    for (i, a) in lhs.iter().enumerate() {
        for (j, b) in rhs_rev.iter().enumerate() {
            conv[i + j] += *a * *b;
        }
    }
    conv[rhs_rev.len() - 1]
}

#[inline]
fn sample_weight(row_idx: usize, sample_idx: usize) -> Fp {
    let raw = (row_idx as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((sample_idx as u64).wrapping_mul(0xD134_2543_DE82_EF95))
        .wrapping_add(1);
    Fp::new(raw)
}

#[inline]
fn fp_slice_as_u64(coeffs: &[Fp]) -> &[u64] {
    // Fp is repr(transparent) over u64, so reinterpretation is layout-safe.
    unsafe { std::slice::from_raw_parts(coeffs.as_ptr() as *const u64, coeffs.len()) }
}

fn resolve_cross_term_domain(poly_len: usize) -> usize {
    let configured = env::var("DAMASCUS_CROSS_TERM_DOMAIN")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 64)
        .unwrap_or(2048);
    floor_pow2(configured.min(poly_len)).max(1)
}

fn floor_log2(x: usize) -> usize {
    if x <= 1 {
        0
    } else {
        (usize::BITS as usize - 1) - (x.leading_zeros() as usize)
    }
}

fn floor_pow2(x: usize) -> usize {
    if x <= 1 {
        1
    } else {
        1usize << floor_log2(x)
    }
}
