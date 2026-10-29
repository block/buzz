//! Scenario-driven relay load generator. See `perf/LOADGEN.md`.
//!
//! Usage:
//!   buzz-loadgen seed <scenario.toml>
//!   buzz-loadgen run <scenario.toml> [--shard i/n] [--report <path>]

use std::path::PathBuf;
use std::sync::Arc;

use buzz_test_client::loadgen::{net::Net, run, scenario::Scenario, seed, stats};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "buzz-loadgen",
    about = "Scenario-driven Buzz relay load generator"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the scenario's communities, channels, history and deep threads.
    Seed { scenario: PathBuf },
    /// Drive the simulated population through the scenario's ramp.
    Run {
        scenario: PathBuf,
        /// Run only clients with global index g where g % n == i.
        #[arg(long, default_value = "0/1", value_parser = run::Shard::parse)]
        shard: run::Shard,
        /// Also write the JSON report here.
        #[arg(long)]
        report: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::CryptoProvider::install_default(
        rustls::crypto::aws_lc_rs::default_provider(),
    );
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let path = match &cli.command {
        Command::Seed { scenario } | Command::Run { scenario, .. } => scenario,
    };
    let scenario = Scenario::parse(&std::fs::read_to_string(path)?)
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let net = Net::new(&scenario).await?;

    match cli.command {
        Command::Seed { .. } => {
            let stats = Arc::new(stats::Stats::default());
            let started = std::time::Instant::now();
            let outcome = seed::seed(&scenario, &net, &stats).await;
            let summary = stats.snapshot();
            let base = stats::Snapshot::default();
            let report = serde_json::json!({
                "run_id": scenario.run_id,
                "ops": stats::ops_json(&summary.since(&base), started.elapsed().as_secs_f64()),
                "failures": summary.reasons_since(&base, 10),
            });
            println!("{report}");
            outcome
        }
        Command::Run { shard, report, .. } => {
            let (outcome, value) = match run::run(scenario, net, shard).await {
                Ok(value) => (Ok(()), value),
                Err(value) => (Err(anyhow::anyhow!("ramp aborted; see report")), value),
            };
            let text = serde_json::to_string_pretty(&value)?;
            println!("{text}");
            if let Some(path) = report {
                std::fs::write(path, text)?;
            }
            outcome
        }
    }
}
