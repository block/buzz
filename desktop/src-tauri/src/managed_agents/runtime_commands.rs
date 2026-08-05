use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, OnceLock,
};

use tauri::{AppHandle, Emitter, Manager};

use super::{
    agent_readiness, append_log_marker, current_instance_id, find_managed_agent_mut,
    load_global_agent_config, load_managed_agents, load_personas, managed_agent_runtime_log_path,
    process_is_running, record_agent_command, resolve_effective_agent_env, save_managed_agents,
    spawn_agent_child, terminate_process, terminate_untracked_pair_runtime,
    write_agent_runtime_receipt, AgentReadiness, BackendKind, ManagedAgentPairRuntime,
    ManagedAgentRuntimeKey, ManagedAgentRuntimeLifecycle, ManagedAgentRuntimeReceipt,
    ManagedAgentRuntimeStatus,
};
use crate::app_state::AppState;

const STATUS_EVENT: &str = "managed-agent-runtime-status";

type RuntimeListResult = Result<Vec<ManagedAgentRuntimeStatus>, String>;

#[derive(Clone)]
struct RuntimeListFlight {
    id: u64,
    result: tokio::sync::watch::Receiver<Option<RuntimeListResult>>,
}

#[derive(Default)]
struct RuntimeListSingleFlight {
    next_id: AtomicU64,
    current: tokio::sync::Mutex<Option<RuntimeListFlight>>,
}

impl RuntimeListSingleFlight {
    async fn run<F, Fut>(self: &Arc<Self>, compute: F) -> RuntimeListResult
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = RuntimeListResult> + Send + 'static,
    {
        let mut receiver = {
            let mut current = self.current.lock().await;
            if let Some(flight) = current.as_ref() {
                flight.result.clone()
            } else {
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                let (sender, receiver) = tokio::sync::watch::channel(None);
                *current = Some(RuntimeListFlight {
                    id,
                    result: receiver.clone(),
                });
                let coordinator = Arc::clone(self);
                tauri::async_runtime::spawn(async move {
                    let result = compute().await;
                    let _ = sender.send(Some(result));
                    let mut current = coordinator.current.lock().await;
                    if current.as_ref().is_some_and(|flight| flight.id == id) {
                        *current = None;
                    }
                });
                receiver
            }
        };

        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            receiver
                .changed()
                .await
                .map_err(|_| "managed runtime status worker stopped unexpectedly".to_string())?;
        }
    }
}

fn runtime_list_single_flight() -> &'static Arc<RuntimeListSingleFlight> {
    static SINGLE_FLIGHT: OnceLock<Arc<RuntimeListSingleFlight>> = OnceLock::new();
    SINGLE_FLIGHT.get_or_init(|| Arc::new(RuntimeListSingleFlight::default()))
}

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

#[tauri::command]
pub async fn list_managed_agent_runtimes(
    app: AppHandle,
) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    // Runtime status is polled frequently by the desktop. The implementation
    // reads agent configuration, inspects child processes, and may update the
    // managed-agent store, so keep that blocking work off Tauri's IPC thread.
    runtime_list_single_flight()
        .run(move || async move {
            tauri::async_runtime::spawn_blocking(move || list_managed_agent_runtimes_blocking(app))
                .await
                .map_err(|error| format!("managed runtime status worker failed: {error}"))?
        })
        .await
}

#[derive(Clone)]
struct RuntimeStatusSnapshot {
    key: ManagedAgentRuntimeKey,
    record: super::ManagedAgentRecord,
    lifecycle: ManagedAgentRuntimeLifecycle,
    pid: Option<u32>,
    error: Option<String>,
    start_nonce: Option<String>,
    emit_after_reap: bool,
}

fn status_for_snapshot(
    app: &AppHandle,
    snapshot: &RuntimeStatusSnapshot,
    inputs: StatusInputs<'_>,
) -> ManagedAgentRuntimeStatus {
    let command = record_agent_command(&snapshot.record, inputs.personas);
    let metadata = super::known_acp_runtime(&command);
    let effective =
        resolve_effective_agent_env(&snapshot.record, inputs.personas, metadata, inputs.global);
    ManagedAgentRuntimeStatus {
        pubkey: snapshot.key.pubkey.clone(),
        relay_url: snapshot.key.relay_url.clone(),
        requested_relay_url: None,
        local_setup: matches!(agent_readiness(&effective), AgentReadiness::Ready),
        lifecycle: snapshot.lifecycle.clone(),
        pid: snapshot.pid,
        error: snapshot.error.clone(),
        log_path: managed_agent_runtime_log_path(app, &snapshot.key)
            .ok()
            .map(|path| path.display().to_string()),
    }
}

