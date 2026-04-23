use std::env;
use std::process::Command;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct CudaDeviceInfo {
    pub available: bool,
    pub summary: String,
}

#[cfg(damascus_cuda_available)]
const CUDA_KERNEL_COMPILED: bool = true;
#[cfg(not(damascus_cuda_available))]
const CUDA_KERNEL_COMPILED: bool = false;

static CUDA_INFO: OnceLock<CudaDeviceInfo> = OnceLock::new();
const NTT_BATCH_PRIMES: usize = 8;

pub fn cuda_device_info() -> &'static CudaDeviceInfo {
    CUDA_INFO.get_or_init(probe_cuda_device)
}

pub fn cuda_backend_ready() -> bool {
    CUDA_KERNEL_COMPILED && cuda_device_info().available
}

fn probe_cuda_device() -> CudaDeviceInfo {
    if env::var("DAMASCUS_GPU").is_ok_and(|v| v == "0") {
        return CudaDeviceInfo {
            available: false,
            summary: "disabled by DAMASCUS_GPU=0".to_string(),
        };
    }

    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,driver_version,memory.total,compute_cap",
            "--format=csv,noheader",
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let first_line = text.lines().next().unwrap_or("").trim().to_string();
            if first_line.is_empty() {
                CudaDeviceInfo {
                    available: false,
                    summary: "nvidia-smi returned empty device list".to_string(),
                }
            } else {
                CudaDeviceInfo {
                    available: true,
                    summary: first_line,
                }
            }
        }
        Ok(o) => CudaDeviceInfo {
            available: false,
            summary: format!("nvidia-smi failed with status {}", o.status),
        },
        Err(e) => CudaDeviceInfo {
            available: false,
            summary: format!("nvidia-smi not available: {e}"),
        },
    }
}

#[cfg(damascus_cuda_available)]
unsafe extern "C" {
    fn damascus_cuda_fold_batch(
        left_lo: *const u64,
        left_hi: *const u64,
        right_lo: *const u64,
        right_hi: *const u64,
        out_lo: *mut u64,
        out_hi: *mut u64,
        len: usize,
        challenge_lo: u64,
        challenge_hi: u64,
    ) -> i32;

    fn damascus_cuda_ntt_batch(
        host_a: *const u64,
        host_b: *const u64,
        host_out: *mut u64,
        n: usize,
        primes: *const u64,
        stage_roots: *const u64,
        inv_roots: *const u64,
        inv_sizes: *const u64,
    ) -> i32;
}

pub fn try_fold_pairs_gpu(left: &[u128], right: &[u128], challenge: u128) -> Option<Vec<u128>> {
    if !cuda_backend_ready() || left.len() != right.len() || left.is_empty() {
        return None;
    }

    #[cfg(damascus_cuda_available)]
    {
        let n = left.len();
        let left_lo: Vec<u64> = left.iter().map(|v| *v as u64).collect();
        let left_hi: Vec<u64> = left.iter().map(|v| (*v >> 64) as u64).collect();
        let right_lo: Vec<u64> = right.iter().map(|v| *v as u64).collect();
        let right_hi: Vec<u64> = right.iter().map(|v| (*v >> 64) as u64).collect();
        let mut out_lo = vec![0u64; n];
        let mut out_hi = vec![0u64; n];
        let rc = unsafe {
            damascus_cuda_fold_batch(
                left_lo.as_ptr(),
                left_hi.as_ptr(),
                right_lo.as_ptr(),
                right_hi.as_ptr(),
                out_lo.as_mut_ptr(),
                out_hi.as_mut_ptr(),
                n,
                challenge as u64,
                (challenge >> 64) as u64,
            )
        };
        if rc == 0 {
            let out: Vec<u128> = out_lo
                .iter()
                .zip(out_hi.iter())
                .map(|(&lo, &hi)| lo as u128 | ((hi as u128) << 64))
                .collect();
            Some(out)
        } else {
            None
        }
    }
    #[cfg(not(damascus_cuda_available))]
    {
        let _ = (left, right, challenge);
        None
    }
}

pub fn try_ntt_batch_gpu(
    a: &[Vec<u64>],
    b: &[Vec<u64>],
    n: usize,
    primes: &[u64; NTT_BATCH_PRIMES],
    stage_roots: &[u64],
    inv_roots: &[u64],
    inv_sizes: &[u64; NTT_BATCH_PRIMES],
) -> Option<Vec<Vec<u64>>> {
    if !cuda_backend_ready()
        || n == 0
        || !n.is_power_of_two()
        || a.len() != NTT_BATCH_PRIMES
        || b.len() != NTT_BATCH_PRIMES
    {
        return None;
    }

    let log_n = n.trailing_zeros() as usize;
    if stage_roots.len() != NTT_BATCH_PRIMES * log_n || inv_roots.len() != NTT_BATCH_PRIMES * log_n
    {
        return None;
    }
    if a.iter().any(|slice| slice.len() != n) || b.iter().any(|slice| slice.len() != n) {
        return None;
    }

    #[cfg(damascus_cuda_available)]
    {
        let mut host_a = Vec::with_capacity(NTT_BATCH_PRIMES * n);
        let mut host_b = Vec::with_capacity(NTT_BATCH_PRIMES * n);
        for slice in a {
            host_a.extend_from_slice(slice);
        }
        for slice in b {
            host_b.extend_from_slice(slice);
        }

        let mut host_out = vec![0u64; NTT_BATCH_PRIMES * n];
        let rc = unsafe {
            damascus_cuda_ntt_batch(
                host_a.as_ptr(),
                host_b.as_ptr(),
                host_out.as_mut_ptr(),
                n,
                primes.as_ptr(),
                stage_roots.as_ptr(),
                inv_roots.as_ptr(),
                inv_sizes.as_ptr(),
            )
        };
        if rc != 0 {
            return None;
        }

        Some(
            host_out
                .chunks_exact(n)
                .map(|chunk| chunk.to_vec())
                .collect(),
        )
    }
    #[cfg(not(damascus_cuda_available))]
    {
        let _ = (a, b, n, primes, stage_roots, inv_roots, inv_sizes);
        None
    }
}
