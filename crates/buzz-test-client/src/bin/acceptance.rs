//! Relay acceptance probe: checks the relay's core jobs once and exits.
//!
//! Prints a JSON report on stdout. Exits 0 when every check passed, 1 when a
//! check failed, and 2 when the probe itself could not run or report.
//!
//! Usage:
//!   buzz-acceptance --relay-url wss://relay.example [--assertion-file <path>]
//!       [--statsd-addr 127.0.0.1:8125] [--tag env:staging]...

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use buzz_test_client::acceptance::{self, metrics, Config};
use clap::Parser;
use nostr::Keys;

/// The longest resolving the metrics address or sending the metrics may take.
const METRICS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(
    name = "buzz-acceptance",
    about = "Check a Buzz relay's core jobs once and report each one"
)]
struct Cli {
    /// The relay's WebSocket URL.
    #[arg(long, env = "BUZZ_RELAY_URL")]
    relay_url: String,
    /// Secret key (hex or nsec) to authenticate and sign as. Without one the
    /// probe uses a new key, which works only on a relay that admits any key.
    #[arg(long, env = "BUZZ_PRIVATE_KEY", hide_env_values = true)]
    private_key: Option<String>,
    /// File holding a NIP-FI assertion for the key, read before every
    /// connection. Keep it fresh from another process.
    #[arg(long, env = "BUZZ_ACCEPTANCE_ASSERTION_FILE")]
    assertion_file: Option<PathBuf>,
    /// The longest any one stage may take, in seconds.
    #[arg(long, default_value_t = 10)]
    stage_timeout_secs: u64,
    /// DogStatsD address (`host:port`) to send metrics to.
    #[arg(long, env = "BUZZ_ACCEPTANCE_STATSD_ADDR")]
    statsd_addr: Option<String>,
    /// Metric name prefix.
    #[arg(long, default_value = "buzz.acceptance")]
    metric_prefix: String,
    /// A `key:value` tag for every metric. Repeat for more.
    #[arg(long = "tag")]
    tags: Vec<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let _ = rustls::crypto::CryptoProvider::install_default(
        rustls::crypto::aws_lc_rs::default_provider(),
    );
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    match probe(Cli::parse()).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("buzz-acceptance: {e:#}");
            ExitCode::from(2)
        }
    }
}

/// Runs the probe, prints its report and sends its metrics. Returns whether
/// every check passed.
async fn probe(cli: Cli) -> anyhow::Result<bool> {
    let keys = match &cli.private_key {
        Some(key) => Keys::parse(key).map_err(|e| anyhow::anyhow!("private key: {e}"))?,
        None => Keys::generate(),
    };
    let statsd = match &cli.statsd_addr {
        Some(addr) => Some(resolve(addr).await?),
        None => None,
    };
    let cfg = Config {
        relay_url: cli.relay_url,
        keys,
        assertion_file: cli.assertion_file,
        stage_timeout: Duration::from_secs(cli.stage_timeout_secs),
    };

    let report = acceptance::run(&cfg).await;
    println!("{}", serde_json::to_string(&report)?);
    if let Some(addr) = statsd {
        let lines = metrics::lines(&cli.metric_prefix, &report, &cli.tags);
        tokio::time::timeout(METRICS_TIMEOUT, metrics::send(addr, &lines))
            .await
            .map_err(|_| anyhow::anyhow!("send metrics to {addr}: timed out"))?
            .map_err(|e| anyhow::anyhow!("send metrics to {addr}: {e}"))?;
    }
    Ok(report.passed)
}

/// Resolves `host:port` once, before the run, so a bad address fails fast.
/// An IP literal never reaches the resolver.
async fn resolve(addr: &str) -> anyhow::Result<SocketAddr> {
    if let Ok(literal) = addr.parse() {
        return Ok(literal);
    }
    tokio::time::timeout(METRICS_TIMEOUT, tokio::net::lookup_host(addr))
        .await
        .map_err(|_| anyhow::anyhow!("resolve statsd address {addr}: timed out"))?
        .map_err(|e| anyhow::anyhow!("resolve statsd address {addr}: {e}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("statsd address {addr} resolved to nothing"))
}
