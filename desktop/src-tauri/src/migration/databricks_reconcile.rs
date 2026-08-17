//! Databricks V1→V2 provider reconciliation for managed-agent records.
//!
//! On Block builds the persisted `provider: "databricks"` is migrated to
//! `"databricks_v2"`; on all builds stale derived provider/model keys are
//! stripped from `env_vars`. Split from `migration.rs` into its own module to
//! stay within the desktop file-size ratchet (same pattern as `backfill.rs` and
//! `migration_secrets.rs`).

use std::path::Path;

use tauri::Manager;

fn reconcile_databricks_v1_to_v2_in_file(path: &Path, rewrite_v1_provider: bool) {
    use crate::managed_agents::is_derived_provider_model_key;
    super::patch_json_records(path, |obj| {
        let mut changed = false;

        // Only rewrite the structured provider field when the baked build env
        // marks this as a Block build (BUZZ_AGENT_PROVIDER == "databricks_v2").
        // OSS users may intentionally select V1 (Model Serving), so we must not
        // silently migrate their provider to V2 (AI Gateway).
        if rewrite_v1_provider && obj.get("provider").and_then(|v| v.as_str()) == Some("databricks")
        {
            let name = obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            eprintln!(
                "buzz-desktop: databricks-v1-to-v2: {name:?}: provider \"databricks\" → \"databricks_v2\"",
            );
            obj.insert(
                "provider".to_string(),
                serde_json::Value::String("databricks_v2".to_string()),
            );
            // Also clear the model field — a V1 model name (e.g. "dbrx-instruct")
            // on a V2 provider would shadow the baked DATABRICKS_MODEL at spawn time
            // (BUZZ_AGENT_MODEL from runtime_metadata_env_vars takes priority in
            // buzz-agent config.rs). Clearing it lets the baked V2 default win.
            if obj.remove("model").is_some() {
                eprintln!(
                    "buzz-desktop: databricks-v1-to-v2: {name:?}: cleared stale V1 model field",
                );
            }
            changed = true;
        }

        // Strip derived provider/model keys from env_vars on ALL records,
        // regardless of rewrite_v1_provider. These keys are re-derived from
        // structured fields at spawn time; stale copies in env_vars silently
        // override the structured fields (last-write-wins in Command::env) and
        // can cause V1 routing even when the provider dropdown shows V2.
        //
        // The check is case-insensitive (matching the established helper)
        // to cover any case-variant that may have been written historically.
        if let Some(serde_json::Value::Object(env_vars)) = obj.get_mut("env_vars") {
            let stale_keys: Vec<String> = env_vars
                .keys()
                .filter(|k| is_derived_provider_model_key(k))
                .cloned()
                .collect();
            for key in stale_keys {
                env_vars.remove(key.as_str());
                eprintln!("buzz-desktop: databricks-v1-to-v2: removed stale env_vars[\"{key}\"]",);
                changed = true;
            }
        }

        changed
    });
}

/// Strip stale derived provider/model keys from `env_vars` in all
/// managed-agent records, and — on Block builds — also migrate any persisted
/// `provider: "databricks"` to `"databricks_v2"`.
///
/// **Block builds** (where `baked_build_env()` contains
/// `BUZZ_AGENT_PROVIDER=databricks_v2`): the structured `provider` field is
/// rewritten V1→V2 because the baked release targets V2 exclusively. Records
/// that were saved before this migration would otherwise silently override the
/// baked value at spawn time (last-write-wins in `Command::env`).
///
/// **OSS builds** (baked env empty): the `provider` field is left alone —
/// V1 (`databricks`) is a valid Model Serving choice for OSS users.
///
/// In both cases, stale `BUZZ_AGENT_PROVIDER` / `BUZZ_AGENT_MODEL` /
/// `GOOSE_PROVIDER` / `GOOSE_MODEL` are stripped from `env_vars`. These keys
/// are always re-derived from structured fields at spawn time; persisted copies
/// silence UI edits and cause stale routing.
///
/// Covers both the current app data dir and the canonical dev data dir
/// (for worktree instances) — same dual-dir pattern as
/// `reconcile_legacy_command_names` and `reconcile_provider_mcp_commands`.
pub(super) fn reconcile_databricks_v1_to_v2(app: &tauri::AppHandle) {
    use crate::managed_agents::baked_build_env;
    // On Block builds, the baked env contains BUZZ_AGENT_PROVIDER=databricks_v2.
    // Use that as a reliable signal that this is a Block build and the V1
    // provider should be migrated. OSS builds have an empty baked env, so
    // rewrite_v1_provider is false and the structured provider is preserved.
    let rewrite_v1_provider = baked_build_env()
        .get("BUZZ_AGENT_PROVIDER")
        .map(|v| v == "databricks_v2")
        .unwrap_or(false);
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let mut dirs = vec![current_dir.clone()];
    if let Some(canonical) = super::canonical_dev_data_dir(&current_dir) {
        if canonical.exists() && canonical != current_dir {
            dirs.push(canonical);
        }
    }
    for dir in dirs {
        let path = dir.join("agents/managed-agents.json");
        if path.exists() {
            reconcile_databricks_v1_to_v2_in_file(&path, rewrite_v1_provider);
        }
    }
}

#[cfg(test)]
#[path = "databricks_reconcile_tests.rs"]
mod tests;
