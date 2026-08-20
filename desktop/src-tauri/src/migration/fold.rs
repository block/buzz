//! Phase 1A.2 (unified agent model): one-way fold of `personas.json` into
//! the unified agent store as key-less definition records.

use std::path::Path;

/// Fold `personas.json` into the unified agent store (Phase 1A.2).
///
/// One-way, versioned by presence: runs only while `personas.json` exists.
/// Each persona becomes a key-less definition record
/// ([`AgentDefinition::into_agent_record`]) appended to `managed-agents.json`
/// via the definition-preserving save; the old file is renamed to
/// `personas.json.bak` so a second boot is a no-op and the data survives for
/// manual recovery. Built-ins are skipped — `merge_personas` regenerates them
/// from code on every load, exactly as before.
///
/// Ordering (see `run_boot_migrations`): runs after the JSON-level
/// `personas.json` migrations (which must see the legacy file) and BEFORE
/// every consumer of the `load/save_personas` shims — `sync_team_personas`,
/// `reconcile_provider_mcp_commands`, and `materialize_agent_runtimes` all
/// read definitions post-fold via [`load_persona_runtimes`]'s unified-store
/// branch.
pub fn fold_personas_into_agent_store(app: &tauri::AppHandle) {
    let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return;
    };
    match fold_personas_in_dir(&base_dir) {
        Ok(None) => {}
        Ok(Some(folded)) => {
            eprintln!(
                "buzz-desktop: persona-store-fold: {folded} definitions folded into the unified store"
            );
        }
        Err(e) => eprintln!("buzz-desktop: persona-store-fold: {e}"),
    }
}

/// Core fold logic, decoupled from the Tauri `AppHandle` for testing.
/// Operates on the raw JSON files — no keyring interaction: instance records
/// are passed through byte-identical, and folded definitions carry no keys.
/// Returns `Ok(None)` when there is no `personas.json` to fold.
fn fold_personas_in_dir(base_dir: &Path) -> Result<Option<usize>, String> {
    let personas_path = base_dir.join("personas.json");
    if !personas_path.exists() {
        return Ok(None);
    }

    let personas = crate::managed_agents::load_personas_from_path(&personas_path)?;

    let agents_path = base_dir.join("managed-agents.json");
    let mut all: Vec<crate::managed_agents::ManagedAgentRecord> = if agents_path.exists() {
        let content = std::fs::read_to_string(&agents_path)
            .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("failed to parse managed-agents.json: {e}"))?
    } else {
        Vec::new()
    };

    let existing: std::collections::HashSet<String> = all
        .iter()
        .filter_map(|record| record.slug.clone())
        .collect();

    let mut folded = 0usize;
    for persona in personas {
        // Built-ins regenerate from code; a slug already in the store means
        // a previous partial fold got that far — never duplicate.
        if persona.is_builtin || existing.contains(&persona.id) {
            continue;
        }
        all.push(persona.into_agent_record());
        folded += 1;
    }

    let payload = serde_json::to_vec_pretty(&all)
        .map_err(|e| format!("failed to serialize unified store: {e}"))?;
    crate::managed_agents::atomic_write_json_restricted(&agents_path, &payload)?;

    // Rename only after the unified store write succeeded — a crash between
    // the two leaves personas.json in place and the fold re-runs idempotently
    // (slug dedup above).
    std::fs::rename(&personas_path, base_dir.join("personas.json.bak"))
        .map_err(|e| format!("failed to retire personas.json: {e}"))?;
    Ok(Some(folded))
}

