use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context};
use buzz_core::replication::ReplicationSourceId;
use buzz_local_relay::declarations::admit_domain_from_journal;
use buzz_local_relay::identity::{LocalIdentityAdapter, RelayPeerTrust};
use buzz_local_relay::{
    parse_bind_address, serve, EventStore, LocalRelay, ReplicationSourceAllowlist, StorageMode,
};
use nostr::PublicKey;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND_ADDRESS: &str = "127.0.0.1:3000";
const DEFAULT_EVENT_LOG: &str = ".buzz-local/events.ndjson";

struct Config {
    bind_address: String,
    storage: StorageMode,
    require_auth: bool,
    peer_trust: Option<PathBuf>,
    artifacts: Option<PathBuf>,
    owner: Option<PublicKey>,
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
        let mut peer_trust = std::env::var("BUZZ_LOCAL_RELAY_PEER_TRUST")
            .ok()
            .map(PathBuf::from);
        let mut artifacts = std::env::var("BUZZ_LOCAL_RELAY_ARTIFACTS")
            .ok()
            .map(PathBuf::from);
        let mut owner = std::env::var("BUZZ_LOCAL_RELAY_OWNER").ok();
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
                "--peer-trust" => {
                    peer_trust = Some(PathBuf::from(
                        args.next().context("--peer-trust requires a file path")?,
                    ));
                }
                "--artifacts" => {
                    artifacts = Some(PathBuf::from(
                        args.next()
                            .context("--artifacts requires a directory path")?,
                    ));
                }
                "--owner" => {
                    owner = Some(args.next().context("--owner requires a pubkey (hex)")?);
                }
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
        let owner = owner
            .map(|hex| {
                PublicKey::from_hex(&hex).with_context(|| format!("invalid owner pubkey {hex:?}"))
            })
            .transpose()?;
        Ok(Self {
            bind_address,
            storage,
            require_auth,
            peer_trust,
            artifacts,
            owner,
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
    // Durable relays get a content-addressed artifact store beside the
    // journal by default; ephemeral relays need an explicit directory.
    let artifacts_dir = config.artifacts.clone().or_else(|| match &config.storage {
        StorageMode::Durable(event_log) => {
            let mut path = event_log.clone().into_os_string();
            path.push(".artifacts");
            Some(PathBuf::from(path))
        }
        StorageMode::Ephemeral => None,
    });
    // A durable relay also persists consumed-proof replay state, so a
    // restart within the proof freshness window still rejects replays.
    let proof_store = match &config.storage {
        StorageMode::Durable(event_log) => {
            let mut path = event_log.clone().into_os_string();
            path.push(".auth-proofs");
            Some(PathBuf::from(path))
        }
        StorageMode::Ephemeral => None,
    };

    // The store opens before the adapters so journal state can govern them:
    // owner-signed admit declarations claim the trust domain wholesale, and
    // the file config is bootstrap-only (runtime evaluation policy in
    // specs/architecture/sovereign-sync-agreement-v0.1-draft.md).
    let store = Arc::new(EventStore::open(config.storage).await?);
    let peer_trust = match &config.owner {
        Some(owner) => match admit_domain_from_journal(&store, owner).await? {
            Some(entries) => {
                tracing::info!(
                    active_heads = entries.len(),
                    "admit domain governed by journal declarations; peer-trust file ignored"
                );
                entries
            }
            None => {
                let entries = config
                    .peer_trust
                    .as_ref()
                    .map(load_peer_trust)
                    .transpose()?
                    .unwrap_or_default();
                tracing::info!(
                    entries = entries.len(),
                    "no owner-signed admit declarations; bootstrap file config governs"
                );
                entries
            }
        },
        None => config
            .peer_trust
            .as_ref()
            .map(load_peer_trust)
            .transpose()?
            .unwrap_or_default(),
    };
    if !peer_trust.is_empty() && !config.require_auth {
        bail!("peer trust requires --require-auth: replication peers are bound cryptographically");
    }
    let relay = if config.require_auth {
        let sources: Vec<ReplicationSourceId> = peer_trust
            .iter()
            .map(|(source, _)| source.clone())
            .collect();
        let adapter = LocalIdentityAdapter::with_peer_trust_and_proof_store(
            peer_trust.into_iter().map(|(_, trust)| trust),
            proof_store,
        )
        .context("failed to open authentication proof store")?;
        LocalRelay::open_full_with_store(
            store,
            Arc::new(ReplicationSourceAllowlist::new(sources)),
            Some(Arc::new(adapter)),
            artifacts_dir,
        )
    } else {
        LocalRelay::open_full_with_store(
            store,
            Arc::new(buzz_local_relay::ReplicationDisabled),
            None,
            artifacts_dir,
        )
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

/// Parses destination-controlled peer trust:
/// `{"<source>": {"principal": "...", "verification_keys": ["<pubkey hex>"]}}`.
fn load_peer_trust(path: &PathBuf) -> anyhow::Result<Vec<(ReplicationSourceId, RelayPeerTrust)>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read peer trust file {}", path.display()))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).context("peer trust file is not valid JSON")?;
    let object = parsed
        .as_object()
        .context("peer trust file must be a JSON object keyed by source ID")?;
    let mut entries = Vec::with_capacity(object.len());
    for (source, config) in object {
        let principal = config["principal"]
            .as_str()
            .with_context(|| format!("source {source:?} requires a string principal"))?;
        let keys = config["verification_keys"]
            .as_array()
            .with_context(|| format!("source {source:?} requires verification_keys"))?
            .iter()
            .map(|value| {
                let hex = value
                    .as_str()
                    .context("verification key must be a string")?;
                let pubkey = PublicKey::from_hex(hex)
                    .with_context(|| format!("invalid verification key {hex:?}"))?;
                Ok((pubkey, format!("{principal}#nostr-key")))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let source_id = ReplicationSourceId::new(source.clone());
        entries.push((
            source_id.clone(),
            RelayPeerTrust::new(source_id, principal, keys),
        ));
    }
    Ok(entries)
}

fn print_help() {
    println!(
        "\
buzz-local-relay — durable single-process Buzz relay

Usage:
  buzz-local-relay [--bind IP:PORT] [--data PATH] [--ephemeral] [--require-auth]
                   [--peer-trust PATH]

Options:
  --bind IP:PORT  Listener address (default: {DEFAULT_BIND_ADDRESS})
  --data PATH     Append-only event log (default: {DEFAULT_EVENT_LOG})
  --ephemeral     Keep events in memory only
  --require-auth  Require NIP-42 WebSocket and NIP-98 HTTP authentication
  --peer-trust PATH  JSON trust config admitting replication peers (needs --require-auth)
  --artifacts DIR    Content-addressed artifact store (default: <data>.artifacts)
  --owner PUBKEY     Owner pubkey (hex); owner-signed admit declarations in the
                     journal then govern peer trust, and --peer-trust becomes
                     bootstrap-only

Environment:
  BUZZ_LOCAL_RELAY_BIND_ADDR
  BUZZ_LOCAL_RELAY_DATA
  BUZZ_LOCAL_RELAY_REQUIRE_AUTH
  BUZZ_LOCAL_RELAY_PEER_TRUST
  BUZZ_LOCAL_RELAY_ARTIFACTS
  BUZZ_LOCAL_RELAY_OWNER"
    );
}
