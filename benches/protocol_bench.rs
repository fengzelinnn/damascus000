use anyhow::{anyhow, ensure, Context, Result};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use damascus_core::{
    DamascusProver, DamascusVerifier, MicroBlock, ModuleCommitment, RuntimeConfig, SystemParams,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct VerifyScenario {
    id: String,
    params: SystemParams,
    initial_commitment: ModuleCommitment,
    micro_blocks: Vec<MicroBlock>,
}

#[derive(Clone)]
struct SummaryRow {
    scenario_id: String,
    file_path: String,
    file_size_bytes: u64,
    ntt_mode: &'static str,
    gpu_mode: &'static str,
    vector_len: usize,
    poly_len: usize,
    rounds: usize,
    preprocess_ms: f64,
    preprocess_bytes: u64,
    preprocess_mib_s: f64,
    fold_total_ms: f64,
    fold_total_bytes: u64,
    fold_total_mib_s: f64,
    verify_total_ms: f64,
    verify_payload_bytes: u64,
    verify_mib_s: f64,
    end_to_end_ms: f64,
    total_measured_bytes: u64,
    overall_mib_s: f64,
}

#[derive(Clone)]
struct StageRow {
    scenario_id: String,
    file_path: String,
    file_size_bytes: u64,
    ntt_mode: &'static str,
    gpu_mode: &'static str,
    round: Option<usize>,
    stage: &'static str,
    duration_ms: f64,
    data_bytes: u64,
    throughput_mib_s: f64,
    payload_bytes: u64,
}

struct ScenarioOutcome {
    verify_scenario: VerifyScenario,
    summary_row: SummaryRow,
    stage_rows: Vec<StageRow>,
}

struct BenchmarkRun {
    verify_scenarios: Vec<VerifyScenario>,
    summary_rows: Vec<SummaryRow>,
    stage_rows: Vec<StageRow>,
}

fn protocol_benchmark(c: &mut Criterion) {
    let run = collect_benchmark_data().expect("collect benchmark data");
    write_reports(&run.summary_rows, &run.stage_rows).expect("write benchmark reports");

    let mut group = c.benchmark_group("damascus_protocol");
    for scenario in run.verify_scenarios {
        let id = scenario.id.clone();
        let params = scenario.params.clone();
        let initial_commitment = scenario.initial_commitment.clone();
        let micro_blocks = scenario.micro_blocks.clone();

        group.bench_function(BenchmarkId::new("verify_all_rounds", id), |b| {
            b.iter(|| {
                let mut verifier =
                    DamascusVerifier::new(params.clone(), initial_commitment.clone());
                for block in &micro_blocks {
                    verifier
                        .update_commitment(black_box(block))
                        .expect("verify update should succeed");
                }
            });
        });
    }
    group.finish();
}

fn collect_benchmark_data() -> Result<BenchmarkRun> {
    let files = collect_input_files()?;
    ensure!(
        !files.is_empty(),
        "no benchmark files resolved from DAMASCUS_BENCH_FILES/DAMASCUS_BENCH_FILE_LIST"
    );

    let gpu_min_elements = env::var("DAMASCUS_GPU_MIN_ELEMS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(16_777_216);

    let mut verify_scenarios = Vec::new();
    let mut summary_rows = Vec::new();
    let mut stage_rows = Vec::new();

    for file_path in files {
        for ntt_enabled in [true, false] {
            for gpu_enabled in [true, false] {
                let outcome =
                    run_single_scenario(&file_path, ntt_enabled, gpu_enabled, gpu_min_elements)?;
                verify_scenarios.push(outcome.verify_scenario);
                summary_rows.push(outcome.summary_row);
                stage_rows.extend(outcome.stage_rows);
            }
        }
    }

    Ok(BenchmarkRun {
        verify_scenarios,
        summary_rows,
        stage_rows,
    })
}

fn run_single_scenario(
    file_path: &Path,
    ntt_enabled: bool,
    gpu_enabled: bool,
    gpu_min_elements: usize,
) -> Result<ScenarioOutcome> {
    let file_meta = fs::metadata(file_path)
        .with_context(|| format!("read metadata for {}", file_path.display()))?;
    ensure!(file_meta.is_file(), "{} is not a file", file_path.display());
    let file_size_bytes = file_meta.len();

    let derived = derive_layout(file_size_bytes);
    let params = params_from_layout(&derived);
    let runtime = RuntimeConfig {
        ntt_enabled,
        parallel_enabled: true,
        gpu_enabled,
        gpu_min_elements,
    };

    let ntt_mode = if ntt_enabled { "ON" } else { "OFF" };
    let gpu_mode = if gpu_enabled { "ON" } else { "OFF" };
    let scenario_id = format!(
        "{}_ntt{}_gpu{}",
        sanitize_id(file_path),
        ntt_mode.to_lowercase(),
        gpu_mode.to_lowercase()
    );
    eprintln!(
        "[bench] scenario={} file={} size={} ntt={} gpu={}",
        scenario_id,
        file_path.display(),
        file_size_bytes,
        ntt_mode,
        gpu_mode
    );

    let preprocess_start = Instant::now();
    let mut prover = DamascusProver::initialize_with_config(file_path, params.clone(), runtime)
        .with_context(|| format!("initialize prover for {}", file_path.display()))?;
    let preprocess_duration = preprocess_start.elapsed();

    let initial_commitment = prover.current_commitment().clone();
    let mut verifier = DamascusVerifier::new(params.clone(), initial_commitment.clone());

    let mut rows = Vec::new();
    rows.push(stage_row(
        &scenario_id,
        file_path,
        file_size_bytes,
        ntt_mode,
        gpu_mode,
        None,
        "preprocess",
        preprocess_duration,
        file_size_bytes,
        0,
    ));

    let mut micro_blocks = Vec::new();
    let mut round_vector_len = params.vector_len;
    let mut round_poly_len = params.poly_len;

    let mut fold_total_duration = Duration::ZERO;
    let mut fold_total_bytes = 0u64;
    let mut verify_total_duration = Duration::ZERO;
    let mut verify_payload_bytes = 0u64;

    for round in 0..prover.rounds_total() {
        let round_output = prover
            .fold_round(round)
            .with_context(|| format!("fold round {}", round))?;

        let verify_start = Instant::now();
        verifier
            .update_commitment(&round_output.micro_block)
            .with_context(|| format!("verify round {}", round))?;
        let verify_duration = verify_start.elapsed();

        let vector_stage_bytes = logical_tensor_bytes(round_vector_len, round_poly_len)?;
        let poly_stage_bytes = logical_tensor_bytes(round_vector_len / 2, round_poly_len)?;
        let fold_round_bytes = vector_stage_bytes.saturating_add(poly_stage_bytes);
        let payload_bytes = bincode::serialize(&round_output.micro_block)
            .context("serialize micro-block")?
            .len() as u64;

        rows.push(stage_row(
            &scenario_id,
            file_path,
            file_size_bytes,
            ntt_mode,
            gpu_mode,
            Some(round),
            "vector_fold",
            round_output.vector_fold_time,
            vector_stage_bytes,
            payload_bytes,
        ));
        rows.push(stage_row(
            &scenario_id,
            file_path,
            file_size_bytes,
            ntt_mode,
            gpu_mode,
            Some(round),
            "poly_fold",
            round_output.poly_fold_time,
            poly_stage_bytes,
            payload_bytes,
        ));
        rows.push(stage_row(
            &scenario_id,
            file_path,
            file_size_bytes,
            ntt_mode,
            gpu_mode,
            Some(round),
            "fold_total",
            round_output.total_round_time,
            fold_round_bytes,
            payload_bytes,
        ));
        rows.push(stage_row(
            &scenario_id,
            file_path,
            file_size_bytes,
            ntt_mode,
            gpu_mode,
            Some(round),
            "verify",
            verify_duration,
            payload_bytes,
            payload_bytes,
        ));

        fold_total_duration += round_output.total_round_time;
        verify_total_duration += verify_duration;
        fold_total_bytes = fold_total_bytes.saturating_add(fold_round_bytes);
        verify_payload_bytes = verify_payload_bytes.saturating_add(payload_bytes);

        micro_blocks.push(round_output.micro_block);
        round_vector_len /= 2;
        round_poly_len /= 2;
    }

    ensure!(
        prover.current_commitment() == verifier.current_commitment(),
        "final commitment mismatch between prover and verifier"
    );

    let end_to_end_duration = preprocess_duration + fold_total_duration + verify_total_duration;
    let total_measured_bytes = file_size_bytes
        .saturating_add(fold_total_bytes)
        .saturating_add(verify_payload_bytes);
    let summary = SummaryRow {
        scenario_id: scenario_id.clone(),
        file_path: file_path.display().to_string(),
        file_size_bytes,
        ntt_mode,
        gpu_mode,
        vector_len: params.vector_len,
        poly_len: params.poly_len,
        rounds: params.rounds,
        preprocess_ms: ms(preprocess_duration),
        preprocess_bytes: file_size_bytes,
        preprocess_mib_s: throughput_mib_per_s(file_size_bytes, preprocess_duration),
        fold_total_ms: ms(fold_total_duration),
        fold_total_bytes,
        fold_total_mib_s: throughput_mib_per_s(fold_total_bytes, fold_total_duration),
        verify_total_ms: ms(verify_total_duration),
        verify_payload_bytes,
        verify_mib_s: throughput_mib_per_s(verify_payload_bytes, verify_total_duration),
        end_to_end_ms: ms(end_to_end_duration),
        total_measured_bytes,
        overall_mib_s: throughput_mib_per_s(total_measured_bytes, end_to_end_duration),
    };

    Ok(ScenarioOutcome {
        verify_scenario: VerifyScenario {
            id: scenario_id,
            params,
            initial_commitment,
            micro_blocks,
        },
        summary_row: summary,
        stage_rows: rows,
    })
}

fn collect_input_files() -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    if let Ok(raw) = env::var("DAMASCUS_BENCH_FILES") {
        paths.extend(parse_path_list(&raw));
    }

    if let Ok(list_file) = env::var("DAMASCUS_BENCH_FILE_LIST") {
        let content = fs::read_to_string(&list_file)
            .with_context(|| format!("read DAMASCUS_BENCH_FILE_LIST file {}", list_file))?;
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                paths.push(PathBuf::from(trimmed));
            }
        }
    }

    ensure!(
        !paths.is_empty(),
        "set DAMASCUS_BENCH_FILES (use ';', ',' or newline separators), or DAMASCUS_BENCH_FILE_LIST"
    );

    let mut resolved = Vec::new();
    for path in paths {
        let meta = fs::metadata(&path)
            .with_context(|| format!("benchmark input not found: {}", path.display()))?;
        ensure!(
            meta.is_file(),
            "benchmark input is not a file: {}",
            path.display()
        );
        resolved.push(path);
    }
    Ok(resolved)
}

