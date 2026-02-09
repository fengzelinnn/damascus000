use crate::algebra::field::{Fp, GOLDILOCKS_MODULUS};
use anyhow::{anyhow, ensure, Result};
use rayon::prelude::*;
use std::sync::OnceLock;

const TWO_ADICITY: usize = 32;
const GENERATOR_FACTORS: [u64; 6] = [2, 3, 5, 17, 257, 65_537];

#[derive(Clone, Debug)]
pub struct NttPlan {
    size: usize,
    inv_size: Fp,
    stage_roots: Vec<Fp>,
    inv_stage_roots: Vec<Fp>,
}

impl NttPlan {
    pub fn new(size: usize) -> Result<Self> {
        ensure!(size.is_power_of_two(), "NTT size must be a power of two");
        ensure!(
            size <= (1usize << TWO_ADICITY),
            "NTT size exceeds two-adicity bound"
        );

        let root = primitive_root_of_unity(size)?;
        let mut stage_roots = Vec::new();
        let mut inv_stage_roots = Vec::new();
        let mut len = 2usize;
        while len <= size {
            stage_roots.push(root.pow((size / len) as u64));
            inv_stage_roots.push(root.inv().pow((size / len) as u64));
            len <<= 1;
        }

        Ok(Self {
            size,
            inv_size: Fp::from(size as u64).inv(),
            stage_roots,
            inv_stage_roots,
        })
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

pub fn convolution(a: &[Fp], b: &[Fp]) -> Result<Vec<Fp>> {
    ensure!(
        !a.is_empty() && !b.is_empty(),
        "empty polynomial in convolution"
    );
    let out_len = a.len() + b.len() - 1;
    let n = out_len.next_power_of_two();

    let mut fa = vec![Fp::zero(); n];
    let mut fb = vec![Fp::zero(); n];
    fa[..a.len()].copy_from_slice(a);
    fb[..b.len()].copy_from_slice(b);

    let plan = NttPlan::new(n)?;
    forward_ntt(&mut fa, &plan)?;
    forward_ntt(&mut fb, &plan)?;

    for (lhs, rhs) in fa.iter_mut().zip(fb) {
        *lhs *= rhs;
    }

    inverse_ntt(&mut fa, &plan)?;
    fa.truncate(out_len);
    Ok(fa)
}

pub fn forward_ntt(values: &mut [Fp], plan: &NttPlan) -> Result<()> {
    ensure!(
        values.len() == plan.size,
        "NTT input length must match plan"
    );
    bit_reverse_permute(values);

    let mut len = 2usize;
    for &wlen in &plan.stage_roots {
        if values.len() / len >= rayon::current_num_threads().saturating_mul(2) {
            values.par_chunks_exact_mut(len).for_each(|chunk| {
                butterfly_chunk(chunk, wlen);
            });
        } else {
            for chunk in values.chunks_exact_mut(len) {
                butterfly_chunk(chunk, wlen);
            }
        }
        len <<= 1;
    }

    Ok(())
}

pub fn inverse_ntt(values: &mut [Fp], plan: &NttPlan) -> Result<()> {
    ensure!(
        values.len() == plan.size,
        "NTT input length must match plan"
    );
    bit_reverse_permute(values);

    let mut len = 2usize;
    for &wlen in &plan.inv_stage_roots {
        if values.len() / len >= rayon::current_num_threads().saturating_mul(2) {
            values.par_chunks_exact_mut(len).for_each(|chunk| {
                butterfly_chunk(chunk, wlen);
            });
        } else {
            for chunk in values.chunks_exact_mut(len) {
                butterfly_chunk(chunk, wlen);
            }
        }
        len <<= 1;
    }

    for x in values.iter_mut() {
        *x *= plan.inv_size;
    }

    Ok(())
}

#[inline]
fn butterfly_chunk(chunk: &mut [Fp], wlen: Fp) {
    let len = chunk.len();
    let (left, right) = chunk.split_at_mut(len / 2);
    let mut w = Fp::one();
    for (u, v) in left.iter_mut().zip(right.iter_mut()) {
        let t = *v * w;
        let x = *u;
        *u = x + t;
        *v = x - t;
        w *= wlen;
    }
}

fn primitive_root_of_unity(size: usize) -> Result<Fp> {
    ensure!(size.is_power_of_two(), "size must be a power of two");
    let generator = multiplicative_generator();
    let exp = (GOLDILOCKS_MODULUS - 1) / (size as u64);
    let root = generator.pow(exp);
    if root.pow(size as u64) != Fp::one() {
        return Err(anyhow!("failed to derive size-th root of unity"));
    }
    if size > 1 && root.pow((size / 2) as u64) == Fp::one() {
        return Err(anyhow!("derived root is not primitive"));
    }
    Ok(root)
}

fn multiplicative_generator() -> Fp {
    static GEN: OnceLock<Fp> = OnceLock::new();
    *GEN.get_or_init(|| {
        for cand in 2u64..10_000 {
            let g = Fp::from(cand);
            let mut ok = true;
            for p in GENERATOR_FACTORS {
                if g.pow((GOLDILOCKS_MODULUS - 1) / p) == Fp::one() {
                    ok = false;
                    break;
                }
            }
            if ok {
                return g;
            }
        }
        panic!("no multiplicative generator found for Goldilocks field");
    })
}

fn bit_reverse_permute(values: &mut [Fp]) {
    let n = values.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            values.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{convolution, forward_ntt, inverse_ntt, NttPlan};
    use crate::algebra::field::Fp;

    #[test]
    fn ntt_round_trip() {
        let mut values: Vec<Fp> = (0u64..8).map(Fp::from).collect();
        let original = values.clone();
        let plan = NttPlan::new(values.len()).expect("plan");
        forward_ntt(&mut values, &plan).expect("forward");
        inverse_ntt(&mut values, &plan).expect("inverse");
        assert_eq!(values, original);
    }

    #[test]
    fn convolution_matches_naive_small_case() {
        let a = vec![Fp::from(1), Fp::from(2), Fp::from(3)];
        let b = vec![Fp::from(4), Fp::from(5)];
        let c = convolution(&a, &b).expect("conv");
        let expected = vec![Fp::from(4), Fp::from(13), Fp::from(22), Fp::from(15)];
        assert_eq!(c, expected);
    }
}
