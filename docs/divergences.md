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
| V9 | Commitment path still carried blinding witnesses and an auxiliary `h` family even though the PoST model only needed binding | `2c86bbf` |

## V2 Finalization

The second-round correction closes the last V2 gap. The active implementation no longer fixes the
ring degree at `n = 64`; preprocessing now chooses the smallest `d >= 6` such that
`N_0 = n_0 = 2^d` can hold the file coefficients, and the fold path halves both dimensions every
round until `N_d = n_d = 1`.

Relevant commits:

- `555a060`: square witness expansion with `N_0 = n_0 = 2^d`
- `00969de`: explicit dual-dimension fold invariants, depth helper, and sweep tests

除此之外，还需要模拟cPoSt、ePoSt、storna、Maat、Lucas-PoSt在同等环境下的数据，根据论文中的描述来完成模拟，