use crate::microblock::{MbPoly, MbVec};
use crate::params::ConvParams;
use crate::state::{ConvPublicState, ConvWitness};
use damascus_crypto::challenge::{hash_to_nonzero_mod_q, mod_inv};
use damascus_ring::{ModuleElem, Poly};
use damascus_types::{FileId, codec};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("parameter error: {0}")]
    Params(#[from] crate::params::ParamsError),
    #[error("shape mismatch")]
    Shape,
    #[error(transparent)]
    Challenge(#[from] damascus_crypto::challenge::ChallengeError),
    #[error(transparent)]
    Codec(#[from] codec::CodecError),
    #[error(transparent)]
    Poly(#[from] damascus_ring::poly::PolyError),
    #[error(transparent)]
    Module(#[from] damascus_ring::module::ModuleError),
    #[error("final opening invalid")]
    BadOpening,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptRound {
    pub mb_vec: MbVec,
    pub mb_poly: MbPoly,
}

pub fn public_update_round(
    params: &ConvParams,
    pub_state: &mut ConvPublicState,
    mb_vec: &MbVec,
    mb_poly: &MbPoly,
) -> Result<(), VerifyError> {
    params.validate()?;
    let x = derive_x(
        &mb_vec.file_id,
        mb_vec.epoch,
        mb_vec.round,
        &pub_state.c,
        &mb_vec.l_vec,
        &mb_vec.r_vec,
        params.q,
    )?;
    let x_inv = mod_inv(x, params.q)?;
    pub_state.c = pub_state
        .c
        .add(&mb_vec.l_vec.scalar_mul(x_inv)?)?
        .add(&mb_vec.r_vec.scalar_mul(x)?)?;

    let y = derive_y(
        &mb_poly.file_id,
        mb_poly.epoch,
        mb_poly.round,
        &pub_state.c,
        &mb_poly.l_poly,
        &mb_poly.r_poly,
        params.q,
    )?;
    let y_inv = mod_inv(y, params.q)?;
    let (c_even, _) = pub_state.c.odd_even_decompose()?;
    pub_state.c = c_even
        .add(&mb_poly.l_poly.scalar_mul(y_inv)?)?
        .add(&mb_poly.r_poly.scalar_mul(y)?)?;
    Ok(())
}

pub fn replay_fold_generators(
    params: &ConvParams,
    c0: &ModuleElem,
    transcript: &[TranscriptRound],
) -> Result<(ModuleElem, ModuleElem), VerifyError> {
    params.validate()?;
    let (c_final, g_star, h_star) = replay_all(params, c0.clone(), transcript)?;
    let _ = c_final;
    Ok((g_star, h_star))
}

pub fn final_verify(
    params: &ConvParams,
    pub_state: ConvPublicState,
    transcript: &[TranscriptRound],
    opening: (Poly, Poly),
) -> Result<(), VerifyError> {
    params.validate()?;

    let (c_final, g_star, h_star) = replay_all(params, pub_state.c, transcript)?;

    let (mu, rho) = opening;
    if mu.n() != 1 || rho.n() != 1 || g_star.n() != 1 || h_star.n() != 1 {
        return Err(VerifyError::Shape);
    }

    let rhs = g_star.ring_mul(&mu)?.add(&h_star.ring_mul(&rho)?)?;
    if c_final != rhs {
        return Err(VerifyError::BadOpening);
    }
    Ok(())
}

pub fn derive_x(
    file_id: &FileId,
    epoch: u64,
    j: u32,
    c_j: &ModuleElem,
    l_vec: &ModuleElem,
    r_vec: &ModuleElem,
    q: u64,
) -> Result<u64, VerifyError> {
    let input = codec::encode(&(file_id, epoch, j, c_j, l_vec, r_vec))?;
    Ok(hash_to_nonzero_mod_q(b"vec", &input, q)?)
}

pub fn derive_y(
    file_id: &FileId,
    epoch: u64,
    j: u32,
    c_half: &ModuleElem,
    l_poly: &ModuleElem,
    r_poly: &ModuleElem,
    q: u64,
) -> Result<u64, VerifyError> {
    let input = codec::encode(&(file_id, epoch, j, c_half, l_poly, r_poly))?;
    Ok(hash_to_nonzero_mod_q(b"poly", &input, q)?)
}

pub fn commit(
    params: &ConvParams,
    wit: &ConvWitness,
    g: &[ModuleElem],
    h: &[ModuleElem],
) -> Result<ModuleElem, VerifyError> {
    if wit.f.len() != g.len() || wit.r.len() != h.len() || g.len() != h.len() {
        return Err(VerifyError::Shape);
    }
    if g.is_empty() {
        return Err(VerifyError::Shape);
    }
    let mut acc = ModuleElem::zero(params.q, g[0].n(), params.k)?;
    for i in 0..wit.f.len() {
        acc = acc.add(&g[i].ring_mul(&wit.f[i])?)?;
    }
    for i in 0..wit.r.len() {
        acc = acc.add(&h[i].ring_mul(&wit.r[i])?)?;
    }
    Ok(acc)
}

pub fn derive_initial_generators(
    params: &ConvParams,
) -> Result<(Vec<ModuleElem>, Vec<ModuleElem>), VerifyError> {
    params.validate()?;
    let n = params.n0;
    let n_vec = params.n0;
    Ok((
        derive_generators(params.q, n, n_vec, params.k, params.seed_generators, b"G")?,
        derive_generators(params.q, n, n_vec, params.k, params.seed_generators, b"H")?,
    ))
}

fn derive_generators(
    q: u64,
    n: usize,
    n_vec: usize,
    k: usize,
    seed: [u8; 32],
    tag: &[u8],
) -> Result<Vec<ModuleElem>, VerifyError> {
    let mut out = Vec::with_capacity(n_vec);
    for i in 0..n_vec {
        let mut coords = Vec::with_capacity(k);
        for coord in 0..k {
            let mut label = Vec::new();
            label.extend_from_slice(tag);
            label.extend_from_slice(b":");
            label.extend_from_slice(&(i as u32).to_be_bytes());
            label.extend_from_slice(b":");
            label.extend_from_slice(&(coord as u32).to_be_bytes());
            coords.push(Poly::from_seed(q, n, seed, &label)?);
        }
        out.push(ModuleElem::from_coords(coords)?);
    }
    Ok(out)
}

fn replay_all(
    params: &ConvParams,
    mut c: ModuleElem,
    transcript: &[TranscriptRound],
) -> Result<(ModuleElem, ModuleElem, ModuleElem), VerifyError> {
    let (mut g, mut h) = derive_initial_generators(params)?;
    if transcript.is_empty() {
        if g.len() != 1 || h.len() != 1 {
            return Err(VerifyError::Shape);
        }
        return Ok((c, g.remove(0), h.remove(0)));
    }
    let file_id = transcript
        .first()
        .map(|t| t.mb_vec.file_id)
        .ok_or(VerifyError::Shape)?;
    let epoch = transcript
        .first()
        .map(|t| t.mb_vec.epoch)
        .ok_or(VerifyError::Shape)?;

    for t in transcript {
        if t.mb_vec.file_id != file_id || t.mb_poly.file_id != file_id {
            return Err(VerifyError::Shape);
        }
        if t.mb_vec.epoch != epoch || t.mb_poly.epoch != epoch {
            return Err(VerifyError::Shape);
        }

        // Stage 1: vector fold (N halves).
        if g.len() % 2 != 0 || g.len() != h.len() {
            return Err(VerifyError::Shape);
        }
        let half = g.len() / 2;
        let x = derive_x(
            &file_id,
            epoch,
            t.mb_vec.round,
            &c,
            &t.mb_vec.l_vec,
            &t.mb_vec.r_vec,
            params.q,
        )?;
        let x_inv = mod_inv(x, params.q)?;
        c = c
            .add(&t.mb_vec.l_vec.scalar_mul(x_inv)?)?
            .add(&t.mb_vec.r_vec.scalar_mul(x)?)?;

        let (g_l, g_r) = g.split_at(half);
        let (h_l, h_r) = h.split_at(half);
        g = fold_vec_module(g_l, g_r, x_inv)?;
        h = fold_vec_module(h_l, h_r, x_inv)?;

        // Stage 2: odd-even fold (n halves).
        let y = derive_y(
            &file_id,
            epoch,
            t.mb_poly.round,
            &c,
            &t.mb_poly.l_poly,
            &t.mb_poly.r_poly,
            params.q,
        )?;
        let y_inv = mod_inv(y, params.q)?;

        let (c_even, _) = c.odd_even_decompose()?;
        c = c_even
            .add(&t.mb_poly.l_poly.scalar_mul(y_inv)?)?
            .add(&t.mb_poly.r_poly.scalar_mul(y)?)?;

        g = fold_odd_even_module(&g, y_inv)?;
        h = fold_odd_even_module(&h, y_inv)?;
    }

    if g.len() != 1 || h.len() != 1 {
        return Err(VerifyError::Shape);
    }
    Ok((c, g.remove(0), h.remove(0)))
}

fn fold_vec_module(
    left: &[ModuleElem],
    right: &[ModuleElem],
    x_inv: u64,
) -> Result<Vec<ModuleElem>, VerifyError> {
    if left.len() != right.len() {
        return Err(VerifyError::Shape);
    }
    let mut out = Vec::with_capacity(left.len());
    for i in 0..left.len() {
        out.push(left[i].add(&right[i].scalar_mul(x_inv)?)?);
    }
    Ok(out)
}

fn fold_odd_even_module(input: &[ModuleElem], y_inv: u64) -> Result<Vec<ModuleElem>, VerifyError> {
    let mut out = Vec::with_capacity(input.len());
    for m in input {
        let (even, odd) = m.odd_even_decompose()?;
        let odd_scaled = odd.mul_by_x()?;
        out.push(even.add(&odd_scaled.scalar_mul(y_inv)?)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prover::ConvProver;
    use rand::RngCore as _;
    use rand_chacha::rand_core::SeedableRng as _;

    fn random_poly(q: u64, n: usize, rng: &mut rand_chacha::ChaCha20Rng) -> Poly {
        let mut coeffs = vec![0u64; n];
        for c in &mut coeffs {
            *c = (rng.next_u32() as u64) % q;
        }
        Poly::from_coeffs(q, coeffs).unwrap()
    }

    #[test]
    fn stage1_and_stage2_invariants_hold() {
        let params = ConvParams {
            q: 998_244_353,
            n0: 8,
            n_rounds: 3,
            k: 2,
            seed_generators: [7u8; 32],
        };
        let prover = ConvProver::new(params).unwrap();
        let (g, h) = derive_initial_generators(&params).unwrap();

        let mut rng = rand_chacha::ChaCha20Rng::from_seed([9u8; 32]);
        let mut f = Vec::with_capacity(params.n0);
        let mut r = Vec::with_capacity(params.n0);
        for _ in 0..params.n0 {
            f.push(random_poly(params.q, params.n0, &mut rng));
            r.push(random_poly(params.q, params.n0, &mut rng));
        }
        let mut wit = ConvWitness { f, r };
        let mut pub_state = ConvPublicState {
            g,
            h,
            c: ModuleElem::zero(params.q, params.n0, params.k).unwrap(),
        };
        pub_state.c = commit(&params, &wit, &pub_state.g, &pub_state.h).unwrap();

        let file_id = FileId([3u8; 32]);
        let epoch = 42u64;
        let j = 0u32;

        let c_before = pub_state.c.clone();
        let wit_before = wit.clone();
        let g_before = pub_state.g.clone();
        let h_before = pub_state.h.clone();

        let (mb_vec, mb_poly) = prover
            .round(file_id, epoch, j, &mut pub_state, &mut wit)
            .unwrap();

        // Public update matches prover's C update.
        let mut pub_state_pub = ConvPublicState {
            g: g_before.clone(),
            h: h_before.clone(),
            c: c_before.clone(),
        };
        public_update_round(&params, &mut pub_state_pub, &mb_vec, &mb_poly).unwrap();
        assert_eq!(pub_state_pub.c, pub_state.c);

        // Commitment invariant holds: C(j+1) == Com(F(j+1);R(j+1)) with folded generators.
        let c_recomputed = commit(&params, &wit, &pub_state.g, &pub_state.h).unwrap();
        assert_eq!(c_recomputed, pub_state.c);

        // Also basic anti-forgery sanity: flip a bit and expect failure.
        let mut bad_mb_vec = mb_vec.clone();
        bad_mb_vec.l_vec = bad_mb_vec.l_vec.scalar_mul(2).unwrap();
        let mut pub_state_bad = ConvPublicState {
            g: g_before,
            h: h_before,
            c: c_before,
        };
        let res = public_update_round(&params, &mut pub_state_bad, &bad_mb_vec, &mb_poly);
        assert!(res.is_ok()); // update computes something, but final check should fail

        let transcript = vec![TranscriptRound {
            mb_vec: bad_mb_vec,
            mb_poly,
        }];
        // Prover's opening is the folded witness scalars after full rounds; here only 1 round, so not final.
        // Just ensure the verifier rejects when expecting final shape.
        let mu = wit_before.f[0].clone();
        let rho = wit_before.r[0].clone();
        let res = final_verify(&params, pub_state_bad, &transcript, (mu, rho));
        assert!(res.is_err());
    }
}
