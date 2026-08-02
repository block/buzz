use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager};

use super::{
    agent_readiness, append_log_marker, current_instance_id, find_managed_agent_mut,
    load_global_agent_config, load_managed_agents, load_personas, managed_agent_runtime_log_path,
    record_agent_command, resolve_effective_agent_env, save_managed_agents, spawn_agent_child,
    terminate_untracked_pair_runtime, write_agent_runtime_receipt, AgentReadiness, BackendKind,
    ManagedAgentPairRuntime, ManagedAgentRuntimeKey, ManagedAgentRuntimeLifecycle,
    ManagedAgentRuntimeReceipt, ManagedAgentRuntimeStatus,
};
#[cfg(not(windows))]
use super::{process_is_running, terminate_process};
use crate::app_state::AppState;

const STATUS_EVENT: &str = "managed-agent-runtime-status";

fn status_for(
    app: &AppHandle,
    record: &super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    runtime: Option<&ManagedAgentPairRuntime>,
    requested_relay_url: Option<String>,
) -> ManagedAgentRuntimeStatus {
    let personas = load_personas(app).unwrap_or_default();
    let global = load_global_agent_config(app).unwrap_or_default();
    status_for_with(
        app,
        record,
        key,
        runtime,
        requested_relay_url,
        StatusInputs {
            personas: &personas,
            global: &global,
        },
    )
}

/// Preloaded per-call-site inputs for [`status_for_with`], so multi-row
/// callers (list, reconcile) hit disk once instead of once per row.
struct StatusInputs<'a> {
    personas: &'a [super::AgentDefinition],
    global: &'a super::GlobalAgentConfig,
}

fn status_for_with(
    app: &AppHandle,
    record: &super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    runtime: Option<&ManagedAgentPairRuntime>,
    requested_relay_url: Option<String>,
    inputs: StatusInputs<'_>,
) -> ManagedAgentRuntimeStatus {
    let StatusInputs { personas, global } = inputs;
    let command = record_agent_command(record, personas);
    let metadata = super::known_acp_runtime(&command);
    let effective = resolve_effective_agent_env(record, personas, metadata, global);
    let local_setup = matches!(agent_readiness(&effective), AgentReadiness::Ready);
    ManagedAgentRuntimeStatus {
        pubkey: key.pubkey.clone(),
        relay_url: key.relay_url.clone(),
        requested_relay_url,
        local_setup,
        lifecycle: runtime
            .map(|runtime| runtime.lifecycle.clone())
            .unwrap_or(ManagedAgentRuntimeLifecycle::Stopped),
        pid: runtime.map(|runtime| runtime.child.id()),
        error: runtime.and_then(|runtime| runtime.error.clone()),
        log_path: managed_agent_runtime_log_path(app, key)
            .ok()
            .map(|path| path.display().to_string()),
    }
}

fn emit_status(app: &AppHandle, status: &ManagedAgentRuntimeStatus) {
    let _ = app.emit(STATUS_EVENT, status);
}

fn observer_lifecycle_key(
    outer_pubkey: &str,
    payload: &super::ManagedAgentRuntimeLifecycleObserverPayload,
) -> Result<ManagedAgentRuntimeKey, String> {
    if !outer_pubkey.eq_ignore_ascii_case(&payload.pubkey) {
        return Err("observer signer does not match lifecycle payload pubkey".into());
    }
    if matches!(
        payload.lifecycle,
        ManagedAgentRuntimeLifecycle::Starting | ManagedAgentRuntimeLifecycle::Stopped
    ) {
        return Err("observer cannot author starting or stopped lifecycle".into());
    }
    if payload.lifecycle == ManagedAgentRuntimeLifecycle::Failed && payload.error.is_none() {
        return Err("failed lifecycle requires an error".into());
    }
    if payload.lifecycle != ManagedAgentRuntimeLifecycle::Failed && payload.error.is_some() {
        return Err("lifecycle error is only valid for failed".into());
    }
    ManagedAgentRuntimeKey::new(payload.pubkey.clone(), &payload.relay_url)
}