fn parse_path_list(raw: &str) -> Vec<PathBuf> {
    raw.split(|c| c == ';' || c == ',' || c == '\n' || c == '\r')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn stage_row(
    scenario_id: &str,
    file_path: &Path,
    file_size_bytes: u64,
    ntt_mode: &'static str,
    gpu_mode: &'static str,
    round: Option<usize>,
    stage: &'static str,
    duration: Duration,
    data_bytes: u64,
    payload_bytes: u64,
) -> StageRow {
    StageRow {
        scenario_id: scenario_id.to_string(),
        file_path: file_path.display().to_string(),
        file_size_bytes,
        ntt_mode,
        gpu_mode,
        round,
        stage,
        duration_ms: ms(duration),
        data_bytes,
        throughput_mib_s: throughput_mib_per_s(data_bytes, duration),
        payload_bytes,
    }
}

fn write_reports(summary_rows: &[SummaryRow], stage_rows: &[StageRow]) -> Result<()> {
    let report_dir = Path::new("target/bench-reports");
    if !report_dir.exists() {
        fs::create_dir_all(report_dir).context("create bench report directory")?;
    }

    let summary_csv_path = report_dir.join("protocol_metrics_summary.csv");
    let summary_md_path = report_dir.join("protocol_metrics_summary.md");
    let stage_csv_path = report_dir.join("protocol_metrics_stages.csv");
    let stage_md_path = report_dir.join("protocol_metrics_stages.md");

    let mut summary_csv = String::from(
        "Scenario,File Path,File Size (Bytes),NTT,GPU,Vector Len,Poly Len,Rounds,Preprocess (ms),Preprocess Bytes,Preprocess Throughput (MiB/s),Fold Total (ms),Fold Total Bytes,Fold Throughput (MiB/s),Verify Total (ms),Verify Payload Bytes,Verify Throughput (MiB/s),End-to-End (ms),Total Measured Bytes,Overall Throughput (MiB/s)\n",
    );
    for row in summary_rows {
        summary_csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{:.3},{},{:.3},{:.3},{},{:.3},{:.3},{},{:.3},{:.3},{},{}\n",
            csv_escape(&row.scenario_id),
            csv_escape(&row.file_path),
            row.file_size_bytes,
            row.ntt_mode,
            row.gpu_mode,
            row.vector_len,
            row.poly_len,
            row.rounds,
            row.preprocess_ms,
            row.preprocess_bytes,
            row.preprocess_mib_s,
            row.fold_total_ms,
            row.fold_total_bytes,
            row.fold_total_mib_s,
            row.verify_total_ms,
            row.verify_payload_bytes,
            row.verify_mib_s,
            row.end_to_end_ms,
            row.total_measured_bytes,
            format!("{:.3}", row.overall_mib_s)
        ));
    }
    fs::write(&summary_csv_path, summary_csv)
        .with_context(|| format!("write {}", summary_csv_path.display()))?;

    let mut summary_md = String::from(
        "| Scenario | File | Size | NTT | GPU | Rounds | Preprocess (ms) | Fold Total (ms) | Verify Total (ms) | End-to-End (ms) | Overall Throughput (MiB/s) |\n",
    );
    summary_md.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for row in summary_rows {
        summary_md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |\n",
            row.scenario_id,
            row.file_path,
            human_size(row.file_size_bytes),
            row.ntt_mode,
            row.gpu_mode,
            row.rounds,
            row.preprocess_ms,
            row.fold_total_ms,
            row.verify_total_ms,
            row.end_to_end_ms,
            row.overall_mib_s
        ));
    }
    fs::write(&summary_md_path, summary_md)
        .with_context(|| format!("write {}", summary_md_path.display()))?;

    let mut stage_csv = String::from(
        "Scenario,File Path,File Size (Bytes),NTT,GPU,Round,Stage,Duration (ms),Data Bytes,Throughput (MiB/s),Payload Bytes\n",
    );
    for row in stage_rows {
        let round_label = row
            .round
            .map(|r| r.to_string())
            .unwrap_or_else(|| "-".to_string());
        stage_csv.push_str(&format!(
            "{},{},{},{},{},{},{},{:.3},{},{:.3},{}\n",
            csv_escape(&row.scenario_id),
            csv_escape(&row.file_path),
            row.file_size_bytes,
            row.ntt_mode,
            row.gpu_mode,
            round_label,
            row.stage,
            row.duration_ms,
            row.data_bytes,
            row.throughput_mib_s,
            row.payload_bytes
        ));
    }
    fs::write(&stage_csv_path, stage_csv)
        .with_context(|| format!("write {}", stage_csv_path.display()))?;

    let mut stage_md = String::from(
        "| Scenario | Round | Stage | Duration (ms) | Data Bytes | Throughput (MiB/s) | Payload Bytes |\n",
    );
    stage_md.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
    for row in stage_rows {
        let round_label = row
            .round
            .map(|r| r.to_string())
            .unwrap_or_else(|| "-".to_string());
        stage_md.push_str(&format!(
            "| {} | {} | {} | {:.3} | {} | {:.3} | {} |\n",
            row.scenario_id,
            round_label,
            row.stage,
            row.duration_ms,
            row.data_bytes,
            row.throughput_mib_s,
            row.payload_bytes
        ));
    }
    fs::write(&stage_md_path, stage_md)
        .with_context(|| format!("write {}", stage_md_path.display()))?;

    Ok(())
}

