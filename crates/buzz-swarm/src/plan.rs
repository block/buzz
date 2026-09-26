//! Bind each identity once, then launch the existing harness on every relay.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{bail, ensure, Context, Result};
use buzz_core::relay::normalize_relay_url;
use nostr::Keys;

use crate::config::{validate_name, Restart, SwarmFile};
use crate::env::Env;
use crate::identity::{GeneratedKey, Identity};

// Identity and desktop handoffs must never override the swarm's delegation.
const RESERVED: &[&str] = &[
    "BUZZ_RELAY_URL",
    "BUZZ_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "BUZZ_AUTH_TAG",
    "BUZZ_OWNER_NSEC",
    "BUZZ_API_TOKEN",
    "BUZZ_ACP_PRIVATE_KEY",
    "BUZZ_ACP_API_TOKEN",
    "BUZZ_ACP_AGENT_OWNER",
    "BUZZ_ACP_SETUP_PAYLOAD",
    "BUZZ_MANAGED_AGENT",
    "BUZZ_MANAGED_AGENT_START_NONCE",
];

#[derive(Clone)]
pub struct AgentPlan {
    pub name: String,
    pub pubkey: String,
    pub relay_url: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub workdir: PathBuf,
    pub env: BTreeMap<String, String>,
    pub env_remove: Vec<String>,
    pub restart: Restart,
    pub max_restarts: u32,
}

pub struct Plan {
    pub owner_pubkey: String,
    pub agents: Vec<AgentPlan>,
    pub generated_keys: Vec<GeneratedKey>,
    /// Default workdirs that do not exist yet; created on start.
    pub new_workdirs: Vec<PathBuf>,
}

