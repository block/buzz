//! Environment-driven configuration for the spawner daemon.

use std::{path::PathBuf, time::Duration};

use anyhow::{bail, Context, Result};
use nostr::Keys;

/// Default agent runtime image. Overridden with `BUZZ_SPAWNER_AGENT_IMAGE`.
pub const DEFAULT_AGENT_IMAGE: &str = "ghcr.io/block/buzz-acp:main";

/// Default per-agent CPU allocation, in thousandths of a core.
pub const DEFAULT_CPU_MILLIS: u32 = 1000;

/// Default per-agent memory limit, in mebibytes.
pub const DEFAULT_MEMORY_MIB: u32 = 2048;

/// Docker label carrying the agent pubkey a container belongs to.
pub const AGENT_LABEL: &str = "com.buzz.agent";

/// Docker label carrying the spec slug a container reconciles.
pub const SLUG_LABEL: &str = "com.buzz.spec-slug";

/// Docker label carrying the spawner pubkey that owns a container. Two spawners
/// sharing one Docker host must not reap each other's containers.
pub const SPAWNER_LABEL: &str = "com.buzz.spawner";

/// Runtime configuration.
pub struct Config {
    /// The spawner's own Nostr identity. Used to authenticate to the relay, to
    /// receive attestation frames, and to author status events.
    pub keys: Keys,
    /// Relay WebSocket URL the spawner itself connects to. In a compose
    /// deployment this is the internal address (`ws://relay:3000`).
    pub relay_url: String,
    /// Relay WebSocket URL handed to agent containers.
    ///
    /// Deliberately separate from [`Self::relay_url`]. Agent containers are not
    /// attached to the relay's internal compose network — an agent runs
    /// arbitrary shell tools, and joining that network would put Postgres,
    /// Redis, and MinIO one `nc` away. So agents reach the relay by its public
    /// address instead, the same way any other client does.
    pub agent_relay_url: String,
    /// Directory holding the state file and per-agent secrets. Must be on a
    /// persistent volume — losing it orphans every running agent's identity.
    pub state_dir: PathBuf,
    /// Container image running the `buzz-acp` harness.
    pub agent_image: String,
    /// Ceiling applied to a spec's requested CPU. Specs above it are clamped.
    pub max_cpu_millis: u32,
    /// Ceiling applied to a spec's requested memory. Specs above it are clamped.
    pub max_memory_mib: u32,
    /// Maximum number of agents this spawner will run at once.
    pub max_agents: usize,
    /// How long to wait for an owner to answer an attestation request before
    /// giving up and reporting failure.
    pub attestation_timeout: Duration,
    /// How often to reconcile even when no event has arrived, so a container
    /// that died out-of-band is noticed.
    pub reconcile_interval: Duration,
    /// ACP agent binary the harness spawns inside each container.
    ///
    /// Operator-only, deliberately. This is a code-execution surface: the
    /// desktop lists `BUZZ_ACP_AGENT_COMMAND` among its reserved env keys for
    /// exactly that reason. It must never be settable from a kind:30178 spec,
    /// which is owner-authored and world-readable — a spec that could name the
    /// binary would let anyone who can publish one choose what runs on the host.
    ///
    /// `None` leaves the image's own default (`buzz-agent`, API-key based). Set
    /// `claude-agent-acp` to run agents on a Claude subscription.
    pub agent_command: Option<String>,
    /// Comma-separated args for [`Self::agent_command`], same rationale.
    pub agent_args: Option<String>,
    /// Display name advertised in this spawner's announcement.
    ///
    /// Defaults to the hostname, because "prod-vps" is a far better thing to
    /// choose from in a picker than 64 hex characters.
    pub name: String,
    /// Longer description shown alongside the name.
    pub description: Option<String>,
    /// Provider used when a spec does not name one.
    ///
    /// The `buzz-agent` harness refuses to start without `BUZZ_AGENT_PROVIDER`,
    /// and a spec is allowed to omit it. The desktop resolves the same gap from
    /// its global agent settings (`resolve_deploy_model_provider`); a spawner
    /// has no such settings, so the operator supplies the host default here.
    pub default_provider: Option<String>,
    /// Model used when a spec does not name one.
    pub default_model: Option<String>,
    /// Environment passed through to every agent container — LLM credentials
    /// live here, never in an event. Collected from `BUZZ_SPAWNER_AGENT_ENV`.
    pub agent_env: Vec<(String, String)>,
}

