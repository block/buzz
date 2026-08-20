//! Entry guard for `confirm_agent_snapshot_import`.
//!
//! Extracted to keep `import.rs` within the file-size ratchet.
//! Included via `#[path]` from `import.rs`.

use crate::app_state::AppState;

/// Captured scope + owner keys checked at the entry boundary of snapshot import.
#[derive(Debug)]
pub(crate) struct AgentSnapshotImportEntry {
    /// Workspace scope that was active at command entry.
    pub captured_scope: crate::managed_agents::scope::WorkspaceAgentScope,
    /// Owner keys validated to agree with `captured_scope.owner_pubkey`.
    pub captured_owner_keys: nostr::Keys,
}

/// Capture the active workspace scope and owner keys, verifying that the owner
/// pubkey matches the captured scope.
///
/// Returns `Err` with a user-facing message when:
/// - No workspace scope is active (`"no active workspace scope"`).
/// - The live signing keys don't match the captured scope's owner pubkey
///   (`"owner pubkey mismatch"`).
///
/// This is the production entry guard shared by the Tauri command and tests.
/// Tests call this directly via `tauri::test::mock_builder()` + `AppState`;
/// the Tauri command calls it then proceeds to Phase 1+.
pub(crate) fn capture_agent_snapshot_import_entry(
    state: &AppState,
) -> Result<AgentSnapshotImportEntry, String> {
    let captured_scope = state
        .capture_active_scope()
        .ok_or("confirm_agent_snapshot_import: no active workspace scope")?;
    let captured_owner_keys = state
        .signing_keys()
        .map_err(|e| format!("confirm_agent_snapshot_import: failed to capture owner keys: {e}"))?;
    if captured_owner_keys.public_key().to_hex() != captured_scope.owner_pubkey {
        return Err(
            "confirm_agent_snapshot_import: owner pubkey mismatch; identity may have changed"
                .to_string(),
        );
    }
    Ok(AgentSnapshotImportEntry {
        captured_scope,
        captured_owner_keys,
    })
}