#[tauri::command]
pub fn put_managed_agent_runtime_lifecycle(
    outer_pubkey: String,
    payload: super::ManagedAgentRuntimeLifecycleObserverPayload,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let key = observer_lifecycle_key(&outer_pubkey, &payload)?;
    let state = app.state::<AppState>();
    let records = load_managed_agents(&app)?;
    let record = records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey))
        .ok_or_else(|| format!("agent {} not found", key.pubkey))?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let runtime = runtimes
        .get_mut(&key)
        .ok_or_else(|| "lifecycle frame does not match a tracked runtime pair".to_string())?;
    if runtime.start_nonce != payload.start_nonce {
        return Err("lifecycle frame does not match the current harness generation".into());
    }
    if runtime
        .child
        .try_wait()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("lifecycle frame arrived after process exit".into());
    }
    runtime.lifecycle = payload.lifecycle;
    runtime.error = payload.error;
    let status = status_for(&app, record, &key, Some(runtime), None);
    emit_status(&app, &status);
    Ok(status)
}

fn commit_polled_terminal_recovery<T>(
    record: &mut super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    recovery_pid: u32,
    retained_runtimes: &std::collections::HashMap<ManagedAgentRuntimeKey, T>,
    clear: impl FnOnce(&mut super::ManagedAgentRecord, &ManagedAgentRuntimeKey) -> Result<bool, String>,
) -> Result<(), String> {
    if !retained_runtimes.contains_key(key) {
        return Err(
            "finalized managed-agent runtime token is unavailable for durable retirement".into(),
        );
    }
    if let Err(error) = clear(record, key) {
        super::record_terminal_proof_pending_recovery_clear(record, key, recovery_pid, &error);
        return Err(error);
    }
    Ok(())
}

fn record_polled_terminal_outcome(
    record: &mut super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    status: std::process::ExitStatus,
    log_error: Option<super::storage::AgentLogError>,
) {
    if status.success() {
        if record.last_error.is_none() {
            record.last_exit_code = status.code();
        }
    } else {
        let error = log_error.unwrap_or_else(|| super::storage::AgentLogError {
            message: format!("harness exited with status {status}"),
            code: None,
        });
        super::record_pending_pair_failure(record, key, status.code(), &error);
    }
}

