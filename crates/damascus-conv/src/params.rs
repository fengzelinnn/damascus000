use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ConvParams {
    pub q: u64,
    pub n0: usize,
    pub n_rounds: usize,
    pub k: usize,
    pub seed_generators: [u8; 32],
}

#[derive(Debug, thiserror::Error)]
pub enum ParamsError {
    #[error("q must be >= 2")]
    BadModulus,
    #[error("n0 must be power-of-two")]
    BadN0,
    #[error("k must be >= 1")]
    BadK,
    #[error("n_rounds inconsistent with n0 (expected n0 == 2^n_rounds)")]
    BadRounds,
}

impl ConvParams {
    pub fn validate(&self) -> Result<(), ParamsError> {
        if self.q < 2 {
            return Err(ParamsError::BadModulus);
        }
        if self.k == 0 {
            return Err(ParamsError::BadK);
        }
        if !self.n0.is_power_of_two() || self.n0 == 0 {
            return Err(ParamsError::BadN0);
        }
        let expected = 1usize.checked_shl(self.n_rounds as u32).unwrap_or(0);
        if expected == 0 || expected != self.n0 {
            return Err(ParamsError::BadRounds);
        }
        Ok(())
    }
}
