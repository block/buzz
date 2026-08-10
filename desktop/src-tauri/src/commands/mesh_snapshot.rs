//! `mesh_snapshot` — community-wide shared-compute snapshot for the UI.
//!
//! Separate file from `commands/mesh_llm.rs` deliberately: that module is at
//! the 1000-line file ceiling enforced by `desktop/scripts/check-file-sizes.mjs`.
//!
//! Read-only. Queries the same member-status + membership filters routing uses,
//! then projects them through [`mesh_llm::snapshot_from_events`], which applies
//! the membership/binding/freshness rules. This never selects a serve target —
//! routing stays with `availability_from_events`.

use tauri::State;

use crate::app_state::AppState;
use crate::mesh_llm;

type CmdResult<T> = Result<T, String>;

/// Snapshot of who is sharing compute in this community right now.
///
/// An empty mesh is a normal state, not an error: the snapshot carries a
/// `reason` string instead of failing, so the card can render an honest empty
/// state. A genuine relay/transport failure still returns `Err`.
#[tauri::command]
pub async fn mesh_snapshot(state: State<'_, AppState>) -> CmdResult<mesh_llm::MeshSnapshot> {
    let events = crate::relay::query_relay(
        &state,
        &[
            mesh_llm::mesh_status_filter(),
            mesh_llm::relay_membership_filter(),
        ],
    )
    .await
    .map_err(|error| format!("Shared compute status query failed: {error}"))?;

    // Identify this member's own device so the card can say "including yours".
    // A missing/locked identity is not fatal here — the snapshot is still
    // useful, just without self-attribution.
    let self_pubkey = state
        .signing_keys()
        .ok()
        .map(|keys| keys.public_key().to_hex());

    Ok(mesh_llm::snapshot_from_events(
        events,
        self_pubkey.as_deref(),
    ))
}
