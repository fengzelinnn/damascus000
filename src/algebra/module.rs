use crate::algebra::field::Fp;
use crate::algebra::poly::Poly;
use anyhow::{ensure, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::array;
use std::convert::TryInto;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModuleElement<const K: usize> {
    pub coords: [Poly; K],
}

impl<const K: usize> ModuleElement<K> {
    pub fn zero(ring_len: usize) -> Self {
        Self {
            coords: array::from_fn(|_| Poly::zero(ring_len)),
        }
    }

    pub fn from_coords(coords: [Poly; K]) -> Result<Self> {
        ensure!(K > 0, "module rank must be > 0");
        let ring_len = coords[0].len();
        ensure!(
            coords.iter().all(|coord| coord.len() == ring_len),
            "module coordinate length mismatch"
        );
        Ok(Self { coords })
    }

    pub fn ring_len(&self) -> usize {
        self.coords[0].len()
    }

    pub fn add(&self, rhs: &Self) -> Result<Self> {
        ensure!(
            self.ring_len() == rhs.ring_len(),
            "module ring length mismatch"
        );
        let coords = array::from_fn(|idx| self.coords[idx].add(&rhs.coords[idx]).expect("shape"));
        Self::from_coords(coords)
    }

    pub fn sub(&self, rhs: &Self) -> Result<Self> {
        ensure!(
            self.ring_len() == rhs.ring_len(),
            "module ring length mismatch"
        );
        let coords = array::from_fn(|idx| self.coords[idx].sub(&rhs.coords[idx]).expect("shape"));
        Self::from_coords(coords)
    }

    pub fn scale(&self, scalar: Fp) -> Result<Self> {
        let coords = array::from_fn(|idx| self.coords[idx].scale(scalar));
        Self::from_coords(coords)
    }

    pub fn add_scaled(&self, rhs: &Self, scalar: Fp) -> Result<Self> {
        self.add(&rhs.scale(scalar)?)
    }

    pub fn ring_mul(&self, scalar: &Poly) -> Result<Self> {
        ensure!(
            self.ring_len() == scalar.len(),
            "module/scalar ring length mismatch"
        );
        let coords = array::from_fn(|idx| self.coords[idx].mul(scalar, true).expect("shape"));
        Self::from_coords(coords)
    }

    pub fn odd_even_decomposition(&self) -> Result<(Self, Self)> {
        let even = array::from_fn(|idx| self.coords[idx].odd_even_decomposition().0);
        let odd = array::from_fn(|idx| self.coords[idx].odd_even_decomposition().1);
        Ok((Self::from_coords(even)?, Self::from_coords(odd)?))
    }

    pub fn mul_by_x(&self) -> Result<Self> {
        let coords = array::from_fn(|idx| self.coords[idx].mul_by_x().expect("shift"));
        Self::from_coords(coords)
    }
}

impl<const K: usize> Serialize for ModuleElement<K> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.coords.as_slice().serialize(serializer)
    }
}

impl<'de, const K: usize> Deserialize<'de> for ModuleElement<K> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let coords = Vec::<Poly>::deserialize(deserializer)?;
        let coords: [Poly; K] = coords
            .try_into()
            .map_err(|_| serde::de::Error::custom("module rank mismatch"))?;
        ModuleElement::from_coords(coords).map_err(serde::de::Error::custom)
    }
}
