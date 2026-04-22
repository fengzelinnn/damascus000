use crate::algebra::field::Fp;
use crate::utils::config::{CRT_PRIMES, MSIS_Q};
use anyhow::{anyhow, ensure, Result};
use num_bigint::BigUint;
use num_traits::{ToPrimitive, Zero};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Debug)]
pub struct NttPlan {
    size: usize,
    modulus: u64,
    inv_size: u64,
    stage_roots: Vec<u64>,
    inv_stage_roots: Vec<u64>,
    bitrev: Vec<usize>,
}

impl NttPlan {
    pub fn new(size: usize, modulus: u64) -> Result<Self> {
        ensure!(size.is_power_of_two(), "NTT size must be a power of two");
        ensure!(
            (modulus - 1) % (size as u64) == 0,
            "size must divide modulus - 1"
        );

        let root = primitive_root_of_unity(size, modulus)?;
        let inv_root = mod_pow(root, (modulus - 2) as u128, modulus);
        let mut stage_roots = Vec::new();
        let mut inv_stage_roots = Vec::new();
        let mut len = 2usize;
        while len <= size {
            stage_roots.push(mod_pow(root, (size / len) as u128, modulus));
            inv_stage_roots.push(mod_pow(inv_root, (size / len) as u128, modulus));
            len <<= 1;
        }

        Ok(Self {
            size,
            modulus,
            inv_size: mod_pow(size as u64, (modulus - 2) as u128, modulus),
            stage_roots,
            inv_stage_roots,
            bitrev: build_bitrev_table(size),
        })
    }

