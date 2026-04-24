You are editing the Damascus fold protocol Rust codebase.

## Problem

`src/algebra/ntt.rs` function `negacyclic_multiply()` at line 157 calls
`try_ntt_batch_gpu()` unconditionally, with no way to disable the GPU path:

    let residues_per_prime = if let Some(gpu_results) = try_ntt_batch_gpu(
        &batched_a, &batched_b, ntt_size, &CRT_PRIMES,
        &stage_roots, &inv_roots, &inv_sizes,
    ) { ... } else { ... };

The `gpu_enabled` flag in `RuntimeConfig` is stored in `DamascusProver` and controls
`fold_vec_poly`, but it is NEVER threaded into `negacyclic_multiply`.  As a result,
when a benchmark scenario sets `gpu_enabled=false`, polynomial multiplication inside
cross-term commitments still runs on the GPU, making GPU=ON and GPU=OFF scenarios
produce nearly identical fold timings.

The call chain that must be updated is:

    prover.rs::fold_round
      → cross_term_vec(committer, witness, g, ntt_enabled)           [prover.rs]
        → commit_with_generators_ntt(witness, g, ntt_enabled)         [sis.rs]
          → ring_mul_with_ntt(scalar, ntt_enabled)                    [module.rs]
            → Poly::mul(rhs, ntt_enabled)                             [poly.rs]
              → negacyclic_multiply(lhs, rhs)                         [ntt.rs]
                → try_ntt_batch_gpu(...)                              [gpu.rs]

## Required changes

Apply each change in dependency order (innermost function first).

### 1. `src/algebra/ntt.rs` — add `gpu_enabled` to `negacyclic_multiply`

Change the signature:

    pub fn negacyclic_multiply(lhs: &[Fp], rhs: &[Fp]) -> Result<Vec<Fp>>

to:

    pub fn negacyclic_multiply(lhs: &[Fp], rhs: &[Fp], gpu_enabled: bool) -> Result<Vec<Fp>>

Inside the function body, guard the GPU call with the new parameter:

    let residues_per_prime = if gpu_enabled {
        if let Some(gpu_results) = try_ntt_batch_gpu(
            &batched_a, &batched_b, ntt_size, &CRT_PRIMES,
            &stage_roots, &inv_roots, &inv_sizes,
        ) {
            gpu_results
                .into_iter()
                .zip(CRT_PRIMES.iter())
                .map(|(values, &modulus)| {
                    let mut reduced = vec![0u64; n];
                    for idx in 0..n {
                        reduced[idx] = mod_sub(values[idx], values[idx + n], modulus);
                    }
                    reduced
                })
                .collect()
        } else {
            cpu_ntt_residues(batched_a, batched_b, plans)?
        }
    } else {
        cpu_ntt_residues(batched_a, batched_b, plans)?
    };

Extract the existing CPU NTT loop (the `else` branch) into a private helper
`fn cpu_ntt_residues(batched_a, batched_b, plans) -> Result<Vec<Vec<u64>>>` to avoid
code duplication.

The tests in `ntt.rs` that call `negacyclic_multiply` directly must be updated to pass
`true` as the third argument (they test correctness, not GPU isolation).

### 2. `src/algebra/poly.rs` — add `gpu_enabled` to `Poly::mul`

Change:

    pub fn mul(&self, rhs: &Self, ntt_enabled: bool) -> Result<Self>

to:

    pub fn mul(&self, rhs: &Self, ntt_enabled: bool, gpu_enabled: bool) -> Result<Self>

Inside, pass `gpu_enabled` to `negacyclic_multiply`:

    let coeffs = if ntt_enabled {
        ntt::negacyclic_multiply(&self.coeffs, &rhs.coeffs, gpu_enabled)?
    } else {
        ntt::naive_negacyclic(&self.coeffs, &rhs.coeffs)
    };

Update every call to `Poly::mul` inside `poly.rs` tests: add `true` as the fourth
argument (existing tests: `mul(&b, true)` → `mul(&b, true, true)`).

