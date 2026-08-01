use super::{
    find_managed_agent_mut, kill_stale_tracked_processes, load_managed_agents, load_personas,
    save_managed_agents, spawn_agent_child, sync_managed_agent_processes, BackendKind,
    ManagedAgentProcess,
};
use crate::app_state::AppState;
use crate::util;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

fn tracked_receipt_pids_for_restore(
    receipts: Result<Vec<(std::path::PathBuf, super::ManagedAgentRuntimeReceipt)>, String>,
    instance_id: &str,
) -> Result<Vec<u32>, String> {
    Ok(receipts?
        .into_iter()
        .filter_map(|(path, receipt)| {
            super::valid_agent_runtime_receipt(&path, &receipt, instance_id).then_some(receipt.pid)
        })
        .collect())
}

/// Outcome of a Phase B spawn attempt for one restore candidate.
///
/// `Skipped` means this exact pair already has a live tracked child. Restore
/// holds the global transition lock from candidate selection through runtime
/// registration, so Stop/delete/restart cannot race this outcome.
enum SpawnOutcome {
    Spawned(super::ManagedAgentRuntimeKey, ManagedAgentProcess),
    Skipped,
    Failed(String),
}
type AgentSpawnResult = (String, SpawnOutcome);

/// Return `true` when an exact live runtime already owns this pair. An exited
/// launcher is fully finalized before replacement; every inspection/finalizer
/// failure reinserts the exact runtime authority.
fn prepare_restore_pair(
    app: &tauri::AppHandle,
    key: &super::ManagedAgentRuntimeKey,
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    let Some(mut runtime) = runtimes.remove(key) else {
        return Ok(false);
    };
    match runtime.child.try_wait() {
        Ok(None) => {
            runtimes.insert(key.clone(), runtime);
            Ok(true)
        }
        Ok(Some(_)) => {
            #[cfg(windows)]
            let finalized =
                super::process_lifecycle::finalize_tracked_runtime(app, key, &mut runtime);
            #[cfg(not(windows))]
            let finalized = runtime
                .child
                .wait()
                .map_err(|error| error.to_string())
                .and_then(|status| {
                    super::remove_agent_runtime_receipt(app, key)?;
                    Ok(status)
                });
            match finalized {
                Ok(_) => Ok(false),
                Err(error) => {
                    runtimes.insert(key.clone(), runtime);
                    Err(format!(
                        "exited restore runtime could not be finalized: {error}"
                    ))
                }
            }
        }
        Err(error) => {
            runtimes.insert(key.clone(), runtime);
            Err(format!(
                "restore runtime exit could not be inspected: {error}"
            ))
        }
    }
}

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

    #[cfg(feature = "mesh-llm")]
    let mesh_preflights = {
        let records = {
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            load_managed_agents(app)?
        };
        let personas = load_personas(app).unwrap_or_default();
        let global = super::load_global_agent_config(app).unwrap_or_default();
        let mut outcomes = std::collections::HashMap::new();
        for record in records
            .into_iter()
            .filter(|record| record.start_on_app_launch && record.backend == BackendKind::Local)
        {
            let mesh_model_id = super::effective_config::resolve_effective_relay_mesh_model_id(
                &record, &personas, &global,
            );
            let result = if mesh_model_id.is_some() {
                crate::commands::ensure_relay_mesh_for_record(app, mesh_model_id.as_deref(), false)
                    .await
            } else {
                Ok(())
            };
            outcomes.insert(record.pubkey, (mesh_model_id, result));
        }
        outcomes
    };

    // Hold one transition lease from candidate selection through registration.
    // Stop/delete/restart therefore cannot complete against a stale Phase-A
    // snapshot and then be undone by this restore.
    let _restore_transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(());
    }

    // ── Phase A (under lock): housekeeping + collect agents to restore ──
    let mut agents_to_start: Vec<super::ManagedAgentRecord>;
    {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;

        if shutdown_started.load(Ordering::SeqCst) {
            return Ok(());
        }

        let mut records = load_managed_agents(app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;
        let (mut changed, _exited) = sync_managed_agent_processes(app, &mut records, &mut runtimes);
        changed |=
            kill_stale_tracked_processes(&mut records, &runtimes, &super::current_instance_id(app));

        let instance_id = super::current_instance_id(app);
        let receipt_pids = tracked_receipt_pids_for_restore(
            super::read_all_agent_runtime_receipts(app),
            &instance_id,
        )?;
        let tracked_pids: Vec<u32> = runtimes
            .values()
            .map(|runtime| runtime.child.id())
            .chain(receipt_pids)
            .collect();
        super::sweep_orphaned_agent_processes(app, &tracked_pids);

        // System-wide sweep: enumerate all user processes and kill any known
        // agent binaries not tracked by this session. Catches orphans whose
        // PID files were already cleaned up (e.g. agent workers in their own
        // process group whose parent harness exited).
        super::sweep_system_agent_processes(&super::current_instance_id(app), &tracked_pids);

        // Dead-instance reaping: find agents belonging to Buzz instances
        // whose desktop process is no longer running and reap them.
        super::reap_dead_instance_agents(&super::current_instance_id(app), &tracked_pids);

        // Exact-path sweep: kill any buzz-acp process whose executable path
        // matches this bundle's harness binary but is not in the tracked set.
        // Complements the env-var sweep above — catches orphans that predate
        // BUZZ_MANAGED_AGENT injection or lost their PID-file receipt.
        //
        // TODO: the three sweeps above each walk the PID table independently.
        // A future consolidation should collect a single shared process snapshot
        // at the top of this block and thread it through all sweep functions,
        // replacing the three separate kernel enumerations.
        super::sweep_untracked_bundle_harnesses(&tracked_pids);

        let candidates: Vec<String> = records
            .iter()
            .filter(|record| record.start_on_app_launch && record.backend == BackendKind::Local)
            .map(|record| record.pubkey.clone())
            .collect();

        #[cfg(feature = "mesh-llm")]
        let candidates = {
            let personas = load_personas(app).unwrap_or_default();
            let global = super::load_global_agent_config(app).unwrap_or_default();
            let mut approved = Vec::new();
            for pubkey in candidates {
                let Some(record) = records.iter_mut().find(|record| record.pubkey == pubkey) else {
                    continue;
                };
                let current_model = super::effective_config::resolve_effective_relay_mesh_model_id(
                    record, &personas, &global,
                );
                let outcome = mesh_preflights.get(&pubkey);
                let error = match outcome {
                    Some((preflight_model, Ok(()))) if *preflight_model == current_model => None,
                    Some((preflight_model, Ok(()))) => Some(format!(
                        "managed agent changed from mesh model {preflight_model:?} to {current_model:?} while restore preflight was in flight; retry restore"
                    )),
                    Some((preflight_model, Err(error))) if *preflight_model == current_model => {
                        Some(error.clone())
                    }
                    Some((preflight_model, Err(_))) => Some(format!(
                        "managed agent changed from mesh model {preflight_model:?} to {current_model:?} while restore preflight was in flight; retry restore"
                    )),
                    None => Some(
                        "managed agent became eligible after restore preflight; retry restore".to_string(),
                    ),
                };
                if let Some(error) = error {
                    record.updated_at = util::now_iso();
                    record.last_error = Some(error);
                    record.last_error_code = None;
                    changed = true;
                } else {
                    approved.push(pubkey);
                }
            }
            approved
        };

        let mut to_start = Vec::new();
        for pubkey in &candidates {
            if let Some(runtime) = runtimes
                .iter_mut()
                .find(|(key, _)| key.pubkey == *pubkey)
                .map(|(_, runtime)| runtime)
            {
                if runtime.child.try_wait().ok().flatten().is_none() {
                    continue;
                }
            }
            if let Some(record) = records.iter_mut().find(|r| r.pubkey == *pubkey) {
                let workspace_relay =
                    crate::relay::relay_ws_url_with_override(&app.state::<AppState>());
                let relay_url =
                    crate::relay::effective_agent_relay_url(&record.relay_url, &workspace_relay);
                if let Ok(key) =
                    super::ManagedAgentRuntimeKey::new(record.pubkey.clone(), &relay_url)
                {
                    if super::recovery_admission_error(app, record, &key)?.is_some() {
                        changed |= super::record_blocked_recovery_admission(record);
                        continue;
                    }
                }
                to_start.push(record.clone());
            }
        }
        agents_to_start = to_start;

        // Re-snapshot persona config for agents about to be restored, matching
        // the interactive spawn path so auto-start agents also pick up the
        // current persona on app launch.
        let personas_for_snapshot = super::load_personas(app).unwrap_or_default();
        for record in records.iter_mut() {
            if !agents_to_start.iter().any(|r| r.pubkey == record.pubkey) {
                continue;
            }
            let Some(persona_id) = record.persona_id.clone() else {
                continue;
            };
            let Some(persona) = personas_for_snapshot.iter().find(|p| p.id == persona_id) else {
                // Orphaned: no current persona to re-snapshot from. Leave the
                // record as-is — `spawn_agent_child` (Phase B below) refuses to
                // spawn it and Phase C persists the refusal to `last_error`.
                continue;
            };
            super::persona_events::apply_persona_snapshot(record, persona);
            record.updated_at = util::now_iso();
            changed = true;
        }
        // Re-collect to_start from the updated records so Phase B spawns the refreshed config.
        agents_to_start = records
            .iter()
            .filter(|r| agents_to_start.iter().any(|s| s.pubkey == r.pubkey))
            .cloned()
            .collect();

        if changed {
            save_managed_agents(app, &records)?;
        }
    }

    if agents_to_start.is_empty() {
        return Ok(());
    }

    // Snapshot the workspace owner pubkey once for the legacy auth_tag fallback.
    // Read outside the per-agent spawn loop so all parallel spawns see the same
    // value and we don't lock `state.keys` repeatedly.
    let owner_hex: Option<String> = state
        .keys
        .lock()
        .map_err(|e| e.to_string())
        .ok()
        .map(|k| k.public_key().to_hex());

    // ── Phase B (transition lock held): resolve commands and spawn in parallel ──
    let spawn_results: Vec<AgentSpawnResult> = std::thread::scope(|scope| {
        let owner_hex_ref = owner_hex.as_deref();
        let handles: Vec<_> = agents_to_start
            .iter()
            .filter(|_| !shutdown_started.load(Ordering::SeqCst))
            .map(|record| {
                let handle = scope.spawn(move || {
                    let workspace_relay =
                        crate::relay::relay_ws_url_with_override(&app.state::<AppState>());
                    let relay_url = crate::relay::effective_agent_relay_url(
                        &record.relay_url,
                        &workspace_relay,
                    );
                    let outcome =
                        match super::ManagedAgentRuntimeKey::new(record.pubkey.clone(), &relay_url)
                        {
                            Ok(key) => match prepare_restore_pair(app, &key) {
                                Ok(true) => SpawnOutcome::Skipped,
                                Err(error) => SpawnOutcome::Failed(error),
                                Ok(false) => {
                                    if let Some(error) =
                                        super::recovery_admission_error(app, record, &key)
                                            .unwrap_or_else(Some)
                                    {
                                        return (
                                            record.pubkey.clone(),
                                            SpawnOutcome::Failed(error),
                                        );
                                    }
                                    match super::terminate_untracked_pair_runtime(app, &key)
                                        .and_then(|()| {
                                            // Restore spawns lazy, matching reconcile
                                            // and manual start. A crashed mid-turn
                                            // session is not resumed by an eager child.
                                            spawn_agent_child(
                                                app,
                                                record,
                                                &key.relay_url,
                                                true,
                                                owner_hex_ref,
                                            )
                                        }) {
                                        Ok(process) => SpawnOutcome::Spawned(key, process),
                                        Err(error) => SpawnOutcome::Failed(error),
                                    }
                                }
                            },
                            Err(error) => SpawnOutcome::Failed(error),
                        };
                    (record.pubkey.clone(), outcome)
                });
                handle
            })
            .collect();

        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    if spawn_results.is_empty() {
        return Ok(());
    }

    // ── Phase C (re-acquire lock): write back PIDs and status to records ──
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;

    let mut successfully_spawned: Vec<String> = Vec::new();

    for (pubkey, outcome) in spawn_results {
        match outcome {
            // Skipped means a concurrent reconcile already owns a live child for
            // this pair; leave its runtime and record state untouched.
            SpawnOutcome::Skipped => continue,
            SpawnOutcome::Spawned(key, mut process) => {
                let Ok(record) = find_managed_agent_mut(&mut records, &pubkey) else {
                    #[cfg(windows)]
                    if let Err(cleanup_error) =
                        super::process_lifecycle::terminate_managed_agent_process(&mut process)
                    {
                        runtimes.insert(key, super::ManagedAgentPairRuntime::starting(process));
                        return Err(format!(
                            "restored runtime lost its record and cleanup remains tracked for retry: {cleanup_error}"
                        ));
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = super::terminate_process(process.child.id());
                        let _ = process.child.wait();
                    }
                    continue;
                };
                let now = util::now_iso();
                let receipt = super::ManagedAgentRuntimeReceipt {
                    key: key.clone(),
                    pid: process.child.id(),
                    desktop_instance_id: super::current_instance_id(app),
                    started_at: now.clone(),
                    windows_job_contained: super::process_has_windows_job(&process),
                };
                if let Err(error) = super::write_agent_runtime_receipt(app, &receipt) {
                    let cleanup_error = match super::runtime::receipt_failure::cleanup(
                        app,
                        process,
                        record,
                        &mut runtimes,
                        key,
                        now.clone(),
                        error,
                    ) {
                        Err(error) => error,
                        Ok(()) => {
                            "receipt cleanup returned without reporting its failure".to_string()
                        }
                    };
                    if record.last_error.is_none() {
                        record.updated_at = now;
                        record.last_error = Some(cleanup_error);
                    }
                    continue;
                }
                record.updated_at = now.clone();
                record.last_started_at = Some(now);
                record.last_stopped_at = None;
                record.last_exit_code = None;
                if !super::has_unverified_job_reap(record) {
                    record.last_error = None;
                }
                runtimes.insert(key, super::ManagedAgentPairRuntime::starting(process));
                successfully_spawned.push(pubkey);
            }
            SpawnOutcome::Failed(error) => {
                let Ok(record) = find_managed_agent_mut(&mut records, &pubkey) else {
                    continue;
                };
                record.updated_at = util::now_iso();
                record.last_error = Some(error);
            }
        }
    }

    // Collect profile reconciliation data for successfully spawned agents before
    // releasing the lock. This mirrors the fire-and-forget pattern in
    // start_managed_agent — ensuring boot-restored agents get the same profile
    // self-healing as UI-started agents.
    let reconcile_personas = super::load_personas(app).unwrap_or_default();
    let reconcile_items: Vec<(String, crate::commands::ProfileReconcileData)> =
        successfully_spawned
            .iter()
            .filter_map(|pubkey| {
                let record = records.iter().find(|r| r.pubkey == *pubkey)?;
                // Resolve the effective harness for the avatar-fallback
                // derivation (the snapshot may be empty/stale for an inherited
                // harness). Mirrors the UI start path.
                let effective_command =
                    crate::managed_agents::record_agent_command(record, &reconcile_personas);
                Some((
                    pubkey.clone(),
                    crate::commands::ProfileReconcileData {
                        private_key_nsec: record.private_key_nsec.clone(),
                        name: record.name.clone(),
                        relay_url: record.relay_url.clone(),
                        avatar_url: record.avatar_url.clone(),
                        auth_tag: record.auth_tag.clone(),
                        pubkey: record.pubkey.clone(),
                        agent_command: effective_command,
                        persona_id: record.persona_id.clone(),
                    },
                ))
            })
            .collect();

    save_managed_agents(app, &records)?;
    drop(runtimes);
    drop(_store_guard);
    drop(_restore_transition);

    // ── Profile reconciliation (fire-and-forget) ────────────────────────────
    // Spawn background tasks to ensure each restored agent's kind:0 profile is
    // published on the relay. Same pattern as the UI start path.
    for (pubkey, data) in reconcile_items {
        let reconcile_app = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = reconcile_app.state::<AppState>();
            if let Err(e) =
                crate::commands::reconcile_agent_profile(&state, &reconcile_app, &pubkey, &data)
                    .await
            {
                eprintln!("buzz-desktop: profile reconciliation failed for agent {pubkey}: {e}");
            }
        });
    }

    Ok(())
}

#[cfg(test)]
mod receipt_discovery_tests {
    #[test]
    fn receipt_store_uncertainty_blocks_restore_admission() {
        let discovery: Result<
            Vec<(
                std::path::PathBuf,
                crate::managed_agents::ManagedAgentRuntimeReceipt,
            )>,
            String,
        > = Err("receipt store indeterminate".to_string());

        let error = super::tracked_receipt_pids_for_restore(discovery, "instance").unwrap_err();
        assert!(error.contains("receipt store indeterminate"));
    }
}
