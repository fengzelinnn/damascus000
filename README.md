# Damascus Fold Rust Prototype

This repository is a Rust prototype that tracks the Damascus Fold paper semantics around:

- `R_q = Z_q[X] / (X^n + 1)` ring arithmetic
- `R_q^k` linear commitments
- full-file witness expansion instead of fixed-size streaming accumulation
- two-stage fold/replay with Fiat-Shamir challenges

The current hard parameters are:

- `q = 5192296858534827628530496329220021`
- `n = 64`
- `k = 8`
- `bytes_per_coeff = 13`

Parameter rationale is documented in [docs/params.md](docs/params.md).

## Repository Layout

```text
.
├── benches/
│   └── protocol_bench.rs
├── docs/
│   ├── divergences.md
│   └── params.md
├── examples/
│   └── full_flow.rs
├── src/
│   ├── algebra/
│   ├── commitment/
│   ├── protocol/
│   └── utils/
└── Cargo.toml
```

## Build And Run

```powershell
cargo build --release
cargo test
cargo run --example full_flow -- .\sample.bin
```

If no input path is provided, the example generates `target/full_flow_input.bin`.

## Benchmark

```powershell
cargo bench --bench protocol_bench
```

By default the benchmark generates files in `target/bench-inputs/` and writes reports to `target/bench-reports/`.

Useful environment variables:

- `DAMASCUS_BENCH_CASE_SIZES=1MiB,8MiB,16MiB`
- `DAMASCUS_BENCH_FILES=<path1>;<path2>`
- `DAMASCUS_BENCH_FILE_LIST=<file-with-paths>`
- `DAMASCUS_NTT=0|1`
- `DAMASCUS_GPU=0|1`

With honest preprocessing enabled, preprocessing throughput is expected to drop sharply compared with the earlier fixed-accumulator prototype. That is the intended behavior.

## Implementation Notes

- `FieldElement` is serialized as fixed-width 16-byte little-endian data.
- `RingElement` uses negacyclic multiplication in `Z_q[X] / (X^n + 1)`.
- `Commitment::commit` returns a module element in `R_q^8`, not a scalar hash or digest.
- File preprocessing expands the whole file into witness coefficients and pads with zeros; it does not use `% capacity` style accumulation.
- `Statement` stores `file_id`, `original_len_bytes`, `d`, `com_0`, `g_0_seed`, and `h_0_seed`.
- Verification replays both fold stages and checks the terminal opening against the folded generators.

## Historical Notes

- Legacy multi-crate experimental code under `crates/` has been retired from the active implementation path.
- Historical divergences and their correction commits are tracked in [docs/divergences.md](docs/divergences.md).
