use crate::algebra::field::Fp;
use crate::algebra::poly::Poly;
use crate::utils::config::{BYTES_PER_COEFF, POLY_DEGREE};
use anyhow::{ensure, Context, Result};
use memmap2::{Mmap, MmapOptions};
use rand::RngCore;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use std::fs::File;
use std::mem::size_of;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct ExpandedFile {
    pub file_id: [u8; 32],
    pub original_len_bytes: u64,
    pub coeff_count: usize,
    pub vector_len: usize,
    pub ring_len: usize,
    pub depth: usize,
    pub message: Vec<Poly>,
}

pub fn mmap_file(path: &Path) -> Result<Mmap> {
    let file =
        File::open(path).with_context(|| format!("failed to open file: {}", path.display()))?;
    let mmap = unsafe { MmapOptions::new().map(&file) }
        .with_context(|| format!("failed to mmap file: {}", path.display()))?;
    Ok(mmap)
}

pub fn expand_file_to_square_polys(
    mmap: &Mmap,
    max_preprocess_bytes: usize,
) -> Result<ExpandedFile> {
    let file_id = *blake3::hash(mmap).as_bytes();
    let coeff_count = mmap.len().div_ceil(BYTES_PER_COEFF).max(1);
    let ring_len = POLY_DEGREE;
    let vector_len = coeff_count
        .div_ceil(ring_len)
        .max(1)
        .checked_next_power_of_two()
        .context("vector witness dimension overflow")?;
    let required_coeff_capacity = vector_len
        .checked_mul(ring_len)
        .context("witness capacity overflow")?;
    let estimated_bytes = required_coeff_capacity
        .checked_mul(size_of::<Fp>())
        .and_then(|bytes| bytes.checked_mul(2))
        .context("preprocessing memory estimate overflow")?;
    ensure!(
        estimated_bytes <= max_preprocess_bytes,
        "full witness expansion requires {estimated_bytes} bytes, above max_preprocess_bytes={max_preprocess_bytes}; split the file into smaller statements"
    );

    let mut coeffs = vec![Fp::zero(); required_coeff_capacity];
    for (idx, chunk) in mmap.chunks(BYTES_PER_COEFF).enumerate() {
        coeffs[idx] = Fp::from_le_chunk(chunk);
    }

    let mut message = Vec::with_capacity(vector_len);
    for row in coeffs.chunks_exact(ring_len) {
        message.push(Poly::new(row.to_vec()));
    }

    Ok(ExpandedFile {
        file_id,
        original_len_bytes: mmap.len() as u64,
        coeff_count,
        vector_len,
        ring_len,
        depth: floor_log2(vector_len.min(ring_len)),
        message,
    })
}

pub fn sample_blinding_polys(
    count: usize,
    poly_len: usize,
    seed: [u8; 32],
    file_id: [u8; 32],
) -> Vec<Poly> {
    let mut nonce_seed = seed;
    for (idx, byte) in file_id.iter().enumerate() {
        nonce_seed[idx % nonce_seed.len()] ^= *byte;
    }

    let mut rng = ChaCha20Rng::from_seed(nonce_seed);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let mut coeffs = Vec::with_capacity(poly_len);
        for _ in 0..poly_len {
            let hi = (rng.next_u64() as u128) << 64;
            let lo = rng.next_u64() as u128;
            coeffs.push(Fp::from(hi | lo));
        }
        out.push(Poly::new(coeffs));
    }
    out
}

fn floor_log2(x: usize) -> usize {
    if x <= 1 {
        0
    } else {
        (usize::BITS as usize - 1) - (x.leading_zeros() as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::{expand_file_to_square_polys, mmap_file};
    use crate::utils::config::{BYTES_PER_COEFF, POLY_DEGREE};
    use std::fs;

    #[test]
    fn expand_preserves_injective_chunks() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let path = temp_dir.path().join("input.bin");
        let payload = vec![0xA5u8; BYTES_PER_COEFF * 3 + 5];
        fs::write(&path, &payload).expect("write");
        let mmap = mmap_file(&path).expect("mmap");
        let expanded = expand_file_to_square_polys(&mmap, usize::MAX).expect("expand");
        assert_eq!(expanded.original_len_bytes, payload.len() as u64);
        assert_eq!(expanded.message.len(), 1);
        assert_eq!(expanded.message[0].len(), POLY_DEGREE);
        assert_eq!(expanded.ring_len, POLY_DEGREE);
    }

    #[test]
    fn expand_rejects_memory_overcommit() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let path = temp_dir.path().join("big.bin");
        fs::write(&path, vec![0x55u8; 4096]).expect("write");
        let mmap = mmap_file(&path).expect("mmap");
        let err = expand_file_to_square_polys(&mmap, 64).expect_err("must fail");
        assert!(err.to_string().contains("split the file"));
    }
}