### 3. `src/algebra/module.rs` — add `gpu_enabled` to `ring_mul_with_ntt`

Change:

    pub fn ring_mul(&self, scalar: &Poly) -> Result<Self> {
        self.ring_mul_with_ntt(scalar, true)
    }

    pub fn ring_mul_with_ntt(&self, scalar: &Poly, ntt_enabled: bool) -> Result<Self>

to:

    pub fn ring_mul(&self, scalar: &Poly) -> Result<Self> {
        self.ring_mul_with_ntt(scalar, true, true)
    }

    pub fn ring_mul_with_ntt(&self, scalar: &Poly, ntt_enabled: bool, gpu_enabled: bool)
        -> Result<Self>

Inside `ring_mul_with_ntt`, pass `gpu_enabled` to `Poly::mul`:

    array::from_fn(|idx| self.coords[idx].mul(scalar, ntt_enabled, gpu_enabled).expect("shape"))

### 4. `src/commitment/sis.rs` — add `gpu_enabled` to `commit_with_generators_ntt`

Change:

    pub fn commit_with_generators_ntt(
        &self,
        witness: &[Poly],
        g: &[ModuleElement<K>],
        ntt_enabled: bool,
    ) -> Result<ModuleElement<K>>

to:

    pub fn commit_with_generators_ntt(
        &self,
        witness: &[Poly],
        g: &[ModuleElement<K>],
        ntt_enabled: bool,
        gpu_enabled: bool,
    ) -> Result<ModuleElement<K>>

Inside the loop, pass `gpu_enabled`:

    acc = acc.add(&g[idx].ring_mul_with_ntt(&witness[idx], ntt_enabled, gpu_enabled)?)?;

Update the two callers within `sis.rs` that call `commit_with_generators_ntt`:
- `commit_with_generators(witness, g)` hardcodes `true, true` (it is a public convenience
  API without GPU context, so defaulting to both enabled is correct)
- `commit_with_ntt(witness, ntt_enabled)` should pass `ntt_enabled, true` (same reasoning:
  when called without an explicit GPU flag, allow GPU)

### 5. `src/protocol/prover.rs` — add `gpu_enabled` to `cross_term_vec` and all call sites

Change `cross_term_vec`:

    fn cross_term_vec(
        committer: &ModuleSisCommitter,
        witness: &[Poly],
        g: &[ModuleCommitment],
        ntt_enabled: bool,
        gpu_enabled: bool,
    ) -> Result<ModuleCommitment> {
        committer.commit_with_generators_ntt(witness, g, ntt_enabled, gpu_enabled)
    }

Update every call to `cross_term_vec` in `fold_round` to pass `self.gpu_enabled` as
the fifth argument:

    cross_term_vec(&self.committer, &msg_left, &g_right, self.ntt_enabled, self.gpu_enabled)
    cross_term_vec(&self.committer, &msg_right, &g_left, self.ntt_enabled, self.gpu_enabled)
    cross_term_vec(&self.committer, &msg_even, &g_odd_scaled, self.ntt_enabled, self.gpu_enabled)
    cross_term_vec(&self.committer, &msg_odd, &g_even, self.ntt_enabled, self.gpu_enabled)

Also update the two `commit_with_generators_ntt` calls used for invariant recomputes
in `fold_round` (lines ~185 and ~253):

    self.committer.commit_with_generators_ntt(
        &folded_message, &folded_g, self.ntt_enabled, self.gpu_enabled)
    self.committer.commit_with_generators_ntt(
        &next_message, &next_g, self.ntt_enabled, self.gpu_enabled)

## Constraints

- Do NOT add comments to the code.
- Do NOT change any function that is not in the call chain above.
- The `ring_mul` convenience method (without ntt/gpu params) keeps its current behaviour
  (`ntt=true, gpu=true`), since it has no access to RuntimeConfig.
- All 31 existing unit tests must continue to pass. Update test call sites where
  necessary by adding `true` as the extra argument.
- Do not change `try_ntt_batch_gpu` or `try_fold_pairs_gpu` signatures in gpu.rs.
