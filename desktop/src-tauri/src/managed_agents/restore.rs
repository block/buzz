use super::{
    bestie_assignment::recover_pending_assignment_cleanup, find_managed_agent_mut,
    kill_stale_tracked_processes, load_managed_agents, load_personas, managed_agents_base_dir,
    save_managed_agents, spawn_agent_child, sync_managed_agent_processes, BackendKind,
    ManagedAgentProcess,
};
use crate::app_state::AppState;
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

/// Schedule launch restore: captures admission now, at scheduling, and
/// returns the deferred restore that runs under that snapshot. A community
/// removed after this call refuses the restore even if it is re-added before
/// the task runs. `sweeps` is Phase A's live process sweeps.
pub fn launch_restore_task<R, S>(
    app: tauri::AppHandle<R>,
    sweeps: S,
) -> impl std::future::Future<Output = Result<(), String>>
where
    R: tauri::Runtime,
    S: FnOnce(&tauri::AppHandle<R>, &[u32]),
{
    let admission = super::AdmissionSnapshot::capture(&app.state::<AppState>());
    async move {
        let state = app.state::<AppState>();
        restore_managed_agents_on_launch(&app, &state.shutdown_started, admission, sweeps).await
    }
}

/// Phase A's sweeps of live, untracked agent processes, skipping `tracked_pids`.
pub fn live_process_sweeps(app: &tauri::AppHandle, tracked_pids: &[u32]) {
    super::sweep_orphaned_agent_processes(app, tracked_pids);

    // System-wide sweep: enumerate all user processes and kill any known
    // agent binaries not tracked by this session. Catches orphans whose
    // PID files were already cleaned up (e.g. agent workers in their own
    // process group whose parent harness exited).
    super::sweep_system_agent_processes(&super::current_instance_id(app), tracked_pids);

    // Dead-instance reaping: find agents belonging to Buzz instances
    // whose desktop process is no longer running and reap them.
    super::reap_dead_instance_agents(&super::current_instance_id(app), tracked_pids);

    // Exact-path sweep: kill any buzz-acp process whose executable path
    // matches this bundle's harness binary but is not in the tracked set.
    // Complements the env-var sweep above — catches orphans that predate
    // BUZZ_MANAGED_AGENT injection or lost their PID-file receipt.
    //
    // TODO: the three sweeps above each walk the PID table independently.
    // A future consolidation should collect a single shared process snapshot
    // at the top of this block and thread it through all sweep functions,
    // replacing the three separate kernel enumerations.
    super::sweep_untracked_bundle_harnesses(tracked_pids);
}

/// Restore managed agents that were running before the app was closed.
///
/// Split into three phases to minimise lock contention with the frontend:
///   A (under lock): sync process state, cleanup, collect agents to start
///   B (no locks):   resolve commands and spawn processes in parallel
///   C (re-lock):    write back PIDs and status to records on disk
async fn restore_managed_agents_on_launch<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    shutdown_started: &AtomicBool,
    admission: super::AdmissionSnapshot,
    sweeps: impl FnOnce(&tauri::AppHandle<R>, &[u32]),
) -> Result<(), String> {
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(());
    }

    let state = app.state::<AppState>();
    // `apply_workspace` still holds the apply lock for this task, so the relay
    // cannot change underneath it; pin it for the spawn loop.
    let restore_relay = crate::relay::relay_ws_url_with_override(&state);
    let restore_relay = restore_relay.as_str();

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
        recover_pending_assignment_cleanup(&managed_agents_base_dir(app)?, |pending_pubkey| {
            records
                .iter()
                .any(|record| record.pubkey.eq_ignore_ascii_case(pending_pubkey))
        })?;
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
        sweeps(app, &tracked_pids);

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

    let reconcile_items = spawn_and_register_restored_agents(
        app,
        shutdown_started,
        &admission,
        restore_relay,
        &agents_to_start,
        owner_hex.as_deref(),
    )?;

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

