use crate::algebra::field::Fp;
use crate::algebra::poly::Poly;
use crate::commitment::sis::{DamascusStatement, ModuleCommitment, ModuleSisCommitter, SisParams};
use crate::protocol::transcript::Transcript;
use crate::utils::config::{RuntimeConfig, SystemParams};
use crate::utils::io;
use anyhow::{ensure, Context, Result};
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalOpening {
    pub m_star: Poly,
    pub r_star: Poly,
}

#[derive(Clone, Debug)]
struct WitnessState {
    message: Vec<Poly>,
    blinding: Vec<Poly>,
    g: Vec<ModuleCommitment>,
    h: Vec<ModuleCommitment>,
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
        let blinding = io::sample_blinding_polys(
            expanded.vector_len,
            expanded.ring_len,
            params.seed_generators,
            expanded.file_id,
        );

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
            .register(
                expanded.file_id,
                expanded.original_len_bytes,
                expanded.depth,
                &message,
                &blinding,
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
                blinding,
                g: families.g.clone(),
                h: families.h.clone(),
            },
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

    pub fn statement(&self) -> &DamascusStatement {
        &self.statement
    }

    pub fn final_opening(&self) -> Option<FinalOpening> {
        (self.round == self.params.rounds && self.witness.message.len() == 1).then(|| {
            FinalOpening {
                m_star: self.witness.message[0].clone(),
                r_star: self.witness.blinding[0].clone(),
            }
        })
    }

    pub fn fold_round(&mut self, round_idx: usize) -> Result<RoundOutput> {
        ensure!(round_idx == self.round, "round index mismatch");
        ensure!(self.round < self.params.rounds, "all rounds have finished");

        let total_start = Instant::now();
        let vector_start = Instant::now();

        let current_ring_len = self.witness.message[0].len();
        let (
            l_vec,
            r_vec,
            vector_fold_commitment,
            folded_message,
            folded_blinding,
            folded_g,
            folded_h,
        ) = if self.witness.message.len() > 1 {
            let mid = self.witness.message.len() / 2;
            let msg_left = self.witness.message[..mid].to_vec();
            let msg_right = self.witness.message[mid..].to_vec();
            let rnd_left = self.witness.blinding[..mid].to_vec();
            let rnd_right = self.witness.blinding[mid..].to_vec();
            let g_left = self.witness.g[..mid].to_vec();
            let g_right = self.witness.g[mid..].to_vec();
            let h_left = self.witness.h[..mid].to_vec();
            let h_right = self.witness.h[mid..].to_vec();

            let l_vec = cross_term_vec(&self.committer, &msg_left, &rnd_left, &g_right, &h_right)
                .context("left vector cross-term failed")?;
            let r_vec = cross_term_vec(&self.committer, &msg_right, &rnd_right, &g_left, &h_left)
                .context("right vector cross-term failed")?;

            let x =
                self.transcript
                    .challenge_vec(round_idx, &self.current_commitment, &l_vec, &r_vec);
            let x_inv = x.inv();

            let vector_fold_commitment = self
                .current_commitment
                .add(&l_vec.scale(x_inv)?)?
                .add(&r_vec.scale(x)?)?;
            let folded_message = fold_vec_poly(&msg_left, &msg_right, x)?;
            let folded_blinding = fold_vec_poly(&rnd_left, &rnd_right, x)?;
            let folded_g = fold_vec_module(&g_left, &g_right, x_inv)?;
            let folded_h = fold_vec_module(&h_left, &h_right, x_inv)?;
            let vector_recomputed = self
                .committer
                .commit_with_generators(&folded_message, &folded_blinding, &folded_g, &folded_h)
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
                folded_blinding,
                folded_g,
                folded_h,
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
                self.witness.blinding.clone(),
                self.witness.g.clone(),
                self.witness.h.clone(),
            )
        };
        let vector_fold_time = vector_start.elapsed();

