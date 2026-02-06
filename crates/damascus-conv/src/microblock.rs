use damascus_ring::ModuleElem;
use damascus_types::FileId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MbVec {
    pub file_id: FileId,
    pub epoch: u64,
    pub round: u32,
    pub l_vec: ModuleElem,
    pub r_vec: ModuleElem,
    pub sig: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MbPoly {
    pub file_id: FileId,
    pub epoch: u64,
    pub round: u32,
    pub l_poly: ModuleElem,
    pub r_poly: ModuleElem,
    pub sig: Vec<u8>,
}
