use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub const MSIS_Q: u128 = 5_192_296_858_534_827_628_530_496_329_220_021;
pub const POLY_DEGREE: usize = 64;
pub const MODULE_RANK: usize = 8;
pub const BYTES_PER_COEFF: usize = 13;
pub const CRT_PRIMES: [u64; 8] = [
    3_892_314_113,
    2_281_701_377,
    2_013_265_921,
    2_885_681_153,
    2_483_027_969,
    1_811_939_329,
    469_762_049,
    4_194_304_001,
];
pub const DEFAULT_GENERATOR_SEED: [u8; 32] = [7u8; 32];
pub const DEFAULT_EPOCH_SEED: [u8; 32] = [11u8; 32];
pub const DEFAULT_MEMORY_LIMIT_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemParams {
    pub module_rank: usize,
    pub vector_len: usize,
    pub poly_len: usize,
    pub rounds: usize,
    pub seed_generators: [u8; 32],
    pub epoch_seed: [u8; 32],
}

impl Default for SystemParams {
    fn default() -> Self {
        Self {
            module_rank: MODULE_RANK,
            vector_len: POLY_DEGREE,
            poly_len: POLY_DEGREE,
            rounds: 0,
            seed_generators: DEFAULT_GENERATOR_SEED,
            epoch_seed: DEFAULT_EPOCH_SEED,
        }
    }
}

impl SystemParams {
    pub fn validate(&self) -> Result<()> {
        ensure!(MSIS_Q % 2 == 1, "MSIS_Q must be odd");
        ensure!(
            POLY_DEGREE.is_power_of_two(),
            "POLY_DEGREE must be a power of two"
        );
        ensure!(MODULE_RANK > 0, "MODULE_RANK must be > 0");
        ensure!(BYTES_PER_COEFF > 0, "BYTES_PER_COEFF must be > 0");
        ensure!(self.module_rank > 0, "module_rank must be > 0");
        ensure!(
            self.vector_len.is_power_of_two(),
            "vector_len must be power of two"
        );
        ensure!(
            self.poly_len.is_power_of_two(),
            "poly_len must be power of two"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub max_preprocess_bytes: usize,
    pub ntt_enabled: bool,
    pub parallel_enabled: bool,
    pub gpu_enabled: bool,
    pub gpu_min_elements: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_preprocess_bytes: DEFAULT_MEMORY_LIMIT_BYTES,
            ntt_enabled: true,
            parallel_enabled: true,
            gpu_enabled: true,
            gpu_min_elements: 2_097_152,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BenchRow {
    pub file_size_label: String,
    pub preprocessing_s: f64,
    pub vec_fold_ms: f64,
    pub poly_fold_ms: f64,
    pub verify_us: f64,
    pub round_record_size_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::{SystemParams, BYTES_PER_COEFF, CRT_PRIMES, MODULE_RANK, POLY_DEGREE};

    #[test]
    fn hard_spec_constants_are_wired() {
        let params = SystemParams::default();
        assert!(params.validate().is_ok());
        assert_eq!(POLY_DEGREE, 64);
        assert_eq!(MODULE_RANK, 8);
        assert_eq!(BYTES_PER_COEFF, 13);
        assert_eq!(CRT_PRIMES.len(), 8);
    }
}
