You are editing the Damascus fold protocol, a Rust codebase at the root of this repository.

## Context

`src/algebra/ntt.rs` function `negacyclic_multiply()` (line 111) performs polynomial
multiplication via CRT: it runs 8 independent NTT passes, one per CRT prime in `CRT_PRIMES`.
Each prime fits in 32 bits, so all arithmetic fits in u64.  The NTT size is `2 * poly_len`,
where `poly_len` is a power of two (typically 64–4096).

For large poly_len (≥ 512), this is the dominant cost and it is embarrassingly parallel
across both the 8 CRT primes and across each prime's N butterfly stages.

The goal of this prompt is to add an optional CUDA kernel that performs all 8 forward NTTs,
the pointwise multiplication, and all 8 inverse NTTs on the GPU in a single launch sequence,
then returns the 8×N results for CRT reconstruction on the CPU.

## Required new file: `cuda/ntt_kernel.cu`

Create a new CUDA source file with:

1. A `ntt_stage_kernel` global kernel that performs one butterfly stage of the Cooley-Tukey
   NTT across a flat array. Signature:
       __global__ void ntt_stage_kernel(uint64_t* data, size_t n, uint64_t wlen, uint64_t modulus)

2. A host function `damascus_cuda_ntt_batch` that accepts:
       const uint64_t* host_a      // 8*n values: prime 0 coeffs [0..n), prime 1 [n..2n), ...
       const uint64_t* host_b      // same layout for rhs
       uint64_t*       host_out    // same layout for output (negacyclic product per prime)
       size_t          n           // NTT size (= 2 * poly_len)
       const uint64_t* primes      // array of 8 CRT prime values
       const uint64_t* stage_roots // 8 * log2(n) forward roots of unity, row-major
       const uint64_t* inv_roots   // 8 * log2(n) inverse roots of unity, row-major
       const uint64_t* inv_sizes   // 8 inverse of n mod each prime
   The function shall:
   a. Allocate device memory for all 8×n arrays
   b. Copy host_a and host_b to device
   c. For each of the 8 primes (can be 8 sequential CUDA streams for concurrency):
      - run log2(n) forward NTT stages on slice a[prime*n .. (prime+1)*n]
      - run log2(n) forward NTT stages on slice b[prime*n .. (prime+1)*n]
      - run pointwise multiply kernel (one thread per element)
      - run log2(n) inverse NTT stages
      - scale by inv_size
   d. Copy result to host_out
   e. Free device memory and return 0 on success, non-zero on any CUDA error.
   Export as: extern "C" __declspec(dllexport) int damascus_cuda_ntt_batch(...)

3. A pointwise multiply kernel:
       __global__ void pointwise_mul_kernel(uint64_t* a, const uint64_t* b, size_t n, uint64_t mod)

## Required changes: `src/utils/gpu.rs`

Add the new FFI binding (gated on `damascus_cuda_available`):

    #[cfg(damascus_cuda_available)]
    unsafe extern "C" {
        fn damascus_cuda_ntt_batch(
            host_a: *const u64, host_b: *const u64, host_out: *mut u64,
            n: usize,
            primes: *const u64, stage_roots: *const u64,
            inv_roots: *const u64, inv_sizes: *const u64,
        ) -> i32;
    }

Add a safe wrapper:

    pub fn try_ntt_batch_gpu(
        a: &[Vec<u64>],   // 8 slices, each of length n
        b: &[Vec<u64>],
        n: usize,
        primes: &[u64; 8],
        stage_roots: &[u64],   // 8 * log2(n) values
        inv_roots:   &[u64],
        inv_sizes:   &[u64; 8],
    ) -> Option<Vec<Vec<u64>>>   // returns 8 slices of length n

## Required changes: `src/algebra/ntt.rs`

In `negacyclic_multiply()`, after CRT prime validation and before the per-prime loop, add a
GPU fast-path:

    use crate::utils::gpu::try_ntt_batch_gpu;
    // collect stage roots and inv_sizes from the 8 cached NttPlans
    // flatten a and b residues into 8×ntt_size arrays
    // call try_ntt_batch_gpu(...)
    // if Some(result), skip the CPU loop and proceed directly to CRT reconstruction
    // if None, fall through to the existing CPU loop unchanged

## `build.rs`

In `build.rs`, add `cuda/ntt_kernel.cu` to the list of files compiled and linked, using the
same nvcc invocation that compiles `fold_kernel.cu`.

## Constraints

- The GPU path is purely additive. If CUDA is not available or the kernel returns an error,
  the existing CPU code path must execute without modification.
- Do not change any CRT reconstruction logic in ntt.rs.
- Do not change any test files.
- The NttPlan cache already exists — extract stage roots from it rather than recomputing.
- Use 8 independent CUDA streams (one per CRT prime) to maximise GPU utilisation.
