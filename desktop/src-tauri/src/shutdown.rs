use tauri::Manager;

use crate::app_state::AppState;
use crate::managed_agents::{
    self, kill_stale_tracked_processes, load_managed_agents, save_managed_agents,
    sync_managed_agent_processes, BackendKind, ManagedAgentProcess,
};
use crate::util;

pub(crate) fn shutdown_managed_agents(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    let mut changed = sync_managed_agent_processes(
        &mut records,
        &mut runtimes,
        &managed_agents::current_instance_id(app),
    );
    changed |= kill_stale_tracked_processes(
        &mut records,
        &runtimes,
        &managed_agents::current_instance_id(app),
    );

    // Stop all tracked agents. Send SIGTERM to all process
    // groups first, then wait for exits in parallel to avoid serial 1s waits.
    struct AgentToStop {
        idx: usize,
        pid: u32,
        runtime: Option<ManagedAgentProcess>,
    }

    let mut to_stop: Vec<AgentToStop> = Vec::new();
    for (idx, record) in records.iter_mut().enumerate() {
        if record.backend != BackendKind::Local {
            continue;
        }
        if record.runtime_pid.is_none() && !runtimes.contains_key(&record.pubkey) {
            continue;
        }
        let runtime = runtimes.remove(&record.pubkey);
        let Some(pid) = runtime
            .as_ref()
            .map(|rt| rt.child.id())
            .or(record.runtime_pid)
        else {
            continue;
        };
        to_stop.push(AgentToStop { idx, pid, runtime });
    }

    if !to_stop.is_empty() {
        changed = true;

        // Fan-out: send SIGTERM to all process groups at once.
        #[cfg(unix)]
        for agent in &to_stop {
            let pgid = -(agent.pid as i32);
            unsafe {
                libc::kill(pgid, libc::SIGTERM);
            }
        }

        // Wait up to 2s for all to exit, checking in a polling loop.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if to_stop
                .iter()
                .all(|a| !managed_agents::process_is_running(a.pid))
            {
                break;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        // Fan-out: SIGKILL any survivors.
        #[cfg(unix)]
        for agent in &to_stop {
            if managed_agents::process_is_running(agent.pid) {
                let pgid = -(agent.pid as i32);
                unsafe {
                    libc::kill(pgid, libc::SIGKILL);
                }
            }
        }

        // Reap children and update records.
        for mut agent in to_stop {
            if let Some(ref mut rt) = agent.runtime {
                // Best-effort reap — don’t block shutdown if the child is stuck
                // in uninterruptible sleep. The zombie will be cleaned up when
                // our process exits and launchd reaps it.
                let _ = rt.child.try_wait();
                // Write log marker (best-effort).
                let record = &records[agent.idx];
                let _ = managed_agents::append_log_marker(
                    &rt.log_path,
                    &format!(
                        "=== stopped {} ({}) at {} ===",
                        record.name,
                        record.pubkey,
                        util::now_iso()
                    ),
                );
            }
            let record = &mut records[agent.idx];
            record.runtime_pid = None;
            record.last_stopped_at = Some(util::now_iso());
            record.updated_at = util::now_iso();
            record.last_exit_code = None;
            record.last_error = None;
        }
    }

    // Final sweep: kill any orphaned agent processes we have PID file receipts
    // for that escaped process-group kills or weren't tracked in records.
    // All tracked PIDs have already been killed above, so pass an empty skip list.
    managed_agents::sweep_orphaned_agent_processes(app, &[]);

    // System-wide sweep: agent workers (goose, buzz-agent, etc.) are spawned
    // in their own process groups by buzz-acp, so group-kills above only
    // reach the harness, not the workers. Scan all user processes and kill any
    // known agent binaries that are still running.
    managed_agents::sweep_system_agent_processes(&managed_agents::current_instance_id(app), &[]);

    // Dead-instance reaping: find agents belonging to Buzz instances
    // whose desktop process is no longer running and reap them.
    managed_agents::reap_dead_instance_agents(&managed_agents::current_instance_id(app), &[]);

    if changed {
        save_managed_agents(app, &records)?;
    }

    Ok(())
}
