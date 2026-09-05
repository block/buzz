//! Final-use authority for launch restore. Lock order is transition → store;
//! the caller keeps both through child registration, just like interactive Start.
use super::super::{private_config_overlay, retention, ManagedAgentRecord};
use crate::app_state::AppState;
use std::{path::Path, sync::MutexGuard};

/// Disk owns only restore intent, not the next backend. Resolve backend later.
pub(super) fn restore_candidate_pubkeys(records: &[ManagedAgentRecord]) -> Vec<String> {
    records
        .iter()
        .filter(|record| record.start_on_app_launch)
        .map(|record| record.pubkey.clone())
        .collect()
}

pub(super) struct RestoreAuthority<'a> {
    _store: MutexGuard<'a, ()>,
    pub records: Vec<ManagedAgentRecord>,
    pub candidates: Vec<ManagedAgentRecord>,
    pub workspace_relay: String,
    pub owner_hex: String,
}

/// Caller holds the runtime transition lock. Reloading membership is necessary:
/// a resolved Phase-A clone cannot tell us whether Stop or Delete happened while
/// preflight awaited, or whether an inbound head changed the next backend.
pub(super) fn lock_restore_authority<'a, R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &'a AppState,
    expected_scope: &Path,
    prepared: &[ManagedAgentRecord],
) -> Result<RestoreAuthority<'a>, String> {
    let store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let scope = retention::active_retention_scope(app, state)?;
    if scope.db_path != expected_scope {
        return Err("workspace changed during managed-agent restore; retry initialization".into());
    }
    private_config_overlay::require_authority_ready(state)?;
    let records = super::super::load_managed_agents(app)?;
    let personas = super::super::load_personas(app)?;
    let mut candidates = Vec::new();
    for record in &records {
        if !record.start_on_app_launch || !prepared.iter().any(|r| r.pubkey == record.pubkey) {
            continue;
        }
        let resolved = private_config_overlay::resolved_local_record(state, record)?;
        if let Some(candidate) = super::finalize_restore_candidate(resolved, &personas) {
            candidates.push(candidate);
        }
    }
    Ok(RestoreAuthority {
        _store: store,
        records,
        candidates,
        workspace_relay: scope.relay_url,
        owner_hex: scope.owner_keys.public_key().to_hex(),
    })
}

#[cfg(test)]
mod tests;
