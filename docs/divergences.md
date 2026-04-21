# Historical Divergences

This file records the main mismatches found during the paper-alignment rewrite and the commits that corrected them.

| ID | Historical divergence | Correction commit |
| --- | --- | --- |
| V1 | Goldilocks field used as a stand-in for the paper ring scalar layer | `555911b` |
| V2 | Fixed-size accumulator preprocessing with modulo indexing | `95b8c13` |
| V3 | Commitment path returned module-shaped data backed by scalar-field prototype semantics | `da0c2de` |
| V4 | Generator families were not fixed into the statement / replay path | `da0c2de`, `b5fce2e` |
| V5 | Fold cross-terms were not carried as full module elements and round payloads were too small | `b5fce2e`, `b439a19` |
| V6 | Fiat-Shamir derivation lacked the final paper-style domain-separated replay tests | `b5fce2e`, `b439a19` |
| V7 | Commitment arithmetic did not run over the negacyclic ring implementation | `555911b`, `da0c2de` |
| V8 | Legacy experimental implementations remained under `crates/` after the root crate became the active path | `5af480a` |

## Current Note

The active implementation no longer uses the earlier fixed-dimension accumulator path. File preprocessing expands the file into witness coefficients, pads with zeros, and commits over module-valued ring data.

One remaining prototype-level simplification is that the active code fixes the ring degree at `n = 64` and grows the witness vector with file size, instead of rebuilding both dimensions as `N_0 = n_0 = 2^d` for every input. That simplification is documented here so the repository state stays auditable.
