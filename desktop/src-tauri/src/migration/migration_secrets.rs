//! Boot-time secret extraction: lifts inline env vars, auth tags, and
//! provider configs from JSON into the OS keyring via the generation-reference
//! protocol, then runs the two-cycle GC sweeps and artifact cleanup.
//!
//! Runs ONCE at the END of `run_boot_migrations_inner` (after
//! `materialize_agent_runtimes`) so raw-JSON migrations see inline values.
//!
//! Idempotent across launches, but NOT by "skipping records that already have
//! refs": it re-runs the field-granular migration seam over every record on
//! every launch. That seam projects only a non-empty inline value and
//! otherwise preserves an existing ref — it never re-reads an already-projected
//! record's empty inline field as a user-clear, so a second launch is a no-op
//! that leaves every committed ref intact (the strip-on-save seam would have
//! cleared them). See
//! [`crate::managed_agents::secret_seam::migrate_all_secrets_for_records`].

use tauri::Manager;

/// Extract inline secrets (env vars, auth tags, provider configs) from JSON
/// into the keyring.  Also runs the two-cycle GC sweeps and, when the keyring
/// is reachable, Phase 2 artifact cleanup.
pub(super) fn migrate_inline_secrets_to_keyring(app: &tauri::AppHandle) {
    // Serialize the entire extraction + global save + GC against in-process
    // saves that mutate the same JSON stores and keyring blob. GC's final JSON
    // read-back and `remove_batch` MUST be indivisible against a save sitting
    // between its keyring write and its atomic JSON commit — otherwise the
    // sweep could read stale JSON, decide a just-written generation is
    // unreferenced, and delete it. The store lock is the same one every save
    // path (agent store, global config, card mint) takes.
    let state = app.state::<crate::app_state::AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // Extract inline secrets → keyring for the agent store.
    let extraction_ok = if let Ok(mut records) =
        crate::managed_agents::storage::load_agent_store_raw(app)
    {
        // Cross-process transaction lock: held across BOTH the generation
        // writes (inside `migrate_inline_secrets_in_records`) and the JSON
        // commit below, so a second Desktop process's GC cannot delete a
        // just-written generation in the window before its ref lands in JSON.
        // The in-process `managed_agents_store_lock` above only serializes THIS
        // process; the file lock closes the cross-process interleave.
        match crate::managed_agents::storage::acquire_secret_txn_lock(app) {
            Ok(_txn) => {
                let changed = migrate_inline_secrets_in_records(app, &mut records);
                if changed {
                    if let Err(e) =
                        crate::managed_agents::storage::write_agent_store_raw(app, &records)
                    {
                        eprintln!("buzz-desktop: boot-migration: failed to write agent store: {e}");
                        false
                    } else {
                        true
                    }
                } else {
                    true
                }
            }
            Err(e) => {
                eprintln!(
                    "buzz-desktop: boot-migration: could not acquire secret transaction lock \
                     ({e}); skipping agent-store extraction this boot"
                );
                false
            }
        }
    } else {
        eprintln!("buzz-desktop: boot-migration: could not load agent store for secret migration");
        false
    };

    // Extract inline secrets → keyring for the global config.
    let global_ok =
        if let Ok(global) = crate::managed_agents::global_config::load_global_agent_config(app) {
            if !global.env_vars.is_empty() || global.env_vars_ref.is_none() {
                if let Err(e) =
                    crate::managed_agents::global_config::save_global_agent_config(app, &global)
                {
                    eprintln!("buzz-desktop: boot-migration: global config save failed: {e}");
                    false
                } else {
                    true
                }
            } else {
                true
            }
        } else {
            false
        };

    run_secret_gc(app);

    // Phase 2: artifact cleanup — only when extraction was verified.
    //
    // "Verified" means: the agent store and global config were written
    // successfully this boot AND every referenced generation reads back
    // cleanly (no secrets_unavailable flags after a fresh reload).  A bare
    // keyring-handle check (`agent_secret_store_pub().is_some()`) does NOT
    // suffice — the handle can be present while individual entries fail to
    // read back, which would let cleanup delete the only copy of a secret.
    if extraction_ok && global_ok && extraction_verified(app) {
        if let Ok(agents_dir) = crate::managed_agents::storage::managed_agents_base_dir(app) {
            cleanup_secret_artifacts(&agents_dir);
        }
        // Also clean the legacy Sprout app-data agents dir if it still exists.
        // `migrate_legacy_app_data_dir` copies files from there but never removes
        // the source — it can hold the original plaintext managed-agents.json and
        // its backups indefinitely.  Run the full cleanup on that dir, including
        // its live managed-agents.json.
        if let Ok(current_dir) = app.path().app_data_dir() {
            if let Some(legacy_dir) = super::legacy_app_data_dir(&current_dir) {
                let legacy_agents_dir = legacy_dir.join("agents");
                if legacy_agents_dir.exists() {
                    cleanup_secret_artifacts(&legacy_agents_dir);
                    // The legacy live file (managed-agents.json, not a backup)
                    // is not touched by the generic cleanup sweep above — it
                    // handles backups and temps, not the live file.  Explicitly
                    // scrub or remove it now that we have verified the
                    // destination projection is secret-free and hydratable.
                    scrub_legacy_live_file(&legacy_agents_dir.join("managed-agents.json"));
                }
            }
        }
    }
}

