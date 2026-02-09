use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemParams {
    pub module_rank: usize,
    pub vector_len: usize,
    pub poly_len: usize,
    pub rounds: usize,
    pub seed_generators: [u8; 32],
}

impl Default for SystemParams {
    fn default() -> Self {
        Self {
            module_rank: 4,
            vector_len: 1024,
            poly_len: 1024,
            rounds: 0,
            seed_generators: [7u8; 32],
        }
    }
}

impl SystemParams {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.module_rank > 0, "module_rank must be > 0");
        ensure!(
            self.vector_len.is_power_of_two(),
            "vector_len must be power of two"
        );
        ensure!(
            self.poly_len.is_power_of_two(),
            "poly_len must be power of two"
        );
        ensure!(
            self.vector_len == self.poly_len,
            "vector_len and poly_len must be equal (square tensor)"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub ntt_enabled: bool,
    pub parallel_enabled: bool,
    pub gpu_enabled: bool,
    pub gpu_min_elements: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            ntt_enabled: true,
            parallel_enabled: true,
            gpu_enabled: true,
            gpu_min_elements: 16_777_216,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BenchRow {
    pub file_size_label: String,
    pub ntt_mode: String,
    pub preprocessing_s: f64,
    pub vec_fold_ms: f64,
    pub poly_fold_ms: f64,
    pub verify_us: f64,
    pub cross_term_size_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::SystemParams;

    #[test]
    fn validate_accepts_square_power_of_two_dims() {
        let params = SystemParams {
            vector_len: 1024,
            poly_len: 1024,
            ..SystemParams::default()
        };
        assert!(params.validate().is_ok());
    }

    #[test]
    fn validate_rejects_non_square_dims() {
        let params = SystemParams {
            vector_len: 2048,
            poly_len: 1024,
            ..SystemParams::default()
        };
        assert!(params.validate().is_err());
    }
}