fn list_managed_agent_runtimes_blocking(
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
    let exited_keys: Vec<_> = runtimes
        .iter_mut()
        .filter_map(|(key, runtime)| match runtime.child.try_wait() {
            Ok(Some(_)) | Err(_) => Some(key.clone()),
            Ok(None) => None,
        })
        .collect();
    let records_changed = !exited_keys.is_empty();
    let mut snapshots = Vec::new();
    for key in exited_keys {
        runtimes.remove(&key);
        super::remove_agent_runtime_receipt(&app, &key);
        state.clear_agent_session_cache(&key);
        if let Some(record) = records
            .iter_mut()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey))
        {
            record.updated_at = crate::util::now_iso();
            record.last_stopped_at = Some(record.updated_at.clone());
            snapshots.push(RuntimeStatusSnapshot {
                key,
                record: record.clone(),
                lifecycle: ManagedAgentRuntimeLifecycle::Stopped,
                pid: None,
                error: None,
                start_nonce: None,
                emit_after_reap: true,
            });
        }
    }
    snapshots.extend(runtimes.iter().filter_map(|(key, runtime)| {
        let record = records
            .iter()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey))?;
        Some(RuntimeStatusSnapshot {
            key: key.clone(),
            record: record.clone(),
            lifecycle: runtime.lifecycle.clone(),
            pid: Some(runtime.child.id()),
            error: runtime.error.clone(),
            start_nonce: Some(runtime.start_nonce.clone()),
            emit_after_reap: false,
        })
    }));
    drop(runtimes);
    // Records are only mutated above when a runtime exited — skip the store
    // rewrite on the common nothing-changed poll.
    if records_changed {
        save_managed_agents(&app, &records)?;
    }
    drop(_store);
    drop(_transition);

    // CLI readiness probes may spawn external processes. They run only after
    // all runtime-management locks have been released.
    let statuses: Vec<_> = snapshots
        .iter()
        .map(|snapshot| {
            status_for_snapshot(
                &app,
                snapshot,
                StatusInputs {
                    personas: &personas,
                    global: &global,
                },
            )
        })
        .collect();

    // Validate that no agent or harness generation changed while the probes
    // ran. Delayed results are never applied to a newer generation.
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let current_records = load_managed_agents(&app)?;
    let mut current_runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    for snapshot in &snapshots {
        let unchanged_record = current_records.iter().any(|record| {
            record.pubkey.eq_ignore_ascii_case(&snapshot.key.pubkey)
                && record.updated_at == snapshot.record.updated_at
        });
        let unchanged_runtime = match snapshot.start_nonce.as_deref() {
            Some(start_nonce) => current_runtimes
                .get_mut(&snapshot.key)
                .is_some_and(|runtime| {
                    runtime.start_nonce == start_nonce
                        && runtime.child.id() == snapshot.pid.unwrap_or_default()
                        && runtime.child.try_wait().ok().flatten().is_none()
                }),
            None => !current_runtimes.contains_key(&snapshot.key),
        };
        if !unchanged_record || !unchanged_runtime {
            return Err("managed runtime changed while status probes were in flight".into());
        }
    }
    drop(current_runtimes);
    drop(_store);
    drop(_transition);

    for (snapshot, status) in snapshots.iter().zip(&statuses) {
        if snapshot.emit_after_reap {
            emit_status(&app, status);
        }
    }
    Ok(statuses)
}

