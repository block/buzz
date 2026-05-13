use super::{
    find_managed_agent_mut, kill_stale_tracked_processes, load_managed_agents, save_managed_agents,
    spawn_agent_child, sync_managed_agent_processes, BackendKind, ManagedAgentProcess,
};
use crate::app_state::AppState;
use crate::util;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// Restore managed agents that were running before the app was closed.
///
/// Split into three phases to minimise lock contention with the frontend:
///   A (under lock): sync process state, cleanup, collect agents to start
///   B (no locks):   resolve commands and spawn processes in parallel
///   C (re-lock):    write back PIDs and status to records on disk
pub fn restore_managed_agents_on_launch(
    app: &tauri::AppHandle,
    shutdown_started: &AtomicBool,
) -> Result<(), String> {
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(());
    }

    let state = app.state::<AppState>();

    // ── Phase A (under lock): housekeeping + collect agents to restore ──
    let agents_to_start: Vec<super::ManagedAgentRecord>;
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
        let mut changed = sync_managed_agent_processes(&mut records, &mut runtimes);
        changed |= kill_stale_tracked_processes(&mut records, &runtimes);

        let tracked_pids: Vec<u32> = records
            .iter()
            .filter_map(|r| r.runtime_pid)
            .chain(runtimes.values().map(|rt| rt.child.id()))
            .collect();
        super::sweep_orphaned_agent_processes(app, &tracked_pids);

        let candidates: Vec<String> = records
            .iter()
            .filter(|record| record.start_on_app_launch && record.backend == BackendKind::Local)
            .map(|record| record.pubkey.clone())
            .collect();

        let mut to_start = Vec::new();
        for pubkey in &candidates {
            if let Some(runtime) = runtimes.get_mut(pubkey) {
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

    // ── Phase B (no locks): resolve commands and spawn processes in parallel ──
    let spawn_results: Vec<(
        String,
        Result<(std::process::Child, std::path::PathBuf), String>,
    )> = std::thread::scope(|scope| {
        let owner_hex_ref = owner_hex.as_deref();
        let handles: Vec<_> = agents_to_start
            .iter()
            .filter(|_| !shutdown_started.load(Ordering::SeqCst))
            .map(|record| {
                let pubkey = record.pubkey.clone();
                let handle = scope.spawn(move || {
                    let result = spawn_agent_child(app, record, owner_hex_ref);
                    (pubkey, result)
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

    for (pubkey, result) in spawn_results {
        let record = match find_managed_agent_mut(&mut records, &pubkey) {
            Ok(r) => r,
            Err(_) => continue,
        };
        match result {
            Ok((child, log_path)) => {
                let now = util::now_iso();
                record.updated_at = now.clone();
                record.runtime_pid = Some(child.id());
                record.last_started_at = Some(now);
                record.last_stopped_at = None;
                record.last_exit_code = None;
                record.last_error = None;
                runtimes.insert(pubkey, ManagedAgentProcess { child, log_path });
            }
            Err(error) => {
                record.updated_at = util::now_iso();
                record.last_error = Some(error);
            }
        }
    }

    save_managed_agents(app, &records)?;

    Ok(())
}