#[tauri::command]
pub fn list_managed_agent_runtimes(
    app: AppHandle,
) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    // This command is polled whenever the members sidebar opens and refetched
    // on every status event — load the per-row status inputs once, outside
    // the locks, instead of hitting disk per row while holding them.
    let personas = load_personas(&app).unwrap_or_default();
    let global = load_global_agent_config(&app).unwrap_or_default();
    let state = app.state::<AppState>();
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let mut inspection_errors = Vec::new();
    let exited_keys: Vec<_> = runtimes
        .iter_mut()
        .filter_map(|(key, runtime)| match runtime.child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(windows)]
                if let Err(error) =
                    super::process_lifecycle::finalize_tracked_runtime(&app, key, runtime)
                {
                    inspection_errors.push((key.clone(), error));
                    return None;
                }
                #[cfg(not(windows))]
                if let Err(error) = super::remove_agent_runtime_receipt(&app, key) {
                    inspection_errors.push((key.clone(), error));
                    return None;
                }
                Some((key.clone(), status))
            }
            Ok(None) => None,
            Err(error) => {
                inspection_errors.push((key.clone(), error.to_string()));
                None
            }
        })
        .collect();
    let records_changed = !exited_keys.is_empty() || !inspection_errors.is_empty();
    let mut terminal_clear_failures = Vec::new();
    let mut terminal_clear_successes = Vec::new();
    let mut statuses = Vec::new();
    for (key, exit_status) in exited_keys {
        let Some(recovery_pid) = runtimes.get(&key).map(|runtime| runtime.child.id()) else {
            continue;
        };
        let log_error = if exit_status.success() {
            None
        } else {
            runtimes
                .get(&key)
                .and_then(|runtime| super::meaningful_agent_error_from_log(&runtime.log_path))
        };
        if let Some(record) = records
            .iter_mut()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey))
        {
            record.updated_at = crate::util::now_iso();
            if !exit_status.success() || record.last_error.is_none() {
                record.last_exit_code = exit_status.code();
            }
            if let Err(error) = commit_polled_terminal_recovery(
                record,
                &key,
                recovery_pid,
                &runtimes,
                |record, key| super::clear_pair_recovery_with_terminal_proof(&app, record, key),
            ) {
                terminal_clear_failures.push((key, error));
                continue;
            }
            record_polled_terminal_outcome(record, &key, exit_status, log_error);
            terminal_clear_successes.push(key.clone());
            let sibling_active = runtimes.keys().any(|runtime_key| {
                runtime_key.pubkey == record.pubkey
                    && !terminal_clear_successes.contains(runtime_key)
            });
            if !sibling_active && !super::has_recovery_uncertainty(&app, record)? {
                record.last_stopped_at = Some(record.updated_at.clone());
            }
            let status = status_for_with(
                &app,
                record,
                &key,
                None,
                None,
                StatusInputs {
                    personas: &personas,
                    global: &global,
                },
            );
            statuses.push(status);
        }
    }
    for (key, error) in inspection_errors {
        if let Some(record) = records
            .iter_mut()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey))
        {
            #[cfg(windows)]
            if let Some(runtime) = runtimes.get(&key) {
                super::runtime::receipt_failure::mark_unverified_runtime(
                    Some(&app),
                    record,
                    &key,
                    runtime.child.id(),
                    crate::util::now_iso(),
                    format!("failed to inspect or finalize process state: {error}"),
                );
            }
            #[cfg(not(windows))]
            {
                record.updated_at = crate::util::now_iso();
                record.last_error = Some(format!("failed to inspect process state: {error}"));
                record.last_error_code = None;
            }
        }
    }
    statuses.extend(
        runtimes
            .iter()
            .filter(|(key, _)| !terminal_clear_successes.contains(key))
            .filter_map(|(key, runtime)| {
                let record = records
                    .iter()
                    .find(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey))?;
                Some(status_for_with(
                    &app,
                    record,
                    key,
                    Some(runtime),
                    None,
                    StatusInputs {
                        personas: &personas,
                        global: &global,
                    },
                ))
            }),
    );
    // Skip the store rewrite on the common nothing-changed poll.
    if records_changed {
        if let Err(error) = save_managed_agents(&app, &records) {
            drop(runtimes);
            let clear_errors = terminal_clear_failures
                .iter()
                .map(|(key, error)| format!("{}: {error}", key.runtime_id()))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(if clear_errors.is_empty() {
                error
            } else {
                format!("{clear_errors}; failed to persist terminal-proof retry markers: {error}")
            });
        }
    }
    for key in &terminal_clear_successes {
        runtimes.remove(key);
        state.clear_agent_session_cache(key);
    }
    drop(runtimes);
    if !terminal_clear_failures.is_empty() {
        return Err(terminal_clear_failures
            .into_iter()
            .map(|(key, error)| format!("{}: {error}", key.runtime_id()))
            .collect::<Vec<_>>()
            .join("; "));
    }
    for status in &statuses {
        emit_status(&app, status);
    }
    Ok(statuses)
}

pub(crate) fn start_managed_agent_runtime_pair_lazy_under_transition(
    pubkey: String,
    relay_url: String,
    transition: &std::sync::MutexGuard<'_, ()>,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair(pubkey, relay_url, true, None, Some(transition), app)
}

pub(crate) fn start_managed_agent_runtime_pair_lazy(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair(pubkey, relay_url, true, None, None, app)
}

#[tauri::command]
pub fn start_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_managed_agent_runtime_pair_lazy(pubkey, relay_url, app)
}