    pub fn modulus(&self) -> u64 {
        self.modulus
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

pub fn forward_ntt(values: &mut [u64], plan: &NttPlan) -> Result<()> {
    ensure!(
        values.len() == plan.size,
        "NTT input length must match plan"
    );
    bit_reverse_permute(values, &plan.bitrev);

    let mut len = 2usize;
    for &wlen in &plan.stage_roots {
        for chunk in values.chunks_exact_mut(len) {
            butterfly_chunk(chunk, wlen, plan.modulus);
        }
        len <<= 1;
    }

    Ok(())
}

pub fn inverse_ntt(values: &mut [u64], plan: &NttPlan) -> Result<()> {
    ensure!(
        values.len() == plan.size,
        "NTT input length must match plan"
    );
    bit_reverse_permute(values, &plan.bitrev);

    let mut len = 2usize;
    for &wlen in &plan.inv_stage_roots {
        for chunk in values.chunks_exact_mut(len) {
            butterfly_chunk(chunk, wlen, plan.modulus);
        }
        len <<= 1;
    }

    for value in values.iter_mut() {
        *value = mod_mul(*value, plan.inv_size, plan.modulus);
    }

    Ok(())
}

pub fn convolution(a: &[Fp], b: &[Fp]) -> Result<Vec<Fp>> {
    ensure!(
        !a.is_empty() && !b.is_empty(),
        "empty polynomial in convolution"
    );
    let mut out = vec![Fp::zero(); a.len() + b.len() - 1];
    for (i, lhs) in a.iter().enumerate() {
        for (j, rhs) in b.iter().enumerate() {
            out[i + j] += *lhs * *rhs;
        }
    }
    Ok(out)
}

pub fn negacyclic_multiply(lhs: &[Fp], rhs: &[Fp]) -> Result<Vec<Fp>> {
    ensure!(
        !lhs.is_empty() && !rhs.is_empty(),
        "cannot multiply empty ring elements"
    );
    ensure!(lhs.len() == rhs.len(), "ring length mismatch");
    ensure!(
        lhs.len().is_power_of_two(),
        "ring length must be power of two"
    );

    let n = lhs.len();
    let ntt_size = n * 2;
    if !CRT_PRIMES
        .iter()
        .all(|prime| (prime - 1) % (ntt_size as u64) == 0)
    {
        return Ok(naive_negacyclic(lhs, rhs));
    }
    let mut residues_per_prime = Vec::with_capacity(CRT_PRIMES.len());

    for &modulus in &CRT_PRIMES {
        let mut a = vec![0u64; ntt_size];
        let mut b = vec![0u64; ntt_size];
        for idx in 0..n {
            a[idx] = (lhs[idx].as_u128() % (modulus as u128)) as u64;
            b[idx] = (rhs[idx].as_u128() % (modulus as u128)) as u64;
        }

        let plan = cached_plan(ntt_size, modulus)?;
        forward_ntt(&mut a, &plan)?;
        forward_ntt(&mut b, &plan)?;
        for (lhs_coeff, rhs_coeff) in a.iter_mut().zip(b.iter()) {
            *lhs_coeff = mod_mul(*lhs_coeff, *rhs_coeff, modulus);
        }
        inverse_ntt(&mut a, &plan)?;

        let mut reduced = vec![0u64; n];
        for idx in 0..n {
            reduced[idx] = mod_sub(a[idx], a[idx + n], modulus);
        }
        residues_per_prime.push(reduced);
    }

    let crt = crt_data();
    let mut coeffs = Vec::with_capacity(n);
    for coeff_idx in 0..n {
        let residues = residues_per_prime
            .iter()
            .map(|residues| residues[coeff_idx])
            .collect::<Vec<_>>();
        coeffs.push(Fp::from_u128(reconstruct_mod_q(&residues, crt)?));
    }
    Ok(coeffs)
}

fn naive_negacyclic(lhs: &[Fp], rhs: &[Fp]) -> Vec<Fp> {
    let mut out = vec![Fp::zero(); lhs.len()];
    for (i, a) in lhs.iter().enumerate() {
        for (j, b) in rhs.iter().enumerate() {
            let mut idx = i + j;
            let mut term = *a * *b;
            if idx >= lhs.len() {
                idx -= lhs.len();
                term = -term;
            }
            out[idx] += term;
        }
    }
    out
}

#[inline]
fn butterfly_chunk(chunk: &mut [u64], wlen: u64, modulus: u64) {
    let len = chunk.len();
    let (left, right) = chunk.split_at_mut(len / 2);
    let mut w = 1u64;
    for (lhs, rhs) in left.iter_mut().zip(right.iter_mut()) {
        let t = mod_mul(*rhs, w, modulus);
        let u = *lhs;
        *lhs = mod_add(u, t, modulus);
        *rhs = mod_sub(u, t, modulus);
        w = mod_mul(w, wlen, modulus);
    }
}

fn cached_plan(size: usize, modulus: u64) -> Result<Arc<NttPlan>> {
    static CACHE: OnceLock<Mutex<HashMap<(usize, u64), Arc<NttPlan>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    {
        let guard = cache.lock().expect("NTT cache lock poisoned");
        if let Some(plan) = guard.get(&(size, modulus)) {
            return Ok(Arc::clone(plan));
        }
    }

    let plan = Arc::new(NttPlan::new(size, modulus)?);
    let mut guard = cache.lock().expect("NTT cache lock poisoned");
    let entry = guard
        .entry((size, modulus))
        .or_insert_with(|| Arc::clone(&plan));
    Ok(Arc::clone(entry))
}

fn primitive_root_of_unity(size: usize, modulus: u64) -> Result<u64> {
    let generator = multiplicative_generator(modulus);
    let root = mod_pow(generator, ((modulus - 1) / size as u64) as u128, modulus);
    if mod_pow(root, size as u128, modulus) != 1 {
        return Err(anyhow!("failed to derive size-th root of unity"));
    }
    if size > 1 && mod_pow(root, (size / 2) as u128, modulus) == 1 {
        return Err(anyhow!("derived root is not primitive"));
    }
    Ok(root)
}

fn multiplicative_generator(modulus: u64) -> u64 {
    let factors = factorize(modulus - 1);
    'candidate: for cand in 2u64..modulus {
        for &factor in &factors {
            if mod_pow(cand, ((modulus - 1) / factor) as u128, modulus) == 1 {
                continue 'candidate;
            }
        }
        return cand;
    }
    panic!("no multiplicative generator found for modulus {modulus}");
}

fn factorize(mut value: u64) -> Vec<u64> {
    let mut factors = Vec::new();
    let mut divisor = 2u64;
    while divisor * divisor <= value {
        if value % divisor == 0 {
            factors.push(divisor);
            while value % divisor == 0 {
                value /= divisor;
            }
        }
        divisor += if divisor == 2 { 1 } else { 2 };
    }
    if value > 1 {
        factors.push(value);
    }
    factors
}

#[inline]
fn mod_add(lhs: u64, rhs: u64, modulus: u64) -> u64 {
    let sum = lhs + rhs;
    if sum >= modulus {
        sum - modulus
    } else {
        sum
    }
}

#[inline]
fn mod_sub(lhs: u64, rhs: u64, modulus: u64) -> u64 {
    if lhs >= rhs {
        lhs - rhs
    } else {
        modulus - (rhs - lhs)
    }
}

#[inline]
fn mod_mul(lhs: u64, rhs: u64, modulus: u64) -> u64 {
    ((lhs as u128 * rhs as u128) % modulus as u128) as u64
}

fn mod_pow(mut base: u64, mut exp: u128, modulus: u64) -> u64 {
    let mut result = 1u64;
    while exp > 0 {
        if exp & 1 == 1 {
            result = mod_mul(result, base, modulus);
        }
        base = mod_mul(base, base, modulus);
        exp >>= 1;
    }
    result
}

fn build_bitrev_table(n: usize) -> Vec<usize> {
    if n == 1 {
        return vec![0];
    }
    let bits = n.trailing_zeros();
    (0..n)
        .map(|idx| (idx.reverse_bits() >> (usize::BITS - bits)) as usize)
        .collect()
}

fn bit_reverse_permute(values: &mut [u64], bitrev: &[usize]) {
    for (idx, &other) in bitrev.iter().enumerate() {
        if idx < other {
            values.swap(idx, other);
        }
    }
}

struct CrtData {
    prefix_products: Vec<BigUint>,
    inverses: Vec<u64>,
    total_product: BigUint,
    half_product: BigUint,
}

fn crt_data() -> &'static CrtData {
    static CRT: OnceLock<CrtData> = OnceLock::new();
    CRT.get_or_init(|| {
        let mut prefix_products = Vec::with_capacity(CRT_PRIMES.len());
        let mut product = BigUint::from(1u8);
        let mut inverses = Vec::with_capacity(CRT_PRIMES.len());
        for &prime in &CRT_PRIMES {
            prefix_products.push(product.clone());
            let prefix_mod_prime = (&product % prime).to_u64().expect("prefix residue");
            inverses.push(mod_pow(prefix_mod_prime, (prime - 2) as u128, prime));
            product *= prime;
        }
        CrtData {
            prefix_products,
            inverses,
            half_product: &product >> 1usize,
            total_product: product,
        }
    })
}

