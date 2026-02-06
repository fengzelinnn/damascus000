use rand::SeedableRng as _;

pub type DeterministicRng = rand_chacha::ChaCha20Rng;

pub fn rng_from_seed(seed: [u8; 32]) -> DeterministicRng {
    DeterministicRng::from_seed(seed)
}
