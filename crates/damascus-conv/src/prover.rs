use crate::microblock::{MbPoly, MbVec};
use crate::params::ConvParams;
use crate::state::{ConvPublicState, ConvWitness};
use crate::verifier::{derive_x, derive_y};
use damascus_crypto::challenge::mod_inv;
use damascus_ring::{ModuleElem, Poly};
use damascus_types::FileId;

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("parameter error: {0}")]
    Params(#[from] crate::params::ParamsError),
    #[error("invalid witness/public state shape")]
    Shape,
    #[error(transparent)]
    Challenge(#[from] damascus_crypto::challenge::ChallengeError),
    #[error(transparent)]
    Poly(#[from] damascus_ring::poly::PolyError),
    #[error(transparent)]
    Module(#[from] damascus_ring::module::ModuleError),
    #[error(transparent)]
    Verify(#[from] crate::verifier::VerifyError),
}

#[derive(Clone, Debug)]
pub struct ConvProver {
    params: ConvParams,
}

impl ConvProver {
    pub fn new(params: ConvParams) -> Result<Self, ProverError> {
        params.validate()?;
        Ok(Self { params })
    }

    pub fn params(&self) -> ConvParams {
        self.params
    }

    pub fn round(
        &self,
        file_id: FileId,
        epoch: u64,
        j: u32,
        pub_state: &mut ConvPublicState,
        wit: &mut ConvWitness,
    ) -> Result<(MbVec, MbPoly), ProverError> {
        self.check_shapes(pub_state, wit)?;

        // Stage 1: vector folding (compress N).
        let n_vec = wit.f.len();
        let half = n_vec / 2;
        let (f_l, f_r) = wit.f.split_at(half);
        let (r_l, r_r) = wit.r.split_at(half);
        let (g_l, g_r) = pub_state.g.split_at(half);
        let (h_l, h_r) = pub_state.h.split_at(half);

        let l_vec = cross_term_vec(&f_l, &r_l, &g_r, &h_r)?;
        let r_vec = cross_term_vec(&f_r, &r_r, &g_l, &h_l)?;

        let x = derive_x(
            &file_id,
            epoch,
            j,
            &pub_state.c,
            &l_vec,
            &r_vec,
            self.params.q,
        )?;
        let x_inv = mod_inv(x, self.params.q)?;

        let c_half = pub_state
            .c
            .add(&l_vec.scalar_mul(x_inv)?)?
            .add(&r_vec.scalar_mul(x)?)?;

        let f_half = fold_vec_poly(f_l, f_r, x)?;
        let r_half = fold_vec_poly(r_l, r_r, x)?;
        let g_half = fold_vec_module(g_l, g_r, x_inv)?;
        let h_half = fold_vec_module(h_l, h_r, x_inv)?;

        pub_state.c = c_half;
        pub_state.g = g_half;
        pub_state.h = h_half;
        wit.f = f_half;
        wit.r = r_half;

        let mb_vec = MbVec {
            file_id,
            epoch,
            round: j,
            l_vec,
            r_vec,
            sig: Vec::new(),
        };

        // Stage 2: odd-even polynomial folding (compress n).
        let c_half_big = pub_state.c.clone();
        let (f_even, f_odd) = odd_even_vec_poly(&wit.f)?;
        let (r_even, r_odd) = odd_even_vec_poly(&wit.r)?;
        let (g_even, g_odd) = odd_even_vec_module_scaled(&pub_state.g)?;
        let (h_even, h_odd) = odd_even_vec_module_scaled(&pub_state.h)?;

        let l_poly = cross_term_vec(&f_even, &r_even, &g_odd, &h_odd)?;
        let r_poly = cross_term_vec(&f_odd, &r_odd, &g_even, &h_even)?;

        let y = derive_y(
            &file_id,
            epoch,
            j,
            &c_half_big,
            &l_poly,
            &r_poly,
            self.params.q,
        )?;
        let y_inv = mod_inv(y, self.params.q)?;

        // Commitment base for the next ring is the even part of C(j+1/2).
        let (c_even, _) = pub_state.c.odd_even_decompose()?;
        pub_state.c = c_even
            .add(&l_poly.scalar_mul(y_inv)?)?
            .add(&r_poly.scalar_mul(y)?)?;

        pub_state.g = fold_poly_module(&g_even, &g_odd, y_inv)?;
        pub_state.h = fold_poly_module(&h_even, &h_odd, y_inv)?;
        wit.f = fold_poly_poly(&f_even, &f_odd, y)?;
        wit.r = fold_poly_poly(&r_even, &r_odd, y)?;

        let mb_poly = MbPoly {
            file_id,
            epoch,
            round: j,
            l_poly,
            r_poly,
            sig: Vec::new(),
        };

        Ok((mb_vec, mb_poly))
    }

    fn check_shapes(
        &self,
        pub_state: &ConvPublicState,
        wit: &ConvWitness,
    ) -> Result<(), ProverError> {
        if pub_state.g.len() != wit.f.len()
            || pub_state.h.len() != wit.r.len()
            || pub_state.g.len() != pub_state.h.len()
        {
            return Err(ProverError::Shape);
        }
        if wit.f.is_empty() || (wit.f.len() % 2 != 0) {
            return Err(ProverError::Shape);
        }
        Ok(())
    }
}

fn cross_term_vec(
    f: &[Poly],
    r: &[Poly],
    g: &[ModuleElem],
    h: &[ModuleElem],
) -> Result<ModuleElem, ProverError> {
    if f.len() != g.len() || r.len() != h.len() || f.len() != r.len() {
        return Err(ProverError::Shape);
    }
    let mut acc = ModuleElem::zero(g[0].q(), g[0].n(), g[0].k())?;
    for i in 0..f.len() {
        acc = acc.add(&g[i].ring_mul(&f[i])?)?;
    }
    for i in 0..r.len() {
        acc = acc.add(&h[i].ring_mul(&r[i])?)?;
    }
    Ok(acc)
}

fn fold_vec_poly(left: &[Poly], right: &[Poly], x: u64) -> Result<Vec<Poly>, ProverError> {
    if left.len() != right.len() {
        return Err(ProverError::Shape);
    }
    let mut out = Vec::with_capacity(left.len());
    for i in 0..left.len() {
        out.push(left[i].add(&right[i].scalar_mul(x)?)?);
    }
    Ok(out)
}

fn fold_vec_module(
    left: &[ModuleElem],
    right: &[ModuleElem],
    x_inv: u64,
) -> Result<Vec<ModuleElem>, ProverError> {
    if left.len() != right.len() {
        return Err(ProverError::Shape);
    }
    let mut out = Vec::with_capacity(left.len());
    for i in 0..left.len() {
        out.push(left[i].add(&right[i].scalar_mul(x_inv)?)?);
    }
    Ok(out)
}

fn odd_even_vec_poly(input: &[Poly]) -> Result<(Vec<Poly>, Vec<Poly>), ProverError> {
    let mut even = Vec::with_capacity(input.len());
    let mut odd = Vec::with_capacity(input.len());
    for p in input {
        let (e, o) = p.odd_even_decompose()?;
        even.push(e);
        odd.push(o);
    }
    Ok((even, odd))
}

fn odd_even_vec_module_scaled(
    input: &[ModuleElem],
) -> Result<(Vec<ModuleElem>, Vec<ModuleElem>), ProverError> {
    let mut even = Vec::with_capacity(input.len());
    let mut odd = Vec::with_capacity(input.len());
    for m in input {
        let (e, o) = m.odd_even_decompose()?;
        even.push(e);
        odd.push(o.mul_by_x()?);
    }
    Ok((even, odd))
}

fn fold_poly_poly(even: &[Poly], odd: &[Poly], y: u64) -> Result<Vec<Poly>, ProverError> {
    if even.len() != odd.len() {
        return Err(ProverError::Shape);
    }
    let mut out = Vec::with_capacity(even.len());
    for i in 0..even.len() {
        out.push(even[i].add(&odd[i].scalar_mul(y)?)?);
    }
    Ok(out)
}

fn fold_poly_module(
    even: &[ModuleElem],
    odd: &[ModuleElem],
    y_inv: u64,
) -> Result<Vec<ModuleElem>, ProverError> {
    if even.len() != odd.len() {
        return Err(ProverError::Shape);
    }
    let mut out = Vec::with_capacity(even.len());
    for i in 0..even.len() {
        out.push(even[i].add(&odd[i].scalar_mul(y_inv)?)?);
    }
    Ok(out)
}