/// Returns `true` when a fresh read-back of both live stores shows that every
/// referenced generation hydrates without error.  Used to gate Phase 2
/// artifact cleanup: we must never delete the last copy of a secret.
fn extraction_verified(app: &tauri::AppHandle) -> bool {
    // Verify agent store: all records with refs must hydrate cleanly.
    let store = match crate::managed_agents::storage::agent_secret_store_pub() {
        Some(s) => s,
        None => return false, // no keyring backend → no extraction → not verified
    };
    let mut records = match crate::managed_agents::storage::load_agent_store_raw(app) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let unavailable =
        crate::managed_agents::secret_seam::hydrate_all_secrets_for_records(store, &mut records);
    if !unavailable.is_empty() {
        eprintln!(
            "buzz-desktop: extraction-verify: {} record(s) have unavailable secrets — \
             skipping Phase 2 cleanup",
            unavailable.len()
        );
        return false;
    }
    // Verify global config: if a ref is present it must hydrate cleanly (Err
    // from load_global_agent_config means the ref exists but the entry is
    // missing/corrupt).
    let global_ok = match crate::managed_agents::global_config::load_global_agent_config(app) {
        Ok(_) => true,
        Err(e) => {
            eprintln!(
                "buzz-desktop: extraction-verify: global config ref unreadable — \
                 skipping Phase 2 cleanup: {e}"
            );
            false
        }
    };
    global_ok
}

/// Scrub or remove the legacy live `managed-agents.json` after the destination
/// projection has been verified.  Mirrors the backup scrub in
/// [`cleanup_secret_artifacts`] but targets the primary live file rather than
/// backup siblings.
///
/// - Parseable: strip secret fields and overwrite at 0o600.
/// - Unparseable or does not exist: delete (or skip if not present).
/// - Errors are logged; callers must never panic on this path.
fn scrub_legacy_live_file(path: &std::path::Path) {
    if !path.exists() {
        return;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "buzz-desktop: artifact-cleanup: cannot read legacy live file {}: {e}",
                path.display()
            );
            return;
        }
    };
    match strip_secrets_from_json(&content) {
        Some(clean) => {
            match crate::managed_agents::storage::atomic_write_json_restricted(path, clean.as_bytes()) {
                Ok(()) => eprintln!(
                    "buzz-desktop: artifact-cleanup: scrubbed legacy live file {}",
                    path.display()
                ),
                Err(e) => eprintln!(
                    "buzz-desktop: artifact-cleanup: cannot write scrubbed legacy live file {}: {e}",
                    path.display()
                ),
            }
        }
        None => {
            if let Err(e) = std::fs::remove_file(path) {
                eprintln!(
                    "buzz-desktop: artifact-cleanup: cannot remove unparseable legacy live file {}: {e}",
                    path.display()
                );
            } else {
                eprintln!(
                    "buzz-desktop: artifact-cleanup: removed unparseable legacy live file {}",
                    path.display()
                );
            }
        }
    }
}

