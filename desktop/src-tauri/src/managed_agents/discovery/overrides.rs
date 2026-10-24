//! The `agent_command_override` decision family: divergence comparison,
//! create-time and update-time resolution, and the record apply. Child module
//! of `discovery` (split under the desktop file-size cap) so the override
//! decisions stay next to the resolution ladder they feed.

use super::{effective_agent_command, known_acp_runtime};

/// Decide whether a user-picked harness command is an explicit per-instance
/// pin or merely the persona's own runtime restated. Returns the override to
/// persist: `Some(picked)` when it diverges from the persona, `None` when it
/// inherits.
///
/// Comparison is by RUNTIME IDENTITY, not raw string: a persona on the `claude`
/// runtime resolves to `claude-agent-acp`, but a client with only the
/// `claude-code-acp` adapter installed sends that command instead. Both map to
/// the same `claude` runtime, so neither is a real divergence — string equality
/// would wrongly bake a pin. An unknown/custom command (no matching runtime)
/// only inherits when it exactly equals the persona command.
pub fn divergent_agent_command_override(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::AgentDefinition],
    picked_command: Option<&str>,
) -> Option<String> {
    let picked = picked_command
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let persona_command = effective_agent_command(persona_id, personas, None);
    let same_runtime = match (
        known_acp_runtime(picked),
        known_acp_runtime(&persona_command),
    ) {
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        _ => picked == persona_command,
    };
    if same_runtime {
        None
    } else {
        Some(picked.to_string())
    }
}

/// Decide the `agent_command_override` to persist at AGENT UPDATE time.
///
/// The edit dialog sends `agent_command` as a tri-state string: the empty
/// "inherit from persona" sentinel (clear the pin), or a concrete command
/// (pin). Resolution:
///
/// - EMPTY / whitespace → the inherit sentinel: always `None` regardless of
///   `harness_override`, so toggling "Inherit runtime from persona" clears the
///   pin.
/// - DELIBERATE OVERRIDE (`harness_override` true, persona linked): the user
///   explicitly picked a runtime/Custom command in the dialog. This is a real
///   pin and is preserved VERBATIM — even when the picked command maps to, or
///   is byte-identical to, the persona's own runtime command. Selecting "Custom
///   command" and saving e.g. `goose` for a goose persona is a deliberate act
///   to freeze the harness against future persona runtime edits; dropping it
///   back to inherit (as [`divergent_agent_command_override`] would) defeats
///   that intent. Unlike the create-time path, there is no byte-identical
///   exception here: at create the command is machine-derived from the persona,
///   so equality means "no user divergence"; at update an equal command reached
///   the force branch only because the user picked Custom, which IS the
///   divergence.
/// - NO OVERRIDE INTENT (`harness_override` false) or NO PERSONA: defer to
///   [`divergent_agent_command_override`], which keeps the persona authoritative
///   and treats a same-runtime restatement as inherit.
pub fn update_time_agent_command_override(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::AgentDefinition],
    picked_command: Option<&str>,
    harness_override: bool,
) -> Option<String> {
    let picked = picked_command
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    if persona_id.is_some() && harness_override {
        return Some(picked.to_string());
    }

    divergent_agent_command_override(persona_id, personas, Some(picked))
}

/// Apply an explicit `agent_command` edit to `record`: persist the override
/// pin decided by [`update_time_agent_command_override`], and on the inherit
/// sentinel (empty/whitespace command) also clear the materialized
/// `record.runtime` AND the persisted per-instance effort column so the
/// resolution ladder falls through to the live definition immediately instead
/// of silently keeping the stale instance copy.
///
/// The clears are guarded on a live persona link: for a definition-less record
/// the materialized runtime is the only harness source left after the override
/// clear, so a stray empty `agent_command` from a non-dialog caller must not
/// change what the agent runs.
///
/// Returns `true` when the pin→inherit transition fired. The caller MUST then,
/// AFTER applying any caller-supplied `env_vars`, strip the record effort env
/// aliases via [`remove_record_effort_aliases`] — clearing them here would be
/// undone by a same-request `env_vars` replacement (see the update boundary in
/// `agent_models_update.rs`), so the alias strip is an update-boundary
/// invariant, not a helper-local one.
#[must_use]
pub fn apply_agent_command_update(
    record: &mut crate::managed_agents::types::ManagedAgentRecord,
    personas: &[crate::managed_agents::types::AgentDefinition],
    agent_command: &str,
    harness_override: bool,
) -> bool {
    record.agent_command_override = update_time_agent_command_override(
        record.persona_id.as_deref(),
        personas,
        Some(agent_command),
        harness_override,
    );
    let inherit_transition = agent_command.trim().is_empty() && record.persona_id.is_some();
    if inherit_transition {
        record.runtime = None;
        // The generic canonical effort column is a per-instance pin; on the
        // pin→inherit transition it is dropped so the agent inherits the
        // persona/global effort. The record effort ENV aliases are stripped by
        // the caller after `env_vars` is applied (see the doc above).
        record.effort_level = None;
    }
    inherit_transition
}

