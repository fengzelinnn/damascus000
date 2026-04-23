You are editing the Damascus fold protocol, a Rust codebase at the root of this repository.

## Context

`src/algebra/poly.rs` line 134 has:

    pub fn mul(&self, rhs: &Self, _ntt_enabled: bool) -> Result<Self> {

The leading underscore on `_ntt_enabled` means the parameter is intentionally unused.
The body always calls `ntt::negacyclic_multiply()` regardless of the flag value.
Benchmarks test NTT=true and NTT=false configurations but they produce identical results
because the flag is never honoured.

`src/algebra/ntt.rs` already has a correct O(n²) fallback at line 167:

    fn naive_negacyclic(lhs: &[Fp], rhs: &[Fp]) -> Vec<Fp>

This function is currently private and used only as an internal fallback inside
`negacyclic_multiply()` when the CRT primes don't support the requested NTT size.

## Task

Make two minimal changes:

### Change 1 — `src/algebra/ntt.rs`

Expose `naive_negacyclic` as a public function so callers outside the module can reach it.
Change:
    fn naive_negacyclic(lhs: &[Fp], rhs: &[Fp]) -> Vec<Fp>
To:
    pub fn naive_negacyclic(lhs: &[Fp], rhs: &[Fp]) -> Vec<Fp>

No other changes to ntt.rs.

### Change 2 — `src/algebra/poly.rs`

In `Poly::mul()` (line 134), rename the parameter from `_ntt_enabled` to `ntt_enabled`
and branch on its value:

    pub fn mul(&self, rhs: &Self, ntt_enabled: bool) -> Result<Self> {
        ensure!(self.len() == rhs.len(), "ring length mismatch");
        let coeffs = if ntt_enabled {
            ntt::negacyclic_multiply(&self.coeffs, &rhs.coeffs)?
        } else {
            ntt::naive_negacyclic(&self.coeffs, &rhs.coeffs)
        };
        Ok(Self::new(coeffs))
    }

No other changes to poly.rs.

## Constraints

- Do not change any test code.
- Do not change the public signature of `negacyclic_multiply`.
- The existing tests in `poly.rs` already call `mul(&b, true)` — they must continue to compile
  and pass without modification.
- Do not add comments unless they explain a non-obvious invariant.
