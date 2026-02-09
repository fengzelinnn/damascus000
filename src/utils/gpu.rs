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
        left: *const u64,
        right: *const u64,
        out: *mut u64,
        len: usize,
        challenge: u64,
    ) -> i32;
}

pub fn try_fold_pairs_gpu(left: &[u64], right: &[u64], challenge: u64) -> Option<Vec<u64>> {
    if !cuda_backend_ready() || left.len() != right.len() || left.is_empty() {
        return None;
    }

    #[cfg(damascus_cuda_available)]
    {
        let mut out = vec![0u64; left.len()];
        let rc = unsafe {
            damascus_cuda_fold_batch(
                left.as_ptr(),
                right.as_ptr(),
                out.as_mut_ptr(),
                left.len(),
                challenge,
            )
        };
        if rc == 0 {
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
