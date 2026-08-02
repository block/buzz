use tauri::Manager;

use crate::app_state::AppState;
use crate::managed_agents::{
    self, kill_stale_tracked_processes, load_managed_agents, save_managed_agents,
    sync_managed_agent_processes, BackendKind,
};
use crate::{prevent_sleep, util};

#[cfg(windows)]
#[derive(Default)]
struct RecordShutdownOutcome {
    stopped_pair: bool,
    last_exit_code: Option<i32>,
    errors: Vec<String>,
}

#[cfg(windows)]
fn apply_record_shutdown_outcomes(
    app: Option<&tauri::AppHandle>,
    records: &mut [managed_agents::ManagedAgentRecord],
    outcomes: Vec<RecordShutdownOutcome>,
) {
    for (idx, outcome) in outcomes.into_iter().enumerate() {
        let recovery_uncertain = match app {
            Some(app) => {
                managed_agents::has_recovery_uncertainty(app, &records[idx]).unwrap_or(true)
            }
            None => managed_agents::has_unverified_job_reap(&records[idx]),
        };
        if !outcome.errors.is_empty() {
            let outcome_has_uncertainty = outcome
                .errors
                .iter()
                .any(|error| error.starts_with(managed_agents::UNVERIFIED_JOB_REAP_PREFIX));
            let record = &mut records[idx];
            record.updated_at = util::now_iso();
            let joined = outcome.errors.join("; ");
            if recovery_uncertain || outcome_has_uncertainty {
                managed_agents::append_shutdown_diagnostic(
                    record,
                    &joined,
                    outcome_has_uncertainty,
                );
            } else {
                record.last_error = Some(joined);
                record.last_error_code = None;
            }
            // At least one pair remains tracked, so do not claim the agent as
            // fully stopped even if a sibling pair exited successfully.
        } else if outcome.stopped_pair && !recovery_uncertain {
            let record = &mut records[idx];
            record.runtime_pid = None;
            record.last_stopped_at = Some(util::now_iso());
            record.updated_at = util::now_iso();
            if managed_agents::pending_pair_failures(record).is_empty() {
                record.last_exit_code = outcome.last_exit_code;
            }
            if !managed_agents::finalize_pending_pair_failures(record) {
                record.last_error = None;
                record.last_error_code = None;
            }
        }
    }
}

#[cfg(windows)]
fn retry_terminal_proof_pending_clears_with(
    records: &mut [managed_agents::ManagedAgentRecord],
    mut clear: impl FnMut(
        &mut managed_agents::ManagedAgentRecord,
        &managed_agents::ManagedAgentRuntimeKey,
    ) -> Result<(), String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    for record in records {
        let keys = managed_agents::terminal_proof_pending_recovery_clears(record);
        for key in keys {
            if let Err(error) = clear(record, &key) {
                errors.push(format!("{} ({}): {error}", record.name, key.runtime_id()));
            }
        }
    }
    errors
}

pub(crate) fn is_restart_request(code: Option<i32>) -> bool {
    code == Some(tauri::RESTART_EXIT_CODE)
}

fn run_retryable_shutdown_attempt(
    shutdown_done: &std::sync::atomic::AtomicBool,
    attempt: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    if shutdown_done
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(());
    }
    match attempt() {
        Ok(()) => Ok(()),
        Err(error) => {
            shutdown_done.store(false, Ordering::SeqCst);
            Err(error)
        }
    }
}

