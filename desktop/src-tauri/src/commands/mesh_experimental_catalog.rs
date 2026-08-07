//! Split-capable model catalog for the experimental tier of Share compute.
//!
//! Its own file because `commands/mesh_llm.rs` sits at 993 of the 1000-line
//! ceiling, and this is a separable concern: the community catalog, not the node
//! lifecycle.

use crate::commands::CmdResult;
use crate::mesh_llm;

/// Models that can be served across several machines.
///
/// Deliberately node-independent — you choose what to share before you start
/// sharing it, so this must work with the runtime off. Reads mesh-llm's own
/// catalog cache from disk, and only downloads when nothing is cached; both are
/// blocking I/O, hence `spawn_blocking` (matching `mesh_model_catalog`).
#[tauri::command]
pub async fn mesh_experimental_catalog() -> CmdResult<mesh_llm::MeshExperimentalCatalog> {
    tokio::task::spawn_blocking(mesh_llm::experimental_catalog)
        .await
        .map_err(|error| format!("mesh experimental catalog task failed: {error}"))
}
