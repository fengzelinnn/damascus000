use anyhow::Context as _;
use damascus_conv::{
    ConvParams, ConvProver, ConvPublicState, ConvWitness, TranscriptRound, final_verify,
    public_update_round,
};
use damascus_ring::{ModuleElem, Poly};
use damascus_types::FileId;
use rand::RngCore as _;
use rand::SeedableRng as _;

#[derive(Clone, Debug)]
pub struct SimResult {
    pub file_id: FileId,
    pub epoch: u64,
    pub c0: ModuleElem,
    pub transcript: Vec<TranscriptRound>,
    pub opening: (Poly, Poly),
}

pub fn file_id_from_bytes(bytes: &[u8]) -> FileId {
    let h = blake3::hash(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(h.as_bytes());
    FileId(out)
}

pub fn witness_from_file(params: &ConvParams, file_bytes: &[u8]) -> anyhow::Result<ConvWitness> {
    params.validate().context("invalid params")?;
    let seed = blake3::hash(file_bytes);
    let mut rng = rand_chacha::ChaCha20Rng::from_seed(*seed.as_bytes());

    let mut f = Vec::with_capacity(params.n0);
    let mut r = Vec::with_capacity(params.n0);
    for _ in 0..params.n0 {
        let mut f_coeffs = vec![0u64; params.n0];
        let mut r_coeffs = vec![0u64; params.n0];
        for c in &mut f_coeffs {
            *c = (rng.next_u32() as u64) % params.q;
        }
        for c in &mut r_coeffs {
            *c = (rng.next_u32() as u64) % params.q;
        }
        f.push(Poly::from_coeffs(params.q, f_coeffs)?);
        r.push(Poly::from_coeffs(params.q, r_coeffs)?);
    }
    Ok(ConvWitness { f, r })
}

pub fn run_epoch(params: ConvParams, file_bytes: &[u8], epoch: u64) -> anyhow::Result<SimResult> {
    params.validate().context("invalid params")?;
    let file_id = file_id_from_bytes(file_bytes);

    let prover = ConvProver::new(params)?;
    let (g0, h0) = damascus_conv::verifier::derive_initial_generators(&params)?;
    let mut wit = witness_from_file(&params, file_bytes)?;

    let mut pub_state = ConvPublicState {
        g: g0.clone(),
        h: h0.clone(),
        c: ModuleElem::zero(params.q, params.n0, params.k)?,
    };
    pub_state.c = damascus_conv::verifier::commit(&params, &wit, &pub_state.g, &pub_state.h)?;
    let c0 = pub_state.c.clone();

    let mut transcript = Vec::with_capacity(params.n_rounds);
    for j in 0..(params.n_rounds as u32) {
        let (mb_vec, mb_poly) = prover.round(file_id, epoch, j, &mut pub_state, &mut wit)?;
        transcript.push(TranscriptRound { mb_vec, mb_poly });
    }

    if wit.f.len() != 1 || wit.r.len() != 1 {
        anyhow::bail!("unexpected final witness shape");
    }
    let opening = (wit.f.remove(0), wit.r.remove(0));

    // Verifier: constant-time public updates + final opening check.
    let mut pub_state_ver = ConvPublicState {
        g: g0,
        h: h0,
        c: c0.clone(),
    };
    for t in &transcript {
        public_update_round(&params, &mut pub_state_ver, &t.mb_vec, &t.mb_poly)?;
    }
    // `final_verify` also replays the transcript and checks opening.
    final_verify(
        &params,
        ConvPublicState {
            g: Vec::new(),
            h: Vec::new(),
            c: c0.clone(),
        },
        &transcript,
        opening.clone(),
    )?;

    Ok(SimResult {
        file_id,
        epoch,
        c0,
        transcript,
        opening,
    })
}
