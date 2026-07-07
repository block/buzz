//! Spawn-time config hash for the restart-required badge.
//!
//! [`spawn_config_hash`] digests the *effective spawned values* — what a
//! process launch of `record` would actually receive — so the UI can compare
//! a running process's hash (stamped on [`super::ManagedAgentProcess`] at
//! spawn) against a recomputation from current disk state and show a
//! "restart required" badge only when a restart would change what runs.
//!
//! Scope rules (decided in #centralize-personas-and-agents, revised in PR
//! #1602 review):
//! - Inputs mirror what a start would actually run: the start/restore paths
//!   re-snapshot the linked persona's prompt/model/provider/env onto the
//!   record immediately before spawning (`start_local_agent_with_preflight`,
//!   `restore_managed_agents_on_launch`), so persona edits to those fields DO
//!   apply on a plain restart and are hashed via the same prospective
//!   re-snapshot. Harness command, args/mcp, env layering, and the record
//!   fields the spawn env writes read are hashed as spawn resolves them.
//! - The relay URL is hashed in resolved form (`effective_agent_relay_url`):
//!   a record with a blank relay spawns against the active workspace relay,
//!   so a workspace relay change means a restart would change what runs.
//! - Channel membership is not an input: agents pick up channel changes live
//!   (#1468), never via restart.
//!
//! The hash never crosses a process or persistence boundary, so
//! `DefaultHasher` (not stable across Rust releases) is sufficient.

use std::hash::{DefaultHasher, Hash, Hasher};

use super::{
    known_acp_runtime, normalize_agent_args,
    persona_events::persona_snapshot_with_agent_config_fallback,
    resolve_effective_agent_env,
    types::{ManagedAgentRecord, PersonaRecord},
};

/// Digest the effective spawn configuration of `record` under the current
/// `personas`, resolving a blank record relay against `workspace_relay`.
/// Pure — no `AppHandle`, no disk, no keyring.
pub(crate) fn spawn_config_hash(
    record: &ManagedAgentRecord,
    personas: &[PersonaRecord],
    workspace_relay: &str,
) -> u64 {
    // Prospective re-snapshot: mirror the mutation start/restore apply to the
    // record right before spawning, so the hash covers what a restart would
    // actually run. Idempotent, so the spawn-time stamp (post-snapshot record)
    // and later recomputes (persisted record) agree when nothing changed.
    let mut record = record.clone();
    if let Some(persona_id) = record.persona_id.clone() {
        if let Some(persona) = personas.iter().find(|p| p.id == persona_id) {
            let snapshot = persona_snapshot_with_agent_config_fallback(
                persona,
                &record.env_vars,
                record.model.as_deref(),
                record.provider.as_deref(),
            );
            if let Some(prompt) = snapshot.system_prompt {
                record.system_prompt = Some(prompt);
            }
            record.model = snapshot.model;
            record.provider = snapshot.provider;
            record.env_vars = snapshot.env_vars;
        }
    }
    let record = &record;

    let effective_command = crate::managed_agents::record_agent_command(record, personas);
    let runtime_meta = known_acp_runtime(&effective_command);
    let effective = resolve_effective_agent_env(record, personas, runtime_meta);

    let mut hasher = DefaultHasher::new();

    // Harness identity and derivations (live-persona-resolved, like spawn).
    record.acp_command.hash(&mut hasher);
    effective_command.hash(&mut hasher);
    normalize_agent_args(&effective_command, record.agent_args.clone()).hash(&mut hasher);
    runtime_meta
        .and_then(|r| r.mcp_command)
        .unwrap_or("")
        .hash(&mut hasher);

    // Effective env layering (baked floor → runtime metadata → user env).
    // BTreeMap iteration is ordered, so this is deterministic.
    effective.env.hash(&mut hasher);

    // Record fields the spawn env writes read directly. The relay is hashed
    // resolved: a blank record relay spawns on the workspace relay, so a
    // workspace relay change must trip the badge.
    crate::relay::effective_agent_relay_url(&record.relay_url, workspace_relay).hash(&mut hasher);
    record.system_prompt.hash(&mut hasher);
    record.model.hash(&mut hasher);
    record.provider.hash(&mut hasher);
    record.auth_tag.hash(&mut hasher);
    record.respond_to.as_str().hash(&mut hasher);
    record.respond_to_allowlist.hash(&mut hasher);
    record.idle_timeout_seconds.hash(&mut hasher);
    record.max_turn_duration_seconds.hash(&mut hasher);
    record.parallelism.hash(&mut hasher);
    record.mcp_toolsets.hash(&mut hasher);
    record.persona_team_dir.hash(&mut hasher);
    record.persona_name_in_team.hash(&mut hasher);

    hasher.finish()
}

#[cfg(test)]
mod tests;
