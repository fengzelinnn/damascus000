use anyhow::Context as _;
use clap::{Parser, Subcommand, ValueEnum};
use damascus_conv::{ConvParams, ConvProver, ConvPublicState, ConvWitness};
use damascus_ring::{ModuleElem, Poly};
use damascus_types::FileId;
use rand::RngCore as _;
use rand_chacha::rand_core::SeedableRng as _;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "damascus")]
#[command(about = "Damascus workspace runner (bench + sim)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one or more flows (sim/bench) in-process.
    Run(RunArgs),
    /// Print the convolution parameters as JSON.
    Params(ParamsArgs),
}

#[derive(Parser, Clone, Debug)]
struct ParamsArgs {
    #[arg(long, default_value_t = 998_244_353)]
    q: u64,
    #[arg(long, default_value_t = 256)]
    n0: usize,
    #[arg(long, default_value_t = 8)]
    rounds: usize,
    #[arg(long, default_value_t = 2)]
    k: usize,
    #[arg(
        long,
        default_value = "0000000000000000000000000000000000000000000000000000000000000000"
    )]
    seed: String,
}

#[derive(Parser, Clone, Debug)]
struct RunArgs {
    /// Which flows to run. Supports comma-delimited values, e.g. `--flow sim-epoch,bench-conv-round`.
    #[arg(long, value_enum, value_delimiter = ',', required = true)]
    flow: Vec<Flow>,

    /// Input file, required for `sim-epoch`.
    #[arg(long)]
    file: Option<String>,
    #[arg(long, default_value_t = 0)]
    epoch: u64,

    // Common Conv parameters.
    #[arg(long, default_value_t = 998_244_353)]
    q: u64,
    #[arg(long, default_value_t = 256)]
    n0: usize,
    #[arg(long, default_value_t = 8)]
    rounds: usize,
    #[arg(long, default_value_t = 2)]
    k: usize,
    #[arg(
        long,
        default_value = "0000000000000000000000000000000000000000000000000000000000000000"
    )]
    seed: String,

    // sim-epoch options.
    #[arg(long, default_value_t = 1)]
    repeat: usize,

    // bench-conv-round options.
    #[arg(long, default_value_t = 50)]
    iters: usize,
    /// Which round index `j` to benchmark (0-based).
    #[arg(long, default_value_t = 0)]
    round: u32,
    #[arg(
        long,
        default_value = "0909090909090909090909090909090909090909090909090909090909090909"
    )]
    witness_seed: String,
    #[arg(
        long,
        default_value = "0303030303030303030303030303030303030303030303030303030303030303"
    )]
    file_id: String,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Flow {
    /// Full epoch simulation + final verification.
    SimEpoch,
    /// Microbench a single `ConvProver::round` at index `j`.
    BenchConvRound,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Command::Run(args) => run(args),
        Command::Params(args) => {
            let params = conv_params_from_args(args.q, args.n0, args.rounds, args.k, &args.seed)?;
            println!("{}", serde_json::to_string_pretty(&params)?);
            Ok(())
        }
    }
}

fn run(args: RunArgs) -> anyhow::Result<()> {
    anyhow::ensure!(!args.flow.is_empty(), "--flow must not be empty");
    let params = conv_params_from_args(args.q, args.n0, args.rounds, args.k, &args.seed)?;

    let needs_file = args.flow.iter().any(|f| matches!(f, Flow::SimEpoch));
    let file_bytes = if needs_file {
        let path = args
            .file
            .as_ref()
            .context("--file is required for --flow sim-epoch")?;
        Some(std::fs::read(path).with_context(|| format!("read file: {path}"))?)
    } else {
        None
    };

    for flow in args.flow {
        match flow {
            Flow::SimEpoch => {
                let bytes = file_bytes.as_deref().expect("checked above");
                run_sim_epoch(&params, bytes, args.epoch, args.repeat)?;
            }
            Flow::BenchConvRound => {
                let witness_seed = parse_32byte_hex(&args.witness_seed, "witness-seed")?;
                let file_id = FileId::from_bytes(parse_32byte_hex(&args.file_id, "file-id")?);
                bench_conv_round(
                    &params,
                    file_id,
                    args.epoch,
                    args.round,
                    args.iters,
                    witness_seed,
                )?;
            }
        }
    }

    Ok(())
}