/// Strip every record-level thinking-effort env alias — all known native keys
/// plus the legacy `BUZZ_AGENT_THINKING_EFFORT` alias — from `env_vars`.
///
/// Called at the `update_managed_agent` boundary on the pin→inherit transition,
/// AFTER caller-supplied `env_vars` have been applied, so the cleared aliases
/// cannot be reintroduced by the same request. Together with the column clear
/// in [`apply_agent_command_update`], this makes the instance drop its entire
/// per-instance effort override atomically at Save.
pub fn remove_record_effort_aliases(env_vars: &mut std::collections::BTreeMap<String, String>) {
    let suppress = crate::managed_agents::config_bridge::effort::effort_suppress_keys();
    env_vars.retain(|k, _| {
        !suppress
            .iter()
            .any(|suppressed| k.eq_ignore_ascii_case(suppressed))
    });
}

/// Apply a same-request `env_vars` replacement and then enforce the pin→inherit
/// effort-alias strip, in that exact order.
///
/// This is the ordering invariant Thufir's plan-of-record pins: the effort
/// column is cleared eagerly inside [`apply_agent_command_update`], but a stale
/// effort env alias in a caller-supplied `env_vars` map submitted in the SAME
/// request would otherwise survive the transition. Applying `env_vars` first,
/// then stripping the aliases only on the transition, guarantees the instance
/// cannot re-pin effort through the generic env channel while inheriting its
/// harness. `env_vars = None` leaves the record's existing env untouched;
/// validation of the supplied map is the caller's responsibility (it runs
/// before this seam at the update boundary).
pub fn apply_env_vars_then_effort_transition(
    record: &mut crate::managed_agents::types::ManagedAgentRecord,
    env_vars: Option<std::collections::BTreeMap<String, String>>,
    inherit_transition: bool,
) {
    if let Some(env_vars) = env_vars {
        record.env_vars = env_vars;
    }
    if inherit_transition {
        remove_record_effort_aliases(&mut record.env_vars);
    }
}

/// Decide the `agent_command_override` to persist at AGENT CREATE time.
///
/// A persona-backed create receives its harness command from
/// `resolvePersonaRuntime` (frontend), which produces a divergent command in two
/// distinct cases that the backend MUST tell apart:
///
/// - DELIBERATE OVERRIDE (`harness_override` true): the user explicitly picked a
///   runtime command in UI that exposes a runtime selector. This is a real pin
///   and is preserved when it differs from the command inheritance would spawn,
///   including installed aliases such as `claude-code-acp`.
/// - MISSING-RUNTIME FALLBACK (`harness_override` false): the persona's runtime
///   isn't installed locally, so `resolvePersonaRuntime` substitutes a fallback
///   default. This is NOT a pin — baking it would freeze the agent on the fallback
///   harness even after the persona's runtime is installed and the persona is
///   re-edited, the exact bug this resolver chain exists to prevent. Stores `None`
///   so the persona stays authoritative.
///
/// `isOverridden` from `resolvePersonaRuntime` cannot distinguish these — it is
/// `true` for BOTH — so the caller must thread the explicit user-intent bit.
///
/// Persona-less creates (`persona_id` is `None`, e.g. the standalone
/// CreateAgentDialog) have no persona to inherit, so the picked command is always a
/// real pin and is preserved via `divergent_agent_command_override` regardless of
/// `harness_override`.
pub fn create_time_agent_command_override(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::AgentDefinition],
    picked_command: Option<&str>,
    harness_override: bool,
) -> Option<String> {
    if persona_id.is_some() && !harness_override {
        return None;
    }

    if persona_id.is_some() && harness_override {
        let picked = picked_command
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let inherited_command = effective_agent_command(persona_id, personas, None);
        return (picked != inherited_command).then(|| picked.to_string());
    }

    divergent_agent_command_override(persona_id, personas, picked_command)
}
