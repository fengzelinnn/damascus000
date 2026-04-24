You are editing the Damascus fold protocol Rust codebase.

## Problem

The GPU fold path in `src/protocol/prover.rs::fold_vec_poly` (lines 309-328) is
never triggered in practice because the threshold `gpu_min_elements` is set orders
of magnitude higher than the actual data sizes used in benchmarks.

Specific values:
- `src/utils/config.rs` line 84:  `gpu_min_elements: 2_097_152`  (2 million)
- `benches/protocol_bench.rs` line 108: `.unwrap_or(16_777_216)`  (16 million)

Actual bench input sizes and resulting coefficient counts:
- A 1 000-byte file  → Round 0 has ~9 000 field elements across all polys
- A 1 MB file       → Round 0 has ~640 000 field elements

With a 16M threshold the GPU fold path is dead code for every bench scenario.
The `gpu_enabled=ON` and `gpu_enabled=OFF` fold timings are therefore always
identical (when NTT is also disabled), making the four-scenario matrix degenerate.

## Required changes

### 1. `src/utils/config.rs`

Change the default value of `gpu_min_elements` from `2_097_152` to `4_096`:

    gpu_min_elements: 4_096,

Rationale: at 4 096 field elements (each 16 bytes) the PCIe round-trip cost
(~65 KB transfer) is roughly balanced by GPU parallelism for this 128-bit field.
Below this threshold CPU SIMD wins; above it the GPU vector unit dominates.

### 2. `benches/protocol_bench.rs`

Change the fallback default in `collect_benchmark_data` from `16_777_216` to `4_096`:

    .unwrap_or(4_096);

The `DAMASCUS_GPU_MIN_ELEMS` environment variable still allows overriding the threshold
at run time; only the hard-coded fallback changes.

## Constraints

- Do not change any other code.
- Do not add comments.
- All existing tests must continue to pass without modification.
