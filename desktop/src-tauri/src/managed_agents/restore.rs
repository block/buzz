use super::{
    find_managed_agent_mut, load_managed_agents, load_personas, save_managed_agents, BackendKind,
};
use crate::app_state::AppState;
use crate::util;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// Backfill the pinned persona snapshot for pre-existing agents created before
/// the record became the spawn source of truth. Runs once at launch, before
/// `restore_managed_agents_on_launch` spawns anything, so no agent boots from an
/// empty snapshot.
///
/// Only records with a `persona_id` but no `persona_source_version` are touched.
/// Records that already have a `persona_source_version` — including those whose
/// `model`/`provider` were clobbered by the old unconditional snapshot code before
/// this fix — are skipped here; they self-heal on the next manual start via the
/// start-path re-snapshot in `start_local_agent_with_preflight`.
/// If the linked persona is gone, we log loudly and leave the record untouched —
/// it stays orphaned and `spawn_agent_child` refuses to start it (see
/// `effective_config::resolve_effective_config`'s `OrphanedInstance` arm).
pub fn backfill_persona_snapshots(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;

    let mut records = load_managed_agents(app)?;
    let needs_backfill = records
        .iter()
        .any(|r| r.persona_id.is_some() && r.persona_source_version.is_none());
    if !needs_backfill {
        return Ok(());
    }

    let personas = load_personas(app)?;
    let mut changed = false;
    for record in records.iter_mut() {
        let Some(persona_id) = record.persona_id.clone() else {
            continue;
        };
        if record.persona_source_version.is_some() {
            continue;
        }
        let Some(persona) = personas.iter().find(|p| p.id == persona_id) else {
            eprintln!(
                "buzz-desktop: persona-snapshot backfill: agent {} links persona {persona_id} which no longer exists; leaving it orphaned — spawn will refuse it",
                record.pubkey
            );
            continue;
        };
        // Layer precedence at read time: persona env < agent env. When the
        // persona leaves model/provider blank, the record's own configured
        // values are preserved — a blank persona must not clobber a
        // user-configured agent. See `apply_persona_snapshot`.
        super::persona_events::apply_persona_snapshot(record, persona);
        record.updated_at = util::now_iso();
        changed = true;
    }

    if changed {
        save_managed_agents(app, &records)?;
    }
    Ok(())
}

/// Restore managed agents that were running before the app was closed.
///
/// Split into three phases to minimise lock contention with the frontend:
///   A (under lock): sync process state, cleanup, collect agents to start
///   B (no locks):   resolve commands and spawn processes in parallel
///   C (re-lock):    write back PIDs and status to records on disk
pub async fn restore_managed_agents_on_launch(
    app: &tauri::AppHandle,
    shutdown_started: &AtomicBool,
) -> Result<(), String> {
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(());
    }
    let state = app.state::<AppState>();
    let candidates = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        load_managed_agents(app)?
            .into_iter()
            .filter(|record| record.start_on_app_launch && record.backend == BackendKind::Local)
            .collect::<Vec<_>>()
    };

    for record in candidates {
        if shutdown_started.load(Ordering::SeqCst) {
            break;
        }
        let workspace_relay = crate::relay::relay_ws_url_with_override(&state);
        let relay_url =
            crate::relay::effective_agent_relay_url(&record.relay_url, &workspace_relay);
        if let Err(error) = super::runtime_commands::start_pair(
            record.pubkey.clone(),
            relay_url,
            true,
            Some(&record.updated_at),
            app.clone(),
        ) {
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|lock_error| lock_error.to_string())?;
            let mut records = load_managed_agents(app)?;
            if let Ok(current) = find_managed_agent_mut(&mut records, &record.pubkey) {
                current.updated_at = util::now_iso();
                current.last_error = Some(error);
                save_managed_agents(app, &records)?;
            }
        }
    }
    Ok(())
}

fn profile_reconcile_completed(outcome: crate::commands::ProfileReconcileOutcome) -> bool {
    outcome == crate::commands::ProfileReconcileOutcome::Reconciled
}

pub(crate) fn spawn_pending_profile_reconciliations(app: &tauri::AppHandle, workspace_relay: &str) {
    let state = app.state::<AppState>();
    if !state
        .managed_agent_profile_reconcile_enabled
        .load(Ordering::Acquire)
    {
        return;
    }
    let items = match crate::commands::load_pending_profile_reconciliations(app, workspace_relay) {
        Ok(items) => items,
        Err(error) => {
            eprintln!("buzz-desktop: failed to load pending profile reconciliations: {error}");
            return;
        }
    };

    for (pubkey, data) in items {
        let reconcile_app = app.clone();
        let relay_url = data
            .target_relay_url
            .clone()
            .unwrap_or_else(|| data.relay_url.clone());
        tauri::async_runtime::spawn(async move {
            let state = reconcile_app.state::<AppState>();
            match crate::commands::reconcile_agent_profile(&state, &reconcile_app, &pubkey, &data)
                .await
            {
                Ok(outcome) if profile_reconcile_completed(outcome) => {
                    if let Err(error) = crate::commands::mark_profile_reconciled(
                        &reconcile_app,
                        &pubkey,
                        &relay_url,
                    ) {
                        eprintln!(
                            "buzz-desktop: failed to record profile reconciliation for agent {pubkey}: {error}"
                        );
                    }
                }
                Ok(_) => {}
                Err(error) => eprintln!(
                    "buzz-desktop: profile reconciliation failed for agent {pubkey}: {error}"
                ),
            }
        });
    }
}

#[cfg(test)]
mod profile_reconcile_tests {
    use super::profile_reconcile_completed;
    use crate::commands::ProfileReconcileOutcome;

    #[test]
    fn skipped_reconciliation_never_retires_pending_work() {
        assert!(profile_reconcile_completed(
            ProfileReconcileOutcome::Reconciled
        ));
        assert!(!profile_reconcile_completed(
            ProfileReconcileOutcome::SkippedDisabled
        ));
    }
}

#[cfg(feature = "mesh-llm")]
fn persist_restore_error(
    app: &tauri::AppHandle,
    state: &AppState,
    pubkey: &str,
    error: String,
) -> Result<(), String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.updated_at = util::now_iso();
    record.last_error = Some(error);
    save_managed_agents(app, &records)
}
