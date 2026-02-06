use anyhow::{Context, Result};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use damascus_core::utils::config::BenchRow;
use damascus_core::{DamascusProver, DamascusVerifier, RuntimeConfig, SystemParams};
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MB: u64 = 1024 * 1024;
const FILE_SIZES: &[(u64, &str)] = &[
    (100 * MB, "100 MB"),
    (500 * MB, "500 MB"),
    (1024 * MB, "1 GB"),
    (2 * 1024 * MB, "2 GB"),
    (4 * 1024 * MB, "4 GB"),
];

#[derive(Clone)]
struct ScenarioRecord {
    label: String,
    ntt_mode: String,
    params: SystemParams,
    initial_commitment: damascus_core::ModuleCommitment,
    micro_block: damascus_core::MicroBlock,
    row: BenchRow,
}

fn protocol_benchmark(c: &mut Criterion) {
    let scenarios = collect_scenarios().expect("collect benchmark scenarios");

    write_reports(&scenarios.iter().map(|s| s.row.clone()).collect::<Vec<_>>())
        .expect("write benchmark reports");

    let mut group = c.benchmark_group("damascus_protocol");
    for scenario in scenarios {
        let id = format!("{}_{}", scenario.label.replace(' ', ""), scenario.ntt_mode);
        let params = scenario.params.clone();
        let initial_commitment = scenario.initial_commitment.clone();
        let micro_block = scenario.micro_block.clone();

        group.bench_function(BenchmarkId::new("verify_update", id), |b| {
            b.iter(|| {
                let mut verifier =
                    DamascusVerifier::new(params.clone(), initial_commitment.clone());
                verifier
                    .update_commitment(black_box(&micro_block))
                    .expect("verify update should succeed");
            });
        });
    }
    group.finish();
}

fn collect_scenarios() -> Result<Vec<ScenarioRecord>> {
    let mut records = Vec::new();
    for &(size, label) in FILE_SIZES {
        let file_path = ensure_bench_file(size)?;
        for ntt_enabled in [true, false] {
            records.push(run_single_scenario(&file_path, label, ntt_enabled)?);
        }
    }
    Ok(records)
}

fn run_single_scenario(file_path: &Path, label: &str, ntt_enabled: bool) -> Result<ScenarioRecord> {
    let params = SystemParams {
        module_rank: 4,
        vector_len: 1024,
        poly_len: 1024,
        rounds: 1,
        seed_generators: [29u8; 32],
    };

    let runtime = RuntimeConfig {
        ntt_enabled,
        parallel_enabled: true,
    };

    let preprocess_start = std::time::Instant::now();
    let mut prover = DamascusProver::initialize_with_config(file_path, params.clone(), runtime)
        .with_context(|| format!("initialize prover for {}", file_path.display()))?;
    let preprocessing_s = preprocess_start.elapsed().as_secs_f64();

    let initial_commitment = prover.current_commitment().clone();
    let mut verifier = DamascusVerifier::new(params.clone(), initial_commitment.clone());
    let round_output = prover.fold_round(0).context("run fold round 0")?;

    let verify_start = std::time::Instant::now();
    verifier
        .update_commitment(&round_output.micro_block)
        .context("verifier update")?;
    let verify_us = verify_start.elapsed().as_secs_f64() * 1_000_000.0;

    let cross_term_payload = (
        &round_output.micro_block.left_vector_commitment,
        &round_output.micro_block.right_vector_commitment,
        &round_output.micro_block.even_poly_commitment,
        &round_output.micro_block.odd_poly_commitment,
    );
    let cross_term_size_bytes = bincode::serialize(&cross_term_payload)
        .context("serialize cross-term payload")?
        .len();

    let row = BenchRow {
        file_size_label: label.to_string(),
        ntt_mode: if ntt_enabled { "ON" } else { "OFF" }.to_string(),
        preprocessing_s,
        vec_fold_ms: round_output.vector_fold_time.as_secs_f64() * 1_000.0,
        poly_fold_ms: round_output.poly_fold_time.as_secs_f64() * 1_000.0,
        verify_us,
        cross_term_size_bytes,
    };

    Ok(ScenarioRecord {
        label: label.to_string(),
        ntt_mode: row.ntt_mode.clone(),
        params,
        initial_commitment,
        micro_block: round_output.micro_block,
        row,
    })
}

fn ensure_bench_file(size: u64) -> Result<PathBuf> {
    let dir = PathBuf::from("target/bench-data");
    if !dir.exists() {
        fs::create_dir_all(&dir).context("create benchmark input directory")?;
    }

    let path = dir.join(format!("input_{}mb.bin", size / MB));
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() == size {
            return Ok(path);
        }
    }

    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;

    file.set_len(size)
        .with_context(|| format!("set file length for {}", path.display()))?;

    let chunk_len = usize::try_from(size.min(MB)).expect("chunk_len fits usize");
    let mut chunk = vec![0u8; chunk_len];
    for (i, b) in chunk.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&chunk)?;
    if size > chunk_len as u64 {
        file.seek(SeekFrom::Start(size - chunk_len as u64))?;
        file.write_all(&chunk)?;
    }
    file.flush()?;
    Ok(path)
}

fn write_reports(rows: &[BenchRow]) -> Result<()> {
    let report_dir = Path::new("target/bench-reports");
    if !report_dir.exists() {
        fs::create_dir_all(report_dir).context("create bench report directory")?;
    }

    let csv_path = report_dir.join("protocol_metrics.csv");
    let md_path = report_dir.join("protocol_metrics.md");

    let mut csv = String::from(
        "File Size,Mode (NTT),Preprocessing (s),Vec Fold (ms),Poly Fold (ms),Verify (us),Cross-Term Size (Bytes)\n",
    );
    for row in rows {
        csv.push_str(&format!(
            "{},{},{:.6},{:.3},{:.3},{:.3},{}\n",
            row.file_size_label,
            row.ntt_mode,
            row.preprocessing_s,
            row.vec_fold_ms,
            row.poly_fold_ms,
            row.verify_us,
            row.cross_term_size_bytes
        ));
    }
    fs::write(&csv_path, csv).with_context(|| format!("write {}", csv_path.display()))?;

    let mut md = String::from(
        "| File Size | Mode (NTT) | Preprocessing (s) | Vec Fold (ms) | Poly Fold (ms) | Verify (us) | Cross-Term Size (Bytes) |\n",
    );
    md.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
    for row in rows {
        md.push_str(&format!(
            "| {} | {} | {:.6} | {:.3} | {:.3} | {:.3} | {} |\n",
            row.file_size_label,
            row.ntt_mode,
            row.preprocessing_s,
            row.vec_fold_ms,
            row.poly_fold_ms,
            row.verify_us,
            row.cross_term_size_bytes
        ));
    }
    fs::write(&md_path, md).with_context(|| format!("write {}", md_path.display()))?;

    Ok(())
}

criterion_group!(benches, protocol_benchmark);
criterion_main!(benches);
