use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=cuda/fold_kernel.cu");
    println!("cargo:rerun-if-env-changed=DAMASCUS_BUILD_CUDA");
    println!("cargo:rerun-if-env-changed=DAMASCUS_CL_PATH");
    println!("cargo:rustc-check-cfg=cfg(damascus_cuda_available)");

    if env::var("DAMASCUS_BUILD_CUDA").is_ok_and(|v| v == "0") {
        println!("cargo:warning=CUDA build disabled by DAMASCUS_BUILD_CUDA=0");
        return;
    }
    if !cfg!(target_os = "windows") {
        return;
    }

    let Some(nvcc_path) = find_nvcc() else {
        println!("cargo:warning=CUDA nvcc not found; GPU fold backend disabled");
        return;
    };
    let Some(cl_path) = find_cl() else {
        println!("cargo:warning=MSVC cl.exe not found; GPU fold backend disabled");
        return;
    };

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let lib_path = out_dir.join("damascus_cuda_fold.lib");
    let kernel_path = PathBuf::from("cuda").join("fold_kernel.cu");

    let status = Command::new(&nvcc_path)
        .arg("--lib")
        .arg("-O3")
        .arg("--use_fast_math")
        .arg("-ccbin")
        .arg(&cl_path)
        .arg("-o")
        .arg(&lib_path)
        .arg(&kernel_path)
        .status();

    match status {
        Ok(s) if s.success() && lib_path.exists() => {
            println!("cargo:rustc-link-search=native={}", out_dir.display());
            println!("cargo:rustc-link-lib=static=damascus_cuda_fold");
            if let Some(cuda_lib_dir) = cuda_lib_dir_from_nvcc(&nvcc_path) {
                println!("cargo:rustc-link-search=native={}", cuda_lib_dir.display());
            }
            println!("cargo:rustc-link-lib=cudart");
            println!("cargo:rustc-cfg=damascus_cuda_available");
            println!(
                "cargo:warning=CUDA fold backend enabled (nvcc={}, cl={})",
                nvcc_path.display(),
                cl_path.display()
            );
        }
        Ok(s) => {
            println!(
                "cargo:warning=nvcc failed with status {:?}; GPU fold backend disabled",
                s.code()
            );
        }
        Err(e) => {
            println!("cargo:warning=failed to invoke nvcc ({e}); GPU fold backend disabled");
        }
    }
}

fn find_nvcc() -> Option<PathBuf> {
    if let Ok(cuda_path) = env::var("CUDA_PATH") {
        let candidate = Path::new(&cuda_path).join("bin").join("nvcc.exe");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    if let Some(path_nvcc) = find_on_path("nvcc.exe") {
        return Some(path_nvcc);
    }

    let cuda_root = Path::new("C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA");
    let entries = fs::read_dir(cuda_root).ok()?;
    let mut versions: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|x| x.path()))
        .filter(|p| p.is_dir())
        .collect();
    versions.sort();
    versions.reverse();
    for ver in versions {
        let candidate = ver.join("bin").join("nvcc.exe");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn find_cl() -> Option<PathBuf> {
    if let Ok(custom) = env::var("DAMASCUS_CL_PATH") {
        let candidate = PathBuf::from(custom);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    if let Some(path_cl) = find_on_path("cl.exe") {
        return Some(path_cl);
    }

    let mut roots = Vec::new();
    if let Ok(vs_install_dir) = env::var("VSINSTALLDIR") {
        roots.push(PathBuf::from(vs_install_dir));
    }
    roots.push(PathBuf::from("C:\\Program Files\\Microsoft Visual Studio"));
    roots.push(PathBuf::from(
        "C:\\Program Files (x86)\\Microsoft Visual Studio",
    ));
    roots.push(PathBuf::from("D:\\VS studio"));

    for root in roots {
        if let Some(cl_path) = find_cl_in_visual_studio_root(&root) {
            return Some(cl_path);
        }
    }
    None
}

fn find_cl_in_visual_studio_root(root: &Path) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }

    let mut candidate_installs = Vec::new();
    if root.join("VC").join("Tools").join("MSVC").exists() {
        candidate_installs.push(root.to_path_buf());
    }

    for level1 in fs::read_dir(root).ok()? {
        let p1 = level1.ok()?.path();
        if !p1.is_dir() {
            continue;
        }
        if p1.join("VC").join("Tools").join("MSVC").exists() {
            candidate_installs.push(p1.clone());
        }
        for level2 in fs::read_dir(&p1).ok().into_iter().flatten() {
            let p2 = level2.ok()?.path();
            if p2.is_dir() && p2.join("VC").join("Tools").join("MSVC").exists() {
                candidate_installs.push(p2);
            }
        }
    }

    for install in candidate_installs {
        let msvc_dir = install.join("VC").join("Tools").join("MSVC");
        let mut versions: Vec<PathBuf> = fs::read_dir(&msvc_dir)
            .ok()?
            .filter_map(|e| e.ok().map(|x| x.path()))
            .filter(|p| p.is_dir())
            .collect();
        versions.sort();
        versions.reverse();
        for ver in versions {
            let cl = ver.join("bin").join("Hostx64").join("x64").join("cl.exe");
            if cl.exists() {
                return Some(cl);
            }
        }
    }
    None
}

fn find_on_path(file_name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|p| p.join(file_name))
        .find(|p| p.exists())
}

fn cuda_lib_dir_from_nvcc(nvcc: &Path) -> Option<PathBuf> {
    let bin_dir = nvcc.parent()?;
    let cuda_root = bin_dir.parent()?;
    let lib_dir = cuda_root.join("lib").join("x64");
    lib_dir.exists().then_some(lib_dir)
}
