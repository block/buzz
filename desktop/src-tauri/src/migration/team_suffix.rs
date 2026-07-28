//! Strip the legacy baked team-instructions suffix from stored prompts.
//!
//! Records written before the runtime team framing landed have their team
//! instructions BAKED into `system_prompt` by the now-removed
//! `compose_prompt()` in buzz-persona:
//!
//! ```text
//! {persona_prompt}\n\n---\n# Team Instructions\n{instructions}
//! ```
//!
//! `with_team()` in `buzz-acp/src/pool.rs` now appends the LIVE
//! `[Team Instructions]` section on top of that stored value, so an affected
//! agent receives two team-instruction blocks per turn — the frozen copy first,
//! the live one second — and the observer feed renders two Team Instructions
//! cards. The frozen copy is not merely redundant: it carries whatever the team
//! roster said the day it was written, so the agent is fed a stale roster ahead
//! of the current one.
//!
//! The fix has to happen at rest. Suppressing the duplicate in the observer
//! parser would hide the symptom while the agent kept receiving the stale bytes.
//!
//! Stripping a definition's prompt also changes its `persona_content_hash`, the
//! drift basis behind the Agents-menu "out of date" badge, so the migration
//! advances the pin of instances that were current before the strip (see
//! [`repin_current_instances`]).

use std::collections::HashMap;
use std::path::Path;

use crate::managed_agents::{
    persona_events::{persona_content_hash, persona_event_content},
    ManagedAgentRecord,
};

/// The exact producer boundary emitted by the removed `compose_prompt()`.
/// Matched byte-for-byte: a bare `---`, a `# Team Instructions` heading at a
/// different position, or a single preceding newline are author content, not a
/// producer boundary, and are left alone.
const TEAM_DELIMITER: &str = "\n\n---\n# Team Instructions\n";

/// Strip the baked team-instructions suffix from every stored `system_prompt`.
///
/// Ordering (see `run_boot_migrations`): runs AFTER
/// `fold_personas_into_agent_store` so definitions folded out of the legacy
/// `personas.json` are cleaned in the same boot, and BEFORE
/// `backfill_standalone_agents` so a manufactured definition never snapshots
/// a suffix this migration is about to remove.
pub fn strip_baked_team_instructions(app: &tauri::AppHandle) {
    let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return;
    };
    match strip_baked_team_instructions_in_dir(&base_dir) {
        Ok(0) => {}
        Ok(stripped) => eprintln!(
            "buzz-desktop: team-suffix-strip: removed the baked team-instructions suffix from \
             {stripped} record(s)"
        ),
        Err(e) => eprintln!("buzz-desktop: team-suffix-strip: {e}"),
    }
}

/// Core logic, decoupled from the Tauri `AppHandle` for testing.
///
/// `base_dir` is the managed-agents base directory (`<AppDataDir>/agents/`).
/// Returns the number of records changed; `Ok(0)` means nothing to do and
/// nothing was written, so a second boot is a clean no-op.
pub(super) fn strip_baked_team_instructions_in_dir(base_dir: &Path) -> Result<usize, String> {
    let agents_path = base_dir.join("managed-agents.json");
    if !agents_path.exists() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(&agents_path)
        .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;
    let mut all: Vec<ManagedAgentRecord> = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse managed-agents.json: {e}"))?;

    // Definition hashes BEFORE the strip: stripping a definition's
    // `system_prompt` changes its `persona_content_hash`, which is the drift
    // basis the Agents menu compares each linked instance's pinned
    // `persona_source_version` against. Captured here so instances that were
    // current before the strip can be re-pinned after it.
    let pre_strip_hashes = definition_hashes(&all);

    // Applies to every record: the definition records carry the suffix as
    // surely as the instances minted from them, and neither `team_id` nor
    // `persona_id` nor a key is evidence either way.
    let mut stripped = 0usize;
    for record in all.iter_mut() {
        let Some(prompt) = record.system_prompt.as_deref() else {
            continue;
        };
        // Last occurrence only, mirroring the observer parser's own
        // `lastIndexOf` guard (`agentSessionTranscriptHelpers.ts`): a persona
        // body may legitimately quote a delimiter-shaped passage, and only the
        // final one is the boundary `compose_prompt()` appended.
        let Some(at) = prompt.rfind(TEAM_DELIMITER) else {
            continue;
        };
        let head = &prompt[..at];
        // An empty head means the record held nothing but the baked suffix.
        // Store `None` rather than `Some("")`, matching the empty-prompt
        // convention in `AgentDefinition::into_agent_record`, so no phantom
        // empty prompt is left behind.
        record.system_prompt = (!head.is_empty()).then(|| head.to_string());
        stripped += 1;
    }

    if stripped == 0 {
        return Ok(0);
    }

    repin_current_instances(&mut all, &pre_strip_hashes);

    // Pre-migration backup, taken ONCE (same contract as the B5 backfill): a
    // re-run after a partial failure must not replace the pristine backup with
    // a half-migrated snapshot. Owner-only from the initial open, and sited next
    // to the resolved store — see `create_restricted_backup_once` and
    // `resolved_backup_path`.
    let bak_path = crate::util::resolved_backup_path(
        &agents_path,
        "managed-agents.json.pre-team-suffix-strip.bak",
    );
    crate::util::create_restricted_backup_once(&bak_path, content.as_bytes())
        .map_err(|e| format!("failed to write pre-strip backup: {e}"))?;

    let payload = serde_json::to_vec_pretty(&all)
        .map_err(|e| format!("failed to serialize managed-agents.json: {e}"))?;
    crate::managed_agents::atomic_write_json_restricted(&agents_path, &payload)?;
    Ok(stripped)
}

/// `slug → persona_content_hash` for every definition record, computed through
/// the same projection the drift indicator uses (`persona_drift_state` in
/// `managed_agents/runtime.rs`).
fn definition_hashes(records: &[ManagedAgentRecord]) -> HashMap<String, String> {
    records
        .iter()
        .filter(|record| record.pubkey.is_empty())
        .filter_map(|record| {
            let definition = record.to_definition_view()?;
            let hash = persona_content_hash(&persona_event_content(&definition));
            Some((definition.id, hash))
        })
        .collect()
}

/// Advance the drift pin of instances that were current before the strip.
///
/// Stripping a definition's prompt changes its `persona_content_hash`, so every
/// linked instance would otherwise light up "out of date" in the Agents menu
/// for a change the user never made — a badge that only clears on the next
/// start, when `apply_persona_snapshot` re-pins. Same conditional as
/// `refresh_builtin_agent_avatars`: move the pin only when it still equals the
/// definition's PRE-strip hash. An instance that had genuinely drifted keeps
/// its stale pin, and its badge.
fn repin_current_instances(
    records: &mut [ManagedAgentRecord],
    pre_strip_hashes: &HashMap<String, String>,
) {
    let post_strip_hashes = definition_hashes(records);
    for record in records.iter_mut() {
        if record.pubkey.is_empty() {
            continue;
        }
        let Some(persona_id) = record.persona_id.as_deref() else {
            continue;
        };
        let (Some(old), Some(new)) = (
            pre_strip_hashes.get(persona_id),
            post_strip_hashes.get(persona_id),
        ) else {
            continue;
        };
        if old != new && record.persona_source_version.as_deref() == Some(old.as_str()) {
            record.persona_source_version = Some(new.clone());
        }
    }
}

#[cfg(test)]
#[path = "team_suffix_tests.rs"]
mod tests;
