//! Spawn-time config snapshot for the restart-required badge.
//!
//! [`SpawnConfigSnapshot`] captures the *effective spawned values* — what a
//! process launch of a record would actually receive. The running process
//! stamps one on [`super::ManagedAgentProcess`] at spawn; the summary builder
//! recomputes a prospective one from current disk state and compares. Drift
//! means a restart would change what runs, and the field-by-field difference
//! is what the UI shows (see [`diff`]).
//!
//! Scope rules (decided in #centralize-personas-and-agents, revised in PR
//! #1602 review):
//! - Inputs mirror what a start would actually run: the start/restore paths
//!   re-snapshot the linked persona's prompt/model/provider/env onto the
//!   record immediately before spawning (`start_local_agent_with_preflight`,
//!   `restore_managed_agents_on_launch`), so persona edits to those fields DO
//!   apply on a plain restart and reach the prospective snapshot via the same
//!   re-snapshot. Harness command, args/mcp, env layering, and the record
//!   fields the spawn env writes read are captured as spawn resolves them.
//! - The relay URL is captured in resolved form (`effective_agent_relay_url`):
//!   every record spawns against the active workspace relay (legacy per-record
//!   pins are ignored), so a workspace relay change means a restart would
//!   change what runs.
//! - Channel membership is not an input: agents pick up channel changes live
//!   (#1468), never via restart.
//!
//! The snapshot never crosses a process or persistence boundary — it is
//! runtime state only, held on the running `ManagedAgentProcess`.

use std::collections::BTreeMap;

use serde::Serialize;

use super::{
    effective_config::{resolve_effective_config, EffectiveConfigResult},
    known_acp_runtime, normalize_agent_args,
    persona_events::preview_prospective_persona_snapshot,
    readiness::EffectiveHarnessDescriptor,
    runtime::{resolve_session_title, SESSION_TITLE_ENV_VAR},
    types::{AgentDefinition, ManagedAgentRecord, TeamRecord},
    GlobalAgentConfig,
};

pub(crate) mod diff;
pub(crate) use diff::{eligible_restart_diff, RestartDiffEntry, TrackedSpawnState};

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

/// The already-resolved values a spawn feeds into its `Command`.
///
/// Taking them rather than re-resolving is what makes the stamp describe the
/// process that was actually launched: a persona/harness/global edit landing
/// between spawn's resolution and the stamp can no longer suppress the badge.
pub(crate) struct SpawnConfigInputs<'a> {
    pub record: &'a ManagedAgentRecord,
    pub descriptor: &'a EffectiveHarnessDescriptor,
    /// Resolved workspace/pair relay — never the record's legacy pin.
    pub relay_url: &'a str,
    pub team_instructions: Option<&'a str>,
    pub system_prompt: Option<&'a str>,
    pub model: Option<&'a str>,
    pub provider: Option<&'a str>,
}

/// The effective spawn configuration of one managed-agent process.
///
/// Serialization invariants (load-bearing — the drift comparison and the diff
/// walk both read `canonical()`):
/// - plain derived `Serialize`: no `flatten`, no `skip_serializing_if`, no
///   custom or fallible field serializers, no colliding serialized names, so
///   every field is always present on both sides of a comparison;
/// - `Option::None` serializes as JSON `null`; a *missing* key is reserved for
///   dynamic-map membership (`env.<KEY>` added/removed);
/// - arrays are atomic leaves — `args` and `respond_to_allowlist` compare and
///   render whole, never element-wise.
///
/// `Debug` is implemented by hand: [`ManagedAgentProcess`] derives `Debug`, so
/// a derived impl here would print env values, auth tags, and CLI arguments.
///
/// [`ManagedAgentProcess`]: super::ManagedAgentProcess
#[derive(Clone, Serialize)]
pub(crate) struct SpawnConfigSnapshot {
    /// The ACP harness binary the desktop launches (`buzz-acp`).
    pub acp_command: String,
    /// The effective agent command the harness drives.
    pub command: String,
    pub args: Vec<String>,
    /// Catalog-derived from `command`; `""` when the runtime has none.
    pub mcp_command: String,
    /// Fully layered process env: baked floor -> runtime metadata ->
    /// definition -> global -> persona -> agent.
    pub env: BTreeMap<String, String>,
    pub relay_url: String,
    pub team_instructions: Option<String>,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    /// `None` when a user env override shadows `BUZZ_ACP_SESSION_TITLE`: spawn
    /// writes the title BEFORE the user env layer, so the override is what
    /// actually runs and it already reaches this snapshot through `env`.
    /// Capturing the record-derived value under an override would badge a
    /// rename that changes nothing.
    pub session_title: Option<String>,
    pub auth_tag: Option<String>,
    pub respond_to: String,
    /// `None` outside allowlist mode — spawn sets
    /// `BUZZ_ACP_RESPOND_TO_ALLOWLIST` only there, so edits to a dormant list
    /// must not badge. Normalized (trim/lowercase/dedup) as the env receives
    /// it, so edits that don't survive normalization must not badge either.
    pub respond_to_allowlist: Option<Vec<String>>,
    pub idle_timeout_seconds: Option<u64>,
    pub max_turn_duration_seconds: Option<u64>,
    pub parallelism: u32,
}