fn conv_params_from_args(
    q: u64,
    n0: usize,
    rounds: usize,
    k: usize,
    seed: &str,
) -> anyhow::Result<ConvParams> {
    let params = ConvParams {
        q,
        n0,
        n_rounds: rounds,
        k,
        seed_generators: parse_32byte_hex(seed, "seed")?,
    };
    params.validate().context("invalid parameters")?;
    Ok(params)
}

fn run_sim_epoch(
    params: &ConvParams,
    file_bytes: &[u8],
    epoch: u64,
    repeat: usize,
) -> anyhow::Result<()> {
    anyhow::ensure!(repeat >= 1, "--repeat must be >= 1");

    let start = Instant::now();
    let mut last = None;
    for _ in 0..repeat {
        last = Some(damascus_sim::run_epoch(*params, file_bytes, epoch)?);
    }
    let elapsed = start.elapsed();

    let res = last.expect("repeat >= 1");
    println!("flow=sim-epoch");
    println!("file_id={}", res.file_id);
    println!("epoch={}", res.epoch);
    println!("rounds={}", res.transcript.len());
    println!("opening_mu0={}", res.opening.0.coeff0());
    println!("opening_rho0={}", res.opening.1.coeff0());
    if repeat > 1 {
        let per = elapsed.as_secs_f64() / (repeat as f64);
        println!(
            "repeat={repeat} total_s={:.6} per_s={:.6}",
            elapsed.as_secs_f64(),
            per
        );
    }
    Ok(())
}

fn bench_conv_round(
    params: &ConvParams,
    file_id: FileId,
    epoch: u64,
    round: u32,
    iters: usize,
    witness_seed: [u8; 32],
) -> anyhow::Result<()> {
    anyhow::ensure!(iters >= 1, "--iters must be >= 1");
    anyhow::ensure!(
        (round as usize) < params.n_rounds,
        "--round must be < n_rounds (got {round}, n_rounds={})",
        params.n_rounds
    );

    let prover = ConvProver::new(*params)?;
    let (g, h) = damascus_conv::verifier::derive_initial_generators(params)?;

    let mut rng = rand_chacha::ChaCha20Rng::from_seed(witness_seed);
    let mut wit = ConvWitness {
        f: (0..params.n0)
            .map(|_| random_poly(params.q, params.n0, &mut rng))
            .collect(),
        r: (0..params.n0)
            .map(|_| random_poly(params.q, params.n0, &mut rng))
            .collect(),
    };
    let mut pub_state = ConvPublicState {
        g,
        h,
        c: ModuleElem::zero(params.q, params.n0, params.k)?,
    };
    pub_state.c = damascus_conv::verifier::commit(params, &wit, &pub_state.g, &pub_state.h)?;

    for j in 0..round {
        let _ = prover.round(file_id, epoch, j, &mut pub_state, &mut wit)?;
    }

    let base_pub = pub_state.clone();
    let base_wit = wit.clone();
    let bench_j = round;

    // Warmup.
    {
        let mut pub_state = base_pub.clone();
        let mut wit = base_wit.clone();
        let out = prover.round(file_id, epoch, bench_j, &mut pub_state, &mut wit)?;
        std::hint::black_box(out);
    }

    let start = Instant::now();
    for _ in 0..iters {
        let mut pub_state = base_pub.clone();
        let mut wit = base_wit.clone();
        let out = prover.round(file_id, epoch, bench_j, &mut pub_state, &mut wit)?;
        std::hint::black_box(out);
    }
    let elapsed = start.elapsed();

    let ns_total = elapsed.as_nanos() as f64;
    let ns_per = ns_total / (iters as f64);
    println!("flow=bench-conv-round");
    println!(
        "round={bench_j} iters={iters} total_s={:.6} ns_per_iter={:.1}",
        elapsed.as_secs_f64(),
        ns_per
    );
    Ok(())
}

fn random_poly(q: u64, n: usize, rng: &mut rand_chacha::ChaCha20Rng) -> Poly {
    let mut coeffs = vec![0u64; n];
    for c in &mut coeffs {
        *c = (rng.next_u32() as u64) % q;
    }
    Poly::from_coeffs(q, coeffs).expect("q>=2 and coeffs in range")
}

fn parse_32byte_hex(hex_str: &str, name: &'static str) -> anyhow::Result<[u8; 32]> {
    let bytes = hex::decode(hex_str).with_context(|| format!("{name} must be hex"))?;
    anyhow::ensure!(bytes.len() == 32, "{name} must be 32 bytes (64 hex chars)");
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}
