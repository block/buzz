use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context};
use buzz_local_relay::identity::LocalIdentityAdapter;
use buzz_local_relay::{parse_bind_address, serve, LocalRelay, StorageMode};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND_ADDRESS: &str = "127.0.0.1:3000";
const DEFAULT_EVENT_LOG: &str = ".buzz-local/events.ndjson";

struct Config {
    bind_address: String,
    storage: StorageMode,
    require_auth: bool,
}

impl Config {
    fn from_args() -> anyhow::Result<Self> {
        let mut bind_address = std::env::var("BUZZ_LOCAL_RELAY_BIND_ADDR")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDRESS.to_string());
        let mut event_log = std::env::var("BUZZ_LOCAL_RELAY_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_EVENT_LOG));
        let mut ephemeral = false;
        let mut require_auth = std::env::var("BUZZ_LOCAL_RELAY_REQUIRE_AUTH")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
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
                "--require-auth" => require_auth = true,
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
            require_auth,
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
    let relay = if config.require_auth {
        // A durable relay also persists consumed-proof replay state, so a
        // restart within the proof freshness window still rejects replays.
        let adapter = match &config.storage {
            StorageMode::Durable(event_log) => {
                let mut proof_store = event_log.clone().into_os_string();
                proof_store.push(".auth-proofs");
                LocalIdentityAdapter::with_proof_store(PathBuf::from(proof_store))
                    .context("failed to open authentication proof store")?
            }
            StorageMode::Ephemeral => LocalIdentityAdapter::new(),
        };
        LocalRelay::open_with_identity(config.storage, Arc::new(adapter)).await?
    } else {
        LocalRelay::open(config.storage).await?
    };
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind {address}"))?;
    let bound = listener.local_addr()?;

    tracing::info!(
        websocket = %format!("ws://{bound}"),
        http = %format!("http://{bound}"),
        require_auth = config.require_auth,
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
  buzz-local-relay [--bind IP:PORT] [--data PATH] [--ephemeral] [--require-auth]

Options:
  --bind IP:PORT  Listener address (default: {DEFAULT_BIND_ADDRESS})
  --data PATH     Append-only event log (default: {DEFAULT_EVENT_LOG})
  --ephemeral     Keep events in memory only
  --require-auth  Require NIP-42 WebSocket and NIP-98 HTTP authentication

Environment:
  BUZZ_LOCAL_RELAY_BIND_ADDR
  BUZZ_LOCAL_RELAY_DATA
  BUZZ_LOCAL_RELAY_REQUIRE_AUTH"
    );
}