fn start_pair(
    pubkey: String,
    relay_url: String,
    lazy: bool,
    expected_updated_at: Option<&str>,
    transition_held: Option<&std::sync::MutexGuard<'_, ()>>,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let state = app.state::<AppState>();
    let _transition = if transition_held.is_some() {
        None
    } else {
        Some(
            state
                .managed_agent_runtime_transition
                .lock()
                .map_err(|e| e.to_string())?,
        )
    };
    if state.shutdown_started.load(Ordering::Acquire) {
        return Err("desktop shutdown has started".into());
    }
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let record = find_managed_agent_mut(&mut records, &pubkey)?;
    if record.backend != BackendKind::Local {
        return Err("managed runtime pairs require a local agent".into());
    }
    if expected_updated_at.is_some_and(|expected| record.updated_at != expected) {
        return Err("managed agent changed while runtime reconciliation was in flight".into());
    }
    let key = ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    if let Some(error) = super::recovery_admission_error(&app, record, &key)? {
        return Err(error);
    }
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    if let Some(runtime) = runtimes.get_mut(&key) {
        match runtime
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
        {
            None => {
                let status = status_for(&app, record, &key, Some(runtime), None);
                return Ok(status);
            }
            Some(_) => {
                #[cfg(windows)]
                super::process_lifecycle::finalize_tracked_runtime(&app, &key, runtime)?;
            }
        }
    }
    runtimes.remove(&key);
    terminate_untracked_pair_runtime(&app, &key)?;

    let owner = state
        .keys
        .lock()
        .ok()
        .map(|keys| keys.public_key().to_hex());
    let process = spawn_agent_child(&app, record, &key.relay_url, lazy, owner.as_deref())?;
    let now = crate::util::now_iso();
    let receipt = ManagedAgentRuntimeReceipt {
        key: key.clone(),
        pid: process.child.id(),
        desktop_instance_id: current_instance_id(&app),
        started_at: now.clone(),
        windows_job_contained: super::process_has_windows_job(&process),
    };
    if let Err(error) = write_agent_runtime_receipt(&app, &receipt) {
        let cleanup_error = match super::runtime::receipt_failure::cleanup(
            &app,
            process,
            record,
            &mut runtimes,
            key,
            now,
            error,
        ) {
            Err(error) => error,
            Ok(()) => return Err("receipt-write failure was not propagated".to_string()),
        };
        drop(runtimes);
        let persistence = save_managed_agents(&app, &records)
            .err()
            .map(|save_error| format!("; failed to persist cleanup state: {save_error}"))
            .unwrap_or_default();
        return Err(format!("{cleanup_error}{persistence}"));
    }
    record.updated_at = now.clone();
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    if !super::has_unverified_job_reap(record) {
        record.last_error = None;
    }
    runtimes.insert(key.clone(), ManagedAgentPairRuntime::starting(process));
    let status = status_for(&app, record, &key, runtimes.get(&key), None);
    drop(runtimes);
    save_managed_agents(&app, &records)?;
    emit_status(&app, &status);
    Ok(status)
}

#[tauri::command]
pub fn stop_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    stop_pair(pubkey, relay_url, None, app)
}

fn stop_pair(
    pubkey: String,
    relay_url: String,
    transition_held: Option<&std::sync::MutexGuard<'_, ()>>,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let state = app.state::<AppState>();
    let _transition = if transition_held.is_some() {
        None
    } else {
        Some(
            state
                .managed_agent_runtime_transition
                .lock()
                .map_err(|e| e.to_string())?,
        )
    };
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let record = find_managed_agent_mut(&mut records, &pubkey)?;
    let key = ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    if let Some(mut runtime) = runtimes.remove(&key) {
        #[cfg(windows)]
        let stop_result =
            super::process_lifecycle::finalize_tracked_runtime(&app, &key, &mut runtime);
        #[cfg(not(windows))]
        let stop_result = if process_is_running(runtime.child.id()) {
            terminate_process(runtime.child.id())
        } else {
            Ok(())
        }
        .and_then(|()| runtime.child.wait().map_err(|e| e.to_string()));
        match stop_result {
            Ok(status) => {
                record.last_exit_code = status.code();
                if let Err(error) =
                    super::clear_pair_recovery_with_terminal_proof(&app, record, &key)
                {
                    super::record_terminal_proof_pending_recovery_clear(
                        record,
                        &key,
                        runtime.child.id(),
                        &error,
                    );
                    runtimes.insert(key.clone(), runtime);
                    drop(runtimes);
                    save_managed_agents(&app, &records)?;
                    return Err(format!(
                        "failed to retire exact-pair recovery authority after terminal proof: {error}"
                    ));
                }
                let _ = append_log_marker(&runtime.log_path, "=== stopped pair runtime ===");
            }
            Err(error) => {
                // Keep the exact child/Job authority and receipt under this
                // pair key until a stop actually succeeds, and persist the
                // failure mutation before returning.
                let recovery_pid = runtime.child.id();
                runtimes.insert(key.clone(), runtime);
                let error = super::runtime::receipt_failure::mark_unverified_runtime(
                    Some(&app),
                    record,
                    &key,
                    recovery_pid,
                    crate::util::now_iso(),
                    format!(
                        "Stop cleanup is incomplete and exact pair authority remains tracked for retry: {error}"
                    ),
                );
                drop(runtimes);
                return match save_managed_agents(&app, &records) {
                    Ok(()) => Err(error),
                    Err(persist_error) => Err(format!(
                        "{error}; additionally failed to persist managed-agent stop failure: {persist_error}"
                    )),
                };
            }
        }
    } else {
        #[cfg(windows)]
        if super::terminal_proof_pending_recovery_clears(record).contains(&key) {
            super::clear_pair_recovery_with_terminal_proof(&app, record, &key)?;
        } else if let Some(error) = super::recovery_admission_error(&app, record, &key)? {
            return Err(error);
        }
        // No runtime is tracked at this key, but a valid prior-session
        // receipt may still point at a live child (e.g. the crash-recovery
        // window for a non-auto-start agent). Terminate that orphan before
        // erasing its receipt — otherwise this "stop" leaves the harness
        // running yet deletes the one artifact sweeps and
        // terminate_untracked_pair_runtime use to find it, and a follow-up
        // start would spawn a duplicate harness for the same pair. On
        // failure the receipt stays on disk (terminate_untracked_pair_runtime
        // only removes it after the child exits), mirroring the tracked
        // path's keep-until-success invariant.
        terminate_untracked_pair_runtime(&app, &key)?;
        super::clear_pair_recovery_with_terminal_proof(&app, record, &key)?;
    }
    #[cfg(not(windows))]
    super::remove_agent_runtime_receipt(&app, &key)?;
    state.clear_agent_session_cache(&key);
    record.updated_at = crate::util::now_iso();
    let sibling_active = runtimes
        .keys()
        .any(|runtime_key| runtime_key.pubkey == record.pubkey);
    if !sibling_active && !super::has_unverified_job_reap(record) {
        record.runtime_pid = None;
        record.last_stopped_at = Some(record.updated_at.clone());
        record.last_error = None;
        record.last_error_code = None;
    }
    let status = status_for(&app, record, &key, None, None);
    drop(runtimes);
    save_managed_agents(&app, &records)?;
    emit_status(&app, &status);
    Ok(status)
}

