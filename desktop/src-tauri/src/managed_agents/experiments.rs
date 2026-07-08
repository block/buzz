//! Desktop mirror of the frontend preview-experiment overrides.
//!
//! The feature-flag source of truth lives in the webview's localStorage
//! (`desktop/src/shared/features/store.ts`), which the Rust spawn path cannot
//! read. The frontend mirrors the overrides map to `experiments.json` in the
//! app data dir via the `set_desktop_experiments` command; spawn-time code
//! reads it back with [`experiment_enabled`].
//!
//! Semantics match `resolveEnabled` on the frontend: an experiment is enabled
//! ONLY on an explicit `true`. Missing file, malformed JSON, absent key, or
//! `false` all resolve to disabled — the app-startup restore path respawns
//! agents before the webview loads, and unknown state must stay on the safe
//! (off) side.

use std::collections::BTreeMap;
use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use super::storage::atomic_write_json_restricted;

/// Preview experiment id for agent-provided friendly tool summaries.
/// Must match the `id` in `preview-features.json`.
pub const ACP_TOOL_SUMMARIES_EXPERIMENT: &str = "acpToolSummaries";

fn experiments_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create app data dir: {error}"))?;
    Ok(dir.join("experiments.json"))
}

/// Persist the full experiment-overrides map (replaces the previous file).
pub fn save_experiments(
    app: &AppHandle,
    experiments: &BTreeMap<String, bool>,
) -> Result<(), String> {
    let path = experiments_store_path(app)?;
    let payload = serde_json::to_vec_pretty(experiments)
        .map_err(|error| format!("failed to serialize experiments: {error}"))?;
    atomic_write_json_restricted(&path, &payload)
}

/// Whether `experiment_id` is explicitly enabled in the mirrored overrides.
/// Any failure to read or parse resolves to `false` (experiment off).
pub fn experiment_enabled(app: &AppHandle, experiment_id: &str) -> bool {
    let raw = experiments_store_path(app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok());
    resolve_experiment_enabled(raw.as_deref(), experiment_id)
}

/// Pure resolution: enabled ONLY on an explicit `true` in well-formed JSON.
/// `None` (missing/unreadable file), malformed JSON, absent key, and `false`
/// all resolve to disabled.
pub(crate) fn resolve_experiment_enabled(raw: Option<&str>, experiment_id: &str) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    parse_experiments(raw)
        .get(experiment_id)
        .copied()
        .unwrap_or(false)
}

/// Parse the mirrored overrides map; malformed input yields an empty map
/// (everything off).
pub(crate) fn parse_experiments(raw: &str) -> BTreeMap<String, bool> {
    serde_json::from_str(raw).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{parse_experiments, resolve_experiment_enabled, ACP_TOOL_SUMMARIES_EXPERIMENT};

    #[test]
    fn parse_reads_explicit_true() {
        let map = parse_experiments(r#"{"acpToolSummaries":true}"#);
        assert_eq!(map.get("acpToolSummaries"), Some(&true));
    }

    #[test]
    fn parse_reads_explicit_false() {
        let map = parse_experiments(r#"{"acpToolSummaries":false}"#);
        assert_eq!(map.get("acpToolSummaries"), Some(&false));
    }

    #[test]
    fn parse_malformed_json_yields_empty_map() {
        assert!(parse_experiments("not json").is_empty());
        assert!(parse_experiments(r#"{"acpToolSummaries":"yes"}"#).is_empty());
        assert!(parse_experiments("").is_empty());
    }

    #[test]
    fn resolve_enabled_only_on_explicit_true() {
        let id = ACP_TOOL_SUMMARIES_EXPERIMENT;
        assert!(resolve_experiment_enabled(
            Some(r#"{"acpToolSummaries":true}"#),
            id
        ));
        assert!(!resolve_experiment_enabled(
            Some(r#"{"acpToolSummaries":false}"#),
            id
        ));
        // Absent key, missing file, and malformed JSON all resolve OFF.
        assert!(!resolve_experiment_enabled(Some(r#"{"other":true}"#), id));
        assert!(!resolve_experiment_enabled(None, id));
        assert!(!resolve_experiment_enabled(Some("not json"), id));
        assert!(!resolve_experiment_enabled(
            Some(r#"{"acpToolSummaries":"yes"}"#),
            id
        ));
    }
}
