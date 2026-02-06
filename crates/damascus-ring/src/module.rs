use crate::poly::{Poly, PolyError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleElem {
    coords: Vec<Poly>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModuleError {
    #[error("invalid module dimension k={0}")]
    BadDimension(usize),
    #[error("coordinate mismatch")]
    CoordMismatch,
    #[error(transparent)]
    Poly(#[from] PolyError),
}

impl ModuleElem {
    pub fn zero(q: u64, n: usize, k: usize) -> Result<Self, ModuleError> {
        if k == 0 {
            return Err(ModuleError::BadDimension(k));
        }
        let mut coords = Vec::with_capacity(k);
        for _ in 0..k {
            coords.push(Poly::zero(q, n)?);
        }
        Ok(Self { coords })
    }

    pub fn from_coords(coords: Vec<Poly>) -> Result<Self, ModuleError> {
        if coords.is_empty() {
            return Err(ModuleError::BadDimension(0));
        }
        let q = coords[0].q();
        let n = coords[0].n();
        for c in &coords {
            if c.q() != q || c.n() != n {
                return Err(ModuleError::CoordMismatch);
            }
        }
        Ok(Self { coords })
    }

    pub fn k(&self) -> usize {
        self.coords.len()
    }

    pub fn q(&self) -> u64 {
        self.coords[0].q()
    }

    pub fn n(&self) -> usize {
        self.coords[0].n()
    }

    pub fn coords(&self) -> &[Poly] {
        &self.coords
    }

    pub fn add(&self, other: &Self) -> Result<Self, ModuleError> {
        self.check_compat(other)?;
        let coords = self
            .coords
            .iter()
            .zip(other.coords.iter())
            .map(|(a, b)| a.add(b))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { coords })
    }

    pub fn scalar_mul(&self, s: u64) -> Result<Self, ModuleError> {
        let coords = self
            .coords
            .iter()
            .map(|p| p.scalar_mul(s))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { coords })
    }

    pub fn ring_mul(&self, r: &Poly) -> Result<Self, ModuleError> {
        if self.q() != r.q() || self.n() != r.n() {
            return Err(ModuleError::CoordMismatch);
        }
        let coords = self
            .coords
            .iter()
            .map(|p| r.negacyclic_mul(p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { coords })
    }

    pub fn odd_even_decompose(&self) -> Result<(Self, Self), ModuleError> {
        let mut even_coords = Vec::with_capacity(self.coords.len());
        let mut odd_coords = Vec::with_capacity(self.coords.len());
        for c in &self.coords {
            let (e, o) = c.odd_even_decompose()?;
            even_coords.push(e);
            odd_coords.push(o);
        }
        Ok((
            Self {
                coords: even_coords,
            },
            Self { coords: odd_coords },
        ))
    }

    pub fn mul_by_x(&self) -> Result<Self, ModuleError> {
        let coords = self
            .coords
            .iter()
            .map(|p| p.mul_by_x())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { coords })
    }

    fn check_compat(&self, other: &Self) -> Result<(), ModuleError> {
        if self.k() != other.k() || self.q() != other.q() || self.n() != other.n() {
            return Err(ModuleError::CoordMismatch);
        }
        Ok(())
    }
}
