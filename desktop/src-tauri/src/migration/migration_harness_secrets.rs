//! Boot-time secret extraction for custom harness definitions.
//!
//! Custom harnesses (`<app-data>/custom_harnesses/*.json`) carry an `env` map
//! that can hold provider secrets (e.g. `ANTHROPIC_API_KEY`). Before the
//! generation-reference projection this map was written plaintext to disk. This
//! migration lifts each inline `env` into the OS keyring under the
//! `harness:<id>:env:<gen>` coordinate, rewrites the file stripped at `0o600`,
//! and — once every projected generation reads back cleanly — scrubs the
//! plaintext-bearing backup/temp artifacts the save path can leave behind.
//!
//! Runs at boot alongside [`super::migration_secrets`], BEFORE
//! `warm_harness_registry_from_dir` hydrates the spawn registry.
//!
//! Idempotent across launches: extraction goes through the W1-safe field
//! migration seam ([`crate::managed_agents::custom_harnesses::migrate_harness_env`]),
//! which projects only a non-empty inline value and otherwise preserves an
//! existing ref. A second launch re-reads already-projected files (empty inline
//! + live ref) and rewrites nothing.

use std::path::Path;

use tauri::Manager;

use crate::managed_agents::custom_harnesses::{
    harness_env_key, migrate_harness_env, HarnessDefinition,
};
use crate::managed_agents::secret_projection::{load_secret, ProjectionStore};

/// Extract inline harness `env` secrets into the keyring, then scrub plaintext
/// artifacts once the projection is verified readable.
///
/// Non-fatal throughout: a keyless build, an unresolvable app-data dir, or an
/// unreadable directory each short-circuit without disturbing the rest of boot.
pub(super) fn migrate_harness_secrets_to_keyring(app: &tauri::AppHandle) {
    let Some(store) = crate::managed_agents::storage::agent_secret_store_pub() else {
        return; // keyless build — env stays inline in the 0o600 JSON
    };
    let Ok(data_dir) = app.path().app_data_dir() else {
        return;
    };
    let dir = data_dir.join("custom_harnesses");
    if !dir.exists() {
        return;
    }
    migrate_harness_secrets_in_dir(store, &dir);
}

/// Store-injected core of [`migrate_harness_secrets_to_keyring`], over a
/// concrete `dir`, so the full extract → verify → scrub flow is testable
/// against a [`ProjectionStore`] fake and a tempdir without an `AppHandle`.
fn migrate_harness_secrets_in_dir<S: ProjectionStore>(store: &S, dir: &Path) {
    // Phase 1: extract inline env → keyring, rewriting each changed file in
    // place (stripped, 0o600). Rewrite the ORIGINAL path, never an id-derived
    // one: a hand-authored file's name may differ from its `id`, and relocating
    // it would orphan the original and duplicate the entry.
    for path in harness_json_files(dir) {
        let Some(mut def) = read_definition(&path) else {
            continue; // unparseable live file — the loader already skips it
        };
        if migrate_harness_env(store, &mut def) {
            match serde_json::to_string_pretty(&def) {
                Ok(json) => {
                    if let Err(e) = crate::managed_agents::storage::atomic_write_json_restricted(
                        &path,
                        json.as_bytes(),
                    ) {
                        eprintln!(
                            "buzz-desktop: harness-migration: failed to rewrite {}: {e}",
                            path.display()
                        );
                    }
                }
                Err(e) => eprintln!(
                    "buzz-desktop: harness-migration: failed to serialize {}: {e}",
                    path.display()
                ),
            }
        }
    }

    // Phase 2: scrub plaintext artifacts — ONLY when every live file's projected
    // env reads back cleanly. A dangling ref means the keyring copy is
    // unavailable, so a backup could be the last recoverable plaintext; leave it.
    if extraction_verified(store, dir) {
        cleanup_harness_artifacts(dir);
    }
}

