//! Tauri surface for importing optional Shepherd execution evidence.

use crate::managed_agents::shepherd::{normalize_shepherd_export, ShepherdEvidenceEnvelope};

/// Normalize a Shepherd flat JSON trace export without retaining raw payloads.
#[tauri::command]
pub fn normalize_shepherd_trace(
    export_json: String,
    source_run_ref: Option<String>,
) -> Result<ShepherdEvidenceEnvelope, String> {
    normalize_shepherd_export(&export_json, source_run_ref)
}