impl Plan {
    /// Validate swarm wiring and identities without persisting or launching.
    /// ACP and model configuration belongs to the runtime and is validated there.
    pub fn build(file: &SwarmFile, only: &[String], env: &Env) -> Result<Self> {
        let owner = file.owner(env);
        let owner_keys = owner
            .as_ref()
            .map(|owner| {
                Keys::parse(owner.nsec.resolve(env)?.expose().trim())
                    .context("invalid owner private key")
            })
            .transpose()?;
        let mut owner_pubkey = owner_keys.as_ref().map(|keys| keys.public_key().to_hex());
        let specs = file.resolved_specs(env)?;
        // Include disabled and unselected agents: their source variables still
        // exist in the parent's environment and must not reach another agent.
        let private_sources: BTreeSet<_> = owner
            .iter()
            .map(|owner| &owner.nsec)
            .chain(specs.iter().filter_map(|spec| spec.nsec.as_ref()))
            .filter_map(|source| source.env_key())
            .map(str::to_owned)
            .collect();
        // A declared source reaches only the agents that declare it, under the
        // name they declare; no child inherits it ambiently. Raw defaults count
        // even when every agent overrides them.
        let defaults = file.defaults()?;
        let declared_sources = specs
            .iter()
            .chain([&defaults])
            .flat_map(|spec| spec.auth_tag.iter().chain(spec.env.values()))
            .filter_map(|source| source.env_key())
            .map(str::to_owned);
        let env_remove: Vec<_> = env
            .keys()
            .filter(|key| key.starts_with("BUZZ_ACP_") || RESERVED.contains(key))
            .map(str::to_owned)
            .chain(private_sources.iter().cloned())
            .chain(declared_sources)
            .collect();
        let mut agents = Vec::new();
        let mut generated_keys = Vec::new();
        let mut new_workdirs = Vec::new();
        let mut seen_names = BTreeSet::new();
        let mut seen_connections = BTreeSet::new();
        let mut selected = BTreeSet::new();

        for spec in &specs {
            let name = spec.name.as_deref().context("each agent needs a name")?;
            validate_name(name)?;
            ensure!(
                seen_names.insert(name.to_ascii_lowercase()),
                "duplicate agent name `{name}`"
            );
            if !spec.enabled.unwrap_or(true)
                || (!only.is_empty() && !only.iter().any(|n| n == name))
            {
                continue;
            }
            let identity = Identity::resolve(
                name,
                spec,
                &file.directory,
                owner_keys.as_ref(),
                owner.as_ref().map_or("", |owner| owner.conditions.as_str()),
                env,
            )
            .with_context(|| format!("agent `{name}`"))?;
            match &owner_pubkey {
                Some(owner) => ensure!(
                    *owner == identity.owner,
                    "agent `{name}` has a different owner"
                ),
                None => owner_pubkey = Some(identity.owner.clone()),
            }
            let program = executable(spec.harness.as_deref().unwrap_or("buzz-acp"), env)?;
            // Default to a private directory per agent, away from `keys/`.
            // Only that default may be created; an explicit workdir must exist.
            let workdir = spec.workdir.clone().unwrap_or_else(|| {
                file.directory
                    .join("workspaces")
                    .join(name.to_ascii_lowercase())
            });
            let create = spec.workdir.is_none() && !workdir.try_exists()?;
            ensure!(
                create || workdir.is_dir(),
                "agent `{name}`: workdir {} is not a directory",
                workdir.display()
            );
            if create {
                new_workdirs.push(workdir.clone());
            }
            let mut child_env = BTreeMap::from([
                ("BUZZ_ACP_AGENT_COMMAND".into(), "buzz-agent".into()),
                ("BUZZ_ACP_MCP_COMMAND".into(), "buzz-dev-mcp".into()),
                ("BUZZ_ACP_DISPLAY_NAME".into(), name.to_owned()),
            ]);
            for (key, source) in &spec.env {
                ensure!(
                    !key.is_empty()
                        && !key.starts_with(|c: char| c.is_ascii_digit())
                        && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
                    "invalid environment variable name `{key}`"
                );
                ensure!(
                    !RESERVED.contains(&key.as_str())
                        && !private_sources.contains(key)
                        && !source
                            .env_key()
                            .is_some_and(|source| private_sources.contains(source)),
                    "`env` may not override identity source `{key}`"
                );
                child_env.insert(
                    key.clone(),
                    source
                        .resolve(env)
                        .with_context(|| format!("agent `{name}`: env.{key}"))?
                        .expose()
                        .to_owned(),
                );
            }
            child_env.insert(
                "BUZZ_PRIVATE_KEY".into(),
                identity.secret.expose().to_owned(),
            );
            child_env.insert(
                "NOSTR_PRIVATE_KEY".into(),
                identity.secret.expose().to_owned(),
            );
            child_env.insert("BUZZ_AUTH_TAG".into(), identity.auth_tag);
            let relays = spec
                .relays
                .as_ref()
                .filter(|relays| !relays.is_empty())
                .with_context(|| format!("agent `{name}` needs a non-empty `relays` list"))?;
            for relay in relays {
                let relay = relay_url(relay)?;
                ensure!(
                    seen_connections
                        .insert((identity.pubkey.clone(), normalize_relay_url(&relay)?)),
                    "duplicate identity on relay {relay}"
                );
                let mut connection_env = child_env.clone();
                connection_env.insert("BUZZ_RELAY_URL".into(), relay.clone());
                agents.push(AgentPlan {
                    name: format!("{name}@{relay}"),
                    pubkey: identity.pubkey.clone(),
                    relay_url: relay,
                    program: program.clone(),
                    args: Vec::new(),
                    workdir: workdir.clone(),
                    env: connection_env,
                    env_remove: env_remove.clone(),
                    restart: spec.restart.unwrap_or_default(),
                    max_restarts: spec.max_restarts.unwrap_or(10),
                });
            }
            selected.insert(name);
            if let Some(key) = identity.generated {
                generated_keys.push(key);
            }
        }
        for name in only {
            ensure!(
                selected.contains(&name.as_str()),
                "--only names an unknown or disabled agent `{name}`"
            );
        }
        ensure!(!agents.is_empty(), "no enabled agents selected");
        Ok(Self {
            owner_pubkey: owner_pubkey.context("no agent owner")?,
            agents,
            generated_keys,
            new_workdirs,
        })
    }
}

// Resolve only the executable Swarm launches. The harness owns its subprocess
// configuration, including PATH lookup for alternative ACP agents and MCPs.
fn executable(name: &str, env: &Env) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let candidates = if name.contains('/') {
        vec![PathBuf::from(name)]
    } else {
        std::env::split_paths(env.get("PATH").unwrap_or_default())
            .map(|directory| directory.join(name))
            .collect()
    };
    let is_executable = |path: &PathBuf| {
        std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    candidates
        .into_iter()
        .find(is_executable)
        .map(std::path::absolute)
        .transpose()?
        .with_context(|| {
            format!("cannot find executable `{name}`; install buzz-acp or Sprig's personality symlinks on PATH")
        })
}

/// Normalize equivalent relay addresses before checking duplicate identities.
pub fn relay_url(community: &str) -> Result<String> {
    let community = community.trim();
    ensure!(!community.is_empty(), "`relays` is empty");
    let url = if let Some(rest) = community.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = community.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if community.starts_with("wss://") || community.starts_with("ws://") {
        community.to_owned()
    } else if let Some((scheme, _)) = community.split_once("://") {
        bail!("`relays: {community}` uses unsupported scheme `{scheme}`");
    } else {
        format!("wss://{community}")
    };
    let parsed = url::Url::parse(&url).context("invalid relay URL")?;
    normalize_relay_url(parsed.as_str())?;
    Ok(if parsed.path() == "/" && parsed.query().is_none() {
        parsed.as_str().trim_end_matches('/').to_owned()
    } else {
        parsed.to_string()
    })
}

#[cfg(test)]
mod tests;
