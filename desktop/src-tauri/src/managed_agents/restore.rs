use super::{
    find_managed_agent_mut, kill_stale_tracked_processes, load_managed_agents, load_personas,
    save_managed_agents, spawn_agent_child, sync_managed_agent_processes, BackendKind,
};
use crate::app_state::AppState;
use crate::util;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::Manager;

const RESTORE_SPAWN_STAGGER: Duration = Duration::from_secs(1);
const RESTORE_SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Pace restore candidates without blocking a Tokio worker.
///
/// The spawn closure owns the heavyweight AppHandle/relay/process work. Keeping
/// only ordering, pacing, and shutdown policy here makes the scheduler directly
/// testable and ensures no synchronous runtime-transition guard crosses an
/// await point.
async fn run_serial_restore_schedule<T, R, E, Shutdown, Spawn, SpawnFuture>(
    candidates: &[T],
    stagger: Duration,
    mut shutdown_started: Shutdown,
    mut spawn: Spawn,
) -> Result<Vec<R>, E>
where
    Shutdown: FnMut() -> bool,
    Spawn: FnMut(&T) -> SpawnFuture,
    SpawnFuture: Future<Output = Result<R, E>>,
{
    let mut results = Vec::with_capacity(candidates.len());

    for (position, candidate) in candidates.iter().enumerate() {
        if shutdown_started() {
            break;
        }

        if position > 0 {
            let deadline = tokio::time::Instant::now() + stagger;
            loop {
                if shutdown_started() {
                    return Ok(results);
                }
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    break;
                }
                tokio::time::sleep(
                    deadline
                        .saturating_duration_since(now)
                        .min(RESTORE_SHUTDOWN_POLL_INTERVAL),
                )
                .await;
            }
            if shutdown_started() {
                break;
            }
        }

        results.push(spawn(candidate).await?);
    }

    Ok(results)
}

