//! Relay acceptance probe: checks the relay's core jobs once and exits.
//!
//! Prints a JSON report on stdout. Exits 0 when every check passed, 1 when a
//! check failed, and 2 when the probe itself could not run or report.
//!
//! Usage:
//!   buzz-acceptance --relay-url wss://relay.example
//!       [--connect-addr relay.internal:80]
//!       [--private-key <key> | --private-key-file <path>] [--assertion-file <path>]
//!       [--statsd-addr 127.0.0.1:8125] [--tag env:staging]...

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use buzz_test_client::acceptance::{self, metrics, Config};
use clap::Parser;
use nostr::Keys;

/// The largest key file the probe reads. An nsec is 63 bytes.
const MAX_KEY_FILE_BYTES: u64 = 1024;

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
    /// Plaintext `host:port` to dial instead of the relay URL's host, such as
    /// the relay's in-cluster service. The URL still sets the `Host` header
    /// and the NIP-42 relay tag.
    #[arg(long, env = "BUZZ_ACCEPTANCE_CONNECT_ADDR")]
    connect_addr: Option<String>,
    /// Secret key (hex or nsec) to authenticate and sign as. Without this or
    /// a key file the probe uses a new key, which works only on a relay that
    /// admits any key.
    #[arg(long, env = "BUZZ_PRIVATE_KEY", hide_env_values = true)]
    private_key: Option<String>,
    /// File holding the secret key (hex or nsec), for a key another process
    /// writes, such as one minted with its NIP-FI assertion before the run.
    #[arg(long, env = "BUZZ_PRIVATE_KEY_FILE", conflicts_with = "private_key")]
    private_key_file: Option<PathBuf>,
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

fn main() -> ExitCode {
    let _ = rustls::crypto::CryptoProvider::install_default(
        rustls::crypto::aws_lc_rs::default_provider(),
    );
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("buzz-acceptance: start runtime: {e}");
            return ExitCode::from(2);
        }
    };
    let result = runtime.block_on(probe(cli));
    // A timed-out hostname lookup keeps running on a blocking thread; do not
    // let it hold the exit.
    runtime.shutdown_timeout(Duration::from_secs(1));
    match result {
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
    let key = match (&cli.private_key, &cli.private_key_file) {
        (Some(key), _) => Some(key.clone()),
        (None, Some(path)) => Some(acceptance::read_secret_file(
            path,
            "private key file",
            MAX_KEY_FILE_BYTES,
        )?),
        (None, None) => None,
    };
    let keys = match key {
        Some(key) => Keys::parse(&key).map_err(|e| anyhow::anyhow!("private key: {e}"))?,
        None => Keys::generate(),
    };
    let statsd = match &cli.statsd_addr {
        Some(addr) => Some(resolve(addr).await?),
        None => None,
    };
    let cfg = Config {
        relay_url: cli.relay_url,
        connect_addr: cli.connect_addr,
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
