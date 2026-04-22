use crate::algebra::field::Fp;
use crate::algebra::poly::Poly;
use crate::utils::config::{BYTES_PER_COEFF, MIN_FOLD_DEPTH};
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
    let coeff_count = coeff_count_for_byte_len(mmap.len() as u64);
    let layout = square_witness_layout_for_coeff_count(coeff_count)
        .context("square witness dimension overflow")?;
    let vector_len = layout.dimension;
    let ring_len = layout.dimension;
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
        depth: layout.depth,
        message,
    })
}

pub fn coeff_count_for_byte_len(byte_len: u64) -> usize {
    usize::try_from(byte_len.div_ceil(BYTES_PER_COEFF as u64))
        .unwrap_or(usize::MAX)
        .max(1)
}

pub fn vector_len_for_coeff_count(coeff_count: usize) -> Option<usize> {
    square_witness_layout_for_coeff_count(coeff_count)
        .ok()
        .map(|layout| layout.dimension)
}

pub fn vector_len_for_file_size(byte_len: u64) -> Option<usize> {
    vector_len_for_coeff_count(coeff_count_for_byte_len(byte_len))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SquareWitnessLayout {
    pub coeff_count: usize,
    pub depth: usize,
    pub dimension: usize,
    pub capacity_coeffs: usize,
}

pub fn square_witness_layout_for_byte_len(byte_len: u64) -> Result<SquareWitnessLayout> {
    square_witness_layout_for_coeff_count(coeff_count_for_byte_len(byte_len))
}

pub fn square_witness_layout_for_coeff_count(coeff_count: usize) -> Result<SquareWitnessLayout> {
    let coeff_count = coeff_count.max(1);
    let mut depth = MIN_FOLD_DEPTH;
    loop {
        let dimension = 1usize
            .checked_shl(depth as u32)
            .context("witness dimension overflow")?;
        let capacity_coeffs = dimension
            .checked_mul(dimension)
            .context("witness coefficient capacity overflow")?;
        if capacity_coeffs >= coeff_count {
            return Ok(SquareWitnessLayout {
                coeff_count,
                depth,
                dimension,
                capacity_coeffs,
            });
        }
        depth = depth.checked_add(1).context("witness depth overflow")?;
    }
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

#[cfg(test)]
mod tests {
    use super::{expand_file_to_square_polys, mmap_file, square_witness_layout_for_byte_len};
    use crate::utils::config::{BYTES_PER_COEFF, MIN_FOLD_DEPTH};
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
        assert_eq!(expanded.depth, MIN_FOLD_DEPTH);
        assert_eq!(expanded.message.len(), 1 << expanded.depth);
        assert_eq!(expanded.message[0].len(), 1 << expanded.depth);
        assert_eq!(expanded.ring_len, 1 << expanded.depth);
    }

    #[test]
    fn square_layout_expands_both_dimensions_for_large_inputs() {
        for bytes in [128u64 * 1024 * 1024, 1024u64 * 1024 * 1024] {
            let layout = square_witness_layout_for_byte_len(bytes).expect("layout");
            assert_eq!(layout.dimension, 1 << layout.depth);
            assert!(layout.depth >= MIN_FOLD_DEPTH);
            assert!(layout.capacity_coeffs >= layout.coeff_count);
            assert!(
                layout.capacity_coeffs * BYTES_PER_COEFF >= bytes as usize,
                "layout cannot hold original bytes"
            );
        }
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