fn reconstruct_mod_q(residues: &[u64], crt: &CrtData) -> Result<u128> {
    ensure!(
        residues.len() == CRT_PRIMES.len(),
        "CRT residue vector length mismatch"
    );

    let modulus_q = BigUint::from(MSIS_Q);
    let mut acc = BigUint::zero();
    for (idx, (&residue, &prime)) in residues.iter().zip(CRT_PRIMES.iter()).enumerate() {
        let current_mod = (&acc % prime).to_u64().expect("CRT accumulator residue");
        let delta = mod_sub(residue, current_mod, prime);
        let correction = mod_mul(delta, crt.inverses[idx], prime);
        acc += &crt.prefix_products[idx] * correction;
    }
    let centered = if acc > crt.half_product {
        let distance = (&crt.total_product - acc) % &modulus_q;
        if distance.is_zero() {
            BigUint::zero()
        } else {
            &modulus_q - distance
        }
    } else {
        acc % &modulus_q
    };
    Ok(centered
        .to_u128()
        .expect("CRT reconstruction must fit in u128"))
}

#[cfg(test)]
mod tests {
    use super::{
        convolution, forward_ntt, inverse_ntt, naive_negacyclic, negacyclic_multiply, NttPlan,
    };
    use crate::algebra::field::Fp;
    use crate::utils::config::{CRT_PRIMES, MIN_RING_DEGREE};

