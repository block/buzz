//! B5 (unified agent model): one-time backfill of standalone agents into
//! definition-linked records. Every keyed record with `persona_id: None`
//! gets a key-less definition manufactured from its own settings, retiring
//! the standalone-create pattern (Wes's Option 2: backfill, not grandfather).

use std::path::Path;

use crate::managed_agents::{
    persona_events::{persona_content_hash, persona_event_content},
    ManagedAgentRecord,
};

/// Manufacture definitions for standalone agents (B5 backfill).
///
/// For each keyed record with `persona_id: None`, append a key-less
/// definition record snapshotting the agent's own config and link the agent
/// to it. Safety rails (pinned in the B5 review gates):
/// - **Idempotent**: linked records are skipped, so a second run is a no-op.
/// - **`.bak` create-if-absent**: the pre-migration backup is taken once and
///   never clobbered — a partial first run must not replace the pristine
///   backup with a half-migrated snapshot on re-run.
/// - **Fail loudly per record**: a record that cannot be backfilled (slug
///   collision) is logged and skipped; the rest proceed.
/// - **No behavior change**: the definition snapshots the record's own
///   values (prompt present-even-if-empty via `to_definition_view`'s
///   `unwrap_or_default`, env COPIED so later instances inherit a working
///   config, quad copied to the definition defaults) and the record gains
///   `persona_source_version` = the new definition's content hash, so
///   neither `spawn_config_hash` nor the drift badge moves.
///
/// The manufactured definition's slug is the agent's pubkey: 64-hex passes
/// the NIP-AP slug grammar on both relay and desktop ends, and agent pubkeys
/// are unique, so the coordinate is collision-free by construction.
pub fn backfill_standalone_agents(app: &tauri::AppHandle) {
    let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return;
    };
    match backfill_standalone_agents_in_dir(&base_dir) {
        Ok(0) => {}
        Ok(backfilled) => {
            eprintln!(
                "buzz-desktop: standalone-backfill: {backfilled} agents linked to manufactured definitions"
            );
        }
        Err(e) => eprintln!("buzz-desktop: standalone-backfill: {e}"),
    }
}

/// Core backfill logic, decoupled from the Tauri `AppHandle` for testing.
/// Returns the number of records backfilled (0 = nothing to do).
fn backfill_standalone_agents_in_dir(base_dir: &Path) -> Result<usize, String> {
    let agents_path = base_dir.join("managed-agents.json");
    if !agents_path.exists() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(&agents_path)
        .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;
    let mut all: Vec<ManagedAgentRecord> = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse managed-agents.json: {e}"))?;

    let needs_backfill =
        |record: &ManagedAgentRecord| !record.pubkey.is_empty() && record.persona_id.is_none();
    if !all.iter().any(needs_backfill) {
        return Ok(0);
    }

    // Pre-migration backup, taken ONCE: a re-run after a partial failure must
    // not overwrite the pristine backup with a half-migrated snapshot.
    let bak_path = base_dir.join("managed-agents.json.pre-backfill.bak");
    if !bak_path.exists() {
        std::fs::write(&bak_path, &content)
            .map_err(|e| format!("failed to write pre-backfill backup: {e}"))?;
    }

    let existing_slugs: std::collections::HashSet<String> =
        all.iter().filter_map(|r| r.slug.clone()).collect();

    let mut manufactured: Vec<ManagedAgentRecord> = Vec::new();
    let mut backfilled = 0usize;
    for record in all.iter_mut().filter(|r| needs_backfill(r)) {
        // Pubkeys are unique so this cannot fire against another manufactured
        // definition — only against a pre-existing definition improbably
        // slugged as this agent's pubkey. Fail loudly, skip, continue: the
        // agent keeps working persona-less (`persona_id: None`), and the
        // backfill retries it on every boot. Recovery path: delete or re-slug
        // the colliding definition, then relaunch.
        if existing_slugs.contains(&record.pubkey) {
            eprintln!(
                "buzz-desktop: standalone-backfill: slug collision for agent {} — skipped; \
                 delete or re-slug the colliding definition to let the next launch backfill it",
                record.pubkey
            );
            continue;
        }

        // Snapshot the record's own config as a definition. Via the same
        // fold path every definition takes: a temporary persona view of the
        // record (prompt unwrap_or_default = present-even-if-empty — the
        // heal source old devices hard-require) folded into a key-less
        // definition record. Quad + env come along so future instances
        // minted from this definition inherit a working config.
        let mut view_source = record.clone();
        view_source.slug = Some(record.pubkey.clone());
        // Standalone agents have no definition-level quad — the INSTANCE
        // fields are the author's intent; copy them up.
        view_source.definition_respond_to = Some(record.respond_to.as_str().to_string());
        view_source.definition_respond_to_allowlist = record.respond_to_allowlist.clone();
        view_source.definition_parallelism = Some(record.parallelism);
        let Some(persona_view) = view_source.to_definition_view() else {
            eprintln!(
                "buzz-desktop: standalone-backfill: agent {} produced no persona view — skipped",
                record.pubkey
            );
            continue;
        };

        // Link the record BEFORE computing the version so the hash covers the
        // definition exactly as manufactured.
        let source_version = persona_content_hash(&persona_event_content(&persona_view));
        let definition = persona_view.into_agent_record();
        record.persona_id = Some(record.pubkey.clone());
        record.persona_source_version = Some(source_version);
        manufactured.push(definition);
        backfilled += 1;
    }

    if backfilled == 0 {
        return Ok(0);
    }
    all.extend(manufactured);
    let payload = serde_json::to_vec_pretty(&all)
        .map_err(|e| format!("failed to serialize unified store: {e}"))?;
    crate::managed_agents::atomic_write_json_restricted(&agents_path, &payload)?;
    Ok(backfilled)
}

#[cfg(test)]
#[path = "backfill_tests.rs"]
mod tests;
