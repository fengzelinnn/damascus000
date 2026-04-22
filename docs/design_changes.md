# Design Changes

## Blinding Removal

This prototype now removes the blinding witness `r` and the auxiliary generator family `h`.
The active commitment relation is:

`com = sum_i f_i * g_i in R_q^k`

The motivating change is specific to the PoST setting. The storage node does not need to hide
the file witness from the verifier, and the protocol does not claim zero knowledge. In this
deployment model, the only property we need from the commitment is binding.

## Security Impact

Binding remains unchanged under the same MSIS assumption. A collision
`sum_i (f_i - f'_i) * g_i = 0` still produces a short non-zero relation in the kernel of the
module generator matrix `[g_1 | ... | g_N]`. The removed `h * r` term was not used by the
binding reduction.

Completeness and round preservation also remain intact. In the original paper proof, the
`h * r` contribution is algebraically parallel to the `g * f` contribution in both the vector
stage and the polynomial stage. Removing `r` and `h` therefore yields the same preservation
identities with fewer summands:

- vector stage: `com_tilde_j = com_j + x_j^-1 * L_vec_j + x_j * R_vec_j`
- polynomial stage: `com_{j+1} = com_tilde_j + y_j^-1 * L_poly_j + y_j * R_poly_j`

Soundness keeps the same structure as well. The binding-based reduction no longer reasons about a
pair `(f_j, r_j)` and instead reasons only about `f_j`, but the contradiction still comes from
two inconsistent openings to the same commitment under MSIS binding.

## Explicit Tradeoff

The removed property is hiding. This implementation now makes that tradeoff explicit:

- kept: correctness, completeness, soundness, binding
- removed: hiding / zero-knowledge style commitment privacy

That trade removes computation, transcript payload, prover state, and generator material without
weakening the PoST property the prototype is trying to model.
