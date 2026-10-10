//! Shared persona → instance field propagation helpers.

use crate::managed_agents::{
    validate_respond_to_allowlist, AgentDefinition, ManagedAgentRecord, RespondTo,
};

/// Propagate a persona definition's display_name rename to linked agent instances.
/// Only instances whose current `name` equals `old_display_name` are updated;
/// pool-named instances (e.g. "Birch", "Compass") keep their individualised name.
/// Updates both `record.name` (relay display name) and `record.display_name`.
/// Returns the pubkeys of the records that were renamed.
pub(super) fn propagate_persona_name_rename(
    records: &mut [ManagedAgentRecord],
    persona_id: &str,
    old_display_name: &str,
    new_display_name: &str,
) -> Vec<String> {
    let mut renamed = Vec::new();
    for record in records.iter_mut() {
        if record.persona_id.as_deref() != Some(persona_id) {
            continue;
        }
        if record.name != old_display_name {
            continue; // pool-named instance — keep its individualised name
        }
        record.name = new_display_name.to_string();
        record.display_name = Some(new_display_name.to_string());
        renamed.push(record.pubkey.clone());
    }
    renamed
}

/// Propagate a definition's respond-to gate onto linked running instances.
///
/// Definition edits write `definition_respond_to` only; the harness reads each
/// instance's `respond_to`. Without this, "Who can talk to this agent" on a
/// definition never reaches buzz-acp.
///
/// `respond_to: None` on the definition means unset — leave instance gates
/// alone. Coercing to owner-only would silently downgrade instances that were
/// set independently (or repaired by hand) whenever an unrelated persona edit
/// re-saved the definition.
///
/// Allowlist entries are replaced only when the definition mode is allowlist
/// (mirrors `update_managed_agent` preserve-across-toggle semantics).
pub(super) fn propagate_persona_respond_to(
    records: &mut [ManagedAgentRecord],
    persona_id: &str,
    definition: &AgentDefinition,
) -> Result<usize, String> {
    let Some(wire) = definition.respond_to.as_deref() else {
        return Ok(0);
    };
    let mode = RespondTo::parse_wire(wire)?;
    let allowlist = if mode == RespondTo::Allowlist {
        validate_respond_to_allowlist(&definition.respond_to_allowlist)?
    } else {
        Vec::new()
    };

    // A stored definition can end up in `allowlist` mode with zero pubkeys —
    // `validate_respond_to_allowlist` accepts an empty list rather than
    // erroring, and the write path that should stop that combination from
    // reaching storage is tracked separately (#2501). Every other consumer of
    // this state rejects it outright: `build_respond_to_env`'s mint guard and
    // `apply_persona_behavior`'s request validation both refuse an empty
    // allowlist. Skip instead of propagating it, so an unrelated persona save
    // can't push a doomed respond_to onto instances that were working.
    if mode == RespondTo::Allowlist && allowlist.is_empty() {
        return Ok(0);
    }

    let mut updated = 0;
    for record in records.iter_mut() {
        if record.persona_id.as_deref() != Some(persona_id) {
            continue;
        }
        if record.pubkey.is_empty() {
            continue;
        }

        let mut changed = record.respond_to != mode;
        record.respond_to = mode;
        if mode == RespondTo::Allowlist && record.respond_to_allowlist != allowlist {
            record.respond_to_allowlist = allowlist.clone();
            changed = true;
        }
        if changed {
            updated += 1;
        }
    }
    Ok(updated)
}
