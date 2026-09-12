use tauri::Manager;

use crate::app_state::AppState;
use crate::managed_agents::{
    self, kill_stale_tracked_processes, load_managed_agents, save_managed_agents,
    sync_managed_agent_processes, BackendKind,
};
use crate::{prevent_sleep, util};

const MANAGED_AGENT_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const MANAGED_AGENT_SHUTDOWN_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(50);

#[cfg(unix)]
fn managed_agent_leader_exited(pid: u32, owns_child: bool) -> bool {
    if owns_child && buzz_terminal::lifecycle::child_exited_without_reaping(pid) {
        return true;
    }

    !managed_agents::process_is_running(pid)
}

#[cfg(not(unix))]
fn managed_agent_leader_exited(pid: u32, _owns_child: bool) -> bool {
    !managed_agents::process_is_running(pid)
}

fn wait_for_managed_agent_shutdown_grace(mut all_leaders_exited: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + MANAGED_AGENT_SHUTDOWN_TIMEOUT;

    loop {
        if all_leaders_exited() {
            #[cfg(unix)]
            {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                std::thread::sleep(buzz_terminal::lifecycle::TERM_GRACE.min(remaining));
            }
            break;
        }

        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        std::thread::sleep(MANAGED_AGENT_SHUTDOWN_POLL_INTERVAL.min(remaining));
    }
}

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
        crate::observed_unread::flush(app);
        crate::channel_head_cache::flush(app);
        app.state::<crate::terminal_runtime::TerminalSessions>()
            .shutdown_all();
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
            app.state::<crate::terminal_runtime::TerminalSessions>()
                .shutdown_all();
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

        // Wait up to 2s for all leaders to exit. `kill(pid, 0)` alone cannot
        // detect an exited direct child: it continues to report a zombie as
        // present until that child is reaped. Observe owned children with
        // waitid(WNOWAIT) instead, keeping their PIDs reserved until the group
        // sweep below has killed any surviving descendants. Once every leader
        // is out, retain those zombies for one short descendant drain window;
        // the original two-second deadline remains the hard ceiling.
        wait_for_managed_agent_shutdown_grace(|| {
            to_stop
                .iter()
                .all(|agent| managed_agent_leader_exited(agent.pid, agent.runtime.is_some()))
        });

        // Fan-out: SIGKILL surviving groups even when their leaders exited
        // politely. A WNOWAIT-observed leader remains a zombie, so kill(0)
        // still holds the group ID safe while this catches descendants that
        // did not honor SIGTERM.
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

    #[cfg(unix)]
    use super::{managed_agent_leader_exited, wait_for_managed_agent_shutdown_grace};

    #[cfg(unix)]
    struct ProcessGroupGuard {
        leader: std::process::Child,
        pgid: i32,
        group_killed: bool,
    }

    #[cfg(unix)]
    impl ProcessGroupGuard {
        fn signal(&self, signal: i32) -> i32 {
            // SAFETY: the fixture leader was spawned with process_group(0), so
            // its PID is the PGID and the negative target names only that group.
            unsafe { libc::kill(-self.pgid, signal) }
        }

        fn kill_group(&mut self) -> i32 {
            let result = self.signal(libc::SIGKILL);
            if result == 0 {
                self.group_killed = true;
            }
            result
        }
    }

    #[cfg(unix)]
    impl Drop for ProcessGroupGuard {
        fn drop(&mut self) {
            if !self.group_killed {
                let _ = self.kill_group();
            }
            let _ = self.leader.wait();
        }
    }

    #[test]
    fn only_tauri_restart_exit_code_requests_a_relaunch() {
        assert!(is_restart_request(Some(tauri::RESTART_EXIT_CODE)));
        assert!(!is_restart_request(None));
        assert!(!is_restart_request(Some(0)));
    }

    #[cfg(unix)]
    #[test]
    fn exited_agent_leaders_are_observed_before_reaping() {
        use std::process::{Command, Stdio};

        let mut children: Vec<_> = (0..3)
            .map(|_| {
                Command::new("/bin/sh")
                    .args(["-c", "exit 0"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("spawn cooperative agent fixture")
            })
            .collect();
        let pids: Vec<_> = children.iter().map(std::process::Child::id).collect();

        wait_for_managed_agent_shutdown_grace(|| {
            pids.iter()
                .all(|pid| managed_agent_leader_exited(*pid, true))
        });

        assert!(
            pids.iter()
                .all(|pid| managed_agent_leader_exited(*pid, true)),
            "cooperative leaders should be detected without reaching the shutdown grace ceiling"
        );
        assert!(
            pids.iter()
                .all(|pid| crate::managed_agents::process_is_running(*pid)),
            "the exit probe must leave child PIDs reserved until group cleanup"
        );

        for child in &mut children {
            assert!(
                child
                    .try_wait()
                    .expect("reap cooperative agent fixture")
                    .is_some(),
                "the production try_wait step must reap a WNOWAIT-observed child"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn exited_leader_leaves_a_short_term_grace_for_group_descendants() {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let temp = tempfile::tempdir().expect("create process-group fixture directory");
        let descendant_ready = temp.path().join("descendant-ready");
        let descendant_pid_path = temp.path().join("descendant-pid");
        let leader_ready = temp.path().join("leader-ready");
        let drained = temp.path().join("descendant-drained");

        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                r#"
trap 'exit 0' TERM
(
  trap '/bin/sleep 0.1; /usr/bin/touch "$BUZZ_TEST_DRAINED"; while :; do /bin/sleep 1; done' TERM
  /usr/bin/touch "$BUZZ_TEST_DESCENDANT_READY"
  while :; do /bin/sleep 1; done
) &
printf '%s\n' "$!" > "$BUZZ_TEST_DESCENDANT_PID"
while [ ! -f "$BUZZ_TEST_DESCENDANT_READY" ]; do /bin/sleep 0.01; done
/usr/bin/touch "$BUZZ_TEST_LEADER_READY"
while :; do /bin/sleep 1; done
"#,
            ])
            .env("BUZZ_TEST_DESCENDANT_READY", &descendant_ready)
            .env("BUZZ_TEST_DESCENDANT_PID", &descendant_pid_path)
            .env("BUZZ_TEST_LEADER_READY", &leader_ready)
            .env("BUZZ_TEST_DRAINED", &drained)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);

        let leader = command.spawn().expect("spawn process-group fixture");
        let leader_pid = leader.id();
        let mut group = ProcessGroupGuard {
            leader,
            pgid: leader_pid as i32,
            group_killed: false,
        };

        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while !leader_ready.exists() && Instant::now() < ready_deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(leader_ready.exists(), "fixture did not arm its TERM traps");

        let descendant_pid: i32 = std::fs::read_to_string(&descendant_pid_path)
            .expect("read descendant PID")
            .trim()
            .parse()
            .expect("parse descendant PID");
        // SAFETY: getpgid only inspects the live fixture PID.
        assert_eq!(
            unsafe { libc::getpgid(descendant_pid) },
            leader_pid as i32,
            "fixture descendant must share the harness process group"
        );

        assert_eq!(group.signal(libc::SIGTERM), 0, "signal fixture group");
        let started = Instant::now();
        wait_for_managed_agent_shutdown_grace(|| managed_agent_leader_exited(leader_pid, true));
        let elapsed = started.elapsed();

        assert!(
            crate::managed_agents::process_is_running(leader_pid),
            "WNOWAIT must reserve the leader PID through descendant cleanup"
        );
        assert_eq!(group.kill_group(), 0, "sweep fixture group");
        assert!(
            group
                .leader
                .try_wait()
                .expect("reap fixture leader")
                .is_some(),
            "the exited leader should be waitable after the group sweep"
        );
        assert!(
            drained.exists(),
            "same-group descendants should finish their TERM handler before SIGKILL"
        );
        assert!(
            elapsed >= buzz_terminal::lifecycle::TERM_GRACE,
            "the post-leader descendant grace must not be skipped"
        );
    }
}