impl SpawnConfigSnapshot {
    /// Assemble the snapshot from values a spawn has already resolved.
    pub(crate) fn from_inputs(inputs: SpawnConfigInputs<'_>) -> Self {
        let SpawnConfigInputs {
            record,
            descriptor,
            relay_url,
            team_instructions,
            system_prompt,
            model,
            provider,
        } = inputs;
        Self {
            acp_command: record.acp_command.clone(),
            command: descriptor.command.clone(),
            args: descriptor.args.clone(),
            mcp_command: known_acp_runtime(&descriptor.command)
                .and_then(|runtime| runtime.mcp_command)
                .unwrap_or("")
                .to_string(),
            env: descriptor.env.clone(),
            relay_url: relay_url.to_string(),
            team_instructions: team_instructions.map(str::to_string),
            system_prompt: system_prompt.map(str::to_string),
            model: model.map(str::to_string),
            provider: provider.map(str::to_string),
            session_title: (!descriptor.env.contains_key(SESSION_TITLE_ENV_VAR))
                .then(|| resolve_session_title(record.display_name.as_deref(), &record.name))
                .flatten(),
            auth_tag: record.auth_tag.clone(),
            respond_to: record.respond_to.as_str().to_string(),
            respond_to_allowlist: (record.respond_to == super::types::RespondTo::Allowlist).then(
                || {
                    // A list spawn would reject is captured raw: the stamped
                    // snapshot comes from a successful spawn, so any invalid
                    // edit correctly compares unequal.
                    super::types::validate_respond_to_allowlist(&record.respond_to_allowlist)
                        .unwrap_or_else(|_| record.respond_to_allowlist.clone())
                },
            ),
            idle_timeout_seconds: record.idle_timeout_seconds,
            max_turn_duration_seconds: record.max_turn_duration_seconds,
            // Hash the effective parallelism so over-cap edits that don't change
            // the running pool size (e.g. 10 → 8, both clamp to 5 on OpenClaw)
            // do not raise a spurious "restart required" badge. Cap crossings
            // (e.g. 8 → 3, where 3 is below the cap) do change the effective
            // pool and must badge. The diff surface consequently displays the
            // effective value — that is correct, it is what actually runs.
            parallelism: super::effective_parallelism(&descriptor.command, record.parallelism),
        }
    }

    /// Canonical JSON projection — the single representation both the drift
    /// comparison and the diff walk read, so a lit badge always has a
    /// non-empty diff and vice versa.
    ///
    /// Infallible by the serialization invariants documented on the struct
    /// (plain derive over strings, scalars, string maps, and string vectors);
    /// a failure here is a broken invariant, never a runtime condition, so it
    /// must not degrade into an empty diff.
    pub(crate) fn canonical(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("SpawnConfigSnapshot serializes infallibly")
    }
}

impl std::fmt::Debug for SpawnConfigSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SpawnConfigSnapshot({})",
            diff::redacted_canonical(&self.canonical())
        )
    }
}

/// Snapshot the effective spawn configuration `record` would get if it were
/// started right now under the current `personas`/`teams`/`global`, resolving
/// a blank record relay against `workspace_relay`.
///
/// Pure — no `AppHandle`, no disk, no keyring. This is the *prospective* side
/// of the comparison; the stamped side is built at spawn from the values that
/// actually fed the child's `Command`.
pub(crate) fn prospective_spawn_config_snapshot(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    teams: &[TeamRecord],
    workspace_relay: &str,
    global: &GlobalAgentConfig,
) -> SpawnConfigSnapshot {
    // Prospective re-snapshot: apply the same `apply_persona_snapshot` the
    // start/restore paths run right before spawning, so this describes what a
    // restart would actually run. Idempotent, so a spawn-time stamp taken
    // after those paths saved the record compares equal when nothing changed.
    // The persona env itself arrives through the descriptor's layered env
    // below; `persona_source_version` is set on the clone but is not an input.
    let record = preview_prospective_persona_snapshot(record, personas);
    let record = &record;

    // Resolve command, args, and env via the single typed descriptor — same
    // path as spawn_agent_child. Dangling harness id falls back to the
    // infallible record_agent_command (no-op: a dangling harness can't be
    // spawned, so the snapshot never matters for that agent).
    let descriptor =
        crate::managed_agents::resolve_effective_harness_descriptor(record, personas, global)
            .unwrap_or_else(|_| {
                let command = crate::managed_agents::record_agent_command(record, personas);
                let args = normalize_agent_args(&command, record.agent_args.clone());
                EffectiveHarnessDescriptor {
                    command,
                    args,
                    env: Default::default(),
                }
            });

    // Prompt, model, and provider all come from ONE `resolve_effective_config`
    // call — the SAME resolve `spawn_agent_child` performs for the env write,
    // so env write and this badge cannot disagree. An orphaned link (missing
    // definition) resolves as if all three were absent: `spawn_agent_child`
    // refuses to spawn an orphan regardless, and `eligible_restart_diff`
    // suppresses the badge for one.
    let (prompt, model, provider) = match resolve_effective_config(record, personas, global) {
        EffectiveConfigResult::Resolved(cfg) => {
            (cfg.system_prompt.value, cfg.model.value, cfg.provider.value)
        }
        EffectiveConfigResult::OrphanedInstance { .. } => (None, None, None),
    };

    SpawnConfigSnapshot::from_inputs(SpawnConfigInputs {
        record,
        descriptor: &descriptor,
        // Resolved, not stored: every record spawns on the workspace relay
        // (legacy pins ignored), so a workspace relay change must badge.
        relay_url: &crate::relay::effective_agent_relay_url(&record.relay_url, workspace_relay),
        team_instructions: effective_team_instructions(record, teams).as_deref(),
        system_prompt: prompt.as_deref(),
        model: model.as_deref(),
        provider: provider.as_deref(),
    })
}

#[cfg(test)]
mod tests;