/// Returns `true` when every live `*.json` file's `env_ref` (if any) hydrates
/// from the keyring without error. A single failure returns `false` so the
/// caller skips the plaintext-artifact scrub this boot.
fn extraction_verified<S: ProjectionStore>(store: &S, dir: &Path) -> bool {
    for path in harness_json_files(dir) {
        let Some(def) = read_definition(&path) else {
            continue;
        };
        // A non-empty inline env is the authoritative fallback (keyring write
        // failed this boot) — it is not projected, so nothing to verify.
        if !def.env.is_empty() {
            continue;
        }
        let id = def.id.clone();
        match load_secret(
            store,
            def.env_ref.as_deref(),
            |gen| harness_env_key(&id, gen),
            &format!("harness:{id} env"),
        ) {
            Ok(_) => {}
            Err(e) => {
                eprintln!(
                    "buzz-desktop: harness-migration: extraction-verify failed for {}: {e} — \
                     skipping artifact scrub this boot",
                    path.display()
                );
                return false;
            }
        }
    }
    true
}

/// Scrub plaintext-bearing artifacts in the custom-harness dir: `*.json.bak`
/// backups (from the save path's backup-swap) and atomic-write temp siblings.
///
/// - Parseable backup → strip `env` and rewrite at `0o600` (non-secret fields
///   survive for manual recovery).
/// - Unparseable backup or atomic-write temp → delete (may carry plaintext).
/// - Live `*.json` files are NOT touched here — Phase 1 already stripped them.
/// - Never follows a symlink that escapes the dir.
fn cleanup_harness_artifacts(dir: &Path) {
    let canonical_dir = match std::fs::canonicalize(dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "buzz-desktop: harness-migration: cannot canonicalize {}: {e}",
                dir.display()
            );
            return;
        }
    };
    let entries = match std::fs::read_dir(&canonical_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "buzz-desktop: harness-migration: cannot read {}: {e}",
                canonical_dir.display()
            );
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let real_path = match std::fs::canonicalize(&path) {
            Ok(p) => p,
            Err(_) => path.clone(),
        };
        if !real_path.starts_with(&canonical_dir) {
            eprintln!(
                "buzz-desktop: harness-migration: skipping symlink escape: {}",
                path.display()
            );
            continue;
        }
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        let fname = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        if is_atomic_write_temp(&fname) {
            if let Err(e) = std::fs::remove_file(&real_path) {
                eprintln!("buzz-desktop: harness-migration: cannot remove temp {fname}: {e}");
            }
            continue;
        }
        if is_harness_backup(&fname) {
            scrub_backup_file(&real_path, &fname);
        }
    }
}

/// Strip the `env` field from a parseable harness backup and rewrite it at
/// `0o600`; delete the file when it cannot be parsed (may carry plaintext).
fn scrub_backup_file(path: &Path, fname: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("buzz-desktop: harness-migration: cannot read backup {fname}: {e}");
            return;
        }
    };
    match strip_env_from_json(&content) {
        Some(clean) => {
            if let Err(e) =
                crate::managed_agents::storage::atomic_write_json_restricted(path, clean.as_bytes())
            {
                eprintln!(
                    "buzz-desktop: harness-migration: cannot write scrubbed backup {fname}: {e}"
                );
            }
        }
        None => {
            if let Err(e) = std::fs::remove_file(path) {
                eprintln!("buzz-desktop: harness-migration: cannot remove unparseable backup {fname}: {e}");
            }
        }
    }
}

/// Remove the `env` object from a harness JSON document, returning the
/// re-serialized string. `None` when the content is not a JSON object.
fn strip_env_from_json(content: &str) -> Option<String> {
    use serde_json::Value;
    let mut v: Value = serde_json::from_str(content).ok()?;
    let Value::Object(map) = &mut v else {
        return None;
    };
    map.remove("env");
    serde_json::to_string_pretty(&v).ok()
}

/// Enumerate the live `*.json` files in `dir` (excludes `.json.bak`, temps, and
/// any non-`.json` sibling). Returns an empty vec on an unreadable directory.
fn harness_json_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect()
}

/// Parse one harness file with a raw (NON-hydrating) serde read. Warnings are
/// swallowed — a malformed live file is left in place for the user to fix,
/// exactly as the loader treats it.
fn read_definition(path: &Path) -> Option<HarnessDefinition> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// True when `fname` is a harness backup: `<id>.json.bak`.
fn is_harness_backup(fname: &str) -> bool {
    match fname.split_once(".json") {
        Some((_, after)) => after.contains(".bak"),
        None => false,
    }
}

/// True when `fname` looks like an atomic-write-file temp: no extension, at
/// least 8 chars, all ASCII hex digits.
fn is_atomic_write_temp(fname: &str) -> bool {
    !fname.contains('.') && fname.len() >= 8 && fname.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
#[path = "migration_harness_secrets_tests.rs"]
mod tests;
