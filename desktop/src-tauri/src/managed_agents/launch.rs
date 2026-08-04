//! Shared launch-contract resolution for every execution path.
//!
//! Local spawn (`runtime.rs`), provider deployment (`agents_deploy.rs`), and
//! execution-node deployment (`execution_nodes.rs`) hand a body the same
//! resolved command, arguments, environment, and launch policy — all three
//! consume the [`LaunchSpec`] this module resolves, so no body can drift
//! from what the same agent would receive on another substrate. What stays
//! path-local is adaptation only: local spawn resolves command names to host
//! paths and applies inherited-environment semantics; substrates and
//! providers do their own equivalent.

use std::collections::BTreeMap;

use buzz_core_pkg::execution::LaunchSpec;

use super::readiness::EffectiveHarnessDescriptor;
use super::{
    known_acp_runtime, resolve_session_title, ManagedAgentRecord, TeamRecord, SESSION_TITLE_ENV_VAR,
};

/// The developer MCP command for one effective agent command — the single
/// derivation shared by the launch contract and the spawn-config snapshot,
/// so the restart badge can never disagree with what a body actually runs.
pub(crate) fn effective_mcp_command(effective_command: &str) -> Option<String> {
    known_acp_runtime(effective_command)
        .and_then(|meta| meta.mcp_command)
        .filter(|command| !command.is_empty())
        .map(str::to_string)
}

/// Resolve the portable launch contract for one managed agent.
///
/// `descriptor` is the single authoritative command/args/env resolution from
/// `resolve_effective_harness_descriptor`; `descriptor.env` becomes
/// `LaunchSpec::env` (the layer user values win from). Everything Buzz sets
/// on its own authority lands in `policy_env`, applied *below* `env` by the
/// consuming substrate — the same ordering local spawn uses.
pub(crate) fn resolve_launch_spec(
    record: &ManagedAgentRecord,
    descriptor: &EffectiveHarnessDescriptor,
    teams: &[TeamRecord],
    effective_prompt: Option<&str>,
    effective_model: Option<&str>,
    effective_provider: Option<&str>,
    owner_pubkey: Option<&str>,
) -> Result<LaunchSpec, String> {
    let runtime_meta = known_acp_runtime(&descriptor.command);
    let mut policy_env: BTreeMap<String, String> = BTreeMap::new();

    if let Some(meta) = runtime_meta {
        policy_env.extend(
            meta.default_env
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string())),
        );
        // Uses "*" because build_mcp_servers() hard-codes the server name.
        if meta.mcp_hooks {
            policy_env.insert("MCP_HOOK_SERVERS".into(), "*".into());
        }
        // Runtime-native model/provider variables (e.g. GOOSE_MODEL) — the
        // same injection local spawn performs, so remote bodies do not fall
        // back to the runtime's own default model.
        for (key, value) in super::runtime_metadata_env_vars(
            meta.model_env_var,
            meta.provider_env_var,
            meta.provider_locked,
            effective_model,
            effective_provider,
        ) {
            policy_env.insert(key.to_string(), value.to_string());
        }
    }

    policy_env.insert("BUZZ_ACP_RELAY_OBSERVER".into(), "true".into());
    policy_env.insert("BUZZ_ACP_LAZY_POOL".into(), "true".into());
    policy_env.insert("BUZZ_ACP_MULTIPLE_EVENT_HANDLING".into(), "steer".into());
    policy_env.insert("BUZZ_ACP_DEDUP".into(), "queue".into());
    policy_env.insert("BUZZ_ACP_AGENTS".into(), record.parallelism.to_string());

    if let Some(value) = effective_prompt {
        policy_env.insert("BUZZ_ACP_SYSTEM_PROMPT".into(), value.to_string());
    }
    if let Some(value) = effective_model {
        policy_env.insert("BUZZ_ACP_MODEL".into(), value.to_string());
    }
    if let Some(value) = record.idle_timeout_seconds {
        policy_env.insert("BUZZ_ACP_IDLE_TIMEOUT".into(), value.to_string());
    }
    if let Some(value) = record.max_turn_duration_seconds {
        policy_env.insert("BUZZ_ACP_MAX_TURN_DURATION".into(), value.to_string());
    }
    if let Some(value) = resolve_session_title(record.display_name.as_deref(), &record.name) {
        policy_env.insert(SESSION_TITLE_ENV_VAR.into(), value);
    }
    if let Some(value) = super::spawn_snapshot::effective_team_instructions(record, teams) {
        policy_env.insert("BUZZ_ACP_TEAM_INSTRUCTIONS".into(), value);
    }

    // Inbound author gate, with the same strict validation on every path.
    // The removal half is handled by consumers: remote substrates start from
    // an empty environment (absence is the unset), local spawn clears the
    // policy-managed keys the resolver left out of the map.
    let (gate_set, _gate_remove) = super::build_respond_to_env(record, owner_pubkey)?;
    for (key, value) in gate_set {
        policy_env.insert(key.to_string(), value);
    }

    // Vestigial-but-live MCP server binary (see `KnownAcpRuntime::mcp_command`):
    // carried as a command *name*; substrates resolve it like `command`.
    let mcp_command = effective_mcp_command(&descriptor.command);

    LaunchSpec::new(
        descriptor.command.clone(),
        descriptor.args.clone(),
        mcp_command,
        descriptor.env.clone(),
        policy_env,
        owner_pubkey.map(str::to_string),
    )
    .map_err(|error| format!("invalid launch contract: {error}"))
}
