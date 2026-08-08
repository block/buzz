//! Loader for user-defined ACP harness definitions.
//!
//! Users drop JSON files into `<app-data>/custom_harnesses/` to register
//! arbitrary ACP-speaking agents without modifying the app or opening a PR.
//! Each file describes a single harness; the loader validates, warns on
//! invalid entries, and never propagates errors to the discovery caller.
//!
//! **Security constraint (Will-ratified):** custom definitions carry NO install
//! shell commands. `can_auto_install` is always `false` for custom entries.
//! Only tier-1 compiled-in runtimes retain install-script power.
//!
//! **Avatar URL security:** custom/preset catalog entries MUST NOT carry
//! user-supplied avatar URLs. `HarnessDefinition` intentionally omits
//! `avatar_url` — all icons are bundled assets keyed via `RUNTIME_LOGOS`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Regex-equivalent predicate for a valid harness ID.
///
/// IDs must match `[a-z0-9_][a-z0-9_-]*` — lowercase alphanumeric plus
/// hyphens and underscores, starting with an alphanumeric or underscore.
/// This mirrors goose's `generate_id` validation and is intentionally
/// more restrictive than the filesystem to prevent path-traversal tricks.
fn is_valid_harness_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Public re-export of `is_valid_harness_id` for callers outside this module
/// (e.g., the `delete_custom_harness` command that must validate caller-supplied ids).
pub(crate) fn is_valid_harness_id_pub(id: &str) -> bool {
    is_valid_harness_id(id)
}

/// User-supplied harness definition deserialized from a JSON file.
///
/// Only the fields a custom harness definition is permitted to carry are
/// included here — install commands and avatar URLs are intentionally absent
/// (security line: no remote icon URLs from user-editable config).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HarnessDefinition {
    /// Unique identifier, must match `[a-z0-9_][a-z0-9_-]*`.
    pub id: String,
    /// Human-readable name shown in the UI.
    pub label: String,
    /// Primary executable name or absolute path. May not be empty.
    pub command: String,
    /// Default CLI arguments passed to the command (array, not split-string).
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables injected at spawn time. Definition env is applied
    /// first and LOSES on conflict with Buzz-injected vars — `BUZZ_MANAGED_AGENT`
    /// is always authoritative and cannot be overridden here.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Link to external docs for manual install/setup instructions.
    #[serde(default)]
    pub install_instructions_url: String,
    /// Human-readable install hint shown in Doctor.
    #[serde(default)]
    pub install_hint: String,
}

/// Scan `dir` for `*.json` files and deserialize each into a `HarnessDefinition`.
///
/// Errors per file are logged with `tracing::warn` and skipped — a single
/// malformed file never fails discovery for the rest.  Returns only
/// structurally valid, individually validated definitions.
///
/// Filtering is applied HERE — at the loader boundary — so every consumer
/// (discovery Phase 3, `warm_harness_registry_from_dir`, and through it the
/// spawn/readiness registry) inherits the same guarantees:
///   * `check_id_collision` — a hand-placed `goose.json` can never shadow a
///     built-in or preset id, even on the cold-launch warm path;
///   * duplicate-id dedup within the directory (first file wins).
///
/// **Callers must supply a fresh `dir` path on every `discover_acp_runtimes`
/// call** — this function performs no caching, mirroring goose's
/// `refresh_custom_providers()` pattern.
pub(crate) fn load_custom_harnesses(dir: &Path) -> Vec<HarnessDefinition> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return vec![],
        Err(err) => {
            tracing::warn!(
                "custom_harnesses: cannot read directory {}: {err}",
                dir.display()
            );
            return vec![];
        }
    };

    let mut definitions = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("custom_harnesses: failed to read {}: {err}", path.display());
                continue;
            }
        };

        let def: HarnessDefinition = match serde_json::from_str(&contents) {
            Ok(d) => d,
            Err(err) => {
                tracing::warn!(
                    "custom_harnesses: invalid JSON in {}: {err}",
                    path.display()
                );
                continue;
            }
        };

        if let Err(reason) = validate_harness_definition(&def) {
            tracing::warn!("custom_harnesses: skipping {} — {reason}", path.display());
            continue;
        }

        // A custom file must never shadow a built-in or preset id — enforced at
        // the loader so the warm path can't admit what discovery would reject.
        if let Err(reason) = check_id_collision(&def.id) {
            tracing::warn!("custom_harnesses: skipping {} — {reason}", path.display());
            continue;
        }

        // Dedup within the directory itself (a file's id is taken from its JSON
        // content, not its filename, so two files can carry the same id).
        if !seen_ids.insert(def.id.clone()) {
            tracing::warn!(
                "custom_harnesses: skipping {} — duplicate id {:?}",
                path.display(),
                def.id
            );
            continue;
        }

        definitions.push(def);
    }

    definitions
}