pub(crate) fn shut_down_app(
    app: &tauri::AppHandle,
    shutdown_done: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    run_retryable_shutdown_attempt(shutdown_done, || {
        app.state::<AppState>()
            .shutdown_started
            .store(true, Ordering::SeqCst);
        prevent_sleep::release(&app.state::<AppState>().prevent_sleep);

        if let Err(error) = shutdown_managed_agents(app) {
            // ExitRequested prevents this exit. Reset the admission flag before
            // returning so the same retained authority can be retried.
            app.state::<AppState>()
                .shutdown_started
                .store(false, Ordering::SeqCst);
            let _ = prevent_sleep::acquire(&app.state::<AppState>().prevent_sleep, app);
            return Err(format!("failed to stop managed agents: {error}"));
        }

        #[cfg(feature = "mesh-llm")]
        shutdown_mesh_runtime(app);

        Ok(())
    })
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
    let (mut changed, _exited) = sync_managed_agent_processes(app, &mut records, &mut runtimes);
    changed |= kill_stale_tracked_processes(
        &mut records,
        &runtimes,
        &managed_agents::current_instance_id(app),
    );

    // Stop all tracked agents. Send SIGTERM to all process
    // groups first, then wait for exits in parallel to avoid serial 1s waits.
    struct AgentToStop {
        idx: Option<usize>,
        #[cfg(windows)]
        key: managed_agents::ManagedAgentRuntimeKey,
        #[cfg(unix)]
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
            #[cfg(unix)]
            let Some(pid) = runtime
                .as_ref()
                .map(|rt| rt.child.id())
                .or(record.runtime_pid)
            else {
                continue;
            };
            #[cfg(windows)]
            if runtime.is_none() {
                continue;
            }
            to_stop.push(AgentToStop {
                idx: Some(idx),
                #[cfg(windows)]
                key,
                #[cfg(unix)]
                pid,
                runtime,
            });
        }
    }

    // A failed pre-registration cleanup can retain exact Job/Child authority
    // even if its record vanished. Shutdown must still drain those runtimes.
    #[cfg(windows)]
    let mut remaining_keys = runtimes.keys().cloned().collect::<Vec<_>>();
    #[cfg(windows)]
    remaining_keys.sort_by_key(managed_agents::ManagedAgentRuntimeKey::runtime_id);
    #[cfg(windows)]
    for key in remaining_keys {
        if let Some(runtime) = runtimes.remove(&key) {
            to_stop.push(AgentToStop {
                idx: records
                    .iter()
                    .position(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey)),
                key,
                runtime: Some(runtime),
            });
        }
    }

    #[cfg(windows)]
    if !to_stop.is_empty() {
        to_stop.sort_by_key(|agent| agent.key.runtime_id());
        changed = true;
        let mut errors = Vec::new();
        let mut outcomes: Vec<RecordShutdownOutcome> = (0..records.len())
            .map(|_| RecordShutdownOutcome::default())
            .collect();

        for mut agent in to_stop {
            let Some(mut runtime) = agent.runtime.take() else {
                let error = format!(
                    "tracked runtime {} was removed without an owned process",
                    agent.key.pubkey
                );
                if let Some(idx) = agent.idx {
                    outcomes[idx].errors.push(error.clone());
                }
                errors.push(error);
                continue;
            };

            match managed_agents::finalize_tracked_runtime(app, &agent.key, &mut runtime) {
                Ok(status) => {
                    if let Some(idx) = agent.idx {
                        if let Err(error) = managed_agents::clear_pair_recovery_with_terminal_proof(
                            app,
                            &mut records[idx],
                            &agent.key,
                        ) {
                            let error = format!(
                                "failed to retire exact-pair recovery authority after terminal proof: {error}"
                            );
                            managed_agents::record_terminal_proof_pending_recovery_clear(
                                &mut records[idx],
                                &agent.key,
                                runtime.child.id(),
                                &error,
                            );
                            outcomes[idx].errors.push(error.clone());
                            errors.push(format!("{}: {error}", records[idx].name));
                            runtimes.insert(agent.key.clone(), runtime);
                            continue;
                        }
                        let record = &records[idx];
                        let _ = managed_agents::append_log_marker(
                            &runtime.log_path,
                            &format!(
                                "=== stopped {} ({}) at {} ===",
                                record.name,
                                record.pubkey,
                                util::now_iso()
                            ),
                        );
                        if outcomes[idx].errors.is_empty() {
                            outcomes[idx].stopped_pair = true;
                            outcomes[idx].last_exit_code = status.code();
                        }
                    } else {
                        let _ = managed_agents::append_log_marker(
                            &runtime.log_path,
                            &format!(
                                "=== stopped unregistered pair {} at {} ===",
                                agent.key.pubkey,
                                util::now_iso()
                            ),
                        );
                    }
                    // `runtime` drops here only after bounded termination,
                    // receipt deletion, launcher reaping, and recovery clear succeeded.
                }
                Err(error) => {
                    let error = if let Some(idx) = agent.idx {
                        managed_agents::receipt_failure::mark_unverified_runtime(
                            Some(app),
                            &mut records[idx],
                            &agent.key,
                            runtime.child.id(),
                            util::now_iso(),
                            format!(
                                "shutdown cleanup is incomplete and exact pair authority remains tracked for retry: {error}"
                            ),
                        )
                    } else {
                        error
                    };
                    let label = agent
                        .idx
                        .map(|idx| records[idx].name.clone())
                        .unwrap_or_else(|| agent.key.pubkey.clone());
                    if let Some(idx) = agent.idx {
                        outcomes[idx].errors.push(error.clone());
                    }
                    errors.push(format!("{label}: {error}"));
                    // Preserve the exact Job/Child authority under its pair key.
                    runtimes.insert(agent.key, runtime);
                }
            }
        }

        apply_record_shutdown_outcomes(Some(app), &mut records, outcomes);

        if !errors.is_empty() {
            save_managed_agents(app, &records)?;
            return Err(format!(
                "managed-agent shutdown incomplete: {}",
                errors.join("; ")
            ));
        }
    }

    #[cfg(windows)]
    let retry_errors = retry_terminal_proof_pending_clears_with(&mut records, |record, key| {
        match managed_agents::clear_pair_recovery_with_terminal_proof(app, record, key)? {
            true => Ok(()),
            false => Err("terminal-proof marker had no exact-pair recovery authority".into()),
        }
    });
    #[cfg(windows)]
    if !retry_errors.is_empty() {
        save_managed_agents(app, &records)?;
        return Err(format!(
            "managed-agent shutdown incomplete: {}",
            retry_errors.join("; ")
        ));
    }

    #[cfg(windows)]
    let unresolved_record = records.iter().find_map(|record| {
        match managed_agents::has_recovery_uncertainty(app, record) {
            Ok(true) => Some(Ok(record)),
            Ok(false) => None,
            Err(error) => Some(Err(error)),
        }
    });
    if let Some(record) = unresolved_record.transpose()? {
        if changed {
            save_managed_agents(app, &records)?;
        }
        return Err(format!(
            "managed-agent shutdown incomplete: recovery authority for {} remains uncertain",
            record.name
        ));
    }

    #[cfg(unix)]
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
            let Some(idx) = agent.idx else {
                continue;
            };
            if let Some(ref mut rt) = agent.runtime {
                // Best-effort reap — don’t block shutdown if the child is stuck
                // in uninterruptible sleep. The zombie will be cleaned up when
                // our process exits and launchd reaps it.
                let _ = rt.child.try_wait();
                // Write log marker (best-effort).
                let record = &records[idx];
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
            let record = &mut records[idx];
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

pub(crate) fn exit_blocked(
    result: Result<(), String>,
    restart_requested: &std::sync::atomic::AtomicBool,
) -> bool {
    if let Err(error) = result {
        eprintln!("buzz-desktop: {error}");
        restart_requested.store(false, std::sync::atomic::Ordering::SeqCst);
        true
    } else {
        false
    }
}

pub(crate) fn report_final_shutdown(result: Result<(), String>) {
    if let Err(error) = result {
        eprintln!("buzz-desktop: final shutdown attempt failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::is_restart_request;

    #[cfg(windows)]
    fn minimal_record() -> crate::managed_agents::ManagedAgentRecord {
        serde_json::from_str(
            r#"{
                "pubkey": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "name": "test",
                "private_key_nsec": "nsec1fake",
                "relay_url": "",
                "acp_command": "buzz-acp",
                "agent_command": "buzz-agent",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": null,
                "model": null,
                "provider": null,
                "env_vars": {},
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_started_at": null,
                "last_stopped_at": null,
                "last_exit_code": null,
                "last_error": null
            }"#,
        )
        .expect("shutdown record fixture")
    }

    #[test]
    fn only_tauri_restart_exit_code_requests_a_relaunch() {
        assert!(is_restart_request(Some(tauri::RESTART_EXIT_CODE)));
        assert!(!is_restart_request(None));
        assert!(!is_restart_request(Some(0)));
    }

    #[cfg(windows)]
    #[test]
    fn mixed_pair_shutdown_preserves_failure_and_does_not_claim_full_stop() {
        let mut record = minimal_record();
        let original_stopped_at = record.last_stopped_at.clone();
        let outcomes = vec![super::RecordShutdownOutcome {
            stopped_pair: true,
            last_exit_code: Some(1),
            errors: vec!["pair still owned for retry".to_string()],
        }];

        super::apply_record_shutdown_outcomes(None, std::slice::from_mut(&mut record), outcomes);

        assert_eq!(
            record.last_error.as_deref(),
            Some("pair still owned for retry")
        );
        assert_eq!(record.last_stopped_at, original_stopped_at);
        assert_eq!(record.last_exit_code, None);
    }

    #[cfg(windows)]
    #[test]
    fn mixed_pair_shutdown_does_not_launder_unverified_reap_marker() {
        let mut record = minimal_record();
        record.runtime_pid = Some(6262);
        let outcomes = vec![super::RecordShutdownOutcome {
            stopped_pair: true,
            last_exit_code: None,
            errors: vec![
                "ordinary sibling failure".to_string(),
                format!(
                    "{} reap status unknown",
                    crate::managed_agents::UNVERIFIED_JOB_REAP_PREFIX
                ),
            ],
        }];

        super::apply_record_shutdown_outcomes(None, std::slice::from_mut(&mut record), outcomes);

        assert!(crate::managed_agents::has_unverified_job_reap(&record));
    }

    #[cfg(windows)]
    #[test]
    fn successful_retry_shutdown_preserves_observed_terminal_failure() {
        let mut record = minimal_record();
        let key = crate::managed_agents::ManagedAgentRuntimeKey::new(
            record.pubkey.clone(),
            "wss://terminal-failure.example",
        )
        .unwrap();
        crate::managed_agents::record_pending_pair_failure(
            &mut record,
            &key,
            Some(17),
            &crate::managed_agents::storage::AgentLogError {
                message: "harness exited with status 17".into(),
                code: None,
            },
        );
        let outcomes = vec![super::RecordShutdownOutcome {
            stopped_pair: true,
            last_exit_code: Some(0),
            errors: Vec::new(),
        }];

        super::apply_record_shutdown_outcomes(None, std::slice::from_mut(&mut record), outcomes);

        assert_eq!(record.last_exit_code, Some(17));
        assert!(record
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("status 17")));
    }

    #[cfg(windows)]
    #[test]
    fn shutdown_error_preserves_combined_recovery_and_pending_failure() {
        let mut record = minimal_record();
        let recovery_key = crate::managed_agents::ManagedAgentRuntimeKey::new(
            record.pubkey.clone(),
            "wss://recovery.example",
        )
        .unwrap();
        let failed_key = crate::managed_agents::ManagedAgentRuntimeKey::new(
            record.pubkey.clone(),
            "wss://failed.example",
        )
        .unwrap();
        let mut authority = crate::managed_agents::ManagedAgentRecoveryAuthority::default();
        authority.mark_pair(&recovery_key, 7171, "recovery remains uncertain".into());
        authority.project_compatibility(&mut record);
        crate::managed_agents::record_pending_pair_failure(
            &mut record,
            &failed_key,
            Some(17),
            &crate::managed_agents::storage::AgentLogError {
                message: "harness exited with status 17".into(),
                code: Some(801),
            },
        );
        record
            .last_error
            .as_mut()
            .unwrap()
            .push_str("; legacy appended shutdown diagnostic");

        super::apply_record_shutdown_outcomes(
            None,
            std::slice::from_mut(&mut record),
            vec![super::RecordShutdownOutcome {
                stopped_pair: false,
                last_exit_code: None,
                errors: vec!["another shutdown attempt failed".into()],
            }],
        );

        let failures = crate::managed_agents::pending_pair_failures(&record);
        assert_eq!(
            failures.get(&failed_key.runtime_id()).map(String::as_str),
            Some("harness exited with status 17")
        );
        assert_eq!(record.last_error_code, Some(801));
        assert!(crate::managed_agents::has_unverified_job_reap(&record));
    }

    #[test]
    fn exit_request_failure_blocks_exit_and_cancels_restart() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let restart = AtomicBool::new(true);
        assert!(super::exit_blocked(Err("cleanup failed".into()), &restart));
        assert!(!restart.load(Ordering::SeqCst));
        assert!(!super::exit_blocked(Ok(()), &restart));
    }

    #[test]
    fn failed_shutdown_attempt_resets_latch_and_allows_retry() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let done = AtomicBool::new(false);
        let attempts = AtomicUsize::new(0);
        let first = super::run_retryable_shutdown_attempt(&done, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err("managed cleanup remains uncertain".to_string())
        });
        assert!(first.is_err());
        assert!(!done.load(Ordering::SeqCst));

        let second = super::run_retryable_shutdown_attempt(&done, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert!(second.is_ok());
        assert!(done.load(Ordering::SeqCst));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[cfg(windows)]
    #[test]
    fn transient_terminal_proof_clear_failure_is_retried_by_next_shutdown() {
        let mut record = minimal_record();
        let key = crate::managed_agents::ManagedAgentRuntimeKey::new(
            record.pubkey.clone(),
            "wss://retry-clear.example",
        )
        .unwrap();
        crate::managed_agents::record_terminal_proof_pending_recovery_clear(
            &mut record,
            &key,
            7401,
            "first clear failed",
        );
        let mut records = vec![record];

        let first =
            super::retry_terminal_proof_pending_clears_with(&mut records, |_record, attempted| {
                assert_eq!(attempted, &key);
                Err("transient sidecar failure".to_string())
            });
        assert_eq!(first.len(), 1);

        let second =
            super::retry_terminal_proof_pending_clears_with(&mut records, |record, attempted| {
                assert_eq!(attempted, &key);
                record.runtime_pid = None;
                record.last_error = None;
                Ok(())
            });
        assert!(second.is_empty());
        assert!(
            crate::managed_agents::terminal_proof_pending_recovery_clears(&records[0]).is_empty()
        );
    }
}
