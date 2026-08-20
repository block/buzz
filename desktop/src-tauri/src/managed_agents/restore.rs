use super::{
    find_managed_agent_mut, kill_stale_tracked_processes, spawn_agent_child,
    sync_managed_agent_processes, BackendKind, ManagedAgentProcess,
};
use crate::app_state::AppState;
#[cfg(feature = "mesh-llm")]
use crate::managed_agents::global_config::load_global_agent_config_at;
use crate::managed_agents::personas::load_personas_at;
use crate::managed_agents::storage::{load_managed_agents_at, save_managed_agents_at};
use crate::util;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// Outcome of a Phase B spawn attempt for one restore candidate.
///
/// `Skipped` covers the case where a concurrently-running startup reconcile
/// already spawned and tracked this exact pair during the Phase A window (the
/// transition lock is only held from Phase B onward). Restore must then leave
/// that live child alone rather than terminate-and-respawn it — mirroring the
/// live-child guard in `start_pair` (`runtime_commands.rs`). Without this,
/// restore would kill reconcile's lazy child by its receipt and replace it with
/// an eager one, flipping the pair's laziness on a startup race.
enum SpawnOutcome {
    /// Boxed: the spawned process carries its full spawn-config snapshot, so an
    /// inline variant would make every `Skipped`/`Failed` outcome pay for it.
    Spawned(super::ManagedAgentRuntimeKey, Box<ManagedAgentProcess>),
    Skipped,
    Failed(String),
}
type AgentSpawnResult = (String, SpawnOutcome);

/// Backfill persona snapshots without acquiring the store lock.
///
/// For use during scope initialization (inside `ensure_scope_ready`), where the
/// scope directory is not yet published as `_ready` and no concurrent reader or
/// writer can legally access it. In all other contexts the store lock must be
/// held by the caller before reading or writing scope definitions.
pub(crate) fn backfill_persona_snapshots_pre_ready(
    definitions_dir: &std::path::Path,
) -> Result<(), String> {
    backfill_persona_snapshots_inner(definitions_dir)
}