pub(crate) fn start_managed_agent_runtime_pair_lazy(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair(pubkey, relay_url, true, None, app)
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
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let state = app.state::<AppState>();
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
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
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    if runtimes
        .get_mut(&key)
        .is_some_and(|runtime| runtime.child.try_wait().ok().flatten().is_none())
    {
        let status = status_for(&app, record, &key, runtimes.get(&key), None);
        return Ok(status);
    }
    runtimes.remove(&key);
    terminate_untracked_pair_runtime(&app, &key)?;

    let owner = state
        .keys
        .lock()
        .ok()
        .map(|keys| keys.public_key().to_hex());
    let mut process = spawn_agent_child(&app, record, &key.relay_url, lazy, owner.as_deref())?;
    let now = crate::util::now_iso();
    let receipt = ManagedAgentRuntimeReceipt {
        key: key.clone(),
        pid: process.child.id(),
        desktop_instance_id: current_instance_id(&app),
        started_at: now.clone(),
    };
    if let Err(error) = write_agent_runtime_receipt(&app, &receipt) {
        let _ = terminate_process(process.child.id());
        let _ = process.child.wait();
        return Err(error);
    }
    record.runtime_pid = None;
    record.updated_at = now.clone();
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_error = None;
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
    let record = find_managed_agent_mut(&mut records, &pubkey)?;
    let key = ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    if let Some(mut runtime) = runtimes.remove(&key) {
        let stop_result = if process_is_running(runtime.child.id()) {
            terminate_process(runtime.child.id())
        } else {
            Ok(())
        }
        .and_then(|()| runtime.child.wait().map_err(|e| e.to_string()));
        match stop_result {
            Ok(status) => {
                record.last_exit_code = status.code();
                let _ = append_log_marker(&runtime.log_path, "=== stopped pair runtime ===");
            }
            Err(error) => {
                // Keep failed teardown visible/manageable instead of
                // orphaning it: the child stays tracked and the receipt
                // stays on disk until a stop actually succeeds.
                runtimes.insert(key, runtime);
                return Err(error);
            }
        }
    } else {
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
    }
    super::remove_agent_runtime_receipt(&app, &key);
    state.clear_agent_session_cache(&key);
    record.runtime_pid = None;
    record.updated_at = crate::util::now_iso();
    record.last_stopped_at = Some(record.updated_at.clone());
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
    stop_managed_agent_runtime(pubkey.clone(), relay_url.clone(), app.clone())?;
    start_pair(pubkey, relay_url, true, None, app)
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
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_list_single_flight_shares_hundreds_of_callers() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let coordinator = Arc::new(RuntimeListSingleFlight::default());
        let barrier = Arc::new(tokio::sync::Barrier::new(201));
        let computations = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let mut callers = Vec::new();

        for _ in 0..200 {
            let coordinator = Arc::clone(&coordinator);
            let barrier = Arc::clone(&barrier);
            let computations = Arc::clone(&computations);
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            callers.push(tokio::spawn(async move {
                barrier.wait().await;
                coordinator
                    .run(move || async move {
                        computations.fetch_add(1, Ordering::SeqCst);
                        let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                        max_active.fetch_max(now_active, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(Vec::new())
                    })
                    .await
            }));
        }
        barrier.wait().await;
        for caller in callers {
            assert!(caller.await.expect("caller task").is_ok());
        }
        assert_eq!(computations.load(Ordering::SeqCst), 1);
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn runtime_list_computation_survives_first_caller_cancellation() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let coordinator = Arc::new(RuntimeListSingleFlight::default());
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let finished = Arc::new(AtomicBool::new(false));
        let first = {
            let coordinator = Arc::clone(&coordinator);
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            let finished = Arc::clone(&finished);
            tokio::spawn(async move {
                coordinator
                    .run(move || async move {
                        started.notify_one();
                        release.notified().await;
                        finished.store(true, Ordering::SeqCst);
                        Ok(Vec::new())
                    })
                    .await
            })
        };
        started.notified().await;
        first.abort();

        let second = coordinator.run(|| async { panic!("a second computation must not start") });
        tokio::pin!(second);
        tokio::select! {
            result = &mut second => panic!("shared computation finished too early: {result:?}"),
            () = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
        }
        release.notify_one();
        assert!(second.await.is_ok());
        assert!(finished.load(Ordering::SeqCst));
    }

    fn payload(
        relay_url: &str,
        lifecycle: ManagedAgentRuntimeLifecycle,
        error: Option<&str>,
    ) -> super::super::ManagedAgentRuntimeLifecycleObserverPayload {
        super::super::ManagedAgentRuntimeLifecycleObserverPayload {
            pubkey: "aa".repeat(32),
            relay_url: relay_url.into(),
            start_nonce: "test-generation".into(),
            lifecycle,
            error: error.map(str::to_owned),
        }
    }

    fn record_with_relay(relay_url: &str) -> super::super::ManagedAgentRecord {
        serde_json::from_str(&format!(
            r#"{{
                "pubkey": "{}",
                "name": "pin-test",
                "relay_url": "{relay_url}",
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": "",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }}"#,
            "aa".repeat(32)
        ))
        .unwrap()
    }

    #[test]
    fn legacy_relay_pin_is_ignored_for_fan_out() {
        // Zero-touch cutover (#2122): a record carrying a creation-era
        // `relay_url` pin must fan out exactly like an unpinned one — the
        // stored field is parsed but never consulted. See
        // `effective_agent_relay_url`.
        let unpinned = record_with_relay("");
        let pinned = record_with_relay("wss://one.example");
        for record in [&unpinned, &pinned] {
            assert_eq!(
                crate::relay::effective_agent_relay_url(&record.relay_url, "wss://two.example"),
                "wss://two.example"
            );
        }
    }

    #[test]
    fn unkeyable_relay_degrades_to_failed_row() {
        // A requested URL that cannot form a pair key must still yield a
        // Failed row keyed by the raw requested string, so one bad community
        // never aborts the rest of the reconcile batch.
        let record = record_with_relay("");
        let status = unkeyable_failed_status(
            &record,
            "not a url".to_string(),
            "relay access probe timed out".to_string(),
            &[],
            &super::super::GlobalAgentConfig::default(),
        );
        assert!(matches!(
            status.lifecycle,
            ManagedAgentRuntimeLifecycle::Failed
        ));
        assert_eq!(status.relay_url, "not a url");
        assert_eq!(status.requested_relay_url.as_deref(), Some("not a url"));
        assert_eq!(status.pubkey, record.pubkey);
        assert_eq!(
            status.error.as_deref(),
            Some("relay access probe timed out")
        );
        assert!(status.pid.is_none());
    }

    #[test]
    fn runtime_key_rejects_non_hex_pubkeys() {
        assert!(ManagedAgentRuntimeKey::new("../not-a-key", "wss://relay.example").is_err());
        assert!(ManagedAgentRuntimeKey::new("gg".repeat(32), "wss://relay.example").is_err());
    }

    #[test]
    fn runtime_key_canonicalizes_hex_pubkeys() {
        let key = ManagedAgentRuntimeKey::new("AA".repeat(32), "wss://relay.example").unwrap();
        assert_eq!(key.pubkey, "aa".repeat(32));
    }

    #[test]
    fn observer_lifecycle_key_preserves_exact_canonical_pair() {
        let first = payload(
            "WSS://Relay.Example:443/",
            ManagedAgentRuntimeLifecycle::Ready,
            None,
        );
        let key = observer_lifecycle_key(&first.pubkey, &first).unwrap();
        assert_eq!(key.pubkey, first.pubkey);
        assert_eq!(key.relay_url, "wss://relay.example");

        let other = payload(
            "wss://other.example",
            ManagedAgentRuntimeLifecycle::Ready,
            None,
        );
        assert_ne!(key, observer_lifecycle_key(&other.pubkey, &other).unwrap());
    }

    #[test]
    fn observer_lifecycle_rejects_cross_agent_and_desktop_states() {
        let ready = payload(
            "wss://relay.example",
            ManagedAgentRuntimeLifecycle::Ready,
            None,
        );
        assert!(observer_lifecycle_key(&"bb".repeat(32), &ready).is_err());

        let stopped = payload(
            "wss://relay.example",
            ManagedAgentRuntimeLifecycle::Stopped,
            None,
        );
        assert!(observer_lifecycle_key(&stopped.pubkey, &stopped).is_err());
    }

    #[test]
    fn observer_lifecycle_enforces_failed_error_contract() {
        let failed = payload(
            "wss://relay.example",
            ManagedAgentRuntimeLifecycle::Failed,
            None,
        );
        assert!(observer_lifecycle_key(&failed.pubkey, &failed).is_err());

        let ready_with_error = payload(
            "wss://relay.example",
            ManagedAgentRuntimeLifecycle::Ready,
            Some("unexpected"),
        );
        assert!(observer_lifecycle_key(&ready_with_error.pubkey, &ready_with_error).is_err());
    }
}
