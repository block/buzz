//! The standalone accumulator daemon.
//!
//! One process, three cooperating pieces:
//! - [`sync`] — relay task: connect → NIP-42 auth → discover channels →
//!   backfill into the local mirror → live tail, with reconnect/backoff.
//! - [`store`] — the SQLite mirror plus fold/artifact storage.
//! - [`http`] — loopback HTTP API exposing status and the fold machinery to
//!   an external client (the future standalone UI).
//!
//! Run it with `cargo run -p buzz-accumulator`.

pub mod folds;
pub mod http;
pub mod status;
pub mod store;
pub mod sync;

use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use nostr::Keys;
use tracing::{info, warn};

/// The team relay. Pinned deliberately: dev shells often export a sandbox
/// `BUZZ_RELAY_URL` that would otherwise silently point the mirror at
/// localhost. Overriding requires the accumulator-specific flag/env below.
pub const DEFAULT_RELAY_URL: &str = "wss://buzz.block.builderlab.xyz";

/// Command-line / environment configuration.
///
/// Identity intentionally has exactly one entry point ([`load_identity`]):
/// today it borrows the person's key from `BUZZ_PRIVATE_KEY` or a key file;
/// making it editable later (keychain, dedicated agent key) touches only that
/// seam.
#[derive(Debug, Parser)]
#[command(
    name = "buzz-accumulator",
    about = "Standalone accumulator daemon: mirrors everything your key can see into local SQLite and folds it into artifacts"
)]
pub struct Config {
    /// Relay websocket URL. Pinned to the team relay by default — the generic
    /// BUZZ_RELAY_URL env var is deliberately ignored.
    #[arg(long, env = "BUZZ_ACCUMULATOR_RELAY_URL", default_value = DEFAULT_RELAY_URL)]
    pub relay: String,

    /// Private key (nsec or hex). The person's key for now.
    #[arg(long, env = "BUZZ_PRIVATE_KEY", hide_env_values = true)]
    pub private_key: Option<String>,

    /// Path to a file containing the private key (nsec or hex), as an
    /// alternative to the environment variable.
    #[arg(long, conflicts_with = "private_key")]
    pub key_file: Option<String>,

    /// Optional NIP-OA ownership tag (only needed when running as an agent
    /// identity instead of the person).
    #[arg(long, env = "BUZZ_AUTH_TAG", hide_env_values = true)]
    pub auth_tag: Option<String>,

    /// SQLite mirror path. Defaults to ~/.buzz-accumulator/accumulator.db.
    #[arg(long, env = "BUZZ_ACCUMULATOR_DB")]
    pub db: Option<String>,

    /// Loopback address for the HTTP status/machinery API.
    #[arg(
        long,
        env = "BUZZ_ACCUMULATOR_HTTP_ADDR",
        default_value = "127.0.0.1:4640"
    )]
    pub http_addr: String,
}

/// The identity seam: resolves the signing key from config.
///
/// Precedence: `--private-key` / `BUZZ_PRIVATE_KEY` → `--key-file`. Anything
/// smarter (macOS keychain, editable identity) replaces the body of this one
/// function.
pub fn load_identity(cfg: &Config) -> anyhow::Result<Keys> {
    if let Some(pk) = cfg.private_key.as_deref().filter(|s| !s.trim().is_empty()) {
        return Keys::parse(pk.trim()).context("invalid BUZZ_PRIVATE_KEY (expected nsec or hex)");
    }
    if let Some(path) = cfg.key_file.as_deref() {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading key file {path}"))?;
        return Keys::parse(raw.trim())
            .with_context(|| format!("invalid key in {path} (expected nsec or hex)"));
    }
    anyhow::bail!("no identity: set BUZZ_PRIVATE_KEY (nsec or hex) or pass --key-file <path>")
}

/// Resolves the mirror path, creating its parent directory.
fn resolve_db_path(cfg: &Config) -> anyhow::Result<String> {
    if let Some(db) = &cfg.db {
        return Ok(db.clone());
    }
    let home = std::env::var("HOME").context("HOME not set; pass --db <path>")?;
    let dir = std::path::Path::new(&home).join(".buzz-accumulator");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir.join("accumulator.db").to_string_lossy().into_owned())
}

/// Daemon entry point: wires store, sync task, and HTTP API together and
/// runs until Ctrl-C.
pub async fn run(cfg: Config) -> anyhow::Result<()> {
    // Explicit rustls provider: without this a multi-package release build
    // that also links aws-lc-rs panics on the first wss:// dial.
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        // Already installed (e.g. by a host embedding the daemon) — fine.
        tracing::debug!("rustls crypto provider was already installed");
    }

    let keys = load_identity(&cfg)?;
    let pubkey = keys.public_key().to_hex();
    let db_path = resolve_db_path(&cfg)?;
    let auth_tag = match cfg.auth_tag.as_deref().filter(|s| !s.is_empty()) {
        Some(raw) => Some(
            buzz_sdk::nip_oa::parse_auth_tag(raw)
                .map_err(|e| anyhow::anyhow!("invalid BUZZ_AUTH_TAG: {e}"))?,
        ),
        None => None,
    };

    info!(relay = %cfg.relay, %pubkey, db = %db_path, http = %cfg.http_addr, "accumulator starting");
    if let Ok(ambient) = std::env::var("BUZZ_RELAY_URL") {
        if ambient != cfg.relay {
            warn!(
                ignored = %ambient,
                using = %cfg.relay,
                "BUZZ_RELAY_URL is set but deliberately ignored; use --relay / BUZZ_ACCUMULATOR_RELAY_URL to override"
            );
        }
    }

    let store = store::Store::open(&db_path)
        .await
        .context("opening mirror db")?;
    let registry = status::StatusRegistry::new(
        &cfg.relay,
        &pubkey,
        &db_path,
        chrono::Utc::now().timestamp(),
    );

    let sync_cfg = sync::SyncConfig {
        relay_url: cfg.relay.clone(),
        keys,
        auth_tag,
    };
    let sync_task = tokio::spawn(sync::run_sync(sync_cfg, store.clone(), registry.clone()));

    let state = http::AppState {
        store,
        registry,
        runner: Arc::new(crate::SubprocessRunner::new()),
        runs: folds::RunGuard::default(),
    };
    let addr: std::net::SocketAddr = cfg
        .http_addr
        .parse()
        .with_context(|| format!("invalid --http-addr {}", cfg.http_addr))?;

    tokio::select! {
        served = http::serve(state, addr) => served?,
        _ = tokio::signal::ctrl_c() => {
            info!("ctrl-c received; shutting down");
        }
    }
    sync_task.abort();
    Ok(())
}