/// Phases B and C of launch restore: spawn `agents_to_start` on `restore_relay`
/// if it is still admitted under the snapshot `apply_workspace` scheduled the
/// restore with, then register them. Split from the sweeps above so tests can
/// drive the real admission and registration without sweeping live processes.
/// Returns the profile reconciliations to run for the agents it registered.
fn spawn_and_register_restored_agents<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    shutdown_started: &AtomicBool,
    admission: &super::AdmissionSnapshot,
    restore_relay: &str,
    agents_to_start: &[super::ManagedAgentRecord],
    owner_hex: Option<&str>,
) -> Result<Vec<(String, crate::commands::ProfileReconcileData)>, String> {
    let state = app.state::<AppState>();
    // Serialize spawning and runtime registration with shutdown cleanup. The
    // shutdown flag is rechecked after taking the lock so shutdown either
    // prevents this transition or waits until every child is tracked and can
    // be terminated.
    // ── Phase B (transition lock held): resolve commands and spawn in parallel ──
    let spawned = spawn_if_admitted(
        &state,
        shutdown_started,
        admission,
        restore_relay,
        |admitted| {
            std::thread::scope(|scope| {
                let owner_hex_ref = owner_hex;
                let handles: Vec<_> = agents_to_start
                    .iter()
                    .filter(|_| !shutdown_started.load(Ordering::SeqCst))
                    .map(|record| {
                        let handle = scope.spawn(move || {
                            let relay_url = crate::relay::effective_agent_relay_url(
                                &record.relay_url,
                                restore_relay,
                            );
                            let outcome = match super::ManagedAgentRuntimeKey::new(
                                record.pubkey.clone(),
                                &relay_url,
                            ) {
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
                                                    admitted,
                                                    true,
                                                    owner_hex_ref,
                                                    None,
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

                handles
                    .into_iter()
                    .map(|h| h.join().unwrap())
                    .collect::<Vec<AgentSpawnResult>>()
            })
        },
    )?;
    let Some((restore_transition, spawn_results)) = spawned else {
        return Ok(Vec::new());
    };

    if spawn_results.is_empty() {
        return Ok(Vec::new());
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

    let mut successfully_spawned: Vec<(String, String)> = Vec::new();

    for (pubkey, outcome) in spawn_results {
        match outcome {
            // Skipped means a concurrent reconcile already owns a live child for
            // this pair; leave its runtime and record state untouched.
            SpawnOutcome::Skipped => continue,
            SpawnOutcome::Spawned(key, mut process) => {
                let Ok(record) = find_managed_agent_mut(&mut records, &pubkey) else {
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
                    super::ManagedAgentPairRuntime::starting(*process),
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
                        about: crate::managed_agents::record_effective_description(
                            record,
                            &reconcile_personas,
                        ),
                    },
                ))
            })
            .collect();

    save_managed_agents(app, &records)?;
    drop(runtimes);
    drop(_store_guard);
    drop(restore_transition);

    Ok(reconcile_items)
}

fn profile_reconcile_completed(outcome: crate::commands::ProfileReconcileOutcome) -> bool {
    outcome == crate::commands::ProfileReconcileOutcome::Reconciled
}

pub(crate) fn spawn_pending_profile_reconciliations(app: &tauri::AppHandle, workspace_relay: &str) {
    let state = app.state::<AppState>();
    if !state
        .managed_agent_profile_reconcile_enabled()
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

#[cfg(feature = "mesh-llm")]
fn persist_restore_error<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
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

/// Restore's check-then-spawn step. Takes the runtime transition lock and runs
/// `spawn` only if shutdown has not started and `restore_relay` is still
/// admitted under the snapshot `apply_workspace` captured when it scheduled
/// this restore; otherwise spawns nothing and returns `None`. On success the
/// lock is returned still held, so the caller registers the spawned children
/// before shutdown or a removal's stop sweep can run.
fn spawn_if_admitted<'a, T>(
    state: &'a AppState,
    shutdown_started: &AtomicBool,
    admission: &super::AdmissionSnapshot,
    restore_relay: &str,
    spawn: impl FnOnce(&super::Admitted<'_>) -> T,
) -> Result<Option<(std::sync::MutexGuard<'a, super::RelayAdmissions>, T)>, String> {
    let transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let spawned = match transition.admit(admission, restore_relay) {
        Ok(admitted) => spawn(&admitted),
        Err(error) => {
            eprintln!("buzz-desktop: skipping managed agent restore: {error}");
            return Ok(None);
        }
    };
    Ok(Some((transition, spawned)))
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

// `build_app_state()` pulls in native Windows DLLs unavailable on the CI runner.
#[cfg(all(test, not(target_os = "windows")))]
mod launch_restore_admission_tests {
    use super::spawn_if_admitted;
    use crate::app_state::{build_app_state, AppState};
    use crate::managed_agents::AdmissionSnapshot;
    use crate::relay::{relay_api_base_url_with_override, relay_ws_url_with_override};
    use std::cell::Cell;
    use std::sync::atomic::AtomicBool;

    const RELAY: &str = "wss://removed.example";

    /// Runs restore's spawn step with `admission` and returns how many times it spawned.
    fn spawns(state: &AppState, shutdown: &AtomicBool, admission: &AdmissionSnapshot) -> u32 {
        let count = Cell::new(0);
        let gate = spawn_if_admitted(state, shutdown, admission, RELAY, |_| {
            count.set(count.get() + 1)
        });
        assert_eq!(gate.unwrap().is_some(), count.get() == 1);
        count.get()
    }

    fn remove(state: &AppState) {
        crate::managed_agents::remove_relay(state, RELAY).unwrap();
    }

    #[test]
    fn restore_scheduled_before_a_removal_spawns_nothing_even_after_readd() {
        let state = build_app_state();
        let shutdown = AtomicBool::new(false);
        *state.relay_url_override.lock().unwrap() = Some(RELAY.into());
        // What `apply_workspace` captures when it schedules the restore.
        let scheduled = AdmissionSnapshot::capture(&state);

        remove(&state);
        crate::managed_agents::readd_relay(&state, RELAY).unwrap();

        assert_eq!(spawns(&state, &shutdown, &scheduled), 0);
        assert_eq!(relay_ws_url_with_override(&state), RELAY);
        assert_eq!(
            relay_api_base_url_with_override(&state),
            "https://removed.example"
        );
        // A restore scheduled after the re-add spawns normally.
        let fresh = AdmissionSnapshot::capture(&state);
        assert_eq!(spawns(&state, &shutdown, &fresh), 1);
    }

    #[test]
    fn shutdown_spawns_nothing_even_when_admitted() {
        let state = build_app_state();
        let shutdown = AtomicBool::new(true);
        let admission = AdmissionSnapshot::capture(&state);
        assert_eq!(spawns(&state, &shutdown, &admission), 0);
    }
}

#[cfg(all(test, not(target_os = "windows")))]
#[path = "restore_admission_tests.rs"]
mod admission_entry_tests;