impl Config {
    /// Load configuration from the process environment.
    pub fn from_env() -> Result<Self> {
        let nsec = require_env("BUZZ_SPAWNER_NSEC")?;
        let keys = Keys::parse(nsec.trim())
            .context("BUZZ_SPAWNER_NSEC is not a valid nsec or hex secret key")?;

        let relay_url = require_env("BUZZ_SPAWNER_RELAY_URL")?;
        check_ws_url("BUZZ_SPAWNER_RELAY_URL", &relay_url)?;

        // Defaulting to the spawner's own relay URL is correct for a single-host
        // setup where both addresses are the same, and wrong for compose, where
        // `ws://relay:3000` does not resolve from the Docker bridge. The compose
        // bundle sets it explicitly; this default keeps a bare `cargo run`
        // working.
        let agent_relay_url =
            std::env::var("BUZZ_SPAWNER_AGENT_RELAY_URL").unwrap_or_else(|_| relay_url.clone());
        check_ws_url("BUZZ_SPAWNER_AGENT_RELAY_URL", &agent_relay_url)?;

        let state_dir = PathBuf::from(
            std::env::var("BUZZ_SPAWNER_STATE_DIR")
                .unwrap_or_else(|_| "/var/lib/buzz-spawner".into()),
        );

        Ok(Self {
            keys,
            relay_url,
            agent_relay_url,
            state_dir,
            agent_image: std::env::var("BUZZ_SPAWNER_AGENT_IMAGE")
                .unwrap_or_else(|_| DEFAULT_AGENT_IMAGE.into()),
            max_cpu_millis: parse_env("BUZZ_SPAWNER_MAX_CPU_MILLIS", 4000)?,
            max_memory_mib: parse_env("BUZZ_SPAWNER_MAX_MEMORY_MIB", 8192)?,
            max_agents: parse_env("BUZZ_SPAWNER_MAX_AGENTS", 16)?,
            attestation_timeout: Duration::from_secs(parse_env(
                "BUZZ_SPAWNER_ATTESTATION_TIMEOUT_SECS",
                600,
            )?),
            reconcile_interval: Duration::from_secs(parse_env(
                "BUZZ_SPAWNER_RECONCILE_INTERVAL_SECS",
                60,
            )?),
            agent_command: non_empty_env("BUZZ_SPAWNER_AGENT_COMMAND"),
            agent_args: non_empty_env("BUZZ_SPAWNER_AGENT_ARGS"),
            name: non_empty_env("BUZZ_SPAWNER_NAME").unwrap_or_else(default_name),
            description: non_empty_env("BUZZ_SPAWNER_DESCRIPTION"),
            default_provider: non_empty_env("BUZZ_SPAWNER_DEFAULT_PROVIDER"),
            default_model: non_empty_env("BUZZ_SPAWNER_DEFAULT_MODEL"),
            agent_env: parse_agent_env(
                &std::env::var("BUZZ_SPAWNER_AGENT_ENV").unwrap_or_default(),
            )?,
        })
    }
}

fn check_ws_url(key: &str, url: &str) -> Result<()> {
    if !url.starts_with("ws://") && !url.starts_with("wss://") {
        bail!("{key} must start with ws:// or wss://");
    }
    Ok(())
}

/// Best-effort hostname for the default advertised name.
fn default_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| "buzz-spawner".to_string())
}

/// Read an optional env var, treating an empty value as absent.
fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn require_env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("{key} is required"))
}

fn parse_env<T: std::str::FromStr>(key: &str, default: T) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    match std::env::var(key) {
        Ok(raw) => raw
            .trim()
            .parse()
            .map_err(|e| anyhow::anyhow!("{key} is not a valid value: {e}")),
        Err(_) => Ok(default),
    }
}

/// Parse `BUZZ_SPAWNER_AGENT_ENV`, a comma-separated list of variable *names*
/// to forward from the spawner's own environment into agent containers.
///
/// Names, not `KEY=VALUE` pairs: this keeps secrets out of the compose file's
/// inline environment block and out of `docker inspect` on the spawner, and it
/// means a rotated credential is picked up by restarting the spawner rather than
/// by editing config. A named variable that is unset in the spawner's
/// environment is a hard error rather than a silent omission — an agent that
/// boots without its API key fails deep inside the harness with a confusing
/// message, so it is better to refuse at startup.
fn parse_agent_env(raw: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for name in raw.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        if !name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            bail!("BUZZ_SPAWNER_AGENT_ENV entry {name:?} is not a valid environment variable name");
        }
        let value = std::env::var(name)
            .with_context(|| format!("BUZZ_SPAWNER_AGENT_ENV names {name}, but it is not set"))?;
        out.push((name.to_string(), value));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_env_rejects_malformed_names() {
        assert!(parse_agent_env("not-a-var").is_err());
        assert!(parse_agent_env("ANTHROPIC_API_KEY=inline-secret").is_err());
    }

    #[test]
    fn agent_env_is_empty_when_unset() {
        assert!(parse_agent_env("").unwrap().is_empty());
        assert!(parse_agent_env("  ,  ").unwrap().is_empty());
    }
}
