//! Unit tests for `managed_agents/scope_init.rs`.
//!
//! Kept in a sibling file so `scope_init.rs` stays under the
//! 1000-line size gate; `#[path]`-included from there.

use super::*;
use tempfile::TempDir;

/// Returns `(TempDir, base_dir)` where `base_dir = tmp.path().join("agents")`.
///
/// Production: `managed_agents_base_dir` returns `<app-data>/agents`.
/// Tests must use that same layout so `legacy_definitions_exist`,
/// `install_staged`, and `read_or_create_canonical_claim` all see files
/// at the correct level.
fn make_base_dir_pair() -> (TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&base_dir).unwrap();
    (tmp, base_dir)
}

/// Write the legacy definition files directly into `base_dir`
/// (i.e. `<app-data>/agents/managed-agents.json` etc.).
fn make_legacy_files(base_dir: &Path) {
    std::fs::create_dir_all(base_dir).unwrap();
    std::fs::write(base_dir.join("managed-agents.json"), b"[]").unwrap();
    std::fs::write(base_dir.join("teams.json"), b"[]").unwrap();
}

#[test]
fn test_fresh_no_legacy_scope_initializes_ready() {
    let (_tmp, base_dir) = make_base_dir_pair();
    let scope_dir = base_dir.join("scopes").join("testscope");
    ensure_scope_ready("testscope", &scope_dir, &base_dir, "test_owner").unwrap();
    assert!(scope_is_ready(&scope_dir), "scope should be Ready");
    // Manifest should indicate FreshNoLegacy.
    let manifest: ScopeManifest =
        serde_json::from_slice(&std::fs::read(scope_dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert!(matches!(manifest.init_kind, ScopeInitKind::FreshNoLegacy));
}

#[test]
fn test_adopted_legacy_scope_copies_files() {
    let (_tmp, base_dir) = make_base_dir_pair();
    make_legacy_files(&base_dir);
    let scope_id = "firstscope";
    let scope_dir = base_dir.join("scopes").join(scope_id);
    ensure_scope_ready(scope_id, &scope_dir, &base_dir, "test_owner").unwrap();
    assert!(scope_is_ready(&scope_dir));
    assert!(
        scope_dir.join("managed-agents.json").exists(),
        "legacy managed-agents.json should be copied"
    );
    let manifest: ScopeManifest =
        serde_json::from_slice(&std::fs::read(scope_dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert!(matches!(manifest.init_kind, ScopeInitKind::AdoptedLegacy));
}

#[test]
fn test_second_scope_legacy_claimed_by_other() {
    let (_tmp, base_dir) = make_base_dir_pair();
    make_legacy_files(&base_dir);

    // First scope claims.
    let scope_a = base_dir.join("scopes").join("scope_a");
    ensure_scope_ready("scope_a", &scope_a, &base_dir, "test_owner").unwrap();

    // Second scope should see LegacyClaimedByOther.
    let scope_b = base_dir.join("scopes").join("scope_b");
    ensure_scope_ready("scope_b", &scope_b, &base_dir, "test_owner").unwrap();
    assert!(scope_is_ready(&scope_b));
    let manifest: ScopeManifest =
        serde_json::from_slice(&std::fs::read(scope_b.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert!(
        matches!(
            manifest.init_kind,
            ScopeInitKind::LegacyClaimedByOther { .. }
        ),
        "second scope should see LegacyClaimedByOther, got {:?}",
        manifest.init_kind
    );
    assert!(
        !scope_b.join("managed-agents.json").exists(),
        "second scope should start empty"
    );
}

#[test]
fn test_idempotent_double_initialize() {
    let (_tmp, base_dir) = make_base_dir_pair();
    make_legacy_files(&base_dir);
    let scope_dir = base_dir.join("scopes").join("idempotent");
    ensure_scope_ready("idempotent", &scope_dir, &base_dir, "test_owner").unwrap();
    // Second call should be a fast no-op.
    ensure_scope_ready("idempotent", &scope_dir, &base_dir, "test_owner").unwrap();
    assert!(scope_is_ready(&scope_dir));
}

#[test]
fn test_staging_cleanup_on_retry() {
    let (_tmp, base_dir) = make_base_dir_pair();
    make_legacy_files(&base_dir);
    let scope_dir = base_dir.join("scopes").join("retry");
    let staging = staging_dir_for(&scope_dir);

    // Simulate an interrupted staging directory.
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("partial.json"), b"garbage").unwrap();

    // ensure_scope_ready should clean it up and succeed.
    ensure_scope_ready("retry", &scope_dir, &base_dir, "test_owner").unwrap();
    assert!(scope_is_ready(&scope_dir));
    assert!(!staging.exists(), "staging dir should be cleaned up");
}

#[test]
fn test_retention_db_claim_takes_precedence() {
    let (_tmp, base_dir) = make_base_dir_pair();
    make_legacy_files(&base_dir);

    // Pre-plant a retention.db with scope_a's claim.
    // retention.db lives at `base_dir/retention.db` (i.e. `<app-data>/agents/retention.db`).
    let retention_db_path = base_dir.join("retention.db");
    let conn = Connection::open(&retention_db_path).unwrap();
    ensure_migration_table(&conn).unwrap();
    conn.execute(
        "INSERT INTO retention_migrations (name, scope_id) VALUES (?1, ?2)",
        params![DEFINITIONS_MIGRATION_NAME, "scope_a"],
    )
    .unwrap();
    drop(conn);

    // scope_b activates first — retention.db says scope_a owns legacy.
    let scope_b = base_dir.join("scopes").join("scope_b");
    ensure_scope_ready("scope_b", &scope_b, &base_dir, "test_owner").unwrap();
    let manifest: ScopeManifest =
        serde_json::from_slice(&std::fs::read(scope_b.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert!(
        matches!(
            manifest.init_kind,
            ScopeInitKind::LegacyClaimedByOther { ref claiming_scope_id }
            if claiming_scope_id == "scope_a"
        ),
        "retention.db claim should win, got {:?}",
        manifest.init_kind
    );

    // scope_a now activates — should adopt legacy.
    let scope_a = base_dir.join("scopes").join("scope_a");
    ensure_scope_ready("scope_a", &scope_a, &base_dir, "test_owner").unwrap();
    let manifest_a: ScopeManifest =
        serde_json::from_slice(&std::fs::read(scope_a.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert!(matches!(manifest_a.init_kind, ScopeInitKind::AdoptedLegacy));
    assert!(scope_a.join("managed-agents.json").exists());
}

/// Crash boundary: claim written, then crash before any file is copied into
/// staging. On retry the staging directory does not exist, so the full
/// staged install runs again. The same scope wins the claim (INSERT OR
/// IGNORE is idempotent) and legacy files are copied correctly.
#[test]
fn test_crash_after_claim_before_staging_resumes_correctly() {
    let (_tmp, base_dir) = make_base_dir_pair();
    make_legacy_files(&base_dir);

    // Simulate: claim was written into the fallback file but no staging dir exists yet.
    // The fallback claim file lives at `base_dir/legacy-claim.json`
    // (no extra "agents" join — base_dir is already `<app-data>/agents`).
    let claim_path = base_dir.join(FALLBACK_CLAIM_FILE);
    let claim = serde_json::json!({"scope_id": "scope_a"});
    std::fs::write(&claim_path, serde_json::to_vec(&claim).unwrap()).unwrap();

    // No staging dir exists — retry runs the full staged install from the claim.
    let scope_a = base_dir.join("scopes").join("scope_a");
    ensure_scope_ready("scope_a", &scope_a, &base_dir, "test_owner").unwrap();

    assert!(scope_is_ready(&scope_a), "scope must be Ready after retry");
    let manifest: ScopeManifest =
        serde_json::from_slice(&std::fs::read(scope_a.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert!(
        matches!(manifest.init_kind, ScopeInitKind::AdoptedLegacy),
        "scope_a owns the claim and must adopt legacy, got {:?}",
        manifest.init_kind
    );
    assert!(
        scope_a.join("managed-agents.json").exists(),
        "legacy files must be copied after retry"
    );
}

/// Crash boundary: staging directory exists (copy was in progress) but the
/// atomic rename never happened. On retry the stale staging dir is cleaned
/// and the full staged install runs again. The retry must not overwrite any
/// post-crash writes that might have landed in the target (the target
/// doesn't exist yet since rename never fired, so there's nothing to
/// overwrite — staging is the only artifact).
#[test]
fn test_crash_during_staging_copy_is_cleaned_on_retry() {
    let (_tmp, base_dir) = make_base_dir_pair();
    make_legacy_files(&base_dir);

    let scope_dir = base_dir.join("scopes").join("scope_retry");
    let staging = staging_dir_for(&scope_dir);

    // Simulate interrupted staging: directory exists with partial content.
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("managed-agents.json"), b"[\"partial\"]").unwrap();
    // No manifest inside staging (write didn't complete).

    ensure_scope_ready("scope_retry", &scope_dir, &base_dir, "test_owner").unwrap();

    assert!(scope_is_ready(&scope_dir));
    assert!(
        !staging.exists(),
        "stale staging dir must be cleaned up on retry"
    );
    // The final managed-agents.json is from the legacy source, not the partial.
    let content = std::fs::read(scope_dir.join("managed-agents.json")).unwrap();
    assert_eq!(
        content, b"[]",
        "managed-agents.json must be from the legacy source after retry"
    );
}

/// Crash boundary: staging complete (manifest written) but rename never
/// happened. Detected by: staging dir exists. On retry, clean staging and
/// re-run; the claim is idempotent so the same scope adopts legacy again.
#[test]
fn test_crash_after_staging_manifest_before_rename_resumes_correctly() {
    let (_tmp, base_dir) = make_base_dir_pair();
    make_legacy_files(&base_dir);

    let scope_dir = base_dir.join("scopes").join("scope_rename");
    let staging = staging_dir_for(&scope_dir);

    // Simulate: staging complete with manifest, but rename never fired.
    std::fs::create_dir_all(&staging).unwrap();
    let manifest = ScopeManifest {
        scope_id: "scope_rename".into(),
        init_kind: ScopeInitKind::AdoptedLegacy,
    };
    std::fs::write(
        staging.join(MANIFEST_FILE),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(staging.join("managed-agents.json"), b"[]").unwrap();
    // Scope dir itself does not exist (rename didn't fire).
    assert!(!scope_dir.exists());

    ensure_scope_ready("scope_rename", &scope_dir, &base_dir, "test_owner").unwrap();

    assert!(scope_is_ready(&scope_dir));
    assert!(!staging.exists(), "staging must be cleaned after retry");
    assert!(
        scope_dir.join("managed-agents.json").exists(),
        "adopted file must be present after retry"
    );
}

/// Crash boundary: atomic rename happened (target dir exists with manifest
/// and legacy files) but the `_ready` marker was never written (migrations
/// didn't complete). On next activation, `ensure_scope_ready` must resume
/// migrations and write the ready marker without re-copying files.
///
/// The post-crash content must be valid JSON so `run_scoped_migrations`
/// step 10 (JSON validation gate) passes and `_ready` is written. The test
/// verifies that the content is NOT overwritten (no re-staging), which is
/// the behaviour that matters: a legitimate post-crash inbound write
/// must survive a resume. Content corruption is a separate failure mode
/// outside the scope of the crash-resume path.
#[test]
fn test_crash_after_rename_before_ready_resumes_migrations() {
    let (_tmp, base_dir) = make_base_dir_pair();
    make_legacy_files(&base_dir);

    let scope_dir = base_dir.join("scopes").join("scope_pre_ready");

    // Simulate: rename already happened — target has manifest + files but no
    // _ready marker. Content is valid JSON so migrations can complete.
    std::fs::create_dir_all(&scope_dir).unwrap();
    let manifest = ScopeManifest {
        scope_id: "scope_pre_ready".into(),
        init_kind: ScopeInitKind::AdoptedLegacy,
    };
    std::fs::write(
        scope_dir.join(MANIFEST_FILE),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    // Valid JSON array — simulates content written before the crash.
    std::fs::write(scope_dir.join("managed-agents.json"), b"[]").unwrap();
    // No _ready marker.
    assert!(!scope_is_ready(&scope_dir));

    ensure_scope_ready("scope_pre_ready", &scope_dir, &base_dir, "test_owner").unwrap();

    assert!(
        scope_is_ready(&scope_dir),
        "ready marker must be written on retry"
    );
    // Post-crash writes in the target must not be overwritten (no staging).
    let content = std::fs::read(scope_dir.join("managed-agents.json")).unwrap();
    assert_eq!(
        content, b"[]",
        "post-crash target content must not be overwritten on retry"
    );
}

/// C2: migration failure must NOT write the `_ready` marker.
///
/// If `fold_personas_in_dir` fails (e.g. due to a corrupt personas.json)
/// `run_scoped_migrations` returns `Err` and `ensure_scope_ready` must
/// propagate that error without writing `_ready`. On a subsequent call with
/// the defect corrected, migrations complete and `_ready` IS written.
#[test]
fn test_migration_failure_withholds_ready_and_retry_succeeds() {
    let (_tmp, base_dir) = make_base_dir_pair();

    // Create the legacy directory with a CORRUPT personas.json.
    std::fs::create_dir_all(&base_dir).unwrap();
    std::fs::write(base_dir.join("managed-agents.json"), b"[]").unwrap();
    // Corrupt personas.json: `fold_personas_in_dir` tries to parse it and
    // returns Err, which run_scoped_migrations propagates.
    std::fs::write(base_dir.join("personas.json"), b"not valid json").unwrap();

    let scope_id = "scope_fail_retry";
    let scope_dir = base_dir.join("scopes").join(scope_id);

    // First attempt: migrations fail, _ready must NOT be written.
    let result = ensure_scope_ready(scope_id, &scope_dir, &base_dir, "test_owner");
    assert!(result.is_err(), "corrupt personas.json must cause Err");
    assert!(
        !scope_is_ready(&scope_dir),
        "_ready must NOT be written when migrations fail"
    );

    // Repair the corrupt file.
    std::fs::write(base_dir.join("personas.json"), b"[]").unwrap();
    // Also repair the scope_dir since ensure_scope_ready may have left it in
    // a partial state — remove it so the state machine reruns from staging.
    if scope_dir.exists() {
        std::fs::remove_dir_all(&scope_dir).unwrap();
    }

    // Second attempt: migrations succeed, _ready IS written.
    ensure_scope_ready(scope_id, &scope_dir, &base_dir, "test_owner").unwrap();
    assert!(
        scope_is_ready(&scope_dir),
        "_ready must be written on successful retry"
    );
}

/// Versioned `_ready` upgrade: a scope whose marker was written by an older
/// pipeline (e.g. "ready" or any non-current version) must be forced through
/// `run_pre_ready_family` again and have its marker upgraded to the current
/// version. An already-current marker is a fast no-op.
#[test]
fn test_old_ready_marker_forces_pre_ready_pipeline_and_upgrades_version() {
    let (_tmp, base_dir) = make_base_dir_pair();
    let scope_id = "versioned-scope";
    let scope_dir = base_dir.join("scopes").join(scope_id);

    // Full initialization with fresh scope → marker written at current version.
    ensure_scope_ready(scope_id, &scope_dir, &base_dir, "test_owner").unwrap();
    assert!(scope_is_ready(&scope_dir), "scope must be ready after init");
    // Verify the marker actually carries the version string.
    let marker_content = std::fs::read_to_string(scope_dir.join(READY_MARKER)).unwrap();
    assert_eq!(
        marker_content.trim(),
        READY_MARKER_VERSION,
        "marker must carry current version"
    );

    // Downgrade the marker to simulate a pre-existing scope from an older build.
    std::fs::write(scope_dir.join(READY_MARKER), b"ready").unwrap();
    assert!(
        !scope_is_ready(&scope_dir),
        "old-version marker must not be considered current"
    );

    // Re-running ensure_scope_ready must upgrade the marker.
    ensure_scope_ready(scope_id, &scope_dir, &base_dir, "test_owner").unwrap();
    assert!(
        scope_is_ready(&scope_dir),
        "scope must be ready after version upgrade"
    );
    let upgraded_content = std::fs::read_to_string(scope_dir.join(READY_MARKER)).unwrap();
    assert_eq!(
        upgraded_content.trim(),
        READY_MARKER_VERSION,
        "upgraded marker must carry current version"
    );
}

/// Production-contract coverage: `base_dir` is `<app-data>/agents`
/// (the real shape from `managed_agents_base_dir`). Legacy files live
/// at `base_dir/{managed-agents,teams}.json`; the scope dir lives at
/// `base_dir/scopes/<scope_id>/`; the fallback claim file lives at
/// `base_dir/legacy-claim.json`. This test verifies the full adoption
/// path using the production layout so any future double-join regresses
/// visibly here rather than silently succeeding on a synthetic tree.
#[test]
fn test_production_shaped_adoption_finds_legacy_files() {
    // `app_data_dir` is the synthetic `<app-data>` root.
    let tmp = tempfile::tempdir().expect("tempdir");
    let app_data_dir = tmp.path();

    // Production: `managed_agents_base_dir` returns `<app-data>/agents`.
    let base_dir = app_data_dir.join("agents");
    std::fs::create_dir_all(&base_dir).unwrap();

    // Legacy files sit directly under `base_dir`.
    std::fs::write(base_dir.join("managed-agents.json"), b"[]").unwrap();
    std::fs::write(base_dir.join("teams.json"), b"[]").unwrap();

    // Scope dir is `base_dir/scopes/<scope_id>/`.
    let scope_id = "prod-shape-scope";
    let scope_dir = base_dir.join("scopes").join(scope_id);

    ensure_scope_ready(scope_id, &scope_dir, &base_dir, "test_owner").unwrap();

    assert!(scope_is_ready(&scope_dir), "scope must be Ready");

    // With the correct layout the scope must adopt legacy (not start fresh).
    let manifest: ScopeManifest =
        serde_json::from_slice(&std::fs::read(scope_dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert!(
        matches!(manifest.init_kind, ScopeInitKind::AdoptedLegacy),
        "production-layout scope must adopt legacy, got {:?}",
        manifest.init_kind
    );
    // Legacy files were copied into the scope.
    assert!(
        scope_dir.join("managed-agents.json").exists(),
        "managed-agents.json must be present in adopted scope"
    );

    // Fallback claim file lives at `base_dir/legacy-claim.json`, NOT at
    // `base_dir/agents/legacy-claim.json` (which would be the double-join
    // path). Verify the correct location was used.
    assert!(
        base_dir.join(FALLBACK_CLAIM_FILE).exists(),
        "fallback claim must be at base_dir/legacy-claim.json, not at a nested path"
    );
    assert!(
        !base_dir.join("agents").join(FALLBACK_CLAIM_FILE).exists(),
        "double-join claim path must NOT exist"
    );
}

/// Old `_ready` marker with a migration failure: the scope stays at the old
/// version until the broken file is repaired IN PLACE (no scope deletion),
/// then the retry advances the marker to the current version.
///
/// This test covers the path where `scope_dir` already has a `MANIFEST_FILE`
/// (staged install completed on a prior run) but carries an old-version
/// `_ready` marker. `ensure_scope_ready` re-runs migrations via the fast-path
/// at `scope_init.rs:158-162`.  A corrupt `personas.json` inside the scope
/// directory makes `migrate_persona_provider_to_runtime_at` return `Err`,
/// which withholds the `v1` upgrade.  After the file is repaired, a retry
/// succeeds and the marker advances — without ever deleting or recreating the
/// scope directory.
#[test]
fn test_migration_failure_withholds_ready_upgrade_repair_in_place() {
    let (_tmp, base_dir) = make_base_dir_pair();
    let scope_id = "scope-repair-in-place";
    let scope_dir = base_dir.join("scopes").join(scope_id);

    // ── Phase 1: full initialization → marker at v1 ─────────────────────────
    ensure_scope_ready(scope_id, &scope_dir, &base_dir, "test_owner").unwrap();
    assert!(
        scope_is_ready(&scope_dir),
        "scope must be v1-ready after init"
    );

    // ── Phase 2: downgrade marker → simulate a pre-existing old-version scope ─
    std::fs::write(scope_dir.join(READY_MARKER), b"ready").unwrap();
    assert!(
        !scope_is_ready(&scope_dir),
        "old-version marker must not be considered current"
    );

    // ── Phase 3: inject a migration failure ──────────────────────────────────
    // Write a corrupt personas.json into the scope_dir so
    // `migrate_persona_provider_to_runtime_at` returns Err when called on
    // the scope_dir by the fast-path at scope_init.rs:158-162.
    std::fs::write(scope_dir.join("personas.json"), b"not valid json").unwrap();

    // Re-run ensure_scope_ready — must fail because migration fails.
    let result = ensure_scope_ready(scope_id, &scope_dir, &base_dir, "test_owner");
    assert!(
        result.is_err(),
        "corrupt personas.json must cause migration Err: {:?}",
        result
    );
    // The marker must NOT have been advanced.
    assert!(
        !scope_is_ready(&scope_dir),
        "_ready must NOT be upgraded when migration fails"
    );
    // Crucially, the scope_dir itself must still exist (repair in place).
    assert!(
        scope_dir.exists(),
        "scope_dir must still exist after migration failure — no deletion allowed"
    );
    assert!(
        scope_dir.join(MANIFEST_FILE).exists(),
        "MANIFEST_FILE must survive migration failure"
    );

    // ── Phase 4: repair the file IN PLACE (no scope deletion) ───────────────
    std::fs::remove_file(scope_dir.join("personas.json")).unwrap();

    // ── Phase 5: retry → marker advances to v1 ───────────────────────────────
    ensure_scope_ready(scope_id, &scope_dir, &base_dir, "test_owner").unwrap();
    assert!(
        scope_is_ready(&scope_dir),
        "scope must be ready after in-place repair and retry"
    );
    let upgraded = std::fs::read_to_string(scope_dir.join(READY_MARKER)).unwrap();
    assert_eq!(
        upgraded.trim(),
        READY_MARKER_VERSION,
        "marker must carry current version after upgrade"
    );
}
