//! Spawn-time config hash for the restart-required badge.
//!
//! [`spawn_config_hash`] digests the *effective spawned values* — what a
//! process launch of `record` would actually receive — so the UI can compare
//! a running process's hash (stamped on [`super::ManagedAgentProcess`] at
//! spawn) against a recomputation from current disk state and show a
//! "restart required" badge only when a restart would change what runs.
//!
//! Since every execution path consumes the shared launch contract
//! (`managed_agents::launch::resolve_launch_spec`), the badge question *is*
//! the contract question: "restart needed" means "the contract changed".
//! The hash therefore digests the contract's effective view — command, args,
//! MCP command, and the merged environment (`policy_env` under `env`, user
//! values winning, exactly as a body applies them) — plus the few spawn
//! inputs that legitimately live outside the contract:
//! - `record.acp_command`: the outer harness binary is host machinery, not
//!   launch configuration;
//! - the relay URL, hashed in resolved form (`effective_agent_relay_url`):
//!   every record spawns against the active workspace relay (legacy
//!   per-record pins are ignored), so a workspace relay change means a
//!   restart would change what runs;
//! - `auth_tag`: carried beside the contract, not inside it;
//! - the resolved provider: a provider flip on a runtime with no provider
//!   env var does not touch the contract, but the mesh-llm path derives
//!   spawn-time transport env from it.
//!
//! Hashing the *merged* environment (not the two maps separately) is what
//! keeps the badge honest under overrides: a session-title rename beneath a
//! user `BUZZ_ACP_SESSION_TITLE` override changes `policy_env` but not what
//! runs, and the merged view does not change either.
//!
//! Scope rules (decided in #centralize-personas-and-agents, revised in PR
//! #1602 review): inputs mirror what a start would actually run — the
//! start/restore paths re-snapshot the linked persona onto the record
//! immediately before spawning, so persona edits DO apply on a plain restart
//! and are hashed via the same prospective re-snapshot. Channel membership
//! is not an input: agents pick up channel changes live (#1468), never via
//! restart.
//!
//! The hash never crosses a process or persistence boundary, so
//! `DefaultHasher` (not stable across Rust releases) is sufficient.

use std::hash::{DefaultHasher, Hash, Hasher};

use super::{
    effective_config::{resolve_effective_config, EffectiveConfigResult},
    normalize_agent_args,
    persona_events::preview_prospective_persona_snapshot,
    types::{AgentDefinition, ManagedAgentRecord, TeamRecord},
    GlobalAgentConfig,
};

/// Resolve the current instructions for this instance's deployment-time team binding.
/// A deleted team deliberately degrades to no team section.
pub(crate) fn effective_team_instructions(
    record: &ManagedAgentRecord,
    teams: &[TeamRecord],
) -> Option<String> {
    teams
        .iter()
        .find(|team| Some(team.id.as_str()) == record.team_id.as_deref())
        .and_then(|team| team.instructions.as_deref())
        .map(str::trim)
        .filter(|instructions| !instructions.is_empty())
        .map(str::to_string)
}

/// Digest the effective spawn configuration of `record` under the current
/// `personas`, resolving a blank record relay against `workspace_relay`.
/// Pure — no `AppHandle`, no disk, no keyring.
pub(crate) fn spawn_config_hash(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    teams: &[TeamRecord],
    workspace_relay: &str,
    global: &GlobalAgentConfig,
) -> u64 {
    // Prospective re-snapshot: apply the same `apply_persona_snapshot` the
    // start/restore paths run right before spawning, so the hash covers what a
    // restart would actually run. Idempotent, so the spawn-time stamp
    // (post-snapshot record) and later recomputes (persisted record) agree
    // when nothing changed. The persona env itself reaches the hash through
    // the descriptor's layered env below; `persona_source_version` is set on
    // the clone but is not a hash input.
    let record = preview_prospective_persona_snapshot(record, personas);
    let record = &record;

    // Resolve command, args, and env via the single typed descriptor — same path
    // as spawn_agent_child.  Dangling harness id falls back to the infallible
    // record_agent_command (no-op: a dangling harness can't be spawned, so the
    // hash never matters for that agent).
    let descriptor =
        crate::managed_agents::resolve_effective_harness_descriptor(record, personas, global)
            .unwrap_or_else(|_| {
                let cmd = crate::managed_agents::record_agent_command(record, personas);
                let args = normalize_agent_args(&cmd, record.agent_args.clone());
                crate::managed_agents::readiness::EffectiveHarnessDescriptor {
                    command: cmd,
                    args,
                    env: Default::default(),
                }
            });

    let mut hasher = DefaultHasher::new();

    // Non-contract spawn inputs (see module docs).
    record.acp_command.hash(&mut hasher);
    crate::relay::effective_agent_relay_url(&record.relay_url, workspace_relay).hash(&mut hasher);
    record.auth_tag.hash(&mut hasher);

    // Prompt, model, and provider come from ONE `resolve_effective_config`
    // call — the SAME resolve `spawn_agent_child` feeds into the contract, so
    // env write and this badge cannot disagree. An orphaned link (missing
    // definition) hashes as if all three were absent: `spawn_agent_child`
    // refuses to spawn an orphan regardless, so this is a display-only
    // convenience, not the spawn gate.
    let (resolved_prompt, resolved_model, resolved_provider) =
        match resolve_effective_config(record, personas, global) {
            EffectiveConfigResult::Resolved(cfg) => {
                (cfg.system_prompt.value, cfg.model.value, cfg.provider.value)
            }
            EffectiveConfigResult::OrphanedInstance { .. } => (None, None, None),
        };
    // The provider outside the contract: the mesh-llm path derives spawn-time
    // transport env from it even when the runtime exports no provider var.
    resolved_provider.hash(&mut hasher);

    // The launch contract's effective view. The owner is deliberately not an
    // input (`owner_hex: None`): both the spawn-time stamp and every
    // recompute use this same resolve, and an owner change never requires a
    // restart badge — it is workspace identity, not agent configuration.
    match super::launch::resolve_launch_spec(
        record,
        &descriptor,
        teams,
        resolved_prompt.as_deref(),
        resolved_model.as_deref(),
        resolved_provider.as_deref(),
        None,
    ) {
        Ok(launch) => {
            launch.command.hash(&mut hasher);
            launch.args.hash(&mut hasher);
            launch.mcp_command.hash(&mut hasher);
            // Merged effective environment: policy below, user env above —
            // the same precedence every body applies. Hashing the merged
            // view (BTreeMap iteration is ordered and deterministic) is what
            // keeps no-op edits quiet: a change beneath a user override does
            // not change what runs, and does not change this map either.
            let mut effective_env = launch.policy_env;
            effective_env.extend(launch.env);
            effective_env.hash(&mut hasher);
        }
        Err(error) => {
            // A record the resolver refuses (e.g. a malformed allowlist
            // edit) cannot produce the hash a successful spawn stamped, so
            // it correctly compares unequal; hash the refusal itself so two
            // differently-broken edits still differ.
            error.hash(&mut hasher);
            record.respond_to_allowlist.hash(&mut hasher);
        }
    }

    hasher.finish()
}

#[cfg(test)]
mod tests;
