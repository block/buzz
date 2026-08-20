//! Entry guard for `confirm_team_snapshot_import`.
//!
//! Extracted to keep `team_snapshot.rs` within the file-size ratchet.
//! Included via `#[path]` from `team_snapshot.rs`.

use crate::app_state::AppState;

/// Captured scope + owner keys checked at the entry boundary of team snapshot import.
#[derive(Debug)]
pub(crate) struct TeamSnapshotImportEntry {
    /// Workspace scope that was active at command entry.
    pub captured_scope: crate::managed_agents::scope::WorkspaceAgentScope,
    /// Owner keys validated to agree with `captured_scope.owner_pubkey`.
    pub captured_owner_keys: nostr::Keys,
}

/// Capture the active workspace scope and owner keys, verifying that the owner
/// pubkey matches the captured scope.
///
/// Returns `Err` with a user-facing message when no scope is active or when the
/// live signing keys don't match the captured scope's owner pubkey.
///
/// This is the production entry guard called by `confirm_team_snapshot_import`
/// and directly by unit tests.
pub(crate) fn capture_team_snapshot_import_entry(
    state: &AppState,
) -> Result<TeamSnapshotImportEntry, String> {
    let captured_scope = state
        .capture_active_scope()
        .ok_or("confirm_team_snapshot_import: no active workspace scope — cannot import")?;
    let captured_owner_keys = state
        .signing_keys()
        .map_err(|e| format!("confirm_team_snapshot_import: failed to capture owner keys: {e}"))?;
    if captured_owner_keys.public_key().to_hex() != captured_scope.owner_pubkey {
        return Err(
            "confirm_team_snapshot_import: owner pubkey mismatch; identity may have changed"
                .to_string(),
        );
    }
    Ok(TeamSnapshotImportEntry {
        captured_scope,
        captured_owner_keys,
    })
}