/// Spawn and register one restore candidate as a single synchronous runtime
/// transition. The scheduler awaits only between calls, so shutdown can never
/// observe a spawned-but-untracked child.
fn restore_one_candidate(
    app: &tauri::AppHandle,
    state: &AppState,
    candidate: &super::ManagedAgentRecord,
    owner_hex: Option<&str>,
    shutdown_started: &AtomicBool,
) -> Result<Option<(String, String)>, String> {
    let _restore_transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(None);
    }

    // Phase A is only a launch-order snapshot. Reload under the same locks used
    // by record mutations and runtime transitions so a user disabling,
    // deleting, or manually starting this agent during a stagger wins over the
    // stale candidate. Keep the store lock through spawn and registration,
    // matching `start_pair`, so the revalidated record cannot change midway.
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let Some(record_index) = records
        .iter()
        .position(|record| record.pubkey == candidate.pubkey)
    else {
        return Ok(None);
    };
    let record = records[record_index].clone();
    if !record.start_on_app_launch || record.backend != BackendKind::Local {
        return Ok(None);
    }
    if record.runtime_pid.is_some_and(super::process_is_running) {
        return Ok(None);
    }

    let workspace_relay = crate::relay::relay_ws_url_with_override(&app.state::<AppState>());
    let relay_url = crate::relay::effective_agent_relay_url(&record.relay_url, &workspace_relay);
    let key = match super::ManagedAgentRuntimeKey::new(record.pubkey.clone(), &relay_url) {
        Ok(key) => key,
        Err(error) => {
            records[record_index].updated_at = util::now_iso();
            records[record_index].last_error = Some(error);
            save_managed_agents(app, &records)?;
            return Ok(None);
        }
    };
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    if runtimes
        .get_mut(&key)
        .is_some_and(|runtime| runtime.child.try_wait().ok().flatten().is_none())
    {
        return Ok(None);
    }
    runtimes.remove(&key);

    let mut process = match super::terminate_untracked_pair_runtime(app, &key).and_then(|()| {
        // Restore spawns lazy, matching reconcile and manual start. Eager
        // restore would silently reintroduce N idle brains on every launch.
        spawn_agent_child(app, &record, &key.relay_url, true, owner_hex)
    }) {
        Ok(process) => process,
        Err(error) => {
            records[record_index].updated_at = util::now_iso();
            records[record_index].last_error = Some(error);
            save_managed_agents(app, &records)?;
            return Ok(None);
        }
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
        records[record_index].updated_at = now;
        records[record_index].last_error = Some(error);
        save_managed_agents(app, &records)?;
        return Ok(None);
    }

    let stored_record = &mut records[record_index];
    stored_record.updated_at = now.clone();
    stored_record.runtime_pid = None;
    stored_record.last_started_at = Some(now);
    stored_record.last_stopped_at = None;
    stored_record.last_exit_code = None;
    stored_record.last_error = None;
    runtimes.insert(
        key.clone(),
        super::ManagedAgentPairRuntime::starting(process),
    );
    save_managed_agents(app, &records)?;
    Ok(Some((record.pubkey, key.relay_url)))
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
/// Split into three phases to keep pacing asynchronous without sacrificing
/// runtime atomicity:
///   A (under lock): snapshot eligible restore candidates
///   B (paced):      revalidate, spawn, and register each candidate atomically
///   C (under lock): collect profile reconciliation data
pub async fn restore_managed_agents_on_launch(
    app: &tauri::AppHandle,
    shutdown_started: &AtomicBool,
) -> Result<(), String> {
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(());
    }

    let state = app.state::<AppState>();

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
                // record as-is — Phase B revalidates and `spawn_agent_child`
                // persists any refusal to `last_error`.
                continue;
            };
            super::persona_events::apply_persona_snapshot(record, persona);
            record.updated_at = util::now_iso();
            changed = true;
        }
        // Re-collect to_start so Phase B receives the refreshed config snapshot.
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
    // Read outside the per-agent spawn loop so every serial spawn sees the same
    // value and we don't lock `state.keys` repeatedly.
    let owner_hex: Option<String> = state
        .keys
        .lock()
        .map_err(|e| e.to_string())
        .ok()
        .map(|k| k.public_key().to_hex());

    #[cfg(feature = "mesh-llm")]
    let agents_to_start = {
        // Preflight against the same resolution spawn uses — `resolve_effective_config`
        // (definition → global fallback). A linked instance's own `provider`/`model`/
        // `relay_mesh` bytes never contribute. See `start_local_agent_with_preflight`
        // in `commands/agents.rs` for the identical rationale on the interactive path.
        let personas = load_personas(app).unwrap_or_default();
        let global = super::load_global_agent_config(app).unwrap_or_default();
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
                persist_restore_error(app, &state, &record.pubkey, error)?;
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

    // ── Phase B: pace serial spawn-and-register transactions ────────────────
    // Launch restore previously spawned every harness concurrently. Even though
    // restored harnesses are lazy, their first wakes can cluster and reproduce
    // the same MCP-heavy initialization contention. Keep one deterministic
    // spawn lane and add a small bounded stagger between candidates. The
    // initialize-only 300s budget + retry in spawn_agent_child is the hard
    // safety net; this stagger simply avoids creating the burst in the first
    // place.
    let owner_hex_ref = owner_hex.as_deref();
    let successfully_spawned = run_serial_restore_schedule(
        &agents_to_start,
        RESTORE_SPAWN_STAGGER,
        || shutdown_started.load(Ordering::SeqCst),
        |record| {
            std::future::ready(restore_one_candidate(
                app,
                &state,
                record,
                owner_hex_ref,
                shutdown_started,
            ))
        },
    )
    .await?
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    if successfully_spawned.is_empty() {
        return Ok(());
    }

    // ── Phase C: collect profile reconciliation data ────────────────────────
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let records = load_managed_agents(app)?;

    // Collect profile reconciliation data for successfully spawned agents before
    // releasing the lock. This mirrors the fire-and-forget pattern in
    // start_managed_agent — ensuring boot-restored agents get the same profile
    // self-healing as UI-started agents.
    let reconcile_personas = super::load_personas(app).unwrap_or_default();
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

    drop(_store_guard);

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
mod restore_schedule_tests {
    use super::run_serial_restore_schedule;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    use std::time::Duration;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn preserves_input_order_and_awaits_exactly_n_minus_one_staggers() {
        let candidates = [3u8, 1, 2];
        let observed = Arc::new(Mutex::new(Vec::new()));
        let observed_by_spawn = Arc::clone(&observed);
        let started = tokio::time::Instant::now();

        let results = run_serial_restore_schedule(
            &candidates,
            Duration::from_millis(10),
            || false,
            move |candidate| {
                observed_by_spawn.lock().unwrap().push(*candidate);
                std::future::ready(Ok::<u8, ()>(*candidate))
            },
        )
        .await
        .unwrap();

        assert_eq!(results, candidates);
        assert_eq!(*observed.lock().unwrap(), candidates);
        assert_eq!(
            tokio::time::Instant::now().duration_since(started),
            Duration::from_millis(20),
            "three candidates require two awaited staggers and no leading delay"
        );
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn shutdown_before_scheduling_spawns_nothing() {
        let shutdown = AtomicBool::new(true);
        let results = run_serial_restore_schedule(
            &[1u8, 2, 3],
            Duration::from_millis(10),
            || shutdown.load(Ordering::SeqCst),
            |candidate| std::future::ready(Ok::<u8, ()>(*candidate)),
        )
        .await
        .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn shutdown_during_stagger_leaves_remaining_candidates_unspawned() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_task = Arc::clone(&shutdown);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            shutdown_for_task.store(true, Ordering::SeqCst);
        });

        let results = run_serial_restore_schedule(
            &[1u8, 2, 3],
            Duration::from_millis(10),
            || shutdown.load(Ordering::SeqCst),
            |candidate| std::future::ready(Ok::<u8, ()>(*candidate)),
        )
        .await
        .unwrap();

        assert_eq!(results, [1]);
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
