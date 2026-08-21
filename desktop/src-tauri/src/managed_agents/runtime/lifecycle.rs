use super::*;
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::Manager;

const RESTART_BACKOFFS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];
const CRASH_LOOP_RESET_AFTER: Duration = Duration::from_secs(60);
const CRASH_LOOP_ERROR: &str =
    "automatic restart stopped after 5 crash-loop attempts; resolve the agent error and restart it manually";

#[derive(Debug, Clone, Copy)]
struct RestartHistory {
    attempts: usize,
    last_attempt_at: Instant,
}

fn restart_histories() -> &'static Mutex<HashMap<ManagedAgentRuntimeKey, RestartHistory>> {
    static HISTORIES: OnceLock<Mutex<HashMap<ManagedAgentRuntimeKey, RestartHistory>>> =
        OnceLock::new();
    HISTORIES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn reserve_restart_attempt(key: &ManagedAgentRuntimeKey, now: Instant) -> Result<Duration, String> {
    let mut histories = restart_histories()
        .lock()
        .map_err(|error| error.to_string())?;
    let history = histories.entry(key.clone()).or_insert(RestartHistory {
        attempts: 0,
        last_attempt_at: now,
    });
    if now.duration_since(history.last_attempt_at) >= CRASH_LOOP_RESET_AFTER {
        history.attempts = 0;
    }
    let Some(delay) = RESTART_BACKOFFS.get(history.attempts).copied() else {
        return Err(CRASH_LOOP_ERROR.to_string());
    };
    history.attempts += 1;
    history.last_attempt_at = now;
    Ok(delay)
}

fn advance_restart_revision(expected: &str, observed: &str, next: String) -> Option<String> {
    (expected == observed).then_some(next)
}

fn record_restart_failure(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    expected_updated_at: &str,
    message: String,
) -> Result<Option<String>, String> {
    let state = app.state::<crate::app_state::AppState>();
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = super::super::load_managed_agents(app)?;
    let Some(record) = records
        .iter_mut()
        .find(|record| record.pubkey == key.pubkey)
    else {
        return Ok(None);
    };
    let Some(next_updated_at) =
        advance_restart_revision(expected_updated_at, &record.updated_at, now_iso())
    else {
        return Ok(None);
    };
    record.updated_at = next_updated_at.clone();
    record.last_error = Some(message);
    record.last_error_code = None;
    super::super::save_managed_agents(app, &records)?;
    Ok(Some(next_updated_at))
}

pub(crate) fn schedule_unexpected_restart(
    app: &AppHandle,
    key: ManagedAgentRuntimeKey,
    mut expected_updated_at: String,
) -> Result<(), String> {
    let state = app.state::<crate::app_state::AppState>();
    if state
        .shutdown_started
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(());
    }
    let initial_delay = reserve_restart_attempt(&key, Instant::now())?;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut delay = initial_delay;
        loop {
            tokio::time::sleep(delay).await;
            let state = app.state::<crate::app_state::AppState>();
            if state
                .shutdown_started
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return;
            }

            let restart_app = app.clone();
            let restart_key = key.clone();
            let expected_for_attempt = expected_updated_at.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                super::super::start_pair(
                    restart_key.pubkey,
                    restart_key.relay_url,
                    true,
                    Some(&expected_for_attempt),
                    restart_app,
                )
                .map(|_| ())
            })
            .await
            .unwrap_or_else(|error| Err(format!("automatic restart task failed: {error}")));

            match result {
                Ok(()) => return,
                Err(error)
                    if error
                        == "managed agent changed while runtime reconciliation was in flight" =>
                {
                    // An explicit stop or intervening edit invalidates this
                    // crash generation. Never resurrect a process after the
                    // user has acted during backoff.
                    return;
                }
                Err(error) => {
                    eprintln!(
                        "buzz-desktop: automatic restart failed for {} on {}: {error}",
                        key.pubkey, key.relay_url
                    );
                    match reserve_restart_attempt(&key, Instant::now()) {
                        Ok(next_delay) => {
                            let next_revision = record_restart_failure(
                                &app,
                                &key,
                                &expected_updated_at,
                                format!("automatic restart failed: {error}"),
                            );
                            match next_revision {
                                Ok(Some(next_revision)) => expected_updated_at = next_revision,
                                Ok(None) => return,
                                Err(persist_error) => {
                                    eprintln!(
                                        "buzz-desktop: failed to persist automatic restart error for {} on {}: {persist_error}",
                                        key.pubkey, key.relay_url
                                    );
                                    return;
                                }
                            }
                            delay = next_delay;
                        }
                        Err(cap_error) => {
                            let _ = record_restart_failure(
                                &app,
                                &key,
                                &expected_updated_at,
                                cap_error.clone(),
                            );
                            eprintln!(
                                "buzz-desktop: {cap_error} for {} on {}",
                                key.pubkey, key.relay_url
                            );
                            return;
                        }
                    }
                }
            }
        }
    });
    Ok(())
}

