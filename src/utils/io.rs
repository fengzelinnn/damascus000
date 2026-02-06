use crate::algebra::field::Fp;
use crate::algebra::poly::Poly;
use anyhow::{Context, Result};
use memmap2::{Mmap, MmapOptions};
use rand::RngCore;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use std::fs::File;
use std::path::Path;

pub fn mmap_file(path: &Path) -> Result<Mmap> {
    let file =
        File::open(path).with_context(|| format!("failed to open file: {}", path.display()))?;
    // SAFETY: read-only mapping; file lives for the duration of map creation.
    let mmap = unsafe { MmapOptions::new().map(&file) }
        .with_context(|| format!("failed to mmap file: {}", path.display()))?;
    Ok(mmap)
}

pub fn mmap_to_fixed_polys(mmap: &Mmap, vector_len: usize, poly_len: usize) -> Vec<Poly> {
    let mut polys = vec![Poly::zero(poly_len); vector_len];
    if mmap.is_empty() {
        return polys;
    }

    let capacity = vector_len * poly_len;
    for (word_idx, chunk) in mmap.chunks(8).enumerate() {
        let mut limb = [0u8; 8];
        limb[..chunk.len()].copy_from_slice(chunk);
        let value = Fp::from(u64::from_le_bytes(limb));
        let pos = word_idx % capacity;
        let poly_idx = pos / poly_len;
        let coeff_idx = pos % poly_len;
        polys[poly_idx].coeffs[coeff_idx] += value;
    }
    polys
}

pub fn sample_blinding_polys(
    count: usize,
    poly_len: usize,
    seed: [u8; 32],
    nonce: u64,
) -> Vec<Poly> {
    let mut nonce_seed = seed;
    for (i, b) in nonce.to_le_bytes().iter().enumerate() {
        nonce_seed[i] ^= *b;
    }

    let mut rng = ChaCha20Rng::from_seed(nonce_seed);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let mut coeffs = Vec::with_capacity(poly_len);
        for _ in 0..poly_len {
            coeffs.push(Fp::from(rng.next_u64()));
        }
        out.push(Poly::new(coeffs));
    }
    out
}
