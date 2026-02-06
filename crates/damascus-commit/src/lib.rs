use damascus_ring::{ModuleElem, Poly};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    #[error("length mismatch")]
    LengthMismatch,
    #[error("invalid parameters")]
    InvalidParams,
    #[error(transparent)]
    Ring(#[from] damascus_ring::poly::PolyError),
    #[error(transparent)]
    Module(#[from] damascus_ring::module::ModuleError),
}

pub trait LinearCommit {
    type Commitment;
    type Opening;

    fn commit(&self, message: &[Poly], opening: &[Poly]) -> Result<Self::Commitment, CommitError>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModuleSisCommitKey {
    pub g: Vec<ModuleElem>,
    pub h: Vec<ModuleElem>,
}

impl ModuleSisCommitKey {
    pub fn new(g: Vec<ModuleElem>, h: Vec<ModuleElem>) -> Result<Self, CommitError> {
        if g.len() != h.len() || g.is_empty() {
            return Err(CommitError::InvalidParams);
        }
        Ok(Self { g, h })
    }
}

impl LinearCommit for ModuleSisCommitKey {
    type Commitment = ModuleElem;
    type Opening = Vec<Poly>;

    fn commit(&self, message: &[Poly], opening: &[Poly]) -> Result<Self::Commitment, CommitError> {
        if message.len() != self.g.len() || opening.len() != self.h.len() {
            return Err(CommitError::LengthMismatch);
        }
        let mut acc = ModuleElem::zero(self.g[0].q(), self.g[0].n(), self.g[0].k())?;
        for i in 0..message.len() {
            acc = acc.add(&self.g[i].ring_mul(&message[i])?)?;
        }
        for i in 0..opening.len() {
            acc = acc.add(&self.h[i].ring_mul(&opening[i])?)?;
        }
        Ok(acc)
    }
}
