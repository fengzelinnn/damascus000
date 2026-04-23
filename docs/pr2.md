You are editing the Damascus fold protocol, a Rust codebase at the root of this repository.

## Context

`src/utils/config.rs` defines:

    pub struct RuntimeConfig {
        pub max_preprocess_bytes: usize,
        pub ntt_enabled: bool,
        pub parallel_enabled: bool,
        pub gpu_enabled: bool,
        pub gpu_min_elements: usize,
    }

`src/protocol/prover.rs` function `initialize_with_config()` receives a `RuntimeConfig`
parameter but never stores it in the `DamascusProver` struct. After initialization the flags
are completely lost, so folding always runs on a single CPU thread and never consults
the caller's preferences.

`rayon` is already a dependency in `Cargo.toml` but is never imported anywhere in `src/`.

## Task

### Change 1 — add fields to `DamascusProver`

In `src/protocol/prover.rs`, add four fields to the `DamascusProver` struct (after `round: usize`):

    ntt_enabled: bool,
    parallel_enabled: bool,
    gpu_enabled: bool,
    gpu_min_elements: usize,

### Change 2 — populate the new fields in `initialize_with_config`

At the end of the `Ok(Self { ... })` block, assign:

    ntt_enabled: config.ntt_enabled,
    parallel_enabled: config.parallel_enabled,
    gpu_enabled: config.gpu_enabled,
    gpu_min_elements: config.gpu_min_elements,

### Change 3 — pass `ntt_enabled` into every `Poly::mul` call

`Poly::mul` already accepts an `ntt_enabled: bool` parameter (after Prompt 1 is applied).
Search `prover.rs` and `commitment/sis.rs` for every call to `.mul(` and change the
hard-coded `true` argument to `self.ntt_enabled` (for calls inside DamascusProver methods)
or thread the flag through helper functions as needed.

### Change 4 — use rayon in `fold_vec_poly` when `parallel_enabled`

Convert the module-level private function `fold_vec_poly` to accept the flag:

    fn fold_vec_poly(left: &[Poly], right: &[Poly], challenge: Fp, parallel: bool)
        -> Result<Vec<Poly>>

When `parallel` is true, replace the sequential iterator with:

    use rayon::prelude::*;
    left.par_iter()
        .zip(right.par_iter())
        .map(|(l, r)| l.add(&r.scale(challenge)))
        .collect()

When `parallel` is false, keep the existing sequential `.iter()` path.

Apply the same treatment to `fold_poly_poly` (same pattern, same signature change).

Update the two call-sites in `fold_round` to pass `self.parallel_enabled`.

Also update the helper signatures for `fold_vec_module` and `fold_poly_module` the same way
(they iterate `ModuleCommitment` slices; use `rayon::prelude::*` on the par path).

## Constraints

- Do not change any test code.
- Do not change `RuntimeConfig` or `config.rs`.
- The public API of `DamascusProver` (method signatures visible from `lib.rs`) must not change.
- Do not add comments beyond what is strictly necessary to explain a non-obvious invariant.
- All existing tests must continue to compile and pass.
