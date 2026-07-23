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
/// Allowlist entries are replaced only when the definition mode is allowlist
/// (mirrors `update_managed_agent` preserve-across-toggle semantics).
pub(super) fn propagate_persona_respond_to(
    records: &mut [ManagedAgentRecord],
    persona_id: &str,
    definition: &AgentDefinition,
) -> Result<usize, String> {
    let mode = match definition.respond_to.as_deref() {
        Some(wire) => RespondTo::parse_wire(wire)?,
        None => RespondTo::default(),
    };
    let allowlist = if mode == RespondTo::Allowlist {
        validate_respond_to_allowlist(&definition.respond_to_allowlist)?
    } else {
        Vec::new()
    };

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