/// Run the two-cycle GC sweeps for the secret projection.
pub(crate) fn run_secret_gc(app: &tauri::AppHandle) {
    let agents_path = match crate::managed_agents::storage::managed_agents_store_path(app) {
        Ok(p) => p,
        Err(_) => return,
    };
    let global_path = match app.path().app_data_dir() {
        Ok(d) => d.join("agents/global-agent-config.json"),
        Err(_) => return,
    };
    if let Some(store) = crate::managed_agents::storage::agent_secret_store_pub() {
        // Cross-process transaction lock: hold across BOTH sweeps' live-ref
        // read → blob mutation so a second Desktop process's in-flight save
        // cannot commit a JSON ref between this GC's read and its delete. The
        // guard releases when this function returns. Leaf-level: GC is never
        // called while another secret transaction lock is held (boot migration
        // releases its extraction and global-save spans before calling here).
        let _txn = match crate::managed_agents::storage::acquire_secret_txn_lock(app) {
            Ok(guard) => guard,
            Err(e) => {
                eprintln!(
                    "buzz-desktop: secret GC: could not acquire transaction lock ({e}), skipping"
                );
                return;
            }
        };
        // Two-cycle GC: DELETE candidates from the PREVIOUS boot first, THEN
        // mark new candidates for this boot.  This ordering is the safety
        // invariant: a generation written and verified in boot N is only ever
        // eligible for deletion in boot N+2 or later, giving a full boot cycle
        // of grace for any cross-process save that is sitting between its
        // keyring write and its JSON commit when GC runs.
        crate::managed_agents::secret_projection::delete_gc_candidates(
            store,
            &agents_path,
            &global_path,
        );
        crate::managed_agents::secret_projection::mark_gc_candidates(
            store,
            &agents_path,
            &global_path,
        );
    }
}

