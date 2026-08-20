//! Live gossip view of the mesh — the peers this machine's runtime is actually
//! connected to right now.
//!
//! Separate from `mesh_llm.rs` because that file sits at the 1000-line ceiling
//! enforced by `desktop/scripts/check-file-sizes.mjs`, and separate from
//! `mesh_snapshot.rs` because the two answer different questions: the snapshot
//! reports every member's last published note (valid 120s, readable with no
//! local node), while this reports live adjacency. See `mesh_llm/peers.rs` for
//! why both exist.

use crate::app_state::AppState;
use crate::commands::CmdResult;
use crate::mesh_llm;
use tauri::State;

/// This machine's live gossip view of the mesh: the peers our runtime is
/// actually connected to right now.
///
/// `connected: false` with no peers means we are not participating, which is
/// distinct from "participating but alone". Never errors on absence — a stopped
/// node is a normal state, not a failure.
#[tauri::command]
pub async fn mesh_live_view(state: State<'_, AppState>) -> CmdResult<mesh_llm::MeshLiveView> {
    let runtime = state.mesh_llm_runtime.lock().await;
    match runtime.as_ref() {
        // A runtime that exists but is mid-start has no gossip view yet; report
        // "not connected" rather than surfacing a transient error.
        Some(runtime) => Ok(runtime.live_view().await.unwrap_or_default()),
        None => Ok(mesh_llm::MeshLiveView::default()),
    }
}