    #[test]
    fn ntt_round_trip() {
        let mut values: Vec<u64> = (0u64..128).collect();
        let original = values.clone();
        let plan = NttPlan::new(values.len(), CRT_PRIMES[0]).expect("plan");
        forward_ntt(&mut values, &plan).expect("forward");
        inverse_ntt(&mut values, &plan).expect("inverse");
        assert_eq!(values, original);
    }

    #[test]
    fn generic_convolution_matches_reference() {
        let a = vec![
            Fp::from(1u64),
            Fp::from(2u64),
            Fp::from(3u64),
            Fp::from(4u64),
        ];
        let b = vec![
            Fp::from(4u64),
            Fp::from(5u64),
            Fp::from(6u64),
            Fp::from(7u64),
        ];
        let c = convolution(&a, &b).expect("conv");
        assert_eq!(c.len(), 7);
        assert_eq!(c[0], Fp::from(4u64));
        assert_eq!(c[3], Fp::from(50u64));
    }

    #[test]
    fn negacyclic_multiply_matches_naive() {
        let lhs = (0..MIN_RING_DEGREE)
            .map(|idx| Fp::from((idx + 1) as u64))
            .collect::<Vec<_>>();
        let rhs = (0..MIN_RING_DEGREE)
            .map(|idx| Fp::from((idx * 3 + 5) as u64))
            .collect::<Vec<_>>();
        let ntt = negacyclic_multiply(&lhs, &rhs).expect("ntt mul");
        let naive = naive_negacyclic(&lhs, &rhs);
        assert_eq!(ntt, naive);
    }

    #[test]
    fn negacyclic_multiply_matches_naive_across_power_of_two_sizes() {
        for &len in &[2usize, 4, 8, 16, 32, MIN_RING_DEGREE] {
            let lhs = (0..len)
                .map(|idx| Fp::from(((idx * 5) + 1) as u64))
                .collect::<Vec<_>>();
            let rhs = (0..len)
                .map(|idx| Fp::from(((idx * 7) + 3) as u64))
                .collect::<Vec<_>>();
            let ntt = negacyclic_multiply(&lhs, &rhs).expect("ntt mul");
            let naive = naive_negacyclic(&lhs, &rhs);
            assert_eq!(ntt, naive, "ring length {len} mismatch");
        }
    }

    #[test]
    fn ntt_roundtrip_across_runtime_sizes() {
        for &len in &[64usize, 128, 256, 512, 1024, 2048] {
            let mut values: Vec<u64> = (0..len as u64)
                .map(|idx| (idx * 17 + 3) % CRT_PRIMES[0])
                .collect();
            let original = values.clone();
            let plan = NttPlan::new(values.len(), CRT_PRIMES[0]).expect("plan");
            forward_ntt(&mut values, &plan).expect("forward");
            inverse_ntt(&mut values, &plan).expect("inverse");
            assert_eq!(values, original, "NTT roundtrip failed for len={len}");
        }
    }

    #[test]
    fn negacyclic_multiply_across_runtime_sizes() {
        for &len in &[64usize, 128, 256, 512, 1024, 2048] {
            let mut lhs = vec![Fp::zero(); len];
            let mut rhs = vec![Fp::zero(); len];
            lhs[len - 1] = Fp::from(3u64);
            rhs[1] = Fp::from(5u64);
            let ntt = negacyclic_multiply(&lhs, &rhs).expect("ntt mul");
            let mut expected = vec![Fp::zero(); len];
            expected[0] = -Fp::from(15u64);
            assert_eq!(ntt, expected, "ring length {len} mismatch");
        }
    }
}
