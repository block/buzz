//! buzz-supervisor — the headless, relay-side analog of Buzz Desktop's
//! Managed Agents. Watches a relay for channels whose `description`
//! declares a `workdir:<path>`, and provisions/heals/tears down a team of
//! `buzz-acp` processes rooted at that directory for each one.
//!
//! This binary does nothing by default. It requires an explicit
//! `--relay-url` (no default — deliberately unlike `buzz-acp`, which
//! defaults to `ws://localhost:3000`) and at least one `--allowed-root`, so
//! a relay operator can never end up running it by accident against the
//! wrong deployment or with an unbounded filesystem scope. See relay.md for
//! the full design rationale.
#![deny(unsafe_code)]

mod config;
mod provision;
mod security;
mod state;

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use nostr::Keys;
use regex::Regex;

use buzz_cli::client::BuzzClient;

use config::SupervisorConfig;
use provision::Ctx;
use security::AllowedRoots;
use state::StateStore;

#[derive(Parser)]
#[command(
    about = "Headless relay-side agent-team supervisor (server analog of Buzz Desktop Managed Agents)",
    after_help = "Example:\n  buzz-supervisor \\\n    --relay-url http://lotto645.lge.com:3000 \\\n    --private-key <owner-cli-key> \\\n    --relay-admin-key <BUZZ_RELAY_PRIVATE_KEY> \\\n    --allowed-root /home/alice/code \\\n    --roles-file /home/alice/.buzz_agents/roles.toml \\\n    --state-dir /home/alice/.buzz_agents/supervisor/state"
)]
struct Args {
    /// Relay base URL. Required, no default — buzz-supervisor only runs
    /// against a relay deployment its operator has deliberately named.
    #[arg(long, env = "BUZZ_SUPERVISOR_RELAY_URL")]
    relay_url: String,

    /// Private key (hex or nsec) for the identity that owns every
    /// provisioned channel membership/profile this instance creates.
    #[arg(long, env = "BUZZ_SUPERVISOR_PRIVATE_KEY")]
    private_key: String,

    /// Relay admin signing key, forwarded to `buzz-admin add-member` for
    /// relay-wide membership registration (see relay-admin's own docs for
    /// why this can't be done over the plain client HTTP API).
    #[arg(long, env = "BUZZ_SUPERVISOR_RELAY_ADMIN_KEY")]
    relay_admin_key: String,

    /// Path to the `buzz-admin` binary.
    #[arg(long, env = "BUZZ_SUPERVISOR_ADMIN_BIN", default_value = "buzz-admin")]
    admin_bin: PathBuf,

    /// Path to the `buzz-acp` binary spawned per role.
    #[arg(long, env = "BUZZ_SUPERVISOR_ACP_BIN", default_value = "buzz-acp")]
    acp_bin: PathBuf,

    /// Directory agents are allowed to work in (repeatable). Any `workdir`
    /// resolving (after symlink/`..` resolution) outside every one of these
    /// is rejected — this is the security boundary, not `channel_add_policy`
    /// or anything relay-side.
    #[arg(long = "allowed-root", required = true)]
    allowed_roots: Vec<PathBuf>,

    /// TOML file defining the team roster (roles → harness/prompt). See
    /// `roles.example.toml` in this crate for the shape.
    #[arg(long, env = "BUZZ_SUPERVISOR_ROLES_FILE")]
    roles_file: PathBuf,

    /// Directory for per-channel state (generated keys, pids, per-role logs).
    #[arg(long, env = "BUZZ_SUPERVISOR_STATE_DIR")]
    state_dir: PathBuf,

    /// Poll interval in seconds.
    #[arg(long, default_value_t = 20)]
    poll_interval_secs: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let config = SupervisorConfig::load(&args.roles_file)?;
    let allowed_roots = AllowedRoots::new(args.allowed_roots.clone())?;
    let state = StateStore::new(args.state_dir.clone())?;

    let keys = Keys::parse(&args.private_key)
        .map_err(|e| anyhow::anyhow!("invalid --private-key: {e}"))?;
    let client = BuzzClient::new(args.relay_url.clone(), keys, None, None)
        .map_err(|e| anyhow::anyhow!("building relay client: {e}"))?;

    // `workdir:` may appear anywhere in the description (e.g. embedded in a
    // longer human-written sentence), and humans won't always type it with
    // zero spacing around the colon — allow whitespace there, but capture
    // the path itself up to the next whitespace.
    let workdir_pattern = Regex::new(r"workdir\s*:\s*(\S+)")?;

    tracing::info!(
        relay_url = %args.relay_url,
        allowed_roots = ?args.allowed_roots,
        roles = ?config.roles.iter().map(|r| &r.name).collect::<Vec<_>>(),
        poll_interval_secs = args.poll_interval_secs,
        "buzz-supervisor starting"
    );

    let ctx = Ctx {
        client,
        config,
        allowed_roots,
        state,
        acp_bin: args.acp_bin,
        admin_bin: args.admin_bin,
        relay_admin_key: args.relay_admin_key,
        relay_url_for_admin: args.relay_url,
        workdir_pattern,
    };

    loop {
        if let Err(e) = provision::run_once(&ctx).await {
            tracing::error!(error = %e, "poll iteration failed");
        }
        tokio::time::sleep(Duration::from_secs(args.poll_interval_secs)).await;
    }
}