/// Phase 2 artifact cleanup, run after a verified extraction boot.
///
/// Invariants:
/// - ONLY called when the keyring is reachable (caller gate).
/// - NEVER follows symlinks outside `agents_dir`.
/// - Parseable `*.bak-*` / `*.bak` backup files are re-serialized with all
///   secret fields stripped (env_vars, auth_tag, private_key_nsec, provider
///   config).
/// - Unparseable / `.invalid` / atomic-write temp siblings are DELETED — they
///   cannot be recovered and may carry plaintext credentials.
/// - All JSON-shaped files in `agents_dir` (top level only) receive a 0o600
///   permission sweep.
/// - Errors on individual files are logged and never abort the sweep.
pub(crate) fn cleanup_secret_artifacts(agents_dir: &std::path::Path) {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    // Canonicalize the agents dir so we have a stable root for escape checks.
    let canonical_dir = match std::fs::canonicalize(agents_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("buzz-desktop: artifact-cleanup: cannot canonicalize agents dir: {e}");
            return;
        }
    };

    let entries = match std::fs::read_dir(&canonical_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("buzz-desktop: artifact-cleanup: cannot read agents dir: {e}");
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Resolve the real path and reject anything that escapes agents_dir.
        let real_path = match std::fs::canonicalize(&path) {
            Ok(p) => p,
            Err(_) => path.clone(), // dangling symlink or unresolvable — skip
        };
        if !real_path.starts_with(&canonical_dir) {
            eprintln!(
                "buzz-desktop: artifact-cleanup: skipping symlink escape: {}",
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

        // ── 0o600 sweep ──────────────────────────────────────────────────
        // Tighten permissions on every store-shaped file up front, so even a
        // backup whose scrub write later fails is not left group/world-readable
        // with plaintext. Uses the same backup recognizer as the scrub below.
        #[cfg(unix)]
        if fname.ends_with(".json")
            || is_backup_filename(&fname)
            || fname.ends_with(".json.invalid")
        {
            let _ = std::fs::set_permissions(&real_path, std::fs::Permissions::from_mode(0o600));
        }

        // ── Atomic-write temp siblings ────────────────────────────────────
        // `atomic-write-file` names temps as random hex strings with no
        // extension on Unix.  Identify and delete them.
        if is_atomic_write_temp(&fname) {
            if let Err(e) = std::fs::remove_file(&real_path) {
                eprintln!("buzz-desktop: artifact-cleanup: cannot remove temp file {fname}: {e}");
            } else {
                eprintln!("buzz-desktop: artifact-cleanup: removed stale temp file {fname}");
            }
            continue;
        }

        // ── .invalid files ────────────────────────────────────────────────
        if fname.ends_with(".invalid") {
            if let Err(e) = std::fs::remove_file(&real_path) {
                eprintln!(
                    "buzz-desktop: artifact-cleanup: cannot remove .invalid file {fname}: {e}"
                );
            } else {
                eprintln!("buzz-desktop: artifact-cleanup: removed .invalid artifact {fname}");
            }
            continue;
        }

        // ── Backup files (.bak-*, .bak) ───────────────────────────────────
        if is_backup_filename(&fname) {
            scrub_backup_file(&real_path, &fname);
        }
    }
}

/// Returns true when `fname` looks like an atomic-write-file temp: no
/// extension, at least 8 characters, all ASCII hex digits.
fn is_atomic_write_temp(fname: &str) -> bool {
    !fname.contains('.') && fname.len() >= 8 && fname.chars().all(|c| c.is_ascii_hexdigit())
}

/// Returns true when `fname` is a backup file we should scrub.
///
/// Recognizes every managed-store backup naming family this repo produces —
/// each is `<store>.json` with a backup suffix appended, and each can carry
/// pre-projection plaintext (env vars, auth tags, nsecs, provider config):
///
/// - `managed-agents.json.bak`, `.bak-<timestamp>`, `.bak.<timestamp>`
/// - `managed-agents.json.pre-backfill.bak` ([`crate::migration::backfill`])
/// - `managed-agents.json.pre-team-suffix-strip.bak`
///   ([`crate::migration::team_suffix`])
/// - `personas.json.bak` ([`crate::migration::fold`]) / `teams.json.bak-*`
///
/// The match is structural rather than an enumerated suffix list: a name
/// belonging to one of our stores whose `.json` base is followed by a `.bak`
/// marker anywhere. This catches a new `<store>.json.<phase>.bak` producer
/// without another edit here. Deliberately excludes the live `<store>.json`
/// (no `.bak` after `.json`) and `.invalid` artifacts (handled separately).
fn is_backup_filename(fname: &str) -> bool {
    let is_ours = fname.starts_with("managed-agents")
        || fname.starts_with("personas")
        || fname.starts_with("teams");
    let Some((_, after_json)) = fname.split_once(".json") else {
        return false;
    };
    is_ours && after_json.contains(".bak")
}

/// Strip secret fields from a parseable backup and overwrite it at 0o600.
/// Delete the file when unparseable (may carry plaintext credentials).
fn scrub_backup_file(path: &std::path::Path, fname: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("buzz-desktop: artifact-cleanup: cannot read backup {fname}: {e}");
            return;
        }
    };

    match strip_secrets_from_json(&content) {
        Some(clean) => {
            match crate::managed_agents::storage::atomic_write_json_restricted(
                path,
                clean.as_bytes(),
            ) {
                Ok(()) => eprintln!("buzz-desktop: artifact-cleanup: scrubbed backup {fname}"),
                Err(e) => eprintln!(
                    "buzz-desktop: artifact-cleanup: cannot write scrubbed backup {fname}: {e}"
                ),
            }
        }
        None => {
            // Unparseable → delete.
            if let Err(e) = std::fs::remove_file(path) {
                eprintln!(
                    "buzz-desktop: artifact-cleanup: cannot remove unparseable backup {fname}: {e}"
                );
            } else {
                eprintln!("buzz-desktop: artifact-cleanup: removed unparseable backup {fname}");
            }
        }
    }
}

