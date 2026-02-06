use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poly {
    q: u64,
    coeffs: Vec<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum PolyError {
    #[error("invalid modulus q={0}")]
    BadModulus(u64),
    #[error("invalid length n={0}")]
    BadLength(usize),
    #[error("modulus mismatch")]
    ModulusMismatch,
    #[error("length mismatch")]
    LengthMismatch,
}

impl Poly {
    pub fn zero(q: u64, n: usize) -> Result<Self, PolyError> {
        Self::validate_params(q, n)?;
        Ok(Self {
            q,
            coeffs: vec![0; n],
        })
    }

    pub fn from_coeffs(q: u64, coeffs: Vec<u64>) -> Result<Self, PolyError> {
        Self::validate_params(q, coeffs.len())?;
        let q_masked = coeffs.into_iter().map(|c| c % q).collect();
        Ok(Self {
            q,
            coeffs: q_masked,
        })
    }

    pub fn q(&self) -> u64 {
        self.q
    }

    pub fn n(&self) -> usize {
        self.coeffs.len()
    }

    pub fn coeffs(&self) -> &[u64] {
        &self.coeffs
    }

    pub fn coeff0(&self) -> u64 {
        self.coeffs.first().copied().unwrap_or(0)
    }

    pub fn add(&self, other: &Self) -> Result<Self, PolyError> {
        self.check_compat(other)?;
        let q = self.q;
        let coeffs = self
            .coeffs
            .iter()
            .zip(other.coeffs.iter())
            .map(|(&a, &b)| (a + b) % q)
            .collect();
        Ok(Self { q, coeffs })
    }

    pub fn sub(&self, other: &Self) -> Result<Self, PolyError> {
        self.check_compat(other)?;
        let q = self.q;
        let coeffs = self
            .coeffs
            .iter()
            .zip(other.coeffs.iter())
            .map(|(&a, &b)| (a + q - (b % q)) % q)
            .collect();
        Ok(Self { q, coeffs })
    }

    pub fn scalar_mul(&self, s: u64) -> Result<Self, PolyError> {
        let q = self.q;
        let s = s % q;
        let coeffs = self
            .coeffs
            .iter()
            .map(|&a| ((a as u128 * s as u128) % q as u128) as u64)
            .collect();
        Ok(Self { q, coeffs })
    }

    pub fn negacyclic_mul(&self, other: &Self) -> Result<Self, PolyError> {
        self.check_compat(other)?;
        let q = self.q;
        let n = self.n();
        let mut out = vec![0u64; n];
        for (i, &ai) in self.coeffs.iter().enumerate() {
            for (j, &bj) in other.coeffs.iter().enumerate() {
                let mut idx = i + j;
                let mut term = (ai as u128 * bj as u128) % (q as u128);
                if idx >= n {
                    idx -= n;
                    if term != 0 {
                        term = (q as u128) - term;
                    }
                }
                out[idx] = ((out[idx] as u128 + term) % (q as u128)) as u64;
            }
        }
        Ok(Self { q, coeffs: out })
    }

    pub fn odd_even_decompose(&self) -> Result<(Self, Self), PolyError> {
        let n = self.n();
        if n % 2 != 0 || n == 0 {
            return Err(PolyError::BadLength(n));
        }
        let half = n / 2;
        let mut even = vec![0u64; half];
        let mut odd = vec![0u64; half];
        for i in 0..half {
            even[i] = self.coeffs[2 * i];
            odd[i] = self.coeffs[2 * i + 1];
        }
        Ok((
            Self {
                q: self.q,
                coeffs: even,
            },
            Self {
                q: self.q,
                coeffs: odd,
            },
        ))
    }

    pub fn mul_by_x(&self) -> Result<Self, PolyError> {
        let n = self.n();
        if n == 0 {
            return Err(PolyError::BadLength(n));
        }
        let q = self.q;
        let mut out = vec![0u64; n];
        let last = self.coeffs[n - 1];
        out[0] = if last == 0 { 0 } else { q - last };
        for i in 1..n {
            out[i] = self.coeffs[i - 1];
        }
        Ok(Self { q, coeffs: out })
    }

    pub fn from_seed(q: u64, n: usize, seed: [u8; 32], label: &[u8]) -> Result<Self, PolyError> {
        Self::validate_params(q, n)?;
        let mut coeffs = vec![0u64; n];
        for i in 0..n {
            let mut input = Vec::with_capacity(label.len() + 4);
            input.extend_from_slice(label);
            input.extend_from_slice(&(i as u32).to_be_bytes());
            let h = blake3::keyed_hash(&seed, &input);
            let mut limb = [0u8; 8];
            limb.copy_from_slice(&h.as_bytes()[0..8]);
            coeffs[i] = u64::from_be_bytes(limb) % q;
        }
        Ok(Self { q, coeffs })
    }

    fn validate_params(q: u64, n: usize) -> Result<(), PolyError> {
        if q < 2 {
            return Err(PolyError::BadModulus(q));
        }
        if n == 0 {
            return Err(PolyError::BadLength(n));
        }
        Ok(())
    }

    fn check_compat(&self, other: &Self) -> Result<(), PolyError> {
        if self.q != other.q {
            return Err(PolyError::ModulusMismatch);
        }
        if self.n() != other.n() {
            return Err(PolyError::LengthMismatch);
        }
        Ok(())
    }
}
