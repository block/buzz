//! Thin entry point for the standalone accumulator daemon.
//!
//! All behavior lives in [`buzz_accumulator::daemon`]; this is config parsing
//! plus logging setup, per the workspace bin convention.

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("buzz_accumulator=info")),
        )
        .compact()
        .init();
    let cfg = buzz_accumulator::daemon::Config::parse();
    buzz_accumulator::daemon::run(cfg).await
}