/// Build a `persona_id → runtime` map from the personas.json sibling of the
/// given managed-agents.json path. Returns an empty map when personas can't be
/// read or parsed — callers then fall back to the record's own snapshot.
pub(super) fn load_persona_runtimes(
    agents_path: &Path,
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Some(dir) = agents_path.parent() else {
        return map;
    };
    // Pre-fold boots read the legacy personas.json; post-fold boots read the
    // key-less definitions sharing the agents store itself (slug + runtime).
    let personas_path = dir.join("personas.json");
    if personas_path.exists() {
        let Ok(content) = std::fs::read_to_string(&personas_path) else {
            return map;
        };
        let Ok(records) = serde_json::from_str::<Vec<serde_json::Value>>(&content) else {
            return map;
        };
        for record in records {
            if let (Some(id), Some(runtime)) = (
                record.get("id").and_then(|v| v.as_str()),
                record.get("runtime").and_then(|v| v.as_str()),
            ) {
                map.insert(id.to_string(), runtime.to_string());
            }
        }
        return map;
    }

    let Ok(content) = std::fs::read_to_string(agents_path) else {
        return map;
    };
    let Ok(records) = serde_json::from_str::<Vec<serde_json::Value>>(&content) else {
        return map;
    };
    for record in records {
        // Definition records: key-less (no/empty pubkey), slug-addressed.
        let keyed = record
            .get("pubkey")
            .and_then(|v| v.as_str())
            .is_some_and(|p| !p.is_empty());
        if keyed {
            continue;
        }
        if let (Some(slug), Some(runtime)) = (
            record.get("slug").and_then(|v| v.as_str()),
            record.get("runtime").and_then(|v| v.as_str()),
        ) {
            map.insert(slug.to_string(), runtime.to_string());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::fold_personas_in_dir;
    use crate::migration::load_persona_runtimes;
    use crate::migration::test_support::{
        read_agents_json, write_agents_json, write_personas_json,
    };

    // ── Persona-store fold (Phase 1A.2) ──────────────────────────────────────────

    fn keyed_agent_json(name: &str, pubkey: &str) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "pubkey": pubkey,
            "relay_url": "ws://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": null,
            "start_on_app_launch": false,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        })
    }

    fn custom_persona_json(id: &str, runtime: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "display_name": format!("Name {id}"),
            "avatar_url": null,
            "system_prompt": format!("Prompt {id}"),
            "runtime": runtime,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z"
        })
    }

    #[test]
    fn fold_moves_custom_personas_and_retires_the_file() {
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([
                custom_persona_json("custom:one", "goose"),
                { "id": "builtin:fizz", "display_name": "Fizz", "system_prompt": "P",
                  "is_builtin": true,
                  "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z" }
            ]),
        );
        write_agents_json(
            dir.path(),
            &serde_json::json!([keyed_agent_json("Keyed", &"k".repeat(64))]),
        );

        let base = dir.path().join("agents");
        let folded = fold_personas_in_dir(&base).unwrap();
        assert_eq!(folded, Some(1), "custom folds, builtin skipped");

        let records = read_agents_json(dir.path());
        assert_eq!(records.len(), 2, "definition + preserved instance");
        let def = records
            .iter()
            .find(|r| r.get("slug").is_some())
            .expect("folded definition present");
        assert_eq!(def["slug"], "custom:one");
        assert_eq!(def["runtime"], "goose");
        // Key-less: pubkey serializes as the empty string (field not skipped).
        assert_eq!(def["pubkey"], "", "definition must be key-less");
        let keyed = records.iter().find(|r| r["name"] == "Keyed").unwrap();
        assert_eq!(keyed["pubkey"].as_str().unwrap().len(), 64);

        assert!(!base.join("personas.json").exists(), "source retired");
        assert!(base.join("personas.json.bak").exists(), ".bak left behind");
    }

    #[test]
    fn fold_is_idempotent_across_partial_runs() {
        // Simulate a crash after the store write but before the rename: the
        // definition is already in the unified store AND personas.json is back.
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([custom_persona_json("custom:one", "goose")]),
        );
        let base = dir.path().join("agents");
        assert_eq!(fold_personas_in_dir(&base).unwrap(), Some(1));

        // Crash simulation: restore personas.json from the .bak.
        std::fs::copy(base.join("personas.json.bak"), base.join("personas.json")).unwrap();
        assert_eq!(
            fold_personas_in_dir(&base).unwrap(),
            Some(0),
            "second run folds nothing (slug dedup)"
        );
        let records = read_agents_json(dir.path());
        assert_eq!(records.len(), 1, "no duplicate definition");
    }

    #[test]
    fn fold_absent_personas_file_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        write_agents_json(
            dir.path(),
            &serde_json::json!([keyed_agent_json("Keyed", &"k".repeat(64))]),
        );
        let base = dir.path().join("agents");
        let before = std::fs::read_to_string(base.join("managed-agents.json")).unwrap();
        assert_eq!(fold_personas_in_dir(&base).unwrap(), None);
        let after = std::fs::read_to_string(base.join("managed-agents.json")).unwrap();
        assert_eq!(before, after, "store untouched when nothing to fold");
    }

    #[test]
    fn post_fold_runtime_map_reads_unified_definitions() {
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([custom_persona_json("custom:one", "goose")]),
        );
        let mut linked = keyed_agent_json("Keyed", &"k".repeat(64));
        linked["persona_id"] = serde_json::json!("custom:one");
        write_agents_json(dir.path(), &serde_json::json!([linked]));
        let base = dir.path().join("agents");
        let agents_path = base.join("managed-agents.json");

        // Pre-fold: map comes from personas.json.
        let pre = load_persona_runtimes(&agents_path);
        assert_eq!(pre.get("custom:one").map(String::as_str), Some("goose"));

        fold_personas_in_dir(&base).unwrap();

        // Post-fold: personas.json is gone; map must come from the unified store
        // and be identical.
        let post = load_persona_runtimes(&agents_path);
        assert_eq!(post.get("custom:one").map(String::as_str), Some("goose"));
    }

    #[test]
    fn fold_dedup_prefers_the_store_over_the_persona_file() {
        // Crash-between-re-run case (Pinky, review): the definition already in
        // the unified store WINS over a personas.json entry with the same slug —
        // the store copy is the one the successful fold wrote, and a user may
        // have edited it since; re-folding the stale file copy would clobber it.
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([custom_persona_json("custom:one", "goose")]),
        );
        let base = dir.path().join("agents");
        assert_eq!(fold_personas_in_dir(&base).unwrap(), Some(1));

        // Post-fold edit in the unified store.
        let mut records = read_agents_json(dir.path());
        records[0]["runtime"] = serde_json::json!("claude");
        write_agents_json(dir.path(), &serde_json::Value::Array(records));

        // Crash simulation: stale personas.json (runtime still "goose") returns.
        std::fs::copy(base.join("personas.json.bak"), base.join("personas.json")).unwrap();
        assert_eq!(fold_personas_in_dir(&base).unwrap(), Some(0));

        let records = read_agents_json(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0]["runtime"], "claude",
            "store copy wins over the stale file copy"
        );
    }
}