/// Validate a deserialized `HarnessDefinition` against the invariants that
/// the rest of the discovery code depends on.
fn validate_harness_definition(def: &HarnessDefinition) -> Result<(), String> {
    if def.id.is_empty() {
        return Err("id must not be empty".into());
    }
    if !is_valid_harness_id(&def.id) {
        return Err(format!(
            "id {:?} does not match [a-z0-9_][a-z0-9_-]* — use lowercase letters, digits, hyphens, and underscores only",
            def.id
        ));
    }
    if def.command.trim().is_empty() {
        return Err("command must not be empty".into());
    }
    if def.label.trim().is_empty() {
        return Err("label must not be empty".into());
    }
    // Args travel to the harness through the comma-delimited
    // `BUZZ_ACP_AGENT_ARGS` env transport (clap `value_delimiter = ','` on the
    // buzz-acp side), so a literal comma inside one argument would silently
    // split into two arguments at runtime. Reject at the validation boundary —
    // shared by save (UI/Tauri) and load (hand-authored files) — so the
    // invariant holds regardless of how the definition arrives.
    if let Some(arg) = def.args.iter().find(|a| a.contains(',')) {
        return Err(format!(
            "args: argument {arg:?} contains a comma — arguments are passed via a \
             comma-delimited transport and would be split at spawn time; \
             use separate argument entries instead"
        ));
    }
    // Validate env keys through the shared boundary validator.  This closes the
    // reserved-key bypass exploit (BUZZ_AUTH_TAG=x forgery shape), rejects
    // NUL bytes that would panic Command::env, and enforces size limits.
    crate::managed_agents::env_vars::validate_user_env_keys(&def.env)
        .map_err(|e| format!("env: {e}"))?;
    // Validate install instructions URL scheme when non-empty.
    if !def.install_instructions_url.is_empty() {
        let url = def.install_instructions_url.trim();
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(format!(
                "installInstructionsUrl must start with https:// or http://, got: {:?}",
                url
            ));
        }
    }
    Ok(())
}

/// Public wrapper so the `save_custom_harness` Tauri command can validate
/// without duplicating the rules.
pub(crate) fn validate_harness_definition_pub(def: &HarnessDefinition) -> Result<(), String> {
    validate_harness_definition(def)
}

// ── Built-in ID set ──────────────────────────────────────────────────────────

/// IDs reserved for the compiled-in catalog. A custom definition whose `id`
/// collides with a built-in or preset is rejected to prevent shadowing (e.g. a
/// file called `cursor.json` hiding the pre-existing tier-2 preset).
///
/// Derived at compile time from `PRESET_HARNESSES` (tier-2) plus the four
/// tier-1 runtimes — no hand-maintained copy.  Adding a preset to
/// `PRESET_HARNESSES` automatically reserves its ID without a separate edit.
fn builtin_ids() -> impl Iterator<Item = &'static str> {
    const TIER1: &[&str] = &["goose", "claude", "codex", "buzz-agent"];
    let tier2 = crate::managed_agents::discovery::preset_harness_ids();
    TIER1.iter().copied().chain(tier2.iter().copied())
}

/// Return an error string if `id` conflicts with a built-in harness ID.
pub(crate) fn check_id_collision(id: &str) -> Result<(), String> {
    if builtin_ids().any(|reserved| reserved.eq_ignore_ascii_case(id)) {
        return Err(format!(
            "id {:?} is reserved for a built-in harness and cannot be overridden",
            id
        ));
    }
    Ok(())
}

// ── Loaded harness registry (F2 — spawn resolution for custom/preset) ────────
//
// `known_acp_runtime` / `known_acp_runtime_exact` only search the static
// `KNOWN_ACP_RUNTIMES` table, so custom and preset harnesses were invisible at
// spawn time, causing silent fallback to buzz-agent.
//
// The fix: `discover_acp_runtimes_from` populates this registry with every
// non-builtin definition after each discovery run. Spawn, readiness, and
// summary paths query `lookup_loaded_harness` to get the live definition for a
// given id or command. If a harness id that an agent references is gone from the
// registry, the caller gets a typed error — never a silent buzz-agent fallback.

use std::sync::{Arc, RwLock};

/// Mutex used by tests to serialize writes to the loaded-harness registry.
///
/// The registry is a process-global singleton.  Parallel test execution can
/// interleave warm → lookup pairs from different tests, causing false failures.
/// Every test that calls `warm_harness_registry_from_dir` or
/// `update_loaded_harness_registry` must hold this guard for the lifetime of
/// its assertion block.
#[cfg(test)]
pub(crate) fn registry_test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Thread-safe registry of non-builtin (preset + custom) harness definitions,
/// populated on every `discover_acp_runtimes_from` call and queried at spawn time.
fn loaded_harness_registry() -> &'static RwLock<Vec<Arc<HarnessDefinition>>> {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<RwLock<Vec<Arc<HarnessDefinition>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Replace the registry contents with `definitions`. Called once per
/// `discover_acp_runtimes_from` run AND on `save_custom_harness` /
/// `delete_custom_harness` so spawn can always resolve the harness without
/// waiting for the next full discovery.
pub(crate) fn update_loaded_harness_registry(definitions: Vec<HarnessDefinition>) {
    let arcs: Vec<Arc<HarnessDefinition>> = definitions.into_iter().map(Arc::new).collect();
    // Use `into_inner` to recover from a poisoned lock — the registry is a
    // plain replaceable Vec with no torn invariant, so poison recovery is safe.
    let mut guard = match loaded_harness_registry().write() {
        Ok(g) => g,
        Err(poisoned) => {
            tracing::warn!("custom_harnesses: loaded-harness registry was poisoned; recovering");
            poisoned.into_inner()
        }
    };
    *guard = arcs;
}

