use anyhow::{Context, Result};
use damascus_core::utils::gpu;
use damascus_core::utils::{
    config::{MODULE_RANK, POLY_DEGREE},
    io::{coeff_count_for_byte_len, vector_len_for_file_size},
};
use damascus_core::{DamascusProver, DamascusVerifier, RuntimeConfig, SystemParams};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() -> Result<()> {
    let file_path = match env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => create_default_input_file()?,
    };

    let input_size_bytes = fs::metadata(&file_path)
        .with_context(|| format!("read metadata for {}", file_path.display()))?
        .len();
    let derived = derive_layout(input_size_bytes);
    let params = params_from_layout(&derived);

    let ntt_enabled = env::var("DAMASCUS_NTT").map(|v| v != "0").unwrap_or(true);
    let gpu_enabled = env::var("DAMASCUS_GPU").map(|v| v != "0").unwrap_or(true);
    let gpu_min_elements = env::var("DAMASCUS_GPU_MIN_ELEMS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2_097_152);
    let runtime = RuntimeConfig {
        max_preprocess_bytes: usize::MAX,
        ntt_enabled,
        parallel_enabled: true,
        gpu_enabled,
        gpu_min_elements,
    };

    println!("input: {}", file_path.display());
    println!(
        "input_size: {} bytes ({})",
        input_size_bytes,
        human_size(input_size_bytes)
    );
    println!(
        "derived params: coeff_count={} vector_len={} poly_len={} rounds={} module_rank={}",
        derived.coeff_count, derived.vector_len, derived.poly_len, derived.rounds, MODULE_RANK
    );

    let preprocess_start = Instant::now();
    let mut prover = DamascusProver::initialize_with_config(&file_path, params.clone(), runtime)?;
    let preprocess_time = preprocess_start.elapsed();
    let mut verifier =
        DamascusVerifier::new(params, prover.statement().clone()).context("new verifier")?;

    println!(
        "preprocess: {:.3}ms",
        preprocess_time.as_secs_f64() * 1_000.0
    );
    println!("rounds: {}", prover.rounds_total());
    println!("ntt_enabled: {}", ntt_enabled);
    println!("gpu_enabled: {}", gpu_enabled);
    println!("gpu_min_elements: {}", gpu_min_elements);
    let gpu_info = gpu::cuda_device_info();
    println!(
        "gpu_probe: available={} compiled_backend={} detail={}",
        gpu_info.available,
        gpu::cuda_backend_ready(),
        gpu_info.summary
    );

    let mut total_fold_ms = 0.0;
    let mut total_verify_ms = 0.0;
    for round in 0..prover.rounds_total() {
        let out = prover
            .fold_round(round)
            .with_context(|| format!("fold round {}", round))?;
        let verify_start = Instant::now();
        verifier
            .update_commitment(&out.micro_block)
            .with_context(|| format!("verify round {}", round))?;
        let verify_time = verify_start.elapsed();

        let block_size = bincode::serialize(&out.micro_block)
            .context("serialize micro-block")?
            .len();
        total_fold_ms += out.total_round_time.as_secs_f64() * 1_000.0;
        total_verify_ms += verify_time.as_secs_f64() * 1_000.0;
        println!(
            "round={} fold(vec={:.3}ms poly={:.3}ms total={:.3}ms) verify={:.3}ms micro_block={}B",
            round,
            out.vector_fold_time.as_secs_f64() * 1_000.0,
            out.poly_fold_time.as_secs_f64() * 1_000.0,
            out.total_round_time.as_secs_f64() * 1_000.0,
            verify_time.as_secs_f64() * 1_000.0,
            block_size
        );
    }

    if prover.current_commitment() != verifier.current_commitment() {
        anyhow::bail!("final commitment mismatch between prover and verifier");
    }
    if let Some(opening) = prover.final_opening() {
        verifier
            .verify_final_opening(&opening)
            .context("verify final opening")?;
    }

    println!(
        "summary: fold_total={:.3}ms verify_total={:.3}ms",
        total_fold_ms, total_verify_ms
    );
    println!("verification successful");
    Ok(())
}

fn create_default_input_file() -> Result<PathBuf> {
    let target_dir = Path::new("target");
    if !target_dir.exists() {
        fs::create_dir_all(target_dir).context("create target dir")?;
    }

    let file_path = target_dir.join("full_flow_input.bin");
    let mut data = Vec::new();
    for i in 0..(1024 * 1024) {
        data.push((i % 251) as u8);
    }
    fs::write(&file_path, data).with_context(|| format!("write {}", file_path.display()))?;
    Ok(file_path)
}

#[derive(Clone, Copy, Debug)]
struct DerivedLayout {
    coeff_count: usize,
    vector_len: usize,
    poly_len: usize,
    rounds: usize,
}

fn derive_layout(input_size_bytes: u64) -> DerivedLayout {
    let coeff_count = coeff_count_for_byte_len(input_size_bytes);
    let vector_len = vector_len_for_file_size(input_size_bytes).unwrap_or(1);
    let poly_len = POLY_DEGREE;
    let rounds = floor_log2(vector_len.max(poly_len));

    DerivedLayout {
        coeff_count,
        vector_len,
        poly_len,
        rounds,
    }
}

fn params_from_layout(layout: &DerivedLayout) -> SystemParams {
    SystemParams {
        module_rank: MODULE_RANK,
        vector_len: layout.vector_len,
        poly_len: layout.poly_len,
        rounds: layout.rounds,
        seed_generators: [11u8; 32],
        epoch_seed: [13u8; 32],
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit_idx = 0usize;
    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    format!("{value:.2} {}", UNITS[unit_idx])
}

fn floor_log2(x: usize) -> usize {
    if x <= 1 {
        0
    } else {
        (usize::BITS as usize - 1) - (x.leading_zeros() as usize)
    }
}
