//! Phase 1A (unified agent model): boot-time materialization of each
//! persona-linked agent record's `runtime` onto the record itself.
//!
//! Child module of `migration` so it reuses the parent's private JSON-patch
//! helpers (`patch_json_records`, `load_persona_runtimes`,
//! `canonical_dev_data_dir`).

use std::path::Path;

use tauri::Manager as _;

use super::{canonical_dev_data_dir, load_persona_runtimes, patch_json_records};

/// Materialize each persona-linked agent record's `runtime` from its linked
/// persona (unified agent model, Phase 1A). After this, spawn resolution reads
/// the record's own runtime (`record_agent_command` step 2) instead of the
/// live persona — same effective command by construction, so the spawn-config
/// hash is unchanged and no running agent shows a spurious restart badge (see
/// `spawn_hash::tests::materializing_runtime_keeps_hash_stable`).
///
/// Idempotent: records that already carry `runtime` are untouched, as are
/// records with no linked persona or a persona without a runtime (both keep
/// resolving through the legacy fallback path unchanged).
pub fn materialize_agent_runtimes(app: &tauri::AppHandle) {
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let mut dirs = vec![current_dir.clone()];
    if let Some(canonical) = canonical_dev_data_dir(&current_dir) {
        if canonical.exists() && canonical != current_dir {
            dirs.push(canonical);
        }
    }
    for dir in dirs {
        let path = dir.join("agents/managed-agents.json");
        if path.exists() {
            materialize_runtimes_in_file(&path);
        }
    }
}

fn materialize_runtimes_in_file(path: &Path) {
    let persona_runtimes = load_persona_runtimes(path);
    if persona_runtimes.is_empty() {
        return;
    }
    patch_json_records(path, |obj| {
        if obj.contains_key("runtime") {
            return false;
        }
        let Some(runtime) = obj
            .get("persona_id")
            .and_then(|v| v.as_str())
            .and_then(|pid| persona_runtimes.get(pid))
        else {
            return false;
        };
        obj.insert(
            "runtime".to_string(),
            serde_json::Value::String(runtime.clone()),
        );
        true
    });
}

#[cfg(test)]
mod tests {
    use super::materialize_runtimes_in_file;
    use crate::migration::test_support::{
        read_agents_json, write_agents_json, write_personas_json,
    };

    #[test]
    fn materialize_runtimes_copies_persona_runtime_onto_record() {
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([{ "id": "persona-1", "displayName": "Alice", "runtime": "goose" }]),
        );
        write_agents_json(
            dir.path(),
            &serde_json::json!([{ "name": "Fizz", "persona_id": "persona-1" }]),
        );
        materialize_runtimes_in_file(&dir.path().join("agents/managed-agents.json"));
        let records = read_agents_json(dir.path());
        assert_eq!(records[0]["runtime"], "goose");
    }

    #[test]
    fn materialize_runtimes_is_idempotent_and_preserves_existing() {
        // A record that already carries runtime (even one diverging from its
        // persona) is never overwritten, and the second run rewrites nothing.
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([{ "id": "persona-1", "displayName": "Alice", "runtime": "goose" }]),
        );
        write_agents_json(
            dir.path(),
            &serde_json::json!([
                { "name": "Fizz", "persona_id": "persona-1", "runtime": "claude" },
                { "name": "Buzz", "persona_id": "persona-1" }
            ]),
        );
        let agents_path = dir.path().join("agents/managed-agents.json");
        materialize_runtimes_in_file(&agents_path);
        let records = read_agents_json(dir.path());
        assert_eq!(
            records[0]["runtime"], "claude",
            "existing runtime preserved"
        );
        assert_eq!(records[1]["runtime"], "goose");

        let before = std::fs::read_to_string(&agents_path).unwrap();
        materialize_runtimes_in_file(&agents_path);
        let after = std::fs::read_to_string(&agents_path).unwrap();
        assert_eq!(before, after, "second run must be a no-op");
    }

    #[test]
    fn materialize_runtimes_skips_unlinked_and_unknown_personas() {
        let dir = tempfile::tempdir().unwrap();
        write_personas_json(
            dir.path(),
            &serde_json::json!([{ "id": "persona-1", "displayName": "Alice", "runtime": "goose" }]),
        );
        write_agents_json(
            dir.path(),
            &serde_json::json!([
                { "name": "NoPersona" },
                { "name": "GonePersona", "persona_id": "deleted" }
            ]),
        );
        let agents_path = dir.path().join("agents/managed-agents.json");
        let before = std::fs::read_to_string(&agents_path).unwrap();
        materialize_runtimes_in_file(&agents_path);
        let after = std::fs::read_to_string(&agents_path).unwrap();
        assert_eq!(before, after, "no linked runtime → untouched file");
    }
}