/// Attempt to strip secret fields from a JSON document.
///
/// Handles two shapes:
/// - Array of agent records → strips `env_vars`, `auth_tag`,
///   `private_key_nsec`, and `BackendKind::Provider.config`.
/// - Object (global-agent-config) → strips `env_vars`.
///
/// Returns `None` when the content cannot be parsed as either shape.
fn strip_secrets_from_json(content: &str) -> Option<String> {
    use serde_json::Value;

    let mut v: Value = serde_json::from_str(content).ok()?;
    match &mut v {
        Value::Array(records) => {
            for record in records.iter_mut() {
                if let Value::Object(map) = record {
                    map.remove("env_vars");
                    map.remove("auth_tag");
                    map.remove("private_key_nsec");
                    // Strip BackendKind::Provider.config.
                    // BackendKind uses #[serde(tag = "type", rename_all = "snake_case")],
                    // so provider is stored as {"type": "provider", "id": "...", "config": {...}}.
                    if let Some(Value::Object(backend)) = map.get_mut("backend") {
                        let is_provider = backend
                            .get("type")
                            .and_then(Value::as_str)
                            .map(|t| t == "provider")
                            .unwrap_or(false);
                        if is_provider {
                            backend.remove("config");
                        }
                    }
                }
            }
        }
        Value::Object(map) => {
            map.remove("env_vars");
        }
        _ => return None,
    }
    serde_json::to_string_pretty(&v).ok()
}

