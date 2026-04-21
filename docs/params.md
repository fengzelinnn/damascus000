# Parameter Notes

The active implementation fixes the following MSIS-style parameters in `src/utils/config.rs`:

- `q = 5192296858534827628530496329220021`
- `n = 64`
- `k = 8`
- `CRT_PRIMES = [3892314113, 2281701377, 2013265921, 2885681153, 2483027969, 1811939329, 469762049, 4194304001]`
- `BYTES_PER_COEFF = 13`

## Rationale

- The modulus is a large odd prime close to `2^112`, which gives a much wider coefficient space than the earlier prototype field.
- `n = 64` keeps negacyclic ring operations and CRT-backed multiplication manageable in the current prototype while still operating over `R_q = Z_q[X] / (X^n + 1)`.
- `k = 8` matches the module rank used by the active commitment layer and benchmark path.
- The CRT prime set consists of NTT-friendly 31-32 bit primes used to reconstruct negacyclic products back into the main modulus.
- `BYTES_PER_COEFF = 13` preserves an injective byte-to-coefficient packing because `13 * 8 = 104 < log2(q)`, so each chunk fits into one field element without information loss.

## Security Intent

This repository uses the large-modulus, higher-rank setting as its strict profile and documents it as a Greyhound-style configuration target with intended `lambda >= 128`.

This is a prototype, not a completed security proof artifact. The parameter file records the constants the code actually uses so audits and benchmarks are anchored to the same values.
