//! Immutable harness descriptor resolution shared by launch and inspection.
use super::{resolve_effective_agent_env_with_def, ManagedAgentRecord};
use crate::managed_agents::{discovery::known_acp_runtime, normalize_agent_args};
use std::collections::BTreeMap;

// ── Typed effective-harness descriptor ───────────────────────────────────────
//
// A single owned type that fully describes what a spawn would run.  Produced
// by `resolve_effective_harness_descriptor` and consumed by spawn_agent_child,
// spawn_snapshot, build_managed_agent_summary, get_agent_models, and
// agent_readiness — so the harness-definition lookup and arg/env resolution
// happen exactly once, in one place.

/// The complete effective description of a harness spawn: resolved command,
/// args, and layered env.  This is the single source of truth for what will
/// actually run — computed once and shared across every consumer that needs
/// the effective values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EffectiveHarnessDescriptor {
    /// The raw effective command string (e.g. `"buzz-agent"`, `"my-acp-agent"`).
    /// Used for `known_acp_runtime` lookup and hashing.
    pub command: String,
    /// Normalized effective args.  Instance args win when non-empty; otherwise
    /// the harness definition's args apply.
    pub args: Vec<String>,
    /// The full layered process env: baked floor → runtime metadata → definition
    /// env → global → persona → agent.
    pub env: BTreeMap<String, String>,
}

/// Resolve the complete harness descriptor from a record + context — the single
/// authoritative path for command, args, and env.
///
/// This is the only place where harness-definition lookup and arg/env layering
/// happen; spawn, hash, summary, and both model-probe paths all consume this.
///
/// Returns `Err("DANGLING_HARNESS_ID:<id>")` when the record (or its linked
/// persona) references a runtime id that no longer exists in the registry —
/// the same typed error produced by `try_record_agent_command`.  Callers that
/// cannot meaningfully continue with a dangling id (e.g. `spawn_agent_child`)
/// propagate the error; callers that degrade gracefully may use
/// `.unwrap_or_else(|_| …)`.
///
/// Does NOT require an `AppHandle` so it is fully unit-testable.
///
/// # Arguments
/// * `record` — the managed agent record
/// * `personas` — all current personas (for command/env resolution)
/// * `global` — global agent config defaults
pub(crate) fn resolve_effective_harness_descriptor(
    record: &ManagedAgentRecord,
    personas: &[crate::managed_agents::types::AgentDefinition],
    global: &crate::managed_agents::GlobalAgentConfig,
) -> Result<EffectiveHarnessDescriptor, String> {
    // A transient projection, never persisted back onto the stable agent/persona.
    let projected;
    let record =
        if let Some(config) = crate::managed_agents::runtime_configurations::selected(record)? {
            projected = {
                let mut copy = record.clone();
                copy.runtime = Some(config.runtime.clone());
                copy.agent_args.clear();
                copy.agent_command_override = None;
                copy
            };
            &projected
        } else {
            record
        };
    let effective_command = crate::managed_agents::try_record_agent_command(record, personas)?;
    let runtime_meta = known_acp_runtime(&effective_command);

    // Look up the harness definition once — used for both args and env.
    // Resolution order: record.runtime → persona.runtime → "".
    let harness_def = {
        let runtime_id = record
            .runtime
            .as_deref()
            .or_else(|| {
                record.persona_id.as_deref().and_then(|pid| {
                    personas
                        .iter()
                        .find(|p| p.id == pid)
                        .and_then(|p| p.runtime.as_deref())
                })
            })
            .unwrap_or("");
        crate::managed_agents::custom_harnesses::lookup_loaded_harness_by_id(runtime_id)
    };

    // Args: explicit non-empty instance args win; otherwise use definition args.
    let args = {
        let record_args = record.agent_args.clone();
        let instance_has_args = record_args.iter().any(|a| !a.trim().is_empty());
        if instance_has_args {
            normalize_agent_args(&effective_command, record_args)
        } else if let Some(ref def) = harness_def {
            normalize_agent_args(&effective_command, def.args.clone())
        } else {
            normalize_agent_args(&effective_command, record_args)
        }
    };

    // Env: full layered resolution (same as resolve_effective_agent_env).
    // Pass harness_def directly to avoid a second lookup.
    let effective_env =
        resolve_effective_agent_env_with_def(record, personas, runtime_meta, global, harness_def);

    let mut descriptor = EffectiveHarnessDescriptor {
        command: effective_command,
        args,
        env: effective_env.env,
    };
    crate::managed_agents::runtime_configurations::apply_descriptor(record, &mut descriptor)?;
    Ok(descriptor)
}