/// Kill stale agent processes from a previous session whose PID is still alive
/// but not tracked in the current `runtimes` map. Updates the record fields and
/// returns `true` if any records were modified.
pub fn kill_stale_tracked_processes(
    records: &mut [ManagedAgentRecord],
    runtimes: &HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    instance_id: &str,
) -> bool {
    kill_stale_tracked_processes_with(
        records,
        runtimes,
        |pid| process_has_buzz_marker(pid, instance_id),
        terminate_process,
    )
}

/// Injectable version of `kill_stale_tracked_processes` for testing.
/// `has_marker(pid)` returns true when the process carries this instance's
/// `BUZZ_MANAGED_AGENT` marker; `kill(pid)` performs the termination.
pub(crate) fn kill_stale_tracked_processes_with(
    records: &mut [ManagedAgentRecord],
    runtimes: &HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    has_marker: impl Fn(u32) -> bool,
    mut kill: impl FnMut(u32) -> Result<(), String>,
) -> bool {
    use crate::managed_agents::BackendKind;

    let mut changed = false;
    for record in records.iter_mut() {
        if record.backend != BackendKind::Local {
            continue;
        }
        let Some(pid) = record.runtime_pid else {
            continue;
        };
        if !runtimes.keys().any(|key| key.pubkey == record.pubkey) {
            // Name-gate is omitted intentionally: custom harnesses use arbitrary
            // binary names not in KNOWN_AGENT_BINARIES. BUZZ_MANAGED_AGENT is the
            // authoritative ownership proof; terminate only if it matches.
            if has_marker(pid) {
                let _ = kill(pid);
            }
            record.runtime_pid = None;
            record.last_stopped_at = Some(crate::util::now_iso());
            record.updated_at = crate::util::now_iso();
            changed = true;
        }
    }
    changed
}

pub fn sync_managed_agent_processes(
    app: &AppHandle,
    records: &mut [ManagedAgentRecord],
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
) -> (bool, Vec<String>) {
    let mut changed = false;
    let mut exited = Vec::new();

    for (key, runtime) in runtimes.iter_mut() {
        let status = match runtime.child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                if let Some(record) = records
                    .iter_mut()
                    .find(|record| record.pubkey == key.pubkey)
                {
                    record.updated_at = now_iso();
                    record.last_error = Some(format!("failed to inspect process state: {error}"));
                    record.last_error_code = None;
                }
                changed = true;
                exited.push(key.clone());
                continue;
            }
        };

        let Some(status) = status else {
            continue;
        };

        if let Some(record) = records
            .iter_mut()
            .find(|record| record.pubkey == key.pubkey)
        {
            record.updated_at = now_iso();
            record.last_stopped_at = Some(now_iso());
            record.last_exit_code = status.code();
            let log_err = if status.success() {
                None
            } else {
                Some(
                    super::super::meaningful_agent_error_from_log(&runtime.log_path)
                        .unwrap_or_else(|| super::super::storage::AgentLogError {
                            message: format!("harness exited with status {status}"),
                            code: None,
                        }),
                )
            };
            record.last_error = log_err.as_ref().map(|e| e.message.clone());
            record.last_error_code = log_err.as_ref().and_then(|e| e.code);
        }

        changed = true;
        exited.push(key.clone());
    }

    let exited_pubkeys: Vec<String> = exited.iter().map(|key| key.pubkey.clone()).collect();
    for key in &exited {
        runtimes.remove(key);
    }

    for key in exited {
        let expected_updated_at = records
            .iter()
            .find(|record| record.pubkey == key.pubkey)
            .map(|record| record.updated_at.clone());
        let result = match expected_updated_at {
            Some(expected) => schedule_unexpected_restart(app, key.clone(), expected),
            None => Err(format!("agent {} not found", key.pubkey)),
        };
        if let Err(error) = result {
            if let Some(record) = records
                .iter_mut()
                .find(|record| record.pubkey == key.pubkey)
            {
                record.updated_at = now_iso();
                record.last_error = Some(error);
                record.last_error_code = None;
                changed = true;
            }
        }
    }

    // `runtime_pid` is legacy bookkeeping. Pair runtimes and receipts are the
    // authoritative lifecycle source; migration cleanup is handled separately.
    for record in records.iter_mut() {
        if record.runtime_pid.take().is_some() {
            record.updated_at = now_iso();
            changed = true;
        }
    }

    (changed, exited_pubkeys)
}

