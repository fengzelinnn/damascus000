use damascus_ring::{ModuleElem, Poly};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConvPublicState {
    pub g: Vec<ModuleElem>,
    pub h: Vec<ModuleElem>,
    pub c: ModuleElem,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConvWitness {
    pub f: Vec<Poly>,
    pub r: Vec<Poly>,
}
