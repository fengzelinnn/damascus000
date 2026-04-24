use crate::algebra::field::Fp;
use crate::algebra::poly::Poly;
use crate::algebra::FieldElement;
use crate::commitment::sis::{DamascusStatement, ModuleCommitment, ModuleSisCommitter, SisParams};
use crate::protocol::transcript::Transcript;
use crate::utils::config::{RuntimeConfig, SystemParams};
use crate::utils::io;
use anyhow::{ensure, Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundRecord {
    pub round: usize,
    pub l_vec: ModuleCommitment,
    pub r_vec: ModuleCommitment,
    pub l_poly: ModuleCommitment,
    pub r_poly: ModuleCommitment,
}

pub type MicroBlock = RoundRecord;

#[derive(Clone, Debug)]
pub struct RoundOutput {
    pub micro_block: MicroBlock,
    pub vector_fold_time: Duration,
    pub poly_fold_time: Duration,
    pub total_round_time: Duration,
}

pub type FinalOpening = FieldElement;

#[derive(Clone, Debug)]
struct WitnessState {
    message: Vec<Poly>,
    g: Vec<ModuleCommitment>,
}

#[derive(Clone, Debug)]
pub struct DamascusProver {
    params: SystemParams,
    committer: ModuleSisCommitter,
    statement: DamascusStatement,
    transcript: Transcript,
    witness: WitnessState,
    current_commitment: ModuleCommitment,
    round: usize,
    ntt_enabled: bool,
    parallel_enabled: bool,
    gpu_enabled: bool,
    gpu_min_elements: usize,
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
        let expanded = io::expand_file_to_square_polys(&mmap, config.max_preprocess_bytes)
            .context("expand file into full witness")?;
        let message = expanded.message;

        params.vector_len = expanded.vector_len;
        params.poly_len = expanded.ring_len;
        params.rounds = expanded.depth;

        let committer = ModuleSisCommitter::new(SisParams {
            seed: params.seed_generators,
        })?;
        let families = committer
            .generators_for(expanded.vector_len, expanded.ring_len)
            .context("derive initial generator families")?;
        let statement = committer
            .register_with_ntt(
                expanded.file_id,
                expanded.original_len_bytes,
                expanded.depth,
                &message,
                config.ntt_enabled,
            )
            .context("register initial statement failed")?;
        let transcript = Transcript::new(&params, &statement);
        let current_commitment = statement.com_0.clone();

        Ok(Self {
            params,
            committer,
            statement,
            transcript,
            witness: WitnessState {
                message,
                g: families.g.clone(),
            },
            current_commitment,
            round: 0,
            ntt_enabled: config.ntt_enabled,
            parallel_enabled: config.parallel_enabled,
            gpu_enabled: config.gpu_enabled,
            gpu_min_elements: config.gpu_min_elements,
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

    pub fn current_dimensions(&self) -> (usize, usize) {
        (self.witness.message.len(), self.witness.message[0].len())
    }

    pub fn statement(&self) -> &DamascusStatement {
        &self.statement
    }

    pub fn final_opening(&self) -> Option<FinalOpening> {
        (self.round == self.params.rounds
            && self.witness.message.len() == 1
            && self.witness.message[0].len() == 1)
            .then(|| self.witness.message[0].coeffs[0])
    }

    pub fn fold_round(&mut self, round_idx: usize) -> Result<RoundOutput> {
        ensure!(round_idx == self.round, "round index mismatch");
        ensure!(self.round < self.params.rounds, "all rounds have finished");

        let total_start = Instant::now();
        let vector_start = Instant::now();

        let current_ring_len = self.witness.message[0].len();
        let (l_vec, r_vec, vector_fold_commitment, folded_message, folded_g) = if self
            .witness
            .message
            .len()
            > 1
        {
            let mid = self.witness.message.len() / 2;
            let msg_left = self.witness.message[..mid].to_vec();
            let msg_right = self.witness.message[mid..].to_vec();
            let g_left = self.witness.g[..mid].to_vec();
            let g_right = self.witness.g[mid..].to_vec();

            let l_vec = cross_term_vec(
                &self.committer,
                &msg_left,
                &g_right,
                self.ntt_enabled,
                self.gpu_enabled,
            )
            .context("left vector cross-term failed")?;
            let r_vec = cross_term_vec(
                &self.committer,
                &msg_right,
                &g_left,
                self.ntt_enabled,
                self.gpu_enabled,
            )
            .context("right vector cross-term failed")?;

            let x =
                self.transcript
                    .challenge_vec(round_idx, &self.current_commitment, &l_vec, &r_vec);
            let x_inv = x.inv();

            let vector_fold_commitment = self
                .current_commitment
                .add(&l_vec.scale(x_inv)?)?
                .add(&r_vec.scale(x)?)?;
            let folded_message = fold_vec_poly(
                &msg_left,
                &msg_right,
                x,
                self.parallel_enabled,
                self.gpu_enabled,
                self.gpu_min_elements,
            )?;
            let folded_g = fold_vec_module(&g_left, &g_right, x_inv, self.parallel_enabled)?;
            let vector_recomputed = self
                .committer
                .commit_with_generators_ntt(
                    &folded_message,
                    &folded_g,
                    self.ntt_enabled,
                    self.gpu_enabled,
                )
                .context("recompute vector stage commitment")?;
            ensure!(
                vector_fold_commitment == vector_recomputed,
                "vector folding invariant failed"
            );
            (
                l_vec,
                r_vec,
                vector_fold_commitment,
                folded_message,
                folded_g,
            )
        } else {
            let zero = ModuleCommitment::zero(current_ring_len);
            let _ =
                self.transcript
                    .challenge_vec(round_idx, &self.current_commitment, &zero, &zero);
            (
                zero.clone(),
                zero,
                self.current_commitment.clone(),
                self.witness.message.clone(),
                self.witness.g.clone(),
            )
        };
        let vector_fold_time = vector_start.elapsed();

        let poly_start = Instant::now();
        let (l_poly, r_poly, next_message, next_g, next_commitment) = if folded_message[0].len() > 1
        {
            let (msg_even, msg_odd) = odd_even_vec_poly(&folded_message)?;
            let (g_even, g_odd_scaled) = odd_even_vec_module_scaled(&folded_g)?;

            let l_poly = cross_term_vec(
                &self.committer,
                &msg_even,
                &g_odd_scaled,
                self.ntt_enabled,
                self.gpu_enabled,
            )
            .context("left poly cross-term failed")?;
            let r_poly = cross_term_vec(
                &self.committer,
                &msg_odd,
                &g_even,
                self.ntt_enabled,
                self.gpu_enabled,
            )
            .context("right poly cross-term failed")?;

            let y = self.transcript.challenge_poly(
                round_idx,
                &vector_fold_commitment,
                &l_poly,
                &r_poly,
            );
            let y_inv = y.inv();
            let (c_even, _) = vector_fold_commitment.odd_even_decomposition()?;
            let next_commitment = c_even.add(&l_poly.scale(y_inv)?)?.add(&r_poly.scale(y)?)?;
            let next_message = fold_poly_poly(&msg_even, &msg_odd, y, self.parallel_enabled)?;
            let next_g = fold_poly_module(&g_even, &g_odd_scaled, y_inv, self.parallel_enabled)?;
            (l_poly, r_poly, next_message, next_g, next_commitment)
        } else {
            let zero = ModuleCommitment::zero(1);
            let _ =
                self.transcript
                    .challenge_poly(round_idx, &vector_fold_commitment, &zero, &zero);
            (
                zero.clone(),
                zero,
                folded_message,
                folded_g,
                vector_fold_commitment.clone(),
            )
        };

        let recomputed = self
            .committer
            .commit_with_generators_ntt(
                &next_message,
                &next_g,
                self.ntt_enabled,
                self.gpu_enabled,
            )
            .context("recompute post-poly commitment")?;
        ensure!(
            next_commitment == recomputed,
            "round folding invariant failed"
        );
        let poly_fold_time = poly_start.elapsed();

        self.witness = WitnessState {
            message: next_message,
            g: next_g,
        };
        self.current_commitment = next_commitment;
        self.round += 1;

        Ok(RoundOutput {
            micro_block: RoundRecord {
                round: round_idx,
                l_vec,
                r_vec,
                l_poly,
                r_poly,
            },
            vector_fold_time,
            poly_fold_time,
            total_round_time: total_start.elapsed(),
        })
    }
}

pub fn depth_j(d: usize, j: usize) -> (usize, usize) {
    assert!(j <= d, "round index exceeds fold depth");
    let dim = 1usize << (d - j);
    (dim, dim)
}

fn cross_term_vec(
    committer: &ModuleSisCommitter,
    witness: &[Poly],
    g: &[ModuleCommitment],
    ntt_enabled: bool,
    gpu_enabled: bool,
) -> Result<ModuleCommitment> {
    committer.commit_with_generators_ntt(witness, g, ntt_enabled, gpu_enabled)
}

fn fold_vec_poly(
    left: &[Poly],
    right: &[Poly],
    challenge: Fp,
    parallel: bool,
    gpu_enabled: bool,
    gpu_min_elements: usize,
) -> Result<Vec<Poly>> {
    ensure!(left.len() == right.len(), "vector fold length mismatch");
    let total_coeffs: usize = left.iter().map(|p| p.len()).sum();

    if gpu_enabled && !left.is_empty() && total_coeffs >= gpu_min_elements {
        use crate::utils::gpu::try_fold_pairs_gpu;

        let left_flat: Vec<u128> = left
            .iter()
            .flat_map(|p| p.coeffs.iter().map(|c| c.as_u128()))
            .collect();
        let right_flat: Vec<u128> = right
            .iter()
            .flat_map(|p| p.coeffs.iter().map(|c| c.as_u128()))
            .collect();
        if let Some(out_flat) = try_fold_pairs_gpu(&left_flat, &right_flat, challenge.as_u128()) {
            let poly_len = left[0].len();
            let polys: Vec<Poly> = out_flat
                .chunks_exact(poly_len)
                .map(|chunk| Poly::new(chunk.iter().map(|&v| Fp::from_u128(v)).collect()))
                .collect();
            return Ok(polys);
        }
    }

    if parallel {
        left.par_iter()
            .zip(right.par_iter())
            .map(|(l, r)| l.add(&r.scale(challenge)))
            .collect::<Result<Vec<_>>>()
    } else {
        left.iter()
            .zip(right.iter())
            .map(|(l, r)| l.add(&r.scale(challenge)))
            .collect()
    }
}

fn fold_vec_module(
    left: &[ModuleCommitment],
    right: &[ModuleCommitment],
    challenge_inv: Fp,
    parallel: bool,
) -> Result<Vec<ModuleCommitment>> {
    ensure!(left.len() == right.len(), "generator fold length mismatch");
    if parallel {
        left.par_iter()
            .zip(right.par_iter())
            .map(|(l, r)| l.add_scaled(r, challenge_inv))
            .collect::<Result<Vec<_>>>()
    } else {
        left.iter()
            .zip(right.iter())
            .map(|(l, r)| l.add_scaled(r, challenge_inv))
            .collect()
    }
}

fn odd_even_vec_poly(input: &[Poly]) -> Result<(Vec<Poly>, Vec<Poly>)> {
    let mut even = Vec::with_capacity(input.len());
    let mut odd = Vec::with_capacity(input.len());
    for poly in input {
        let (e, o) = poly.odd_even_decomposition();
        even.push(e);
        odd.push(o);
    }
    Ok((even, odd))
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

fn fold_poly_poly(even: &[Poly], odd: &[Poly], challenge: Fp, parallel: bool) -> Result<Vec<Poly>> {
    ensure!(even.len() == odd.len(), "poly fold length mismatch");
    if parallel {
        even.par_iter()
            .zip(odd.par_iter())
            .map(|(e, o)| e.add(&o.scale(challenge)))
            .collect::<Result<Vec<_>>>()
    } else {
        even.iter()
            .zip(odd.iter())
            .map(|(e, o)| e.add(&o.scale(challenge)))
            .collect()
    }
}

fn fold_poly_module(
    even: &[ModuleCommitment],
    odd_scaled: &[ModuleCommitment],
    challenge_inv: Fp,
    parallel: bool,
) -> Result<Vec<ModuleCommitment>> {
    ensure!(
        even.len() == odd_scaled.len(),
        "generator poly fold length mismatch"
    );
    if parallel {
        even.par_iter()
            .zip(odd_scaled.par_iter())
            .map(|(e, o)| e.add_scaled(o, challenge_inv))
            .collect::<Result<Vec<_>>>()
    } else {
        even.iter()
            .zip(odd_scaled.iter())
            .map(|(e, o)| e.add_scaled(o, challenge_inv))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        depth_j, fold_poly_module, fold_poly_poly, fold_vec_module, fold_vec_poly,
        odd_even_vec_module_scaled, odd_even_vec_poly, DamascusProver, RoundRecord,
    };
    use crate::commitment::sis::{DamascusStatement, ModuleCommitment};
    use crate::protocol::verifier::DamascusVerifier;
    use crate::utils::config::{
        RuntimeConfig, SystemParams, BYTES_PER_COEFF, MIN_FOLD_DEPTH, MODULE_RANK,
    };
    use crate::utils::io::square_witness_layout_for_byte_len;
    use std::fs;

    fn write_pattern_file(path: &std::path::Path, size: usize, tweak: u8) {
        let payload = (0..size)
            .map(|idx| (idx as u8).wrapping_mul(17).wrapping_add(tweak))
            .collect::<Vec<_>>();
        fs::write(path, payload).expect("write pattern file");
    }

    fn byte_len_for_depth(d: usize) -> usize {
        assert!(d >= MIN_FOLD_DEPTH);
        if d == MIN_FOLD_DEPTH {
            1
        } else {
            ((1usize << (2 * (d - 1))) + 1) * BYTES_PER_COEFF
        }
    }

    fn zero_statement_for_depth(d: usize) -> DamascusStatement {
        DamascusStatement {
            file_id: [d as u8; 32],
            original_len_bytes: byte_len_for_depth(d) as u64,
            d,
            com_0: ModuleCommitment::zero(1 << d),
            g_0_seed: [17u8; 32],
        }
    }

    fn zero_round_record(d: usize, round: usize) -> RoundRecord {
        let current = depth_j(d, round);
        let next = depth_j(d, round + 1);
        RoundRecord {
            round,
            l_vec: ModuleCommitment::zero(current.1),
            r_vec: ModuleCommitment::zero(current.1),
            l_poly: ModuleCommitment::zero(next.1),
            r_poly: ModuleCommitment::zero(next.1),
        }
    }

    #[test]
    fn distinct_megabyte_files_produce_distinct_initial_commitments() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file_a = temp_dir.path().join("a.bin");
        let file_b = temp_dir.path().join("b.bin");
        write_pattern_file(&file_a, 1 << 20, 3);
        write_pattern_file(&file_b, 1 << 20, 4);

        let prover_a = DamascusProver::initialize(&file_a, SystemParams::default()).expect("a");
        let prover_b = DamascusProver::initialize(&file_b, SystemParams::default()).expect("b");
        assert_ne!(prover_a.current_commitment(), prover_b.current_commitment());
        assert_ne!(prover_a.statement().file_id, prover_b.statement().file_id);
    }

    #[test]
    fn initialize_errors_when_full_expansion_exceeds_memory_limit() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file = temp_dir.path().join("big.bin");
        write_pattern_file(&file, 1 << 20, 7);

        let err = DamascusProver::initialize_with_config(
            &file,
            SystemParams::default(),
            RuntimeConfig {
                max_preprocess_bytes: 1024,
                ..RuntimeConfig::default()
            },
        )
        .expect_err("must fail");
        assert!(
            err.to_string().contains("max_preprocess_bytes")
                || err.to_string().contains("expand file")
        );
    }

    #[test]
    fn honest_transcript_replays_and_tampering_is_rejected() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file = temp_dir.path().join("honest.bin");
        write_pattern_file(&file, 8192, 9);

        let params = SystemParams::default();
        let mut prover = DamascusProver::initialize(&file, params.clone()).expect("prover");
        let mut verifier =
            DamascusVerifier::new(params, prover.statement().clone()).expect("verifier");

        let rounds = prover.rounds_total();
        let mut records = Vec::new();
        for round in 0..rounds {
            let out = prover.fold_round(round).expect("fold round");
            verifier
                .update_commitment(&out.micro_block)
                .expect("verify round");
            records.push(out.micro_block);
        }
        let opening = prover.final_opening().expect("final opening");
        verifier
            .verify_final_opening(&opening)
            .expect("honest opening");

        let mut bad_record = records[0].clone();
        bad_record.l_vec.coords[0].coeffs[0] += crate::algebra::field::Fp::one();
        let mut bad_verifier =
            DamascusVerifier::new(SystemParams::default(), prover.statement().clone())
                .expect("verifier");
        bad_verifier
            .update_commitment(&bad_record)
            .expect("bad first round still replays");
        for record in records.iter().skip(1) {
            bad_verifier
                .update_commitment(record)
                .expect("replay bad path");
        }
        let err = bad_verifier
            .verify_final_opening(&opening)
            .expect_err("tampered transcript must fail");
        assert!(err.to_string().contains("mismatch") || err.to_string().contains("opening"));
    }

    #[test]
    fn round_record_serialization_matches_module_payload_scale() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file = temp_dir.path().join("sized.bin");
        write_pattern_file(&file, 8192, 13);

        let params = SystemParams::default();
        let mut prover = DamascusProver::initialize(&file, params).expect("prover");
        let out = prover.fold_round(0).expect("first fold");
        let encoded = bincode::serialize(&out.micro_block).expect("serialize");
        let coeff_count = [
            &out.micro_block.l_vec,
            &out.micro_block.r_vec,
            &out.micro_block.l_poly,
            &out.micro_block.r_poly,
        ]
        .into_iter()
        .map(|term| {
            term.coords
                .iter()
                .map(|coord| coord.coeffs.len())
                .sum::<usize>()
        })
        .sum::<usize>();
        let raw_coeff_bytes = coeff_count * crate::algebra::field::Fp::SERDE_BYTES;
        assert!(
            encoded.len() >= raw_coeff_bytes,
            "round record was unexpectedly compressed: {} < {}",
            encoded.len(),
            raw_coeff_bytes
        );
        assert!(
            encoded.len() >= 4 * MODULE_RANK * 32 * crate::algebra::field::Fp::SERDE_BYTES,
            "round record is too small for four module-valued cross-terms"
        );
    }

    #[test]
    fn single_round_preserves_commitment_through_vector_and_poly_stages() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file = temp_dir.path().join("preserve.bin");
        write_pattern_file(&file, 8192, 31);

        let mut prover =
            DamascusProver::initialize(&file, SystemParams::default()).expect("prover");
        let transcript = prover.transcript.clone();
        let current_commitment = prover.current_commitment.clone();
        let current_message = prover.witness.message.clone();
        let current_g = prover.witness.g.clone();

        let out = prover.fold_round(0).expect("round 0");

        let mid = current_message.len() / 2;
        let msg_left = current_message[..mid].to_vec();
        let msg_right = current_message[mid..].to_vec();
        let g_left = current_g[..mid].to_vec();
        let g_right = current_g[mid..].to_vec();

        let x = transcript.challenge_vec(
            0,
            &current_commitment,
            &out.micro_block.l_vec,
            &out.micro_block.r_vec,
        );
        let x_inv = x.inv();
        let vector_fold_commitment = current_commitment
            .add(&out.micro_block.l_vec.scale(x_inv).expect("x^-1 * l_vec"))
            .and_then(|c| c.add(&out.micro_block.r_vec.scale(x).expect("x * r_vec")))
            .expect("vector fold commitment");
        let folded_message =
            fold_vec_poly(&msg_left, &msg_right, x, false, false, 0).expect("folded message");
        let folded_g = fold_vec_module(&g_left, &g_right, x_inv, false).expect("folded g");
        let vector_recomputed = prover
            .committer
            .commit_with_generators(&folded_message, &folded_g)
            .expect("vector recomputed");
        assert_eq!(vector_fold_commitment, vector_recomputed);

        let (msg_even, msg_odd) = odd_even_vec_poly(&folded_message).expect("odd/even message");
        let (g_even, g_odd_scaled) = odd_even_vec_module_scaled(&folded_g).expect("odd/even g");
        let y = transcript.challenge_poly(
            0,
            &vector_fold_commitment,
            &out.micro_block.l_poly,
            &out.micro_block.r_poly,
        );
        let y_inv = y.inv();
        let next_message = fold_poly_poly(&msg_even, &msg_odd, y, false).expect("next message");
        let next_g = fold_poly_module(&g_even, &g_odd_scaled, y_inv, false).expect("next g");
        let (c_even, _) = vector_fold_commitment
            .odd_even_decomposition()
            .expect("commitment odd/even");
        let next_commitment = c_even
            .add(&out.micro_block.l_poly.scale(y_inv).expect("y^-1 * l_poly"))
            .and_then(|c| c.add(&out.micro_block.r_poly.scale(y).expect("y * r_poly")))
            .expect("next commitment");
        let recomputed = prover
            .committer
            .commit_with_generators(&next_message, &next_g)
            .expect("poly recomputed");
        assert_eq!(next_commitment, recomputed);
        assert_eq!(&next_commitment, prover.current_commitment());
    }

    #[test]
    fn serialized_round_record_byte_flip_is_rejected() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file = temp_dir.path().join("tamper.bin");
        write_pattern_file(&file, 8192, 21);

        let params = SystemParams::default();
        let mut prover = DamascusProver::initialize(&file, params.clone()).expect("prover");
        let mut verifier =
            DamascusVerifier::new(params, prover.statement().clone()).expect("verifier");

        let out = prover.fold_round(0).expect("fold");
        let opening_rounds = prover.rounds_total();
        let mut rest = Vec::new();
        for round in 1..opening_rounds {
            rest.push(prover.fold_round(round).expect("later fold").micro_block);
        }
        let opening = prover.final_opening().expect("final opening");

        let mut encoded = bincode::serialize(&out.micro_block).expect("serialize");
        let flip_idx = encoded
            .len()
            .saturating_sub(crate::algebra::field::Fp::SERDE_BYTES / 2);
        encoded[flip_idx] ^= 0x01;

        match bincode::deserialize::<RoundRecord>(&encoded) {
            Ok(record) => {
                verifier
                    .update_commitment(&record)
                    .expect("tampered round should still deserialize");
                for record in &rest {
                    verifier
                        .update_commitment(record)
                        .expect("replay remainder");
                }
                let err = verifier
                    .verify_final_opening(&opening)
                    .expect_err("tampered serialized record must fail");
                assert!(
                    err.to_string().contains("mismatch") || err.to_string().contains("opening")
                );
            }
            Err(_) => {}
        }
    }

    #[test]
    fn dual_dimension_fold_halves_each_round_and_round_record_size_tracks_ring_degree() {
        for &d in &[6usize, 8, 10] {
            let byte_len = byte_len_for_depth(d);
            let layout = square_witness_layout_for_byte_len(byte_len as u64).expect("layout");
            assert_eq!(layout.depth, d);

            let params = SystemParams::default();
            let mut verifier =
                DamascusVerifier::new(params, zero_statement_for_depth(d)).expect("verifier");

            let mut total_coeffs = (1usize << d) * (1usize << d);
            for round in 0..d {
                let expected = depth_j(d, round);
                assert_eq!(verifier.current_dimensions(), expected);

                let record = zero_round_record(d, round);
                verifier.update_commitment(&record).expect("replay round");
                let encoded = bincode::serialize(&record).expect("serialize");
                println!(
                    "depth={d} round={round} N_j={} n_j={} record_bytes={}",
                    expected.0,
                    expected.1,
                    encoded.len()
                );
                let min_record_bytes =
                    2 * MODULE_RANK * expected.1 * crate::algebra::field::Fp::SERDE_BYTES;
                assert!(
                    encoded.len() >= min_record_bytes,
                    "round {round} record too small: {} < {}",
                    encoded.len(),
                    min_record_bytes
                );

                let next = depth_j(d, round + 1);
                assert_eq!(verifier.current_dimensions(), next);

                let next_total_coeffs = next.0 * next.1;
                assert_eq!(next_total_coeffs * 4, total_coeffs);
                total_coeffs = next_total_coeffs;
            }

            assert_eq!(depth_j(d, d), (1, 1));
            let opening = crate::algebra::FieldElement::zero();
            verifier
                .verify_final_opening(&opening)
                .expect("final verify");
        }
    }

    #[test]
    fn e2e_succeeds_across_depth_sweep() {
        for &d in &[6usize, 7, 8, 9, 10] {
            let params = SystemParams::default();
            let mut verifier =
                DamascusVerifier::new(params, zero_statement_for_depth(d)).expect("verifier");

            for round in 0..d {
                verifier
                    .update_commitment(&zero_round_record(d, round))
                    .expect("replay");
            }

            let opening = crate::algebra::FieldElement::zero();
            verifier
                .verify_final_opening(&opening)
                .expect("verify final opening");
        }
    }

    #[test]
    fn every_serialized_micro_block_byte_flip_is_rejected() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file = temp_dir.path().join("full-byte-fuzz.bin");
        write_pattern_file(&file, 8192, 41);

        let params = SystemParams::default();
        let mut prover = DamascusProver::initialize(&file, params.clone()).expect("prover");
        let statement = prover.statement().clone();
        let mut records = Vec::new();
        for round in 0..prover.rounds_total() {
            records.push(prover.fold_round(round).expect("fold").micro_block);
        }
        let opening = prover.final_opening().expect("final opening");
        let mut verifier =
            DamascusVerifier::new(params.clone(), statement.clone()).expect("verifier");
        let mut verifier_prefixes = Vec::with_capacity(records.len());
        for record in &records {
            verifier_prefixes.push(verifier.clone());
            verifier
                .update_commitment(record)
                .expect("replay honest prefix");
        }
        let encoded_records = records
            .iter()
            .map(|record| bincode::serialize(record).expect("serialize"))
            .collect::<Vec<_>>();

        for (record_idx, encoded) in encoded_records.iter().enumerate() {
            for byte_idx in 0..encoded.len() {
                let mut tampered = encoded.clone();
                tampered[byte_idx] ^= 0x01;

                let mut verifier = verifier_prefixes[record_idx].clone();
                let mut rejected = false;

                let candidate = match bincode::deserialize::<RoundRecord>(&tampered) {
                    Ok(record) => record,
                    Err(_) => {
                        rejected = true;
                        records[record_idx].clone()
                    }
                };

                if !rejected && verifier.update_commitment(&candidate).is_err() {
                    rejected = true;
                }

                if !rejected {
                    for original in records.iter().skip(record_idx + 1) {
                        if verifier.update_commitment(original).is_err() {
                            rejected = true;
                            break;
                        }
                    }
                }

                if !rejected && verifier.verify_final_opening(&opening).is_err() {
                    rejected = true;
                }

                assert!(
                    rejected,
                    "tampered record {record_idx} byte {byte_idx} was accepted"
                );
            }
        }
    }
}
