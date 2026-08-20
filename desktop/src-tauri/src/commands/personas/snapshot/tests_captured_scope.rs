//! Behavioral tests for captured-scope relay and owner-key invariants in snapshot import.
//!
//! These tests call production functions from the snapshot import path — not
//! copies of their logic — to prove the captured-scope contracts hold when a
//! workspace switch or identity change races an in-flight import.
//!
//! `capture_agent_snapshot_import_entry` is the production boundary guard used
//! by `confirm_agent_snapshot_import` at command entry. Tests call it directly
//! with a `tauri::test::mock_builder()` AppState, exercising the real scope
//! capture and owner-key agreement checks without needing an AppHandle.
//!
//! Kept in a sibling file so `tests.rs` stays within the file-size ratchet.
//! Included via `#[path]` from `tests.rs`.

use crate::app_state::{build_app_state, AppState};
use crate::commands::personas::snapshot::import::capture_agent_snapshot_import_entry;
use crate::managed_agents::scope::{
    current_scope_generation, next_scope_generation, WorkspaceAgentScope,
};

fn make_scope_with_keys(tmp: &tempfile::TempDir, owner_keys: &nostr::Keys) -> WorkspaceAgentScope {
    let gen = current_scope_generation();
    WorkspaceAgentScope {
        scope_id: "test-scope".to_string(),
        relay_url: "wss://captured.example".to_string(),
        owner_pubkey: owner_keys.public_key().to_hex(),
        definitions_dir: tmp.path().to_path_buf(),
        generation: gen,
    }
}

fn build_import_state(owner_keys: nostr::Keys) -> AppState {
    let state = build_app_state();
    {
        let mut locked = state.identity_lifecycle_keys_guard().unwrap();
        *locked = owner_keys;
    }
    state
}

/// `capture_agent_snapshot_import_entry` with no active workspace scope → rejects
/// with "no active workspace scope" before any other processing.
///
/// Calls the real production entry guard. Proves the no-scope fail-closed contract.
#[test]
fn test_confirm_agent_snapshot_import_no_scope_rejected() {
    let owner_keys = nostr::Keys::generate();
    let state = build_import_state(owner_keys);
    // No scope committed — stays None.

    let result = capture_agent_snapshot_import_entry(&state);

    assert!(
        result.is_err(),
        "no scope must reject the import entry guard"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("no active workspace scope"),
        "error must describe missing scope: {err}"
    );
}

/// `capture_agent_snapshot_import_entry` with a mismatched owner pubkey → rejects
/// with "owner pubkey mismatch" before any file I/O.
///
/// Calls the real production entry guard. Simulates a concurrent identity import
/// that replaced the signing key between scope capture and Phase 1.
/// This is the identity-switch-before-store rejection test.
#[test]
fn test_confirm_agent_snapshot_import_owner_mismatch_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let scope_keys = nostr::Keys::generate();
    let other_keys = nostr::Keys::generate();

    let scope = make_scope_with_keys(&tmp, &scope_keys);
    // State holds other_keys — pubkey differs from scope.owner_pubkey.
    let state = build_import_state(other_keys);
    state.commit_active_scope(scope);

    let result = capture_agent_snapshot_import_entry(&state);

    assert!(
        result.is_err(),
        "owner mismatch must reject the import entry guard"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("owner pubkey mismatch") || err.contains("mismatch"),
        "error must describe owner pubkey mismatch: {err}"
    );
}

/// `capture_agent_snapshot_import_entry` with owner keys matching the scope's
/// owner pubkey → succeeds, returning captured scope and owner keys.
///
/// Calls the real production entry guard. Proves: scope capture → owner key
/// check PASSES → `AgentSnapshotImportEntry` returned with matching pubkeys.
#[test]
fn test_confirm_agent_snapshot_import_matching_owner_passes_entry_guard() {
    let tmp = tempfile::tempdir().unwrap();
    let owner_keys = nostr::Keys::generate();
    let scope = make_scope_with_keys(&tmp, &owner_keys);
    let expected_pubkey = owner_keys.public_key().to_hex();
    let state = build_import_state(owner_keys);
    state.commit_active_scope(scope.clone());

    let result = capture_agent_snapshot_import_entry(&state);

    assert!(
        result.is_ok(),
        "matching owner must pass the entry guard: {:?}",
        result.err()
    );
    let entry = result.unwrap();
    assert_eq!(
        entry.captured_scope.owner_pubkey, expected_pubkey,
        "captured scope must carry the expected owner pubkey"
    );
    assert_eq!(
        entry.captured_owner_keys.public_key().to_hex(),
        expected_pubkey,
        "captured owner keys must match the scope owner pubkey"
    );
}

/// Switch-between-Phase3a-and-Phase3b: `validate_scope_generation` correctly
/// rejects a stale scope when the global generation was advanced after capture.
///
/// This is the switch-between-store-and-profile guard test for agent snapshot
/// import. The Phase 3a write guard calls `validate_scope_generation` under the
/// store lock to detect a workspace switch that raced the in-flight import.
///
/// Tests the production `validate_scope_generation` function directly — the
/// exact guard that fires inside Phase 3a of `confirm_agent_snapshot_import`.
///
/// Holds `SCOPE_GENERATION_TEST_LOCK` because `next_scope_generation()` mutates
/// the process-global generation counter. Without the lock, the bump can race
/// any async test holding a captured generation (e.g., generation-stability
/// tests in `global_agent_config_epoch_tests` or
/// `runtime_commands_concurrency_tests`), causing spurious stale-detection
/// failures in the generation-sensitive path.
#[test]
fn test_scope_generation_guard_rejects_stale_scope_for_import() {
    let _gen_guard = crate::managed_agents::scope::SCOPE_GENERATION_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let owner_keys = nostr::Keys::generate();

    // Capture a scope at the current generation.
    let scope = make_scope_with_keys(&tmp, &owner_keys);

    // Simulate a workspace switch — advance the global generation.
    next_scope_generation();

    // The captured scope's generation is now stale.
    let validation_result = crate::managed_agents::scope::validate_scope_generation(&scope);

    assert!(
        validation_result.is_err(),
        "stale scope must be rejected by validate_scope_generation"
    );
    let err = validation_result.unwrap_err();
    assert!(
        err.contains("stale") || err.contains("generation"),
        "error must describe stale generation: {err}"
    );
}