        let poly_start = Instant::now();
        let (l_poly, r_poly, next_message, next_blinding, next_g, next_h, next_commitment) =
            if folded_message[0].len() > 1 {
                let (msg_even, msg_odd) = odd_even_vec_poly(&folded_message)?;
                let (rnd_even, rnd_odd) = odd_even_vec_poly(&folded_blinding)?;
                let (g_even, g_odd_scaled) = odd_even_vec_module_scaled(&folded_g)?;
                let (h_even, h_odd_scaled) = odd_even_vec_module_scaled(&folded_h)?;

                let l_poly = cross_term_vec(
                    &self.committer,
                    &msg_even,
                    &rnd_even,
                    &g_odd_scaled,
                    &h_odd_scaled,
                )
                .context("left poly cross-term failed")?;
                let r_poly = cross_term_vec(&self.committer, &msg_odd, &rnd_odd, &g_even, &h_even)
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
                let next_message = fold_poly_poly(&msg_even, &msg_odd, y)?;
                let next_blinding = fold_poly_poly(&rnd_even, &rnd_odd, y)?;
                let next_g = fold_poly_module(&g_even, &g_odd_scaled, y_inv)?;
                let next_h = fold_poly_module(&h_even, &h_odd_scaled, y_inv)?;
                (
                    l_poly,
                    r_poly,
                    next_message,
                    next_blinding,
                    next_g,
                    next_h,
                    next_commitment,
                )
            } else {
                let zero = ModuleCommitment::zero(1);
                let _ = self.transcript.challenge_poly(
                    round_idx,
                    &vector_fold_commitment,
                    &zero,
                    &zero,
                );
                (
                    zero.clone(),
                    zero,
                    folded_message,
                    folded_blinding,
                    folded_g,
                    folded_h,
                    vector_fold_commitment.clone(),
                )
            };

        let recomputed = self
            .committer
            .commit_with_generators(&next_message, &next_blinding, &next_g, &next_h)
            .context("recompute post-poly commitment")?;
        ensure!(
            next_commitment == recomputed,
            "round folding invariant failed"
        );
        let poly_fold_time = poly_start.elapsed();

        self.witness = WitnessState {
            message: next_message,
            blinding: next_blinding,
            g: next_g,
            h: next_h,
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

fn cross_term_vec(
    committer: &ModuleSisCommitter,
    witness: &[Poly],
    blinding: &[Poly],
    g: &[ModuleCommitment],
    h: &[ModuleCommitment],
) -> Result<ModuleCommitment> {
    committer.commit_with_generators(witness, blinding, g, h)
}

fn fold_vec_poly(left: &[Poly], right: &[Poly], challenge: Fp) -> Result<Vec<Poly>> {
    ensure!(left.len() == right.len(), "vector fold length mismatch");
    left.iter()
        .zip(right.iter())
        .map(|(l, r)| l.add(&r.scale(challenge)))
        .collect()
}

fn fold_vec_module(
    left: &[ModuleCommitment],
    right: &[ModuleCommitment],
    challenge_inv: Fp,
) -> Result<Vec<ModuleCommitment>> {
    ensure!(left.len() == right.len(), "generator fold length mismatch");
    left.iter()
        .zip(right.iter())
        .map(|(l, r)| l.add_scaled(r, challenge_inv))
        .collect()
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

fn fold_poly_poly(even: &[Poly], odd: &[Poly], challenge: Fp) -> Result<Vec<Poly>> {
    ensure!(even.len() == odd.len(), "poly fold length mismatch");
    even.iter()
        .zip(odd.iter())
        .map(|(e, o)| e.add(&o.scale(challenge)))
        .collect()
}

fn fold_poly_module(
    even: &[ModuleCommitment],
    odd_scaled: &[ModuleCommitment],
    challenge_inv: Fp,
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

#[cfg(test)]
mod tests {
    use super::{DamascusProver, RoundRecord};
    use crate::protocol::verifier::DamascusVerifier;
    use crate::utils::config::MODULE_RANK;
    use crate::utils::config::{RuntimeConfig, SystemParams};
    use std::fs;

    fn write_pattern_file(path: &std::path::Path, size: usize, tweak: u8) {
        let payload = (0..size)
            .map(|idx| (idx as u8).wrapping_mul(17).wrapping_add(tweak))
            .collect::<Vec<_>>();
        fs::write(path, payload).expect("write pattern file");
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
}
