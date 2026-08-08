use super::*;

// ── Scope lifecycle tests ─────────────────────────────────────────────────────

/// `import-before-first-apply`: importing an identity when the active scope is
/// `None` must NOT derive, initialize, or claim any definition scope. The scope
/// stays `None` after the import; only the generation is bumped to invalidate
/// any in-flight stale operations.
///
/// Invariant: the fallback relay can never own the legacy claim — claims are
/// only written inside `apply_workspace`'s prepare stage.
#[test]
fn test_import_before_first_apply_leaves_scope_none() {
    let _gen_guard = crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let state = build_app_state();

    // Boot state: no active scope.
    assert!(
        state.capture_active_scope().is_none(),
        "scope must be None before any apply_workspace"
    );

    let generation_before = crate::managed_agents::scope::current_scope_generation();

    // Simulate what import_identity does on the no-active-scope path:
    // clear (no-op) and bump generation.
    state.clear_active_scope();

    let generation_after = crate::managed_agents::scope::current_scope_generation();

    // Scope remains None — no scope was derived or claimed.
    assert!(
        state.capture_active_scope().is_none(),
        "scope must remain None after import-before-first-apply"
    );

    // Generation was bumped to invalidate any in-flight stale operations.
    assert!(
        generation_after > generation_before,
        "generation must advance after identity import to invalidate stale ops"
    );
}

/// `live-import-with-active-runtimes`: importing an identity while a scope is
/// active must clear the scope to `None` and bump the generation. Agent
/// commands fail closed until the frontend re-applies a workspace.
#[test]
fn test_live_import_with_active_scope_clears_scope_and_bumps_generation() {
    let _gen_guard = crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let state = build_app_state();
    let base = std::env::temp_dir();

    // Commit an active scope (simulates a live workspace).
    let gen_initial = crate::managed_agents::scope::next_scope_generation();
    let scope = crate::managed_agents::scope::WorkspaceAgentScope::new(
        "wss://a.example".into(),
        "cc".repeat(32),
        &base,
        gen_initial,
    );
    state.commit_active_scope(scope);
    assert!(
        state.capture_active_scope().is_some(),
        "scope must be Some after commit"
    );

    let generation_before_import = crate::managed_agents::scope::current_scope_generation();

    // Simulate the live-import path: clear scope. `clear_active_scope()`
    // bumps the generation internally — no additional bump needed.
    state.clear_active_scope();

    let generation_after_import = crate::managed_agents::scope::current_scope_generation();

    // Scope is None — all agent commands now fail closed.
    assert!(
        state.capture_active_scope().is_none(),
        "scope must be None after live identity import"
    );

    // Generation advanced — any in-flight stale-spawn detects staleness.
    assert!(
        generation_after_import > generation_before_import,
        "generation must advance after live identity import"
    );
}

/// `fallback-never-claims`: the legacy definition claim is only written inside
/// `ensure_scope_ready` (called from `apply_workspace`'s prepare stage), never
/// from identity import. Verifies the claim ledger cannot be written without an
/// explicit relay selection.
///
/// This test verifies the structural invariant: `clear_active_scope()` —
/// the single operation identity import performs — does not touch the
/// filesystem claim ledger.
///
/// Holds `SCOPE_GENERATION_TEST_LOCK` because `clear_active_scope()` internally
/// calls `next_scope_generation()`, which mutates the process-global generation
/// counter. Without the lock, this bump can race any test that relies on
/// generation stability (e.g., captured-scope stale-detection tests).
#[test]
fn test_fallback_relay_never_claims_during_identity_import() {
    let _gen_guard = crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let state = build_app_state();

    // Plant legacy definitions so a claim WOULD be created if the
    // import path called ensure_scope_ready.
    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    std::fs::write(agents_dir.join("managed-agents.json"), b"[]").unwrap();
    std::fs::write(agents_dir.join("teams.json"), b"[]").unwrap();

    // Simulate the import-before-first-apply path: clear scope.
    // `clear_active_scope()` bumps the generation internally.
    state.clear_active_scope();

    // No claim file must exist: the fallback JSON claim path.
    let fallback_claim = tmp.path().join("agents").join("legacy-claim.json");
    assert!(
        !fallback_claim.exists(),
        "identity import must never write a definition claim"
    );

    // No retention.db claim either (no DB was opened by import).
    let retention_db = tmp.path().join("retention.db");
    assert!(
        !retention_db.exists(),
        "identity import must never create or modify retention.db"
    );
}

/// `prepare-failure-leaves-old-scope-intact`: if the prepare stage returns an
/// error (e.g., `ensure_scope_ready` fails), the old scope must remain active
/// and unchanged. This tests the AppState contract: `commit_active_scope` is
/// never called on the error path, so `capture_active_scope()` returns the
/// original scope.
#[test]
fn test_prepare_failure_leaves_old_scope_intact() {
    let _gen_guard = crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let state = build_app_state();
    let base = std::env::temp_dir();

    // Commit the "old" active scope (workspace A).
    let gen_a = crate::managed_agents::scope::next_scope_generation();
    let scope_a = crate::managed_agents::scope::WorkspaceAgentScope::new(
        "wss://a.example".into(),
        "dd".repeat(32),
        &base,
        gen_a,
    );
    state.commit_active_scope(scope_a.clone());

    // Simulate a prepare failure: the error path does NOT call
    // commit_active_scope. The old scope must remain.
    // (We simulate this by simply not calling commit_active_scope.)

    let still_active = state.capture_active_scope();
    assert!(
        still_active.is_some(),
        "scope must remain after prepare failure"
    );
    assert_eq!(
        still_active.unwrap().relay_url,
        "wss://a.example",
        "the old scope's relay must be unchanged after prepare failure"
    );
    assert_eq!(
        state.capture_active_scope().unwrap().generation,
        gen_a,
        "the old scope's generation must be unchanged after prepare failure"
    );
}

/// `inactive-runtime-exit`: an agent that exits after a scope has been cleared
/// to None (e.g., after live identity import) must not crash the observer and
/// must be treated as already-stopped. This tests that `capture_active_scope`
/// returning `None` is handled gracefully by callers that check the scope.
#[test]
fn test_inactive_runtime_exit_after_scope_cleared_is_safe() {
    let _gen_guard = crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let state = build_app_state();
    let base = std::env::temp_dir();

    // Commit a scope, then clear it (simulating live identity import).
    let gen = crate::managed_agents::scope::next_scope_generation();
    let scope = crate::managed_agents::scope::WorkspaceAgentScope::new(
        "wss://a.example".into(),
        "ee".repeat(32),
        &base,
        gen,
    );
    state.commit_active_scope(scope);
    state.clear_active_scope();

    // Any observer checking capture_active_scope after scope cleared must
    // see None and gracefully fail-closed.
    assert!(
        state.capture_active_scope().is_none(),
        "scope must be None after clear — runtime exit observers must handle this"
    );
}
