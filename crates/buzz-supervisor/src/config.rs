//! Team roster configuration — which roles get provisioned per channel and
//! how each one is launched. Loaded from a TOML file at startup.

use serde::Deserialize;
use std::fs;
use std::path::Path;

fn default_agent_command() -> String {
    "claude-agent-acp".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleConfig {
    /// Stable identifier, used for state-file paths (e.g. "coder").
    pub name: String,
    /// Human-readable name published in the agent's `kind:0`/`kind:10100`
    /// profiles (e.g. "Coder").
    pub display_name: String,
    /// `BUZZ_ACP_AGENT_COMMAND` for this role's `buzz-acp` process.
    #[serde(default = "default_agent_command")]
    pub agent_command: String,
    /// Optional file whose contents become `BUZZ_ACP_SYSTEM_PROMPT`.
    pub system_prompt_file: Option<String>,
    /// If true, this role is launched with `--subscribe all` instead of the
    /// default `mentions` — see relay.md's "one listener per channel" note.
    /// At most one role per team should set this.
    #[serde(default)]
    pub subscribe_all: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorConfig {
    /// Roles that get a fresh keypair + a spawned `buzz-acp` process per
    /// provisioned channel.
    pub roles: Vec<RoleConfig>,
    /// Pubkeys added as plain (no process) channel members to every
    /// provisioned channel — e.g. an externally-bridged Reviewer identity.
    #[serde(default)]
    pub shared_members: Vec<String>,
    /// Pubkeys always included in every role's `respond_to_allowlist`,
    /// beyond the team's own generated pubkeys — typically the human
    /// owner(s) who should be able to trigger any role directly.
    #[serde(default)]
    pub extra_allowlist: Vec<String>,
}

impl SupervisorConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading roles file {}: {e}", path.display()))?;
        let config: Self = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("parsing roles file {}: {e}", path.display()))?;
        if config.roles.is_empty() {
            anyhow::bail!("roles file {} defines no roles", path.display());
        }
        let subscribe_all_count = config.roles.iter().filter(|r| r.subscribe_all).count();
        if subscribe_all_count > 1 {
            tracing::warn!(
                count = subscribe_all_count,
                "more than one role has subscribe_all=true — they will all react to every \
                 unaddressed message in the channel, which is usually not intended"
            );
        }
        Ok(config)
    }
}