/// Look up a loaded (non-builtin) harness by **id**. Returns `None` when the id
/// is unknown. Uses `into_inner` to recover from a poisoned lock so a panic in
/// one thread never permanently blocks all spawn attempts.
pub(crate) fn lookup_loaded_harness_by_id(id: &str) -> Option<Arc<HarnessDefinition>> {
    let guard = match loaded_harness_registry().read() {
        Ok(g) => g,
        Err(poisoned) => {
            tracing::warn!(
                "custom_harnesses: loaded-harness registry read lock was poisoned; recovering"
            );
            poisoned.into_inner()
        }
    };
    guard.iter().find(|d| d.id == id).cloned()
}

/// Warm the loaded-harness registry synchronously from `custom_dir`.
///
/// Must be called **before** `restore_managed_agents_on_launch` so that cold
/// relaunches can resolve custom/preset harness ids without a full discover
/// round-trip (which is driven by the frontend and arrives later).
///
/// This is intentionally lightweight: it only loads the custom JSON files and
/// the static preset list — no PATH probing, no availability checks.
pub(crate) fn warm_harness_registry_from_dir(custom_dir: Option<&std::path::Path>) {
    // Load only the preset list from the discovery module (static, free).
    let preset_defs = crate::managed_agents::discovery::preset_harness_definitions();
    let custom_defs = custom_dir.map(load_custom_harnesses).unwrap_or_default();
    let mut all: Vec<HarnessDefinition> = preset_defs;
    all.extend(custom_defs);
    update_loaded_harness_registry(all);
}

/// Publish the loaded-harness registry from a **fresh** directory read while
/// holding the persist mutex — the discovery-side counterpart of
/// `save_and_warm` / `delete_and_warm`.
///
/// `discover_acp_runtimes_from` runs slow auth probes between its directory
/// scan and its registry publish. Publishing the pre-probe snapshot could
/// clobber a `save_and_warm` that landed in that window, leaving a freshly
/// saved harness unresolvable at spawn until the next discovery. Re-reading
/// the directory at publish time, under the same mutex as save/delete, makes
/// the documented guarantee ("the registry always reflects disk at warm time")
/// actually hold. Only the publish is locked — never the probes.
pub(crate) fn warm_harness_registry_locked(custom_dir: Option<&std::path::Path>) {
    let _guard = persist_mutex().lock().unwrap_or_else(|e| e.into_inner());
    warm_harness_registry_from_dir(custom_dir);
}

/// Global mutex that serializes save/delete filesystem mutations and the
/// subsequent registry warm as a single atomic unit.
///
/// Without this lock, two concurrent `save_custom_harness` calls could
/// interleave: A writes file-A, B writes file-B, B re-warms (sees only B),
/// A re-warms (sees A + B, wins) — but if timing goes the other way B's warm
/// wins and misses A. Holding the lock for the write+warm pair guarantees that
/// the registry always reflects the complete set of files on disk at the time
/// of the warm.
fn persist_mutex() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};
    static PERSIST: OnceLock<Mutex<()>> = OnceLock::new();
    PERSIST.get_or_init(|| Mutex::new(()))
}

/// Write `definition` to `dir`, then atomically warm the loaded-harness
/// registry.  Callers MUST use this function (not `save_custom_harness_to_dir`
/// or `warm_harness_registry_from_dir` separately) so that concurrent saves
/// cannot produce a stale registry snapshot.
pub(crate) fn save_and_warm(
    dir: &Path,
    definition: &HarnessDefinition,
    rename_old_id: Option<&str>,
) -> Result<SaveOutcome, String> {
    let _guard = persist_mutex().lock().unwrap_or_else(|e| e.into_inner());
    let outcome = save_custom_harness_to_dir(dir, definition, rename_old_id)?;
    warm_harness_registry_from_dir(Some(dir));
    Ok(outcome)
}

/// Delete `id.json` from `dir`, then atomically warm the loaded-harness
/// registry.  Callers MUST use this function (not `fs::remove_file` +
/// `warm_harness_registry_from_dir` separately) for the same reason as
/// `save_and_warm`.
pub(crate) fn delete_and_warm(dir: &Path, id: &str) -> Result<(), String> {
    let _guard = persist_mutex().lock().unwrap_or_else(|e| e.into_inner());
    let target = dir.join(format!("{id}.json"));
    match std::fs::remove_file(&target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("failed to delete harness {id:?}: {e}")),
    }
    warm_harness_registry_from_dir(Some(dir));
    Ok(())
}

/// The outcome of a successful [`save_custom_harness_to_dir`] call.
#[allow(dead_code)] // Fields consumed by tests; production callers use the Result shape.
pub(crate) struct SaveOutcome {
    /// The path of the newly written harness file.
    pub target_path: std::path::PathBuf,
    /// The path of the old file that was removed on a rename, if any.
    /// `None` when the id was unchanged or this was a new harness.
    pub removed_old_path: Option<std::path::PathBuf>,
}

