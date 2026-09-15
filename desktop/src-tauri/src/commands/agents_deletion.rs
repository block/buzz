//! Managed-agent deletion: prepare, sign, revalidate, then destructive commit.
use super::*;

pub(super) async fn delete_managed_agent_with<R: tauri::Runtime>(
    pubkey: String,
    force_remote_delete: Option<bool>,
    app: AppHandle<R>,
) -> Result<(), String> {
    use tauri::Manager;
    crate::owner_authorization::require_owned_workspace(&app.state::<AppState>())?;
    // Snapshot the exact target before signing. No destructive operation or
    // process synchronization is allowed before the signatures finish.
    let (record_input, work) = {
        let state = app.state::<AppState>();
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(&app)?;
        // Replay even on repeated deletion: the missing record is the durable
        // evidence that cleanup must finish, not a reason to skip its journal.
        recover_pending_assignment_cleanup(&managed_agents_base_dir(&app)?, |pending_pubkey| {
            records
                .iter()
                .any(|record| record.pubkey.eq_ignore_ascii_case(pending_pubkey))
        })?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        if record.backend != BackendKind::Local
            && record.backend_agent_id.is_some()
            && !force_remote_delete.unwrap_or(false)
        {
            return Err(
                "cannot delete a deployed remote agent without force_remote_delete: true".into(),
            );
        }
        (
            serde_json::to_value(record).map_err(|e| e.to_string())?,
            deletion_witness::prepare_agent_deletion(&app, &state, &pubkey),
        )
    };
    let witness = deletion_witness::sign_agent_deletion(work).await;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        {
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let mut records = load_managed_agents(&app)?;
            let current = records.iter().find(|record| record.pubkey == pubkey);
            if current
                .map(serde_json::to_value)
                .transpose()
                .map_err(|e| e.to_string())?
                != Some(record_input)
            {
                return Err("agent deletion conflict: record changed; retry deletion".into());
            }
            let base_dir = managed_agents_base_dir(&app)?;
            // Assignment cleanup opens retention databases itself. Its existing
            // journal/rollback wrapper must surround (not run inside) the head
            // transaction. A head conflict returns through that wrapper, restoring
            // every cleared assignment before propagating; failed restores keep
            // the existing durable recovery journal.
            run_managed_agent_deletion(&base_dir, &pubkey, &mut records, |records| {
                deletion_witness::commit_agent_deletions(vec![witness], || {
                    let mut runtimes = state
                        .managed_agent_processes
                        .lock()
                        .map_err(|error| error.to_string())?;

                    let (sync_changed, exited_pubkeys) = sync_managed_agent_processes(
                        records,
                        &mut runtimes,
                        &current_instance_id(&app),
                    );
                    if sync_changed {
                        save_managed_agents(&app, records)?;
                    }
                    for pubkey in &exited_pubkeys {
                        state.clear_agent_session_caches(pubkey);
                    }
                    // Guard: reject deletion of deployed remote agents unless explicitly forced.
                    // This turns "don't orphan remote infra" from a UI convention into a backend
                    // invariant — a buggy or compromised IPC caller cannot silently orphan a live
                    // remote deployment. The frontend sends force_remote_delete: true only after
                    // the user confirms the orphan warning.
                    if let Some(record) = records.iter().find(|r| r.pubkey == pubkey) {
                        if record.backend != BackendKind::Local
                            && record.backend_agent_id.is_some()
                            && !force_remote_delete.unwrap_or(false)
                        {
                            return Err(
                        "cannot delete a deployed remote agent without force_remote_delete: true"
                            .to_string(),
                    );
                        }
                    }

                    if !records.iter().any(|record| record.pubkey == pubkey) {
                        return Err(format!("agent {pubkey} not found"));
                    }
                    if let Some(record) = records.iter_mut().find(|record| record.pubkey == pubkey)
                    {
                        stop_managed_agent_process(&app, record, &mut runtimes)?;
                    }
                    state.clear_agent_session_caches(&pubkey);
                    records.retain(|record| record.pubkey != pubkey);
                    save_managed_agents(&app, records)?;
                    crate::managed_agents::delete_agent_key(&pubkey);
                    // Tombstone after confirmed removal (inside lock; every published
                    // agent tombstones). The NIP-IA kind:9035 archive request — which
                    // stops the identity appearing in member pickers and autocomplete —
                    // is enqueued in the SAME transaction, its `persona_id` derived from
                    // the retained 30177 head.
                    Ok(())
                })
            })?;
        }
        try_regenerate_nest(&app);
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