#[tauri::command]
pub fn restart_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let state = app.state::<AppState>();
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    stop_pair(
        pubkey.clone(),
        relay_url.clone(),
        Some(&_transition),
        app.clone(),
    )?;
    start_pair(
        pubkey,
        relay_url,
        true,
        None,
        Some(&_transition),
        app.clone(),
    )
}

/// Probe whether this agent can operate on `requested_relay_url`.
///
/// Runs a bounded authenticated query with the agent's own keys (NIP-42 +
/// NIP-OA auth tag). Auth success is the spawn-eligibility signal: NIP-29
/// membership (kind 39002) cannot exist before the agent's harness first
/// connects to a relay, so gating on membership *presence* could never
/// bootstrap a pair on a newly configured community — it only rediscovered
/// pairs that had already run. A rejected or timed-out probe surfaces as a
/// Failed status row instead of a silent skip.
async fn probe_agent_relay_access(
    state: &AppState,
    record: super::ManagedAgentRecord,
    requested_relay_url: String,
) -> Result<(super::ManagedAgentRecord, ManagedAgentRuntimeKey, String), String> {
    let key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), &requested_relay_url)?;
    let keys = nostr::Keys::parse(record.private_key_nsec.trim())
        .map_err(|error| format!("invalid managed-agent key: {error}"))?;
    let api_base = crate::relay::relay_http_base_url(&key.relay_url);
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::relay::query_relay_at_with_keys(
            state,
            &api_base,
            &[serde_json::json!({"kinds": [39002], "#p": [record.pubkey]})],
            &keys,
            record.auth_tag.as_deref(),
        ),
    )
    .await
    .map_err(|_| "relay access probe timed out".to_string())??;
    Ok((record, key, requested_relay_url))
}

/// Build the `Failed` status row for a probe failure whose requested relay URL
/// cannot even form a pair key (so there is no canonical `relay_url` to key on).
/// The raw requested URL stands in for both the identity and the requested
/// field so the batch still degrades this one community to a visible row
/// instead of aborting every other community's row.
fn unkeyable_failed_status(
    record: &super::ManagedAgentRecord,
    requested: String,
    error: String,
    personas: &[super::AgentDefinition],
    global: &super::GlobalAgentConfig,
) -> ManagedAgentRuntimeStatus {
    let command = record_agent_command(record, personas);
    let metadata = super::known_acp_runtime(&command);
    let effective = resolve_effective_agent_env(record, personas, metadata, global);
    ManagedAgentRuntimeStatus {
        pubkey: record.pubkey.clone(),
        relay_url: requested.clone(),
        requested_relay_url: Some(requested),
        local_setup: matches!(agent_readiness(&effective), AgentReadiness::Ready),
        lifecycle: ManagedAgentRuntimeLifecycle::Failed,
        pid: None,
        error: Some(error),
        log_path: None,
    }
}

