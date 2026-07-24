use std::path::PathBuf;

use anyhow::{bail, Context};
use buzz_local_relay::{parse_bind_address, serve, LocalRelay, StorageMode};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND_ADDRESS: &str = "127.0.0.1:3000";
const DEFAULT_EVENT_LOG: &str = ".buzz-local/events.ndjson";

struct Config {
    bind_address: String,
    storage: StorageMode,
}

impl Config {
    fn from_args() -> anyhow::Result<Self> {
        let mut bind_address = std::env::var("BUZZ_LOCAL_RELAY_BIND_ADDR")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDRESS.to_string());
        let mut event_log = std::env::var("BUZZ_LOCAL_RELAY_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_EVENT_LOG));
        let mut ephemeral = false;
        let mut args = std::env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--bind" => {
                    bind_address = args.next().context("--bind requires an IP:PORT value")?;
                }
                "--data" => {
                    event_log = PathBuf::from(args.next().context("--data requires a file path")?);
                }
                "--ephemeral" => ephemeral = true,
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                unknown => bail!("unknown argument {unknown:?}; use --help"),
            }
        }

        let storage = if ephemeral {
            StorageMode::Ephemeral
        } else {
            StorageMode::Durable(event_log)
        };
        Ok(Self {
            bind_address,
            storage,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_args()?;
    let address = parse_bind_address(&config.bind_address)
        .with_context(|| format!("invalid bind address {:?}", config.bind_address))?;
    let relay = LocalRelay::open(config.storage).await?;
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind {address}"))?;
    let bound = listener.local_addr()?;

    tracing::info!(
        websocket = %format!("ws://{bound}"),
        http = %format!("http://{bound}"),
        "Buzz local relay is ready"
    );
    serve(listener, relay).await?;
    Ok(())
}

fn print_help() {
    println!(
        "\
buzz-local-relay — durable single-process Buzz relay

Usage:
  buzz-local-relay [--bind IP:PORT] [--data PATH] [--ephemeral]

Options:
  --bind IP:PORT  Listener address (default: {DEFAULT_BIND_ADDRESS})
  --data PATH     Append-only event log (default: {DEFAULT_EVENT_LOG})
  --ephemeral     Keep events in memory only

Environment:
  BUZZ_LOCAL_RELAY_BIND_ADDR
  BUZZ_LOCAL_RELAY_DATA"
    );
}
