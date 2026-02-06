use criterion::{Criterion, criterion_group, criterion_main};
use damascus_conv::{ConvParams, ConvProver, ConvPublicState, ConvWitness};
use damascus_ring::{ModuleElem, Poly};
use damascus_types::FileId;
use rand::RngCore as _;
use rand_chacha::rand_core::SeedableRng as _;

fn random_poly(q: u64, n: usize, rng: &mut rand_chacha::ChaCha20Rng) -> Poly {
    let mut coeffs = vec![0u64; n];
    for c in &mut coeffs {
        *c = (rng.next_u32() as u64) % q;
    }
    Poly::from_coeffs(q, coeffs).unwrap()
}

fn bench_conv_round(c: &mut Criterion) {
    let params = ConvParams {
        q: 998_244_353,
        n0: 256,
        n_rounds: 8,
        k: 2,
        seed_generators: [7u8; 32],
    };
    let prover = ConvProver::new(params).unwrap();
    let (g, h) = damascus_conv::verifier::derive_initial_generators(&params).unwrap();

    let mut rng = rand_chacha::ChaCha20Rng::from_seed([9u8; 32]);
    let wit = ConvWitness {
        f: (0..params.n0)
            .map(|_| random_poly(params.q, params.n0, &mut rng))
            .collect(),
        r: (0..params.n0)
            .map(|_| random_poly(params.q, params.n0, &mut rng))
            .collect(),
    };
    let mut pub_state = ConvPublicState {
        g,
        h,
        c: ModuleElem::zero(params.q, params.n0, params.k).unwrap(),
    };
    pub_state.c =
        damascus_conv::verifier::commit(&params, &wit, &pub_state.g, &pub_state.h).unwrap();

    c.bench_function("conv_round_j0", |b| {
        b.iter(|| {
            let mut pub_state = pub_state.clone();
            let mut wit = wit.clone();
            let _ = prover
                .round(FileId([3u8; 32]), 1, 0, &mut pub_state, &mut wit)
                .unwrap();
        })
    });
}

criterion_group!(benches, bench_conv_round);
criterion_main!(benches);
