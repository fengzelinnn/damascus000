use anyhow::Context as _;
use clap::{Parser, Subcommand};
use damascus_conv::ConvParams;

#[derive(Parser)]
#[command(name = "damascus-cli")]
#[command(about = "Damascus prototype CLI (conv-focused)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    Params {
        #[command(subcommand)]
        cmd: ParamsCmd,
    },
    Run {
        #[arg(long)]
        file: String,
        #[arg(long, default_value_t = 0)]
        epoch: u64,
        #[arg(long, default_value_t = 998_244_353)]
        q: u64,
        #[arg(long, default_value_t = 8)]
        n0: usize,
        #[arg(long, default_value_t = 3)]
        rounds: usize,
        #[arg(long, default_value_t = 2)]
        k: usize,
        #[arg(
            long,
            default_value = "0000000000000000000000000000000000000000000000000000000000000000"
        )]
        seed: String,
    },
}

#[derive(Subcommand)]
enum ParamsCmd {
    Gen {
        #[arg(long, default_value_t = 998_244_353)]
        q: u64,
        #[arg(long, default_value_t = 8)]
        n0: usize,
        #[arg(long, default_value_t = 3)]
        rounds: usize,
        #[arg(long, default_value_t = 2)]
        k: usize,
        #[arg(
            long,
            default_value = "0000000000000000000000000000000000000000000000000000000000000000"
        )]
        seed: String,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Command::Params { cmd } => match cmd {
            ParamsCmd::Gen {
                q,
                n0,
                rounds,
                k,
                seed,
            } => {
                let params = ConvParams {
                    q,
                    n0,
                    n_rounds: rounds,
                    k,
                    seed_generators: parse_seed(&seed)?,
                };
                params.validate().context("invalid parameters")?;
                println!("{}", serde_json::to_string_pretty(&params)?);
                Ok(())
            }
        },
        Command::Run {
            file,
            epoch,
            q,
            n0,
            rounds,
            k,
            seed,
        } => {
            let params = ConvParams {
                q,
                n0,
                n_rounds: rounds,
                k,
                seed_generators: parse_seed(&seed)?,
            };
            let bytes = std::fs::read(&file).with_context(|| format!("read file: {file}"))?;
            let res = damascus_sim::run_epoch(params, &bytes, epoch)?;
            println!("file_id={}", res.file_id);
            println!("epoch={}", res.epoch);
            println!("rounds={}", res.transcript.len());
            println!("opening_mu0={}", res.opening.0.coeff0());
            println!("opening_rho0={}", res.opening.1.coeff0());
            Ok(())
        }
    }
}

fn parse_seed(hex_str: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = hex::decode(hex_str).context("seed must be hex")?;
    anyhow::ensure!(bytes.len() == 32, "seed must be 32 bytes (64 hex chars)");
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}
