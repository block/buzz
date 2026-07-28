//! Shared persona → instance field propagation helpers.
//!
//! A persona definition and the managed-agent instances linked to it are
//! separate records. Some definition edits must reach the running instances
//! (the harness only ever reads the instance record), and each helper here
//! covers one such field.

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

/// Propagate a definition's respond-to gate onto its linked running instances.
///
/// The "Who can talk to this agent" control edits the *definition*, writing
/// `definition_respond_to`. But the definition's gate is copied onto an
/// instance's `respond_to` at mint time only, and the harness (`buzz-acp`)
/// spawns from that instance field. Without this propagation a post-mint edit
/// never reaches the harness, so the agent keeps booting `owner-only` and no
/// one but the owner can talk to it (#2501).
///
/// The definition gate is authoritative for every linked instance: on a
/// definition edit all instances converge to it. The allowlist is replaced
/// only when the definition mode is `allowlist` — for other modes the
/// instance's stored allowlist is left untouched so a later toggle back to
/// allowlist doesn't lose the entries (mirrors the preserve-across-toggle
/// semantics documented on `ManagedAgentRecord::respond_to_allowlist`).
///
/// Key-less definition rows (empty `pubkey`) are skipped — they are not
/// running instances. Returns the number of instance records actually changed.
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
            continue; // key-less definition row, not a running instance
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