fn backfill_persona_snapshots_inner(definitions_dir: &std::path::Path) -> Result<(), String> {
    let mut records = load_managed_agents_at(definitions_dir)?;
    let needs_backfill = records
        .iter()
        .any(|r| r.persona_id.is_some() && r.persona_source_version.is_none());
    if !needs_backfill {
        return Ok(());
    }

    let personas = load_personas_at(definitions_dir)?;
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
        save_managed_agents_at(definitions_dir, &records)?;
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

    // Capture scope at function entry — all three phases (A, B, C) use this
    // single captured definitions_dir so a concurrent workspace switch cannot
    // write Phase C's results into a different scope's store than Phase A read.
    let scope = state
        .capture_active_scope()
        .ok_or_else(|| "restore_managed_agents_on_launch: no active workspace scope".to_string())?;
    let definitions_dir = scope.definitions_dir.clone();

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

        let mut records = load_managed_agents_at(&definitions_dir)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;
        let (mut changed, _exited) = sync_managed_agent_processes(
            &mut records,
            &mut runtimes,
            &super::current_instance_id(app),
        );
        changed |=
            kill_stale_tracked_processes(&mut records, &runtimes, &super::current_instance_id(app));

        let tracked_pids: Vec<u32> = runtimes
            .values()
            .map(|runtime| runtime.child.id())
            .chain(
                super::read_all_agent_runtime_receipts(app)
                    .into_iter()
                    .filter_map(|(path, receipt)| {
                        super::valid_agent_runtime_receipt(
                            &path,
                            &receipt,
                            &super::current_instance_id(app),
                        )
                        .then_some(receipt.pid)
                    }),
            )
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
            if let Some(record) = records.iter().find(|r| r.pubkey == *pubkey) {
                if let Some(pid) = record.runtime_pid {
                    if super::process_is_running(pid) {
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
        let personas_for_snapshot = load_personas_at(&definitions_dir).unwrap_or_default();
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
            save_managed_agents_at(&definitions_dir, &records)?;
        }
    }

    if agents_to_start.is_empty() {
        return Ok(());
    }

    // Snapshot the workspace owner pubkey once for the legacy auth_tag fallback.
    // Read outside the per-agent spawn loop so all parallel spawns see the same
    // value and we don't re-read the identity repeatedly.
    let owner_hex: Option<String> = state.current_pubkey().ok().map(|pk| pk.to_hex());

    #[cfg(feature = "mesh-llm")]
    let agents_to_start = {
        // Preflight against the same resolution spawn uses — `resolve_effective_config`
        // (definition → global fallback). A linked instance's own `provider`/`model`/
        // `relay_mesh` bytes never contribute. See `start_local_agent_with_preflight`
        // in `commands/agents.rs` for the identical rationale on the interactive path.
        // Use the captured scope's definitions_dir for both loads so they read from
        // the same scope as Phase A.
        let personas = load_personas_at(&definitions_dir).unwrap_or_default();
        let global = load_global_agent_config_at(&definitions_dir).unwrap_or_default();
        let mut mesh_preflight_failures = std::collections::HashSet::new();
        for record in &agents_to_start {
            let mesh_model_id = super::effective_config::resolve_effective_relay_mesh_model_id(
                record, &personas, &global,
            );
            if mesh_model_id.is_none() {
                continue;
            }
            // Auto-start after relaunch: re-resolve a live bootstrap target and
            // dial it. Skip (with an actionable error) only when no live target
            // serves this model right now.
            if let Err(error) =
                crate::commands::ensure_relay_mesh_for_record(app, mesh_model_id.as_deref(), false)
                    .await
            {
                persist_restore_error(app, &state, &record.pubkey, &definitions_dir, error)?;
                mesh_preflight_failures.insert(record.pubkey.clone());
            }
        }
        agents_to_start
            .into_iter()
            .filter(|record| !mesh_preflight_failures.contains(&record.pubkey))
            .collect::<Vec<_>>()
    };
    if agents_to_start.is_empty() {
        return Ok(());
    }

    // Serialize spawning and runtime registration with shutdown cleanup. The
    // shutdown flag is rechecked after taking the lock so shutdown either
    // prevents this transition or waits until every child is tracked and can
    // be terminated.
    let restore_transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(());
    }

    // ── Phase B (transition lock held): resolve commands and spawn in parallel ──
    let spawn_results: Vec<AgentSpawnResult> = std::thread::scope(|scope_s| {
        let owner_hex_ref = owner_hex.as_deref();
        // Use the captured scope's relay — not live state — so a mid-flight
        // workspace switch cannot re-target Phase B spawns to the new relay.
        let captured_relay = &scope.relay_url;
        let handles: Vec<_> = agents_to_start
            .iter()
            .filter(|_| !shutdown_started.load(Ordering::SeqCst))
            .map(|record| {
                let handle = scope_s.spawn(move || {
                    let relay_url =
                        crate::relay::effective_agent_relay_url(&record.relay_url, captured_relay);
                    let outcome =
                        match super::ManagedAgentRuntimeKey::new(record.pubkey.clone(), &relay_url)
                        {
                            Ok(key) => {
                                // F2: if a concurrent startup reconcile already
                                // tracked a live child for this exact pair during
                                // the Phase A window, leave it alone. Mirrors the
                                // live-child guard in `start_pair`.
                                let already_live = app
                                    .state::<AppState>()
                                    .managed_agent_processes
                                    .lock()
                                    .ok()
                                    .and_then(|mut runtimes| {
                                        runtimes.get_mut(&key).map(|runtime| {
                                            runtime.child.try_wait().ok().flatten().is_none()
                                        })
                                    })
                                    .unwrap_or(false);
                                if already_live {
                                    SpawnOutcome::Skipped
                                } else {
                                    match super::terminate_untracked_pair_runtime(app, &key)
                                        .and_then(|()| {
                                            // F1: restore spawns lazy, matching
                                            // reconcile and manual start. Eager on
                                            // restore buys nothing — a crashed
                                            // mid-turn session is not resumed by an
                                            // eager child — and silently reintroduces
                                            // N idle brains on every launch.
                                            spawn_agent_child(
                                                app,
                                                record,
                                                &key.relay_url,
                                                true,
                                                owner_hex_ref,
                                            )
                                        }) {
                                        Ok(process) => {
                                            SpawnOutcome::Spawned(key, Box::new(process))
                                        }
                                        Err(error) => SpawnOutcome::Failed(error),
                                    }
                                }
                            }
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
    // Use the same captured definitions_dir from function entry so Phase C
    // writes to the same scope Phase A read from, even if a workspace switch
    // occurred during Phase B.
    //
    // Validate generation BEFORE acquiring store lock — if the scope changed
    // during Phase B we must terminate any successfully-spawned children and
    // abort rather than inserting stale-scope processes into the runtime map.
    if let Err(stale_msg) = crate::managed_agents::scope::validate_scope_generation(&scope) {
        // Scope changed mid-restore — terminate all children we spawned and
        // remove their receipts. The new scope's own restore pass will spawn
        // the correct agents.
        for (pubkey, outcome) in &spawn_results {
            if let SpawnOutcome::Spawned(ref key, ref process) = *outcome {
                eprintln!(
                    "buzz-desktop: restore: {stale_msg}; terminating stale child for {pubkey}"
                );
                let _ = super::terminate_process(process.child.id());
                super::remove_agent_runtime_receipt(app, key);
            }
        }
        return Ok(());
    }
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents_at(&definitions_dir)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;

    let mut successfully_spawned: Vec<(String, String)> = Vec::new();

    for (pubkey, outcome) in spawn_results {
        match outcome {
            // Skipped means a concurrent reconcile already owns a live child for
            // this pair; leave its runtime and record state untouched.
            SpawnOutcome::Skipped => continue,
            SpawnOutcome::Spawned(key, mut process) => {
                let Ok(record) = find_managed_agent_mut(&mut records, &pubkey) else {
                    // Record was deleted between Phase B and Phase C — terminate
                    // the spawned child and remove its receipt to avoid a leaked
                    // process with no record to track it.
                    eprintln!(
                        "buzz-desktop: restore: record for {} was deleted during spawn; \
                         terminating stale child",
                        pubkey
                    );
                    let _ = super::terminate_process(process.child.id());
                    super::remove_agent_runtime_receipt(app, &key);
                    continue;
                };
                let now = util::now_iso();
                let receipt = super::ManagedAgentRuntimeReceipt {
                    key: key.clone(),
                    pid: process.child.id(),
                    desktop_instance_id: super::current_instance_id(app),
                    started_at: now.clone(),
                };
                if let Err(error) = super::write_agent_runtime_receipt(app, &receipt) {
                    let _ = super::terminate_process(process.child.id());
                    let _ = process.child.wait();
                    record.updated_at = now;
                    record.last_error = Some(error);
                    continue;
                }
                record.updated_at = now.clone();
                record.runtime_pid = None;
                record.last_started_at = Some(now);
                record.last_stopped_at = None;
                record.last_exit_code = None;
                record.last_error = None;
                runtimes.insert(
                    key.clone(),
                    super::ManagedAgentPairRuntime::starting(
                        *process,
                        Some(scope.scope_id.clone()),
                    ),
                );
                // Carry the spawn key's relay into profile reconciliation so
                // the background task queries/publishes on the relay this
                // spawn was actually keyed to — not whatever workspace is
                // active when the task eventually executes.
                successfully_spawned.push((pubkey, key.relay_url.clone()));
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
    let reconcile_personas = load_personas_at(&definitions_dir).unwrap_or_default();
    let reconcile_items: Vec<(String, crate::commands::ProfileReconcileData)> =
        successfully_spawned
            .iter()
            .filter_map(|(pubkey, spawn_relay)| {
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
                        // Pin the relay this spawn was keyed to (see the
                        // successfully_spawned push above) so the deferred
                        // task cannot resolve a post-switch workspace.
                        target_relay_url: Some(spawn_relay.clone()),
                        avatar_url: record.avatar_url.clone(),
                        auth_tag: record.auth_tag.clone(),
                        pubkey: record.pubkey.clone(),
                        agent_command: effective_command,
                        persona_id: record.persona_id.clone(),
                    },
                ))
            })
            .collect();

    save_managed_agents_at(&definitions_dir, &records)?;
    drop(runtimes);
    drop(_store_guard);
    drop(restore_transition);

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
    _app: &tauri::AppHandle,
    state: &AppState,
    pubkey: &str,
    definitions_dir: &std::path::Path,
    error: String,
) -> Result<(), String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents_at(definitions_dir)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.updated_at = util::now_iso();
    record.last_error = Some(error);
    save_managed_agents_at(definitions_dir, &records)
}
