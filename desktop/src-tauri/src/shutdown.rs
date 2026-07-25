use tauri::Manager;

use crate::app_state::AppState;
use crate::managed_agents::{
    self, kill_stale_tracked_processes, load_managed_agents, save_managed_agents,
    sync_managed_agent_processes, BackendKind,
};
use crate::{prevent_sleep, util};

pub(crate) fn is_restart_request(code: Option<i32>) -> bool {
    code == Some(tauri::RESTART_EXIT_CODE)
}

pub(crate) fn shut_down_app(app: &tauri::AppHandle, shutdown_done: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering;

    app.state::<AppState>()
        .shutdown_started
        .store(true, Ordering::SeqCst);
    if !shutdown_done.swap(true, Ordering::SeqCst) {
        prevent_sleep::release(&app.state::<AppState>().prevent_sleep);
        if let Err(error) = shutdown_managed_agents(app) {
            eprintln!("buzz-desktop: failed to stop managed agents: {error}");
        }
        #[cfg(feature = "mesh-llm")]
        shutdown_mesh_runtime(app);
    }
}

/// Install SIGINT/SIGTERM/SIGHUP cleanup on ctrlc's dedicated handler thread.
#[cfg(unix)]
pub(crate) fn install_signal_handler(
    app: tauri::AppHandle,
    shutdown_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;

    if let Err(error) = ctrlc::set_handler(move || {
        app.state::<AppState>()
            .shutdown_started
            .store(true, Ordering::SeqCst);
        if !shutdown_done.swap(true, Ordering::SeqCst) {
            let _ = shutdown_managed_agents(&app);
            #[cfg(feature = "mesh-llm")]
            shutdown_mesh_runtime(&app);
        }
        #[cfg(all(feature = "mesh-llm", target_os = "macos"))]
        hard_exit_after_mesh_shutdown();
        #[cfg(not(all(feature = "mesh-llm", target_os = "macos")))]
        std::process::exit(0);
    }) {
        eprintln!("buzz-desktop: failed to register signal handler: {error}");
    }
}

#[cfg(all(feature = "mesh-llm", target_os = "macos"))]
fn updated_macos_binary(current_binary: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos_directory = current_binary.parent()?;
    if macos_directory.file_name()? != "MacOS" {
        return None;
    }
    let contents_directory = macos_directory.parent()?;
    if contents_directory.file_name()? != "Contents" {
        return None;
    }
    let info_plist =
        plist::from_file::<_, plist::Dictionary>(contents_directory.join("Info.plist")).ok()?;
    let binary_name = info_plist.get("CFBundleExecutable")?.as_string()?;
    Some(macos_directory.join(binary_name))
}

#[cfg(all(feature = "mesh-llm", target_os = "macos"))]
pub(crate) fn relaunch_after_mesh_shutdown(app: &tauri::AppHandle) -> ! {
    use std::process::Command;

    tauri_plugin_single_instance::destroy(app);
    let env = app.env();
    match tauri::process::current_binary(&env) {
        Ok(current_binary) => {
            let binary = updated_macos_binary(&current_binary).unwrap_or(current_binary);
            if let Err(error) = Command::new(binary)
                .args(env.args_os.iter().skip(1))
                .spawn()
            {
                eprintln!("buzz-desktop: failed to relaunch app: {error}");
            }
        }
        Err(error) => eprintln!("buzz-desktop: failed to locate app for relaunch: {error}"),
    }
    hard_exit_after_mesh_shutdown();
}

#[cfg(all(feature = "mesh-llm", target_os = "macos"))]
pub(crate) fn hard_exit_after_mesh_shutdown() -> ! {
    // SAFETY: all Buzz-managed subprocesses and the embedded Mesh runtime have
    // been stopped. `_exit` intentionally skips only process-global C++
    // destructors and buffered stdio; no application state remains observable.
    unsafe { libc::_exit(0) }
}

#[cfg(feature = "mesh-llm")]
pub(crate) fn shutdown_mesh_runtime(app: &tauri::AppHandle) {
    let app = app.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let runtime = state.mesh_llm_runtime.lock().await.take();
        let result = match runtime {
            Some(runtime) => runtime.stop().await,
            None => Ok(()),
        };
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("buzz-desktop: failed to stop Mesh runtime: {error}"),
        Err(error) => eprintln!("buzz-desktop: timed out stopping Mesh runtime: {error}"),
    }
}

pub(crate) fn shutdown_managed_agents(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _restore_transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    let (mut changed, _exited) = sync_managed_agent_processes(
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
        runtime: Option<managed_agents::ManagedAgentPairRuntime>,
    }

    let mut to_stop: Vec<AgentToStop> = Vec::new();
    for (idx, record) in records.iter().enumerate() {
        if record.backend != BackendKind::Local {
            continue;
        }
        // Drain every tracked pair for this record, not just the first — an
        // agent can run one harness per community, and each pair gets the
        // graceful SIGTERM → 2s wait → SIGKILL fan-out with a stop log
        // marker, instead of falling through to the orphan sweep's 200ms
        // grace below.
        for key in managed_agents::managed_agent_runtime_keys(&runtimes, &record.pubkey) {
            let runtime = runtimes.remove(&key);
            let Some(pid) = runtime
                .as_ref()
                .map(|rt| rt.child.id())
                .or(record.runtime_pid)
            else {
                continue;
            };
            to_stop.push(AgentToStop { idx, pid, runtime });
        }
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

#[cfg(test)]
mod tests {
    use super::is_restart_request;

    #[test]
    fn only_tauri_restart_exit_code_requests_a_relaunch() {
        assert!(is_restart_request(Some(tauri::RESTART_EXIT_CODE)));
        assert!(!is_restart_request(None));
        assert!(!is_restart_request(Some(0)));
    }
}
