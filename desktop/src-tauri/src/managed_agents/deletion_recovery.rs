//! Finish only explicitly journaled deletion; relay-only heads are not garbage.
use super::retention::{active_retention_scope, deletion_intent, open_retention_db};
use super::{
    bestie_assignment::{recover_pending_assignment_cleanup, with_agent_assignments_cleared},
    ManagedAgentRecord,
};
use tauri::{AppHandle, Manager};

/// Preserve assignment rollback around the lifecycle-record removal only.
/// Later key/overlay cleanup is retryable and must not restore deleted assignments.
/// Caller holds the managed-agent store lock.
pub(crate) fn run_managed_agent_deletion<T>(
    base_dir: &std::path::Path,
    pubkey: &str,
    records: &mut Vec<ManagedAgentRecord>,
    delete: impl FnOnce(&mut Vec<ManagedAgentRecord>) -> Result<T, String>,
) -> Result<T, String> {
    recover_pending_assignment_cleanup(base_dir, |pending_pubkey| {
        records
            .iter()
            .any(|record| record.pubkey.eq_ignore_ascii_case(pending_pubkey))
    })?;
    with_agent_assignments_cleared(base_dir, pubkey, || delete(records))
}

/// Caller holds transition then store lock. Each completed prefix remains
/// retryable from the exact journal entry, including failed key deletion.
pub(crate) fn finish<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &crate::app_state::AppState,
    conn: &rusqlite::Connection,
    owner: &str,
    pubkey: &str,
) -> Result<(), String> {
    let result = finish_with_key_cleanup(
        app,
        state,
        conn,
        owner,
        pubkey,
        super::storage::try_delete_agent_key,
    );
    if result.is_err() {
        state
            .managed_agent_authority_ready
            .store(false, std::sync::atomic::Ordering::Release);
    }
    result
}

fn finish_with_key_cleanup<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &crate::app_state::AppState,
    conn: &rusqlite::Connection,
    owner: &str,
    pubkey: &str,
    delete_key: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    if !deletion_intent::pending(conn, owner, pubkey)? {
        return Ok(());
    }
    let mut records = super::load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    run_managed_agent_deletion(
        &super::managed_agents_base_dir(app)?,
        pubkey,
        &mut records,
        |records| {
            if let Some(record) = records.iter_mut().find(|record| record.pubkey == pubkey) {
                super::stop_managed_agent_process(app, record, &mut runtimes)?;
            }
            records.retain(|record| record.pubkey != pubkey);
            super::save_managed_agents(app, records)
        },
    )?;
    state
        .private_managed_agent_overlay
        .lock()
        .map_err(|error| error.to_string())?
        .remove(pubkey);
    delete_key(pubkey)?;
    state.clear_agent_session_caches(pubkey);
    deletion_intent::finish(conn, owner, pubkey)
}

/// Must complete before history admission, boot publication, and hydration.
pub(crate) fn recover<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let state = app.state::<crate::app_state::AppState>();
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let scope = active_retention_scope(app, &state)?;
    let conn = open_retention_db(&scope.db_path)?;
    let owner = scope.owner_keys.public_key().to_hex();
    for pubkey in deletion_intent::agents(&conn, &owner)? {
        finish(app, &state, &conn, &owner, &pubkey)?;
    }
    Ok(())
}