fn logical_tensor_bytes(vector_len: usize, poly_len: usize) -> Result<u64> {
    let elements = (vector_len as u128)
        .checked_mul(poly_len as u128)
        .ok_or_else(|| anyhow!("tensor size overflow: {} x {}", vector_len, poly_len))?;
    let bytes = elements.checked_mul(8).ok_or_else(|| {
        anyhow!(
            "byte size overflow for tensor {} x {}",
            vector_len,
            poly_len
        )
    })?;
    u64::try_from(bytes).map_err(|_| anyhow!("tensor bytes exceed u64"))
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn throughput_mib_per_s(bytes: u64, duration: Duration) -> f64 {
    let secs = duration.as_secs_f64();
    if secs == 0.0 {
        0.0
    } else {
        bytes as f64 / (1024.0 * 1024.0) / secs
    }
}

fn sanitize_id(path: &Path) -> String {
    let raw = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("input_file");
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
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

#[derive(Clone, Copy, Debug)]
struct DerivedLayout {
    vector_len: usize,
    poly_len: usize,
    rounds: usize,
}

fn derive_layout(input_size_bytes: u64) -> DerivedLayout {
    let words =
        usize::try_from((input_size_bytes.saturating_add(7) / 8).max(1)).unwrap_or(usize::MAX);

    let configured_poly = env::var("DAMASCUS_POLY_LEN")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1024);
    let padded_poly_len = pad_pow2(configured_poly.max(2));
    let raw_vector_len = words.div_ceil(padded_poly_len).max(2);
    let padded_vector_len = pad_pow2(raw_vector_len);
    let configured_max_vector = env::var("DAMASCUS_MAX_TENSOR_DIM")
        .ok()
        .or_else(|| env::var("DAMASCUS_MAX_VECTOR_LEN").ok())
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 2)
        .unwrap_or(16_384);
    let max_vector_len = floor_pow2(configured_max_vector).max(2);
    let square_len = padded_vector_len.max(padded_poly_len).min(max_vector_len);
    let rounds = floor_log2(square_len);

    DerivedLayout {
        vector_len: square_len,
        poly_len: square_len,
        rounds,
    }
}

fn params_from_layout(layout: &DerivedLayout) -> SystemParams {
    SystemParams {
        module_rank: 4,
        vector_len: layout.vector_len,
        poly_len: layout.poly_len,
        rounds: layout.rounds,
        seed_generators: [29u8; 32],
    }
}

fn floor_log2(x: usize) -> usize {
    if x <= 1 {
        0
    } else {
        (usize::BITS as usize - 1) - (x.leading_zeros() as usize)
    }
}

fn pad_pow2(value: usize) -> usize {
    if value <= 1 {
        1
    } else {
        value
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX / 2 + 1)
    }
}

fn floor_pow2(value: usize) -> usize {
    if value <= 1 {
        1
    } else {
        1usize << floor_log2(value)
    }
}

criterion_group!(benches, protocol_benchmark);
criterion_main!(benches);