/// Supervise managed child processes for the lifetime of the Desktop backend.
/// This task is owned by Tauri setup rather than the renderer, so closing the
/// window cannot suspend crash detection or restart scheduling.
pub async fn supervise_managed_agent_processes(app: AppHandle) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        let state = app.state::<crate::app_state::AppState>();
        if state
            .shutdown_started
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return;
        }

        let sync_app = app.clone();
        let result =
            tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
                let state = sync_app.state::<crate::app_state::AppState>();
                let _transition = state
                    .managed_agent_runtime_transition
                    .lock()
                    .map_err(|error| error.to_string())?;
                if state
                    .shutdown_started
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    return Ok(Vec::new());
                }
                let _store = state
                    .managed_agents_store_lock
                    .lock()
                    .map_err(|error| error.to_string())?;
                let mut records = super::super::load_managed_agents(&sync_app)?;
                let mut runtimes = state
                    .managed_agent_processes
                    .lock()
                    .map_err(|error| error.to_string())?;
                let (changed, exited_pubkeys) =
                    sync_managed_agent_processes(&sync_app, &mut records, &mut runtimes);
                if changed {
                    super::super::save_managed_agents(&sync_app, &records)?;
                }
                Ok(exited_pubkeys)
            })
            .await;

        match result {
            Ok(Ok(exited_pubkeys)) => {
                let state = app.state::<crate::app_state::AppState>();
                for pubkey in exited_pubkeys {
                    state.clear_agent_session_caches(&pubkey);
                }
            }
            Ok(Err(error)) => eprintln!("buzz-desktop: managed-agent supervisor: {error}"),
            Err(error) => eprintln!("buzz-desktop: managed-agent supervisor task: {error}"),
        }
    }
}

#[cfg(test)]
mod restart_tests {
    use super::*;

    #[test]
    fn restart_backoff_is_bounded_and_resets_after_stability() {
        let key = ManagedAgentRuntimeKey::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "ws://localhost:3000",
        )
        .unwrap();
        restart_histories().lock().unwrap().remove(&key);
        let start = Instant::now();
        for (index, expected) in RESTART_BACKOFFS.iter().copied().enumerate() {
            assert_eq!(
                reserve_restart_attempt(&key, start + Duration::from_millis(index as u64)).unwrap(),
                expected
            );
        }
        assert_eq!(
            reserve_restart_attempt(&key, start + Duration::from_secs(1)).unwrap_err(),
            CRASH_LOOP_ERROR
        );
        assert_eq!(
            reserve_restart_attempt(
                &key,
                start + CRASH_LOOP_RESET_AFTER + Duration::from_secs(1),
            )
            .unwrap(),
            RESTART_BACKOFFS[0]
        );
        restart_histories().lock().unwrap().remove(&key);
    }

    #[test]
    fn own_failure_revision_allows_retry_but_external_edit_cancels() {
        let key = ManagedAgentRuntimeKey::new(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "ws://localhost:3000",
        )
        .unwrap();
        restart_histories().lock().unwrap().remove(&key);
        let start = Instant::now();
        assert_eq!(
            reserve_restart_attempt(&key, start).unwrap(),
            RESTART_BACKOFFS[0]
        );
        let next = advance_restart_revision("crash-v1", "crash-v1", "retry-v2".to_string());
        assert_eq!(next.as_deref(), Some("retry-v2"));
        assert_eq!(
            reserve_restart_attempt(&key, start + Duration::from_secs(1)).unwrap(),
            RESTART_BACKOFFS[1]
        );
        assert_eq!(
            advance_restart_revision("retry-v2", "external-v3", "retry-v4".to_string()),
            None
        );
        restart_histories().lock().unwrap().remove(&key);
    }
}
