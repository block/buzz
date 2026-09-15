//! Boot-time disk↔relay reconcile for managed-agent (kind:30177) events.
//!
//! `run_event_sync` already reconciles personas (30175) and teams (30176)
//! into the retention store at boot; managed agents were the missing leg —
//! their events were enqueued only on the interactive save path
//! (`retain_managed_agent_pending`), so a record edited on disk between
//! launches, or a save whose publish was missed, silently diverged from the
//! relay. This module mirrors `migrate_personas_in_dir`: per-coordinate
//! content diff, monotonic `created_at` bump, retain with `pending_sync = 1`
//! for the existing flush loop.
//!
//! Best-effort contract (decided in #centralize-personas-and-agents):
//! - No file watcher — hand edits are picked up at next boot only.
//! - No deletion reconcile — a record absent from `managed-agents.json` is
//!   left untouched in retention; a truncated or partial file must never
//!   trigger tombstones.
//! - A malformed store fails loudly: the broken file is preserved as
//!   `managed-agents.json.invalid` (see [`super::storage::backup_invalid_store`])
//!   and an error is returned, never silently skipped.

use super::ManagedAgentRecord;
use crate::active_user_signer::ActiveUserSigner;
use std::path::Path;
use tauri::Manager;

mod publication;
pub(crate) use publication::{finish_agent_record, prepare_agent_record, AgentRetentionWork};

/// Reconcile agent definitions after the persona/team/catalog boot legs.
/// Every signing await is outside both the store lock and SQLite connection.
pub(crate) async fn reconcile_agents_to_events<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    signer: &ActiveUserSigner,
    db_path: &Path,
) {
    let Ok(base_dir) = super::managed_agents_base_dir(app) else {
        return;
    };
    let state = app.state::<crate::app_state::AppState>();
    match reconcile_agents_in_dir_at(&base_dir, signer, db_path, &state.managed_agents_store_lock)
        .await
    {
        Ok(0) => {}
        Ok(count) => {
            eprintln!("buzz-desktop: agent-event-reconcile: {count} agents reconciled to retention")
        }
        Err(e) => eprintln!("buzz-desktop: agent-event-reconcile: {e}"),
    }
}

fn read_agent_records(base_dir: &Path) -> Result<Vec<ManagedAgentRecord>, String> {
    let path = base_dir.join("managed-agents.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;
    serde_json::from_str(&content).map_err(|e| {
        super::storage::backup_invalid_store(&path);
        format!("failed to parse managed-agents.json (preserved as .invalid): {e}")
    })
}

async fn reconcile_agents_in_dir_at(
    base_dir: &Path,
    signer: &ActiveUserSigner,
    db_path: &Path,
    store_lock: &std::sync::Mutex<()>,
) -> Result<u32, String> {
    let records = {
        let _guard = store_lock.lock().map_err(|e| e.to_string())?;
        read_agent_records(base_dir)?
    };
    let mut count = 0;
    for record in records.iter().filter(|record| !record.pubkey.is_empty()) {
        let work = {
            let _guard = store_lock.lock().map_err(|e| e.to_string())?;
            let current = read_agent_records(base_dir)?;
            let Some(record) = current
                .iter()
                .find(|current| current.pubkey == record.pubkey)
            else {
                continue;
            };
            prepare_agent_record(db_path, signer, record)
        };
        if finish_agent_record(work, base_dir, store_lock).await? {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
async fn reconcile_agents_in_dir(base_dir: &Path, keys: &nostr::Keys) -> Result<u32, String> {
    reconcile_agents_in_dir_at(
        base_dir,
        &ActiveUserSigner::local(keys.clone()),
        &base_dir.join("retention.db"),
        &std::sync::Mutex::new(()),
    )
    .await
}

#[cfg(test)]
async fn retain_agent_record(
    db_path: &Path,
    keys: &nostr::Keys,
    record: &ManagedAgentRecord,
) -> Result<bool, String> {
    match prepare_agent_record(db_path, &ActiveUserSigner::local(keys.clone()), record)? {
        Some(work) => work.sign().await?.commit(std::slice::from_ref(record)),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests;