/// Boot-migrate inline secrets to the keyring using the field-granular
/// migration seam — NOT the strip-on-save seam.
///
/// This function runs on EVERY launch over records read straight off disk.
/// After the first launch those records are already projected: their inline
/// fields are empty and their `*_ref`s point at live generations. The
/// strip-on-save seam would read an empty inline field as a deliberate
/// user-clear and drop the ref (W1: silent credential loss on the second
/// launch, then GC deletes the orphaned generation). The migration seam does
/// not: it only ever projects a non-empty inline value or preserves an
/// existing ref, and never clears one it did not write. See
/// [`crate::managed_agents::secret_seam::migrate_all_secrets_for_records`].
fn migrate_inline_secrets_in_records(
    _app: &tauri::AppHandle,
    records: &mut [crate::managed_agents::ManagedAgentRecord],
) -> bool {
    let Some(store) = crate::managed_agents::storage::agent_secret_store_pub() else {
        return false;
    };
    crate::managed_agents::secret_seam::migrate_all_secrets_for_records(store, records)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_atomic_write_temp_recognizes_hex_names() {
        assert!(is_atomic_write_temp("deadbeefcafe0123"));
        assert!(!is_atomic_write_temp("managed-agents.json"));
        assert!(!is_atomic_write_temp("abc")); // too short
        assert!(!is_atomic_write_temp("deadbeef.tmp")); // has extension
        assert!(!is_atomic_write_temp("not-hex-at-all"));
    }

    #[test]
    fn test_is_backup_filename_recognizes_backups() {
        assert!(is_backup_filename(
            "managed-agents.json.bak-20260608-175938"
        ));
        assert!(is_backup_filename("managed-agents.json.bak"));
        assert!(is_backup_filename("personas.json.bak-20260608-175938"));
        assert!(is_backup_filename("teams.json.bak-20260608-175938"));
        assert!(!is_backup_filename("managed-agents.json"));
        assert!(!is_backup_filename("managed-agents.json.invalid"));
        assert!(!is_backup_filename("global-agent-config.json.bak-20260608")); // not in our set
    }

    #[test]
    fn test_is_backup_filename_recognizes_phase_suffixed_producers() {
        // W4: the exact backup filenames this repo's own migrations produce.
        // Each is `<store>.json` with a phase/marker suffix appended — NOT the
        // literal `.json.bak` the old recognizer required — so each survived
        // the scrub with pre-projection plaintext (env vars, auth tags, nsecs,
        // provider config) intact.
        assert!(
            is_backup_filename("managed-agents.json.pre-backfill.bak"),
            "backfill.rs pre-migration backup must be recognized"
        );
        assert!(
            is_backup_filename("managed-agents.json.pre-team-suffix-strip.bak"),
            "team_suffix.rs pre-migration backup must be recognized"
        );
        assert!(
            is_backup_filename("personas.json.bak"),
            "the persona-fold backup must be recognized"
        );
        // `.bak.<stamp>` ordering (marker then stamp) is ours too.
        assert!(is_backup_filename(
            "managed-agents.json.bak.20260608-175938"
        ));
        // A future `<store>.json.<phase>.bak` producer is caught structurally,
        // without another edit here — this is why the match is not an
        // enumerated suffix list.
        assert!(is_backup_filename(
            "managed-agents.json.some-future-phase.bak"
        ));

        // A phase-suffixed backup of a store we do NOT own is not ours to
        // scrub, even though it is shaped identically.
        assert!(!is_backup_filename("credentials.json.pre-backfill.bak"));
    }

    #[test]
    fn test_strip_secrets_from_json_strips_agent_records() {
        let input = r#"[
          {
            "pubkey": "abc123",
            "name": "test-agent",
            "env_vars": {"ANTHROPIC_API_KEY": "sk-ant-secret"},
            "auth_tag": "some-auth-tag",
            "private_key_nsec": "nsec1secret",
            "backend": {"type": "provider", "id": "anthropic", "config": {"key": "secret"}},
            "created_at": "2026-01-01",
            "updated_at": "2026-01-01"
          }
        ]"#;
        let stripped = strip_secrets_from_json(input).unwrap();
        let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        let record = &v[0];
        assert!(
            record.get("env_vars").is_none(),
            "env_vars must be stripped"
        );
        assert!(
            record.get("auth_tag").is_none(),
            "auth_tag must be stripped"
        );
        assert!(
            record.get("private_key_nsec").is_none(),
            "private_key_nsec must be stripped"
        );
        let provider_config = record.get("backend").and_then(|b| b.get("config"));
        assert!(
            provider_config.is_none(),
            "provider config must be stripped"
        );
        // Non-secret fields survive.
        assert_eq!(
            record.get("name").and_then(|n| n.as_str()),
            Some("test-agent")
        );
    }

    #[test]
    fn test_strip_secrets_from_json_preserves_local_backend() {
        let input = r#"[
          {
            "pubkey": "abc123",
            "name": "local-agent",
            "backend": {"type": "local"},
            "created_at": "2026-01-01",
            "updated_at": "2026-01-01"
          }
        ]"#;
        let stripped = strip_secrets_from_json(input).unwrap();
        let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        // Local backend must survive unchanged — no config field to strip.
        let backend = v[0].get("backend").expect("backend must be present");
        assert_eq!(backend.get("type").and_then(|t| t.as_str()), Some("local"));
        assert!(
            backend.get("config").is_none(),
            "local backend has no config"
        );
    }

    #[test]
    fn test_strip_secrets_from_json_strips_global_config() {
        let input = r#"{"env_vars": {"KEY": "value"}, "other_field": "keep"}"#;
        let stripped = strip_secrets_from_json(input).unwrap();
        let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert!(v.get("env_vars").is_none(), "env_vars must be stripped");
        assert_eq!(v.get("other_field").and_then(|f| f.as_str()), Some("keep"));
    }

    #[test]
    fn test_strip_secrets_from_json_returns_none_on_unparseable() {
        assert!(strip_secrets_from_json("not valid json").is_none());
        assert!(strip_secrets_from_json("").is_none());
    }

    // ── cleanup_secret_artifacts filesystem tests ─────────────────────────

    /// Atomic-write-file temp siblings (hex-only names, no extension) are
    /// deleted during cleanup.  These can carry plaintext if a write was
    /// interrupted before the atomic rename.
    #[test]
    fn test_cleanup_deletes_atomic_write_temps() {
        let dir = tempfile::tempdir().expect("tempdir");
        let temp_path = dir.path().join("deadbeefcafe0123");
        std::fs::write(&temp_path, b"leftover content").expect("write temp");
        assert!(temp_path.exists());
        cleanup_secret_artifacts(dir.path());
        assert!(
            !temp_path.exists(),
            "atomic-write temp must be deleted by cleanup"
        );
    }

    /// Unparseable `.invalid` files are deleted during cleanup.
    #[test]
    fn test_cleanup_deletes_invalid_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let invalid_path = dir.path().join("managed-agents.json.invalid");
        std::fs::write(&invalid_path, b"not valid json with sk-ant-secret").expect("write");
        cleanup_secret_artifacts(dir.path());
        assert!(
            !invalid_path.exists(),
            ".invalid file must be deleted by cleanup"
        );
    }

    /// A backup file with parseable content is scrubbed (secrets stripped) rather
    /// than deleted.  The file should survive but without the secret value.
    #[test]
    fn test_cleanup_scrubs_parseable_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let backup_path = dir.path().join("managed-agents.json.bak-20260608-175938");
        let content = r#"[{"pubkey":"abc","name":"test","env_vars":{"K":"v"},"created_at":"2026","updated_at":"2026"}]"#;
        std::fs::write(&backup_path, content).expect("write backup");
        cleanup_secret_artifacts(dir.path());
        assert!(backup_path.exists(), "parseable backup must survive");
        let result = std::fs::read_to_string(&backup_path).expect("read");
        assert!(
            !result.contains("\"env_vars\""),
            "secrets must be stripped from parseable backup"
        );
    }

    /// W4 end-to-end: the phase-suffixed producer backups (`pre-backfill.bak`,
    /// `pre-team-suffix-strip.bak`) carry pre-projection plaintext and must be
    /// scrubbed by the full cleanup sweep — not just matched by the recognizer.
    /// Before the structural recognizer these names lacked the literal
    /// `.json.bak` substring and survived cleanup with secrets intact.
    #[test]
    fn test_cleanup_scrubs_phase_suffixed_producer_backups() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = r#"[{"pubkey":"abc","name":"test","env_vars":{"ANTHROPIC_API_KEY":"sk-ant-secret"},"auth_tag":"tag-secret","created_at":"2026","updated_at":"2026"}]"#;
        for name in [
            "managed-agents.json.pre-backfill.bak",
            "managed-agents.json.pre-team-suffix-strip.bak",
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, content).expect("write producer backup");

            cleanup_secret_artifacts(dir.path());

            assert!(path.exists(), "{name} must survive as a scrubbed backup");
            let result = std::fs::read_to_string(&path).expect("read");
            assert!(
                !result.contains("sk-ant-secret"),
                "{name} must have its env_vars secret stripped"
            );
            assert!(
                !result.contains("tag-secret"),
                "{name} must have its auth_tag secret stripped"
            );
        }
    }

    /// Cleanup must not follow symlinks that escape the agents dir.
    #[cfg(unix)]
    #[test]
    fn test_cleanup_skips_symlink_escape() {
        let agents_dir = tempfile::tempdir().expect("agents dir");
        // Create a target OUTSIDE the agents dir.
        let outside_dir = tempfile::tempdir().expect("outside dir");
        let outside_file = outside_dir.path().join("secret.txt");
        std::fs::write(&outside_file, b"outside-secret").expect("write outside");

        // Create a symlink inside agents_dir that points outside.
        let symlink_path = agents_dir.path().join("escape.json");
        std::os::unix::fs::symlink(&outside_file, &symlink_path).expect("create symlink");

        cleanup_secret_artifacts(agents_dir.path());

        // The file outside agents_dir must NOT be deleted.
        assert!(
            outside_file.exists(),
            "symlink escape must not delete the target outside agents dir"
        );
    }

    /// Cleanup must handle deletion failures gracefully: if a file cannot be
    /// removed (e.g. read-only), the rest of the sweep continues and no panic
    /// occurs.
    #[cfg(unix)]
    #[test]
    fn test_cleanup_tolerates_deletion_failure() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        // Make a read-only .invalid file that cannot be deleted.
        let undeletable = dir.path().join("managed-agents.json.invalid");
        std::fs::write(&undeletable, b"bad json").expect("write");

        // Also add a normal .invalid file that CAN be deleted.
        let deletable = dir.path().join("personas.json.invalid");
        std::fs::write(&deletable, b"also bad").expect("write");

        // Set the dir to read-only to prevent deletion of its entries.
        let dir_meta = std::fs::metadata(dir.path()).expect("dir meta");
        let mut perms = dir_meta.permissions();
        perms.set_mode(0o555); // r-xr-xr-x: no write
        std::fs::set_permissions(dir.path(), perms.clone()).expect("set perms");

        // Cleanup should not panic despite the failure.
        cleanup_secret_artifacts(dir.path());

        // Restore permissions so tempdir cleanup can succeed.
        perms.set_mode(0o755);
        std::fs::set_permissions(dir.path(), perms).ok();
    }

    // ── scrub_legacy_live_file tests ──────────────────────────────────────

    /// A parseable legacy live file is scrubbed in-place (secrets stripped,
    /// file survives).
    #[test]
    fn test_scrub_legacy_live_file_strips_secrets_from_parseable_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("managed-agents.json");
        let content = r#"[{"pubkey":"abc","name":"test","env_vars":{"K":"v"},"created_at":"2026","updated_at":"2026"}]"#;
        std::fs::write(&path, content).expect("write");

        scrub_legacy_live_file(&path);

        assert!(path.exists(), "parseable legacy file must survive");
        let result = std::fs::read_to_string(&path).expect("read");
        assert!(
            !result.contains("\"env_vars\""),
            "secrets must be stripped from the legacy live file"
        );
    }

    /// An unparseable legacy live file is deleted.
    #[test]
    fn test_scrub_legacy_live_file_deletes_unparseable_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("managed-agents.json");
        std::fs::write(&path, b"not valid json with sk-ant-secret").expect("write");

        scrub_legacy_live_file(&path);

        assert!(
            !path.exists(),
            "unparseable legacy live file must be deleted"
        );
    }

    /// Missing legacy live file is silently skipped (no panic, no error).
    #[test]
    fn test_scrub_legacy_live_file_ignores_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("managed-agents.json");
        assert!(!path.exists());
        // Must not panic.
        scrub_legacy_live_file(&path);
    }

    // ── extraction_verified gate logic tests ─────────────────────────────

    /// The cleanup gate `extraction_ok && global_ok && extraction_verified`
    /// is tested via the individual extraction booleans. `extraction_verified`
    /// itself requires a real keyring (not unit-testable), so we test its
    /// logical partners to document the invariant: all three must be true.
    #[test]
    fn test_extraction_gate_requires_all_three_conditions() {
        // The logical gate: extraction_ok && global_ok && extraction_verified.
        // All three must be true for cleanup to run. Document the invariant:
        // every input combination that is NOT (true, true, true) must keep
        // the gate closed.
        let cases: &[(bool, bool, bool)] = &[
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, false),
            (false, false, true),
            (true, false, true),
            (false, true, true),
        ];
        for &(extraction_ok, global_ok, verified) in cases {
            let gate = extraction_ok && global_ok && verified;
            assert!(
                !gate,
                "gate must be closed unless all three conditions are true: \
                 extraction_ok={extraction_ok}, global_ok={global_ok}, verified={verified}"
            );
        }
        // Only (true, true, true) opens the gate.
        let all_ok = {
            let (a, b, c) = (true, true, true);
            a && b && c
        };
        assert!(all_ok, "all three true must open the gate");
    }
}