/// Write a harness definition to `dir/<id>.json` using a backup-swap strategy
/// that is safe on all platforms (including Windows where `fs::rename` over an
/// existing file fails with "access denied"):
///
/// 1. Serialize the definition and write it to a unique temp file via
///    `atomic_write_file`.
/// 2. If the target already exists, rename it to `<target>.bak` (the backup).
/// 3. `commit()` the temp file (renames temp → target).
///    * On success: delete `.bak` (best-effort; a stale `.bak` is harmless).
///    * On failure: restore `.bak` → target so the original is never lost.
/// 4. If `rename_old_id` is `Some`, remove `<dir>/<old_id>.json` after the
///    new file is committed (non-fatal if NotFound).
///
/// The caller is responsible for full validation (id, env, etc.) BEFORE
/// calling this function — no validation is performed here.
pub(crate) fn save_custom_harness_to_dir(
    dir: &Path,
    definition: &HarnessDefinition,
    rename_old_id: Option<&str>,
) -> Result<SaveOutcome, String> {
    use atomic_write_file::AtomicWriteFile;
    use std::io::Write;

    let json = serde_json::to_string_pretty(definition)
        .map_err(|e| format!("failed to serialize harness definition: {e}"))?;

    let target_path = dir.join(format!("{}.json", definition.id));
    let bak_path = dir.join(format!("{}.json.bak", definition.id));

    // Stage write to temp file.
    let mut file = AtomicWriteFile::open(&target_path)
        .map_err(|e| format!("failed to open {}: {e}", target_path.display()))?;
    file.write_all(json.as_bytes())
        .map_err(|e| format!("failed to write harness definition: {e}"))?;

    // Back up the existing target before committing so we can restore on failure.
    let had_backup = if target_path.exists() {
        std::fs::rename(&target_path, &bak_path).map_err(|e| {
            format!(
                "failed to back up {} before replace: {e}",
                target_path.display()
            )
        })?;
        true
    } else {
        false
    };

    // Commit temp → target.  If commit fails and we made a backup, restore it.
    if let Err(e) = file.commit() {
        if had_backup {
            if let Err(restore_err) = std::fs::rename(&bak_path, &target_path) {
                // Both commit and restore failed — surface both so the user
                // can recover manually.
                return Err(format!(
                    "failed to finalize harness definition: {e}; \
                     also failed to restore backup: {restore_err} \
                     (original may be at {})",
                    bak_path.display()
                ));
            }
        }
        return Err(format!("failed to finalize harness definition: {e}"));
    }

    // Commit succeeded — remove the backup (best-effort; stale .bak is harmless).
    if had_backup {
        let _ = std::fs::remove_file(&bak_path);
    }

    // Remove old file on id rename, after the new file is committed.
    let mut removed_old_path = None;
    if let Some(old_id) = rename_old_id {
        let old_path = dir.join(format!("{old_id}.json"));
        match std::fs::remove_file(&old_path) {
            Ok(()) => removed_old_path = Some(old_path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                // New file is already committed — log and continue so the
                // registry re-warm picks up the new id.
                tracing::warn!(
                    "save_custom_harness_to_dir: failed to remove old harness {old_id:?}: {e}"
                );
            }
        }
    }

    Ok(SaveOutcome {
        target_path,
        removed_old_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── ID validation ────────────────────────────────────────────────────────

    #[test]
    fn valid_id_lowercase_with_hyphen() {
        assert!(is_valid_harness_id("my-agent"));
    }

    #[test]
    fn valid_id_underscore_start() {
        assert!(is_valid_harness_id("_my_agent"));
    }

    #[test]
    fn valid_id_alphanumeric() {
        assert!(is_valid_harness_id("agent42"));
    }

    #[test]
    fn invalid_id_uppercase() {
        assert!(!is_valid_harness_id("MyAgent"));
    }

    #[test]
    fn invalid_id_starts_with_hyphen() {
        assert!(!is_valid_harness_id("-bad-id"));
    }

    #[test]
    fn invalid_id_empty() {
        assert!(!is_valid_harness_id(""));
    }

    #[test]
    fn invalid_id_path_traversal() {
        assert!(!is_valid_harness_id("../etc/passwd"));
    }

    // ── Collision check ──────────────────────────────────────────────────────

    #[test]
    fn builtin_ids_are_rejected() {
        // Tier-1 hard-coded IDs must always be reserved.
        for id in &["goose", "claude", "codex", "buzz-agent"] {
            assert!(check_id_collision(id).is_err(), "{id} should be rejected");
        }
        // Tier-2 preset IDs must also be reserved (derived from PRESET_HARNESSES).
        for id in crate::managed_agents::discovery::preset_harness_ids() {
            assert!(check_id_collision(id).is_err(), "{id} should be rejected");
        }
    }

    #[test]
    fn unknown_id_passes_collision_check() {
        assert!(check_id_collision("my-custom-agent").is_ok());
    }

    // ── File loading ─────────────────────────────────────────────────────────

    #[test]
    fn load_valid_json_returns_definition() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("my-agent.json"),
            r#"{"id":"my-agent","label":"My Agent","command":"my-agent-bin"}"#,
        )
        .unwrap();

        let defs = load_custom_harnesses(dir.path());
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].id, "my-agent");
        assert_eq!(defs[0].label, "My Agent");
        assert_eq!(defs[0].command, "my-agent-bin");
    }

    #[test]
    fn load_skips_non_json_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("my-agent.toml"), r#"id = "my-agent""#).unwrap();

        let defs = load_custom_harnesses(dir.path());
        assert_eq!(defs.len(), 0, "non-JSON file should be ignored");
    }

    #[test]
    fn load_skips_invalid_json_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bad.json"), "{ not valid json").unwrap();

        // Must not panic or propagate an error.
        let defs = load_custom_harnesses(dir.path());
        assert_eq!(defs.len(), 0);
    }

    #[test]
    fn load_skips_definition_with_invalid_id() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Bad.json"),
            r#"{"id":"Bad-Id","label":"Bad","command":"bad"}"#,
        )
        .unwrap();

        let defs = load_custom_harnesses(dir.path());
        assert_eq!(
            defs.len(),
            0,
            "invalid id should cause the entry to be skipped"
        );
    }

    #[test]
    fn load_skips_definition_with_empty_command() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("empty-cmd.json"),
            r#"{"id":"empty-cmd","label":"Empty","command":""}"#,
        )
        .unwrap();

        let defs = load_custom_harnesses(dir.path());
        assert_eq!(
            defs.len(),
            0,
            "empty command should cause the entry to be skipped"
        );
    }

    #[test]
    fn load_skips_definition_with_non_http_install_url() {
        // installInstructionsUrl must start with https:// or http://.
        // A bare path, javascript: URI, or other scheme is rejected.
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("bad-url.json"),
            r#"{"id":"bad-url","label":"Bad","command":"bad-bin","installInstructionsUrl":"file:///etc/passwd"}"#,
        )
        .unwrap();

        let defs = load_custom_harnesses(dir.path());
        assert_eq!(
            defs.len(),
            0,
            "non-http install URL should cause the entry to be skipped"
        );
    }

    #[test]
    fn load_accepts_empty_or_https_install_url() {
        let dir = tempfile::tempdir().unwrap();
        // Empty URL is fine (optional field).
        fs::write(
            dir.path().join("no-url.json"),
            r#"{"id":"no-url","label":"No URL","command":"no-url-bin"}"#,
        )
        .unwrap();
        // https:// is accepted.
        fs::write(
            dir.path().join("good-url.json"),
            r#"{"id":"good-url","label":"Good URL","command":"good-bin","installInstructionsUrl":"https://example.com/install"}"#,
        )
        .unwrap();

        let defs = load_custom_harnesses(dir.path());
        assert_eq!(
            defs.len(),
            2,
            "empty and https:// URLs must both be accepted"
        );
    }

    #[test]
    fn load_missing_dir_returns_empty_vec() {
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("does_not_exist");

        let defs = load_custom_harnesses(&nonexistent);
        assert_eq!(defs.len(), 0);
    }

    #[test]
    fn load_continues_after_one_bad_entry() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bad.json"), "!!!").unwrap();
        fs::write(
            dir.path().join("good.json"),
            r#"{"id":"good-one","label":"Good","command":"good-binary"}"#,
        )
        .unwrap();

        let defs = load_custom_harnesses(dir.path());
        assert_eq!(defs.len(), 1, "bad entry skipped, good entry loaded");
        assert_eq!(defs[0].id, "good-one");
    }

    #[test]
    fn load_applies_id_collision_check() {
        // A custom file whose id shadows a built-in ("goose") must be dropped
        // BY THE LOADER — `load_custom_harnesses` is the enforcement boundary
        // shared by both the warm path and discovery. This exercises the real
        // loader against a real file, not just the helper predicate.
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("goose.json"),
            r#"{"id":"goose","label":"Not Goose","command":"goose","args":["--evil"]}"#,
        )
        .unwrap();
        assert!(
            load_custom_harnesses(dir.path()).is_empty(),
            "loader must drop a file shadowing a builtin id"
        );
        assert!(check_id_collision("goose").is_err());
        assert!(check_id_collision("custom-goose").is_ok());
    }

    #[test]
    fn load_dedups_duplicate_ids_first_file_wins() {
        // Two files carrying the same custom id: the loader must keep exactly
        // one definition (directory-order first wins; the duplicate is dropped).
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.json"),
            r#"{"id":"custom-dup","label":"First","command":"first-cmd"}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("b.json"),
            r#"{"id":"custom-dup","label":"Second","command":"second-cmd"}"#,
        )
        .unwrap();
        let loaded = load_custom_harnesses(dir.path());
        assert_eq!(
            loaded.len(),
            1,
            "loader must dedup duplicate ids within the directory"
        );
        assert_eq!(loaded[0].id, "custom-dup");
    }

    // ── Round-trip via save_custom_harness_to_dir (B-4) ─────────────────────
    //
    // These tests exercise the REAL persistence helper, not raw fs::write.
    // They prove: create, same-ID edit (backup-swap), rename (old file removed),
    // backup file cleaned up on success.

    fn make_def(id: &str, label: &str) -> HarnessDefinition {
        HarnessDefinition {
            id: id.to_string(),
            label: label.to_string(),
            command: format!("{id}-bin"),
            args: vec![],
            env: BTreeMap::new(),
            install_instructions_url: String::new(),
            install_hint: String::new(),
        }
    }

    #[test]
    fn save_to_dir_create_writes_file_and_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let def = make_def("my-harness", "My Harness");

        let outcome = save_custom_harness_to_dir(dir.path(), &def, None).unwrap();

        assert_eq!(outcome.target_path, dir.path().join("my-harness.json"));
        assert!(outcome.removed_old_path.is_none(), "no old file on create");

        let loaded = load_custom_harnesses(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "my-harness");
        assert_eq!(loaded[0].label, "My Harness");
    }

    #[test]
    fn save_to_dir_same_id_edit_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = make_def("my-harness", "V1 Label");
        save_custom_harness_to_dir(dir.path(), &v1, None).unwrap();

        // Same-ID edit: label changes.
        let v2 = make_def("my-harness", "V2 Label");
        let outcome = save_custom_harness_to_dir(dir.path(), &v2, None).unwrap();

        // No old-path reported (id unchanged).
        assert!(outcome.removed_old_path.is_none());

        let loaded = load_custom_harnesses(dir.path());
        assert_eq!(loaded.len(), 1, "same-id edit must not duplicate entries");
        assert_eq!(loaded[0].label, "V2 Label", "v2 content must be present");
    }

    #[test]
    fn save_to_dir_backup_is_cleaned_up_after_same_id_edit() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = make_def("my-harness", "V1");
        save_custom_harness_to_dir(dir.path(), &v1, None).unwrap();

        let v2 = make_def("my-harness", "V2");
        save_custom_harness_to_dir(dir.path(), &v2, None).unwrap();

        // .bak file must be gone after a successful commit.
        let bak = dir.path().join("my-harness.json.bak");
        assert!(
            !bak.exists(),
            ".bak file must be removed after successful same-id edit"
        );
    }

    #[test]
    fn save_to_dir_rename_removes_old_file_and_creates_new() {
        let dir = tempfile::tempdir().unwrap();
        let old_def = make_def("old-id", "Old");
        save_custom_harness_to_dir(dir.path(), &old_def, None).unwrap();

        // Rename: new id, old_id supplied.
        let new_def = make_def("new-id", "New");
        let outcome = save_custom_harness_to_dir(dir.path(), &new_def, Some("old-id")).unwrap();

        // The outcome carries the old path that was removed.
        let expected_old = dir.path().join("old-id.json");
        assert_eq!(
            outcome.removed_old_path,
            Some(expected_old.clone()),
            "removed_old_path must be the old file"
        );

        // Old file gone, new file present.
        assert!(!expected_old.exists(), "old-id.json must be removed");
        let loaded = load_custom_harnesses(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "new-id");
    }

    #[test]
    fn save_to_dir_rename_nonexistent_old_id_is_non_fatal() {
        // rename_old_id pointing to a file that does not exist must succeed
        // (NotFound is silently ignored by the helper).
        let dir = tempfile::tempdir().unwrap();
        let def = make_def("alpha", "Alpha");
        let outcome = save_custom_harness_to_dir(dir.path(), &def, Some("ghost-id")).unwrap();

        // New file created, no old path removed.
        assert_eq!(outcome.target_path, dir.path().join("alpha.json"));
        assert!(
            outcome.removed_old_path.is_none(),
            "NotFound old-id must not be reported as removed"
        );
        assert!(load_custom_harnesses(dir.path()).len() == 1);
    }

    #[test]
    fn save_to_dir_roundtrip_with_env_preserves_values() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = BTreeMap::new();
        env.insert("MY_KEY".to_string(), "my_value".to_string());
        let def = HarnessDefinition {
            id: "env-harness".to_string(),
            label: "Env Harness".to_string(),
            command: "env-bin".to_string(),
            args: vec!["--flag".to_string()],
            env,
            install_instructions_url: "https://example.com".to_string(),
            install_hint: "Install from example.com".to_string(),
        };

        save_custom_harness_to_dir(dir.path(), &def, None).unwrap();

        let loaded = load_custom_harnesses(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].args, vec!["--flag"]);
        assert_eq!(
            loaded[0].env.get("MY_KEY").map(String::as_str),
            Some("my_value"),
            "env must round-trip through save_custom_harness_to_dir"
        );
    }

    // ── B-3: env validation boundary (validate_harness_definition_pub integration) ──

    #[test]
    fn validate_rejects_malformed_key_with_equals_sign() {
        // BUZZ_AUTH_TAG=x is the documented reserved-key bypass shape:
        // the key contains '=' so Command::env would produce
        // `BUZZ_AUTH_TAG=x=forged` in the child env.
        let mut env = BTreeMap::new();
        env.insert("BUZZ_AUTH_TAG=x".to_string(), "forged".to_string());
        let def = HarnessDefinition {
            id: "bad-env".to_string(),
            label: "Bad".to_string(),
            command: "bad-bin".to_string(),
            args: vec![],
            env,
            install_instructions_url: String::new(),
            install_hint: String::new(),
        };
        let err = validate_harness_definition_pub(&def).unwrap_err();
        assert!(
            err.contains("env var keys must match"),
            "malformed key must be rejected: {err}"
        );
        assert!(
            err.contains("BUZZ_AUTH_TAG"),
            "error must name the offending key: {err}"
        );
    }

    #[test]
    fn validate_rejects_reserved_key_buzz_managed_agent() {
        // BUZZ_MANAGED_AGENT and BUZZ_MANAGED_AGENT_START_NONCE are the
        // ownership markers — supplying them in a definition must be rejected.
        let mut env = BTreeMap::new();
        env.insert(
            "BUZZ_MANAGED_AGENT".to_string(),
            "fake-instance".to_string(),
        );
        let def = HarnessDefinition {
            id: "bad-marker".to_string(),
            label: "Bad".to_string(),
            command: "bad-bin".to_string(),
            args: vec![],
            env,
            install_instructions_url: String::new(),
            install_hint: String::new(),
        };
        let err = validate_harness_definition_pub(&def).unwrap_err();
        assert!(
            err.contains("reserved by Buzz"),
            "ownership marker key must be rejected: {err}"
        );
    }

    #[test]
    fn validate_rejects_reserved_key_case_insensitive() {
        // BUZZ_PRIVATE_KEY in any casing must be blocked.
        let mut env = BTreeMap::new();
        env.insert("buzz_private_key".to_string(), "secret".to_string());
        let def = HarnessDefinition {
            id: "ci-marker".to_string(),
            label: "CI".to_string(),
            command: "ci-bin".to_string(),
            args: vec![],
            env,
            install_instructions_url: String::new(),
            install_hint: String::new(),
        };
        let err = validate_harness_definition_pub(&def).unwrap_err();
        assert!(
            err.contains("reserved by Buzz"),
            "reserved key must be blocked case-insensitively: {err}"
        );
    }

    #[test]
    fn validate_rejects_nul_byte_in_value() {
        // A NUL in a value would cause Command::env to panic at spawn time.
        let mut env = BTreeMap::new();
        env.insert("MY_KEY".to_string(), "val\x00ue".to_string());
        let def = HarnessDefinition {
            id: "nul-val".to_string(),
            label: "NUL".to_string(),
            command: "nul-bin".to_string(),
            args: vec![],
            env,
            install_instructions_url: String::new(),
            install_hint: String::new(),
        };
        let err = validate_harness_definition_pub(&def).unwrap_err();
        assert!(
            err.contains("NUL bytes"),
            "NUL value must be rejected at validation: {err}"
        );
    }

    #[test]
    fn validate_rejects_value_over_per_value_size_limit() {
        use crate::managed_agents::env_vars::MAX_ENV_VALUE_BYTES;
        let mut env = BTreeMap::new();
        // One byte over the per-value cap.
        env.insert("BIG_VAL".to_string(), "x".repeat(MAX_ENV_VALUE_BYTES + 1));
        let def = HarnessDefinition {
            id: "big-val".to_string(),
            label: "Big".to_string(),
            command: "big-bin".to_string(),
            args: vec![],
            env,
            install_instructions_url: String::new(),
            install_hint: String::new(),
        };
        let err = validate_harness_definition_pub(&def).unwrap_err();
        assert!(
            err.contains("per-value limit"),
            "oversized value must be rejected: {err}"
        );
    }

    #[test]
    fn validate_accepts_well_formed_env() {
        let mut env = BTreeMap::new();
        env.insert("ANTHROPIC_API_KEY".to_string(), "sk-test-123".to_string());
        env.insert("MODEL_VERSION".to_string(), "claude-3".to_string());
        let def = HarnessDefinition {
            id: "good-env".to_string(),
            label: "Good".to_string(),
            command: "good-bin".to_string(),
            args: vec![],
            env,
            install_instructions_url: String::new(),
            install_hint: String::new(),
        };
        assert!(
            validate_harness_definition_pub(&def).is_ok(),
            "well-formed definition must pass validation"
        );
    }

    // ── Comma-in-args validation (transport-lossiness guard) ─────────────────

    /// A definition whose args contain a literal comma must be rejected at the
    /// validation boundary — the comma-delimited `BUZZ_ACP_AGENT_ARGS`
    /// transport would silently split it into two args at spawn time.
    #[test]
    fn validate_rejects_comma_in_args() {
        let mut def = make_def("comma-args", "Comma");
        def.args = vec!["--name".to_string(), "a,b".to_string()];
        let err = validate_harness_definition_pub(&def).unwrap_err();
        assert!(
            err.contains("comma"),
            "error must explain the comma transport limit, got: {err}"
        );
    }

    /// Comma-free args pass — including args with spaces and special chars.
    #[test]
    fn validate_accepts_comma_free_args() {
        let mut def = make_def("clean-args", "Clean");
        def.args = vec!["acp".to_string(), "--flag=x y".to_string()];
        assert!(validate_harness_definition_pub(&def).is_ok());
    }

    /// The loader shares the same validator: a hand-authored file with a comma
    /// in args is skipped, so the invariant holds regardless of how the
    /// definition arrives (UI save or hand-edited JSON).
    #[test]
    fn load_skips_definition_with_comma_in_args() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("comma.json"),
            r#"{"id":"comma-file","label":"Comma","command":"comma-bin","args":["a,b"]}"#,
        )
        .unwrap();
        assert!(
            load_custom_harnesses(dir.path()).is_empty(),
            "comma-in-args definition must be skipped at the loader boundary"
        );
    }

    // ── Discovery publish under persist_mutex (stale-snapshot regression) ────

    /// Discovery's registry publish must re-read the directory at publish time
    /// (under the persist mutex), not push a snapshot taken before the auth
    /// probes ran. Regression shape: discovery scans dir → user saves harness X
    /// (save_and_warm warms the registry with X) → discovery finishes. If
    /// discovery published its pre-save snapshot, X would be on disk but
    /// unresolvable at spawn until the next discover.
    ///
    /// Deterministic interleaving: we simulate it by calling the publish seam
    /// (`warm_harness_registry_locked`) after a save that happened "during"
    /// discovery — the fresh-read semantics mean the just-saved definition
    /// survives the publish.
    #[test]
    fn discovery_publish_after_concurrent_save_keeps_saved_harness() {
        let _lock = registry_test_lock();
        let dir = tempfile::tempdir().unwrap();

        // Discovery "scans" the dir while it is empty (stale snapshot would be []).
        let stale_snapshot = load_custom_harnesses(dir.path());
        assert!(stale_snapshot.is_empty());

        // A save lands mid-discovery (save_and_warm: write + warm).
        let def = make_def("mid-save", "Mid Save");
        save_and_warm(dir.path(), &def, None).unwrap();
        assert!(lookup_loaded_harness_by_id("mid-save").is_some());

        // Discovery publishes — the locked warm re-reads the directory, so the
        // just-saved harness must survive (a stale-snapshot publish would
        // clobber it).
        warm_harness_registry_locked(Some(dir.path()));
        assert!(
            lookup_loaded_harness_by_id("mid-save").is_some(),
            "publish must re-read the directory, not clobber the mid-discovery save"
        );
    }

    /// Same shape for delete: a delete landing mid-discovery must not be
    /// resurrected by the discovery publish.
    #[test]
    fn discovery_publish_after_concurrent_delete_keeps_harness_gone() {
        let _lock = registry_test_lock();
        let dir = tempfile::tempdir().unwrap();

        let def = make_def("mid-delete", "Mid Delete");
        save_and_warm(dir.path(), &def, None).unwrap();

        // Discovery "scans" while the file exists (stale snapshot would contain it).
        let stale_snapshot = load_custom_harnesses(dir.path());
        assert_eq!(stale_snapshot.len(), 1);

        // Delete lands mid-discovery.
        delete_and_warm(dir.path(), "mid-delete").unwrap();
        assert!(lookup_loaded_harness_by_id("mid-delete").is_none());

        // Discovery publishes — fresh read keeps it gone.
        warm_harness_registry_locked(Some(dir.path()));
        assert!(
            lookup_loaded_harness_by_id("mid-delete").is_none(),
            "publish must not resurrect a harness deleted mid-discovery"
        );
    }

    // ── Registry warm path ───────────────────────────────────────────────────

    /// After `warm_harness_registry_from_dir` the registry contains preset +
    /// custom definitions and `lookup_loaded_harness_by_id` resolves them.
    #[test]
    fn warm_registry_then_lookup_finds_custom_and_preset_entries() {
        let _lock = registry_test_lock();
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("my-custom.json"),
            r#"{"id":"my-custom","label":"My Custom","command":"my-custom-bin"}"#,
        )
        .unwrap();

        warm_harness_registry_from_dir(Some(dir.path()));

        // Custom entry must be findable.
        let found = lookup_loaded_harness_by_id("my-custom");
        assert!(
            found.is_some(),
            "warm registry must contain the custom entry"
        );
        assert_eq!(found.unwrap().command, "my-custom-bin");

        // At least one preset entry must be in the registry (e.g. "cursor").
        let preset = lookup_loaded_harness_by_id("cursor");
        assert!(
            preset.is_some(),
            "warm registry must contain preset entries"
        );
    }

    /// `warm_harness_registry_from_dir` with `None` still loads presets.
    #[test]
    fn warm_registry_with_no_custom_dir_loads_presets_only() {
        let _lock = registry_test_lock();
        warm_harness_registry_from_dir(None);
        // At least the "cursor" preset must be present.
        assert!(
            lookup_loaded_harness_by_id("cursor").is_some(),
            "presets must be reachable even without a custom dir"
        );
    }

    /// `warm_harness_registry_from_dir` followed by `update_loaded_harness_registry`
    /// with an empty slice clears the registry (transactional save/delete contract).
    #[test]
    fn warm_then_clear_registry_empties_lookup() {
        let _lock = registry_test_lock();
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("tmp-agent.json"),
            r#"{"id":"tmp-agent","label":"Tmp","command":"tmp-bin"}"#,
        )
        .unwrap();

        warm_harness_registry_from_dir(Some(dir.path()));
        assert!(lookup_loaded_harness_by_id("tmp-agent").is_some());

        // Simulate delete — re-warm with empty dir.
        let empty_dir = tempfile::tempdir().unwrap();
        warm_harness_registry_from_dir(Some(empty_dir.path()));
        assert!(
            lookup_loaded_harness_by_id("tmp-agent").is_none(),
            "deleted harness must not appear after re-warm"
        );
    }

    // ── Legacy avatarUrl regression (F1) ─────────────────────────────────────

    /// A JSON file that contains a legacy `avatarUrl` field (from pre-BYOH code)
    /// must still deserialize without error (unknown-field handling) and the
    /// loaded `HarnessDefinition` must NOT carry the URL — the field is absent
    /// from the struct so serde drops it.
    #[test]
    fn legacy_avatar_url_in_json_is_silently_dropped_on_load() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("legacy.json"),
            r#"{
                "id": "legacy-agent",
                "label": "Legacy Agent",
                "command": "legacy-bin",
                "avatarUrl": "https://tracking.example.com/logo.png"
            }"#,
        )
        .unwrap();

        let defs = load_custom_harnesses(dir.path());
        // The file must deserialize successfully (serde ignores unknown fields).
        assert_eq!(defs.len(), 1, "legacy file with avatarUrl must still load");
        assert_eq!(defs[0].id, "legacy-agent");
        // HarnessDefinition has no avatar_url field — prove the URL cannot
        // be routed to a catalog entry by serializing back and checking.
        let json = serde_json::to_string(&defs[0]).unwrap();
        assert!(
            !json.contains("https://tracking.example.com"),
            "serialized HarnessDefinition must not contain the legacy avatar URL"
        );
    }

    // ── Preset id reservation ────────────────────────────────────────────────

    /// All preset ids must be blocked by `check_id_collision`.
    #[test]
    fn preset_ids_are_reserved_and_cannot_be_used_as_custom_ids() {
        // Derived from PRESET_HARNESSES — no hard-coded copy here so this test
        // automatically covers any future preset additions.
        for id in crate::managed_agents::discovery::preset_harness_ids() {
            assert!(
                check_id_collision(id).is_err(),
                "preset id {id:?} should be rejected by check_id_collision"
            );
        }
    }
}
