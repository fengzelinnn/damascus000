You are editing the Damascus fold protocol, a Rust codebase at the root of this repository.

## Problem

The existing CUDA kernel at `cuda/fold_kernel.cu` computes field arithmetic over
Goldilocks prime  MODULUS = 2^64 - 2^32 + 1 (a 64-bit value).

The Rust field element `Fp` uses  MSIS_Q = 5_192_296_858_534_827_628_530_496_329_220_021
which is approximately 2^112 − 75, a 112-bit value stored as u128.

Because u128 does not exist in CUDA, you must represent each field element as two u64 limbs
(lo = bits 0..63, hi = bits 48..111) and implement 128-bit Montgomery or Barrett reduction
in CUDA using __uint128_t (which is available on nvcc ≥ 10 as a compiler extension for
device code).

The FFI interface in `src/utils/gpu.rs` currently passes elements as `*const u64` / `u64`.
It must be changed to pass elements as pairs of u64 (lo, hi) per field element.

## Required changes

### 1. `cuda/fold_kernel.cu` — rewrite field arithmetic for MSIS_Q

Replace the entire file content with the following:

- Define:
    constexpr __uint128_t MSIS_Q =
        ((__uint128_t)0x3FFFFFFFFFFFFFFULL << 64) | 0xFFFFFFFFFFFFFFB5ULL;
  (This encodes 2^112 − 75 using the upper 48 bits and lower 64 bits.)

- Represent each Fp element as two u64 values: `uint64_t lo, hi` where the full value is
  `((__uint128_t)hi << 64) | lo`.

- Implement device functions:
    __device__ __uint128_t fp_load(uint64_t lo, uint64_t hi)
    __device__ void fp_store(uint64_t* lo_out, uint64_t* hi_out, __uint128_t v)
    __device__ __uint128_t fp_add(__uint128_t a, __uint128_t b)   // (a+b) % MSIS_Q
    __device__ __uint128_t fp_mul(__uint128_t a, __uint128_t b)   // (a*b) % MSIS_Q, use __uint128_t arithmetic

- Rewrite `fold_kernel` to accept parallel arrays:
    const uint64_t* left_lo,  const uint64_t* left_hi,
    const uint64_t* right_lo, const uint64_t* right_hi,
    uint64_t*       out_lo,   uint64_t*       out_hi,
    size_t len,
    uint64_t challenge_lo,    uint64_t challenge_hi
  Each element is reconstructed as `__uint128_t`, the fold `out = left + right * challenge`
  is computed in MSIS_Q, and results are split back into lo/hi pairs.

- Rewrite the host function `damascus_cuda_fold_batch` to accept and pass the split lo/hi
  arrays. The exported C symbol stays `damascus_cuda_fold_batch` but with the new signature:
    int damascus_cuda_fold_batch(
        const uint64_t* left_lo,  const uint64_t* left_hi,
        const uint64_t* right_lo, const uint64_t* right_hi,
        uint64_t*       out_lo,   uint64_t*       out_hi,
        size_t len,
        uint64_t challenge_lo,    uint64_t challenge_hi);

### 2. `src/utils/gpu.rs` — update FFI and `try_fold_pairs_gpu`

Replace the unsafe `extern "C"` block:

    #[cfg(damascus_cuda_available)]
    unsafe extern "C" {
        fn damascus_cuda_fold_batch(
            left_lo: *const u64, left_hi: *const u64,
            right_lo: *const u64, right_hi: *const u64,
            out_lo: *mut u64, out_hi: *mut u64,
            len: usize,
            challenge_lo: u64, challenge_hi: u64,
        ) -> i32;
    }

Replace `try_fold_pairs_gpu` with:

    pub fn try_fold_pairs_gpu(left: &[u128], right: &[u128], challenge: u128) -> Option<Vec<u128>> {
        if !cuda_backend_ready() || left.len() != right.len() || left.is_empty() {
            return None;
        }
        #[cfg(damascus_cuda_available)]
        {
            let n = left.len();
            let left_lo:  Vec<u64> = left.iter().map(|v| *v as u64).collect();
            let left_hi:  Vec<u64> = left.iter().map(|v| (*v >> 64) as u64).collect();
            let right_lo: Vec<u64> = right.iter().map(|v| *v as u64).collect();
            let right_hi: Vec<u64> = right.iter().map(|v| (*v >> 64) as u64).collect();
            let mut out_lo = vec![0u64; n];
            let mut out_hi = vec![0u64; n];
            let rc = unsafe {
                damascus_cuda_fold_batch(
                    left_lo.as_ptr(), left_hi.as_ptr(),
                    right_lo.as_ptr(), right_hi.as_ptr(),
                    out_lo.as_mut_ptr(), out_hi.as_mut_ptr(),
                    n,
                    challenge as u64, (challenge >> 64) as u64,
                )
            };
            if rc == 0 {
                let out: Vec<u128> = out_lo.iter().zip(out_hi.iter())
                    .map(|(&lo, &hi)| lo as u128 | ((hi as u128) << 64))
                    .collect();
                Some(out)
            } else {
                None
            }
        }
        #[cfg(not(damascus_cuda_available))]
        { let _ = (left, right, challenge); None }
    }

### 3. `src/protocol/prover.rs` — call GPU fold path in `fold_vec_poly`

After Prompt 2 is applied, `DamascusProver` has `gpu_enabled: bool` and `gpu_min_elements: usize`.

In `fold_vec_poly`, add a GPU fast-path before the CPU path:

    fn fold_vec_poly(left: &[Poly], right: &[Poly], challenge: Fp,
                     parallel: bool, gpu_enabled: bool, gpu_min_elements: usize)
        -> Result<Vec<Poly>>
    {
        ensure!(left.len() == right.len(), "vector fold length mismatch");
        let total_coeffs: usize = left.iter().map(|p| p.len()).sum();

        if gpu_enabled && total_coeffs >= gpu_min_elements {
            // Flatten to u128, run GPU fold, reshape back
            use crate::utils::gpu::try_fold_pairs_gpu;
            let left_flat: Vec<u128>  = left.iter().flat_map(|p| p.coeffs.iter().map(|c| c.as_u128())).collect();
            let right_flat: Vec<u128> = right.iter().flat_map(|p| p.coeffs.iter().map(|c| c.as_u128())).collect();
            if let Some(out_flat) = try_fold_pairs_gpu(&left_flat, &right_flat, challenge.as_u128()) {
                let poly_len = left[0].len();
                let polys: Vec<Poly> = out_flat.chunks_exact(poly_len)
                    .map(|chunk| Poly::new(chunk.iter().map(|&v| Fp::from_u128(v)).collect()))
                    .collect();
                return Ok(polys);
            }
            // fall through to CPU path if GPU returns None
        }

        // CPU path (parallel or sequential, from Prompt 2)
        ...
    }

Update the call-site in `fold_round` to pass `self.gpu_enabled` and `self.gpu_min_elements`.

## Constraints

- Do not change `src/algebra/field.rs`.
- `Fp::as_u128()` and `Fp::from_u128()` already exist — use them as-is.
- Do not touch any test files.
- The GPU path is a best-effort optimisation; if `try_fold_pairs_gpu` returns `None` (CUDA
  not compiled, no device, or error), execution must silently fall back to the CPU path.
- The `__uint128_t` type is supported by nvcc on CUDA ≥ 10 for device code. Do not use PTX
  inline assembly unless `__uint128_t` arithmetic proves insufficient.
