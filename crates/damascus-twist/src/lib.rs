use damascus_types::FileId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TwistParams {
    pub n0: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwistCommitment {
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwistRound {
    pub file_id: FileId,
    pub epoch: u64,
    pub round: u32,
    pub l: Vec<u8>,
    pub r: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum TwistError {
    #[error("not implemented in prototype")]
    NotImplemented,
}

pub fn commit(
    _params: &TwistParams,
    _file_id: FileId,
    _data: &[u8],
) -> Result<TwistCommitment, TwistError> {
    Err(TwistError::NotImplemented)
}