/// Spawn a lazy harness pair for every eligible (agent, community) pair.
///
/// Eligibility is deliberately gated on `start_on_app_launch`: auto-start is
/// the *proactive fan-out* policy — "keep this agent warm in every community" —
/// not a correctness prerequisite. A manual-start agent still works on demand
/// everywhere: attaching it to a channel ensures its pair, an @mention wakes a
/// pair, the members sidebar and Settings controls start pairs, and restore
/// preserves running pairs across relaunch. Fanning out warm-socket pairs for
/// agents the user chose *not* to auto-start would contradict that choice, so
/// reconcile leaves them alone until something explicitly asks for them.
#[tauri::command]
pub async fn reconcile_managed_agent_runtimes(
    communities: Vec<super::ManagedAgentCommunityTarget>,
    app: AppHandle,
) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    use futures_util::{stream, StreamExt};

    let records = load_managed_agents(&app)?;
    let mut jobs = Vec::new();
    for community in communities {
        for record in records
            .iter()
            .filter(|record| record.start_on_app_launch && record.backend == BackendKind::Local)
        // The legacy per-record relay pin is deliberately ignored here — see
        // `effective_agent_relay_url`. Every local auto-start agent fans out
        // to every configured community.
        {
            jobs.push((record.clone(), community.relay_url.clone()));
        }
    }
    let probes: Vec<_> = stream::iter(jobs)
        .map(|(record, requested)| {
            let state = app.state::<AppState>();
            async move {
                let fallback_record = record.clone();
                let fallback_requested = requested.clone();
                probe_agent_relay_access(&state, record, requested)
                    .await
                    .map_err(|error| (fallback_record, fallback_requested, error))
            }
        })
        .buffer_unordered(6)
        .collect()
        .await;

    // start_pair does blocking work (std mutexes, process spawn, receipt
    // writes, and up-to-2s exit polling in terminate_untracked_pair_runtime),
    // so run the post-probe start loop off the async workers, matching the
    // restart flows.
    tokio::task::spawn_blocking(move || {
        let personas = load_personas(&app).unwrap_or_default();
        let global = load_global_agent_config(&app).unwrap_or_default();
        let mut rows = Vec::new();
        for probe in probes {
            match probe {
                Ok((record, key, requested)) => {
                    match start_pair(
                        record.pubkey.clone(),
                        key.relay_url.clone(),
                        true,
                        Some(&record.updated_at),
                        None,
                        app.clone(),
                    ) {
                        Ok(mut status) => {
                            status.requested_relay_url = Some(requested);
                            rows.push(status);
                        }
                        Err(error) => {
                            let mut status = status_for_with(
                                &app,
                                &record,
                                &key,
                                None,
                                Some(requested),
                                StatusInputs {
                                    personas: &personas,
                                    global: &global,
                                },
                            );
                            status.lifecycle = ManagedAgentRuntimeLifecycle::Failed;
                            status.error = Some(error);
                            rows.push(status);
                        }
                    }
                }
                Err((record, requested, error)) => {
                    // Per-community degradation: a relay URL that cannot even
                    // form a pair key gets a Failed row (with the raw
                    // requested URL) like any other probe failure, instead of
                    // aborting every other community's row.
                    let status =
                        match ManagedAgentRuntimeKey::new(record.pubkey.clone(), &requested) {
                            Ok(key) => {
                                let mut status = status_for_with(
                                    &app,
                                    &record,
                                    &key,
                                    None,
                                    Some(requested),
                                    StatusInputs {
                                        personas: &personas,
                                        global: &global,
                                    },
                                );
                                status.lifecycle = ManagedAgentRuntimeLifecycle::Failed;
                                status.error = Some(error);
                                status
                            }
                            Err(_) => unkeyable_failed_status(
                                &record, requested, error, &personas, &global,
                            ),
                        };
                    rows.push(status);
                }
            }
        }
        rows
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[cfg(test)]
#[path = "runtime_commands/tests.rs"]
mod tests;
