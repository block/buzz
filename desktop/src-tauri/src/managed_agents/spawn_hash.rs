//! Spawn-time config hash for the restart-required badge.
//!
//! [`spawn_config_hash`] digests the *effective spawned values* — what a
//! process launch of `record` would actually receive — so the UI can compare
//! a running process's hash (stamped on [`super::ManagedAgentProcess`] at
//! spawn) against a recomputation from current disk state and show a
//! "restart required" badge only when a restart would change what runs.
//!
//! Scope rules (decided in #centralize-personas-and-agents):
//! - Inputs mirror `spawn_agent_child`: the live-persona-resolved harness
//!   command (persona runtime edits DO propagate on restart), its derived
//!   args/mcp command, the effective env layering, and the record fields the
//!   spawn env writes read.
//! - Persona prompt/model/provider edits are EXCLUDED: spawn reads the pinned
//!   record snapshot, so a restart would not apply them — that drift is
//!   `persona_out_of_date`'s respawn signal, not a restart signal.
//! - Channel membership is not an input: agents pick up channel changes live
//!   (#1468), never via restart.
//!
//! The hash never crosses a process or persistence boundary, so
//! `DefaultHasher` (not stable across Rust releases) is sufficient.

use std::hash::{DefaultHasher, Hash, Hasher};

use super::{
    effective_agent_command, known_acp_runtime, normalize_agent_args, resolve_effective_agent_env,
    types::{ManagedAgentRecord, PersonaRecord},
};

/// Digest the effective spawn configuration of `record` under the current
/// `personas`. Pure — no `AppHandle`, no disk, no keyring.
pub(crate) fn spawn_config_hash(record: &ManagedAgentRecord, personas: &[PersonaRecord]) -> u64 {
    let effective_command = effective_agent_command(
        record.persona_id.as_deref(),
        personas,
        record.agent_command_override.as_deref(),
    );
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

    // Record fields the spawn env writes read directly.
    record.relay_url.hash(&mut hasher);
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
