use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, OnceLock,
};

use tauri::{AppHandle, Emitter, Manager};

use super::{
    agent_readiness, append_log_marker, current_instance_id, find_managed_agent_mut,
    load_global_agent_config, load_managed_agents, load_personas, managed_agent_runtime_log_path,
    managed_process_identity, record_agent_command, reserve_managed_agent_start,
    resolve_effective_agent_env, save_managed_agents, spawn_agent_child, terminate_managed_process,
    terminate_untracked_pair_runtime, write_agent_runtime_receipt, AgentReadiness, BackendKind,
    ManagedAgentPairRuntime, ManagedAgentRuntimeKey, ManagedAgentRuntimeLifecycle,
    ManagedAgentRuntimeReceipt, ManagedAgentRuntimeStatus,
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
    let local_setup = local_setup_for(record, StatusInputs { personas, global });
    status_for_with_local_setup(app, key, runtime, requested_relay_url, local_setup)
}

fn local_setup_for(record: &super::ManagedAgentRecord, inputs: StatusInputs<'_>) -> bool {
    local_setup_for_with(record, inputs, agent_readiness)
}

fn local_setup_for_with<F>(
    record: &super::ManagedAgentRecord,
    inputs: StatusInputs<'_>,
    evaluate_readiness: F,
) -> bool
where
    F: FnOnce(&super::readiness::EffectiveAgentEnv) -> AgentReadiness,
{
    let StatusInputs { personas, global } = inputs;
    let command = record_agent_command(record, personas);
    let metadata = super::known_acp_runtime(&command);
    let effective = resolve_effective_agent_env(record, personas, metadata, global);
    matches!(evaluate_readiness(&effective), AgentReadiness::Ready)
}

fn status_for_with_local_setup(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    runtime: Option<&ManagedAgentPairRuntime>,
    requested_relay_url: Option<String>,
    local_setup: bool,
) -> ManagedAgentRuntimeStatus {
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
    let personas = load_personas(&app).unwrap_or_default();
    let global = load_global_agent_config(&app).unwrap_or_default();
    let local_setup = local_setup_for(
        record,
        StatusInputs {
            personas: &personas,
            global: &global,
        },
    );
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
    let status = status_for_with_local_setup(&app, &key, Some(runtime), None, local_setup);
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
struct RuntimeProcessSnapshot {
    key: ManagedAgentRuntimeKey,
    lifecycle: ManagedAgentRuntimeLifecycle,
    tracked_pid: u32,
    status_pid: Option<u32>,
    error: Option<String>,
    start_nonce: String,
    exited: bool,
    exit_code: Option<i32>,
}

#[derive(Clone)]
struct RuntimeStatusSnapshot {
    process: RuntimeProcessSnapshot,
    record: super::ManagedAgentRecord,
}

fn status_for_snapshot(
    app: &AppHandle,
    snapshot: &RuntimeStatusSnapshot,
    inputs: StatusInputs<'_>,
) -> ManagedAgentRuntimeStatus {
    let local_setup = local_setup_for(&snapshot.record, inputs);
    ManagedAgentRuntimeStatus {
        pubkey: snapshot.process.key.pubkey.clone(),
        relay_url: snapshot.process.key.relay_url.clone(),
        requested_relay_url: None,
        local_setup,
        lifecycle: snapshot.process.lifecycle.clone(),
        pid: snapshot.process.status_pid,
        error: snapshot.process.error.clone(),
        log_path: managed_agent_runtime_log_path(app, &snapshot.process.key)
            .ok()
            .map(|path| path.display().to_string()),
    }
}

fn collect_runtime_process_snapshots(
    state: &AppState,
) -> Result<(u64, Vec<RuntimeProcessSnapshot>), String> {
    // Phase 1: hold the runtime-management locks only long enough to inspect
    // child generations and copy immutable scalar state. No file or command
    // discovery and no authentication process is allowed in this scope.
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let store_generation = super::managed_agent_store_generation();
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    let snapshots = runtimes
        .iter_mut()
        .map(|(key, runtime)| {
            let tracked_pid = runtime.child.id();
            let (exited, exit_code, error) = match runtime.child.try_wait() {
                Ok(Some(status)) => (
                    true,
                    status.code(),
                    Some(format!(
                        "managed agent runtime exited unexpectedly ({status})"
                    )),
                ),
                Err(error) => (
                    true,
                    None,
                    Some(format!("failed to inspect managed agent runtime: {error}")),
                ),
                Ok(None) => (false, None, runtime.error.clone()),
            };
            RuntimeProcessSnapshot {
                key: key.clone(),
                lifecycle: if exited {
                    ManagedAgentRuntimeLifecycle::Failed
                } else {
                    runtime.lifecycle.clone()
                },
                tracked_pid,
                status_pid: (!exited).then_some(tracked_pid),
                error,
                start_nonce: runtime.start_nonce.clone(),
                exited,
                exit_code,
            }
        })
        .collect();
    Ok((store_generation, snapshots))
}

fn ensure_store_generation_unchanged(
    expected: u64,
    current: u64,
    context: &str,
) -> Result<(), String> {
    if current == expected {
        Ok(())
    } else {
        Err(format!("managed-agent store changed {context}"))
    }
}

fn runtime_generation_matches(
    expected_nonce: &str,
    expected_pid: u32,
    live_nonce: &str,
    live_pid: u32,
    live_exited: bool,
) -> bool {
    expected_nonce == live_nonce && expected_pid == live_pid && live_exited
}

fn persist_exited_runtime_snapshots(
    app: &AppHandle,
    state: &AppState,
    store_generation: u64,
    snapshots: &[RuntimeStatusSnapshot],
    records: &mut [super::ManagedAgentRecord],
) -> Result<(), String> {
    let exited: Vec<_> = snapshots
        .iter()
        .filter(|snapshot| snapshot.process.exited)
        .collect();
    if exited.is_empty() {
        return Ok(());
    }

    // Phase 4: only exited-runtime persistence reacquires the locks. Verify
    // both the store and process generation before applying delayed results.
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    ensure_store_generation_unchanged(
        store_generation,
        super::managed_agent_store_generation(),
        "while status probes were in flight",
    )?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    for snapshot in &exited {
        let process = &snapshot.process;
        let unchanged = runtimes.get_mut(&process.key).is_some_and(|runtime| {
            let live_nonce = runtime.start_nonce.clone();
            let live_pid = runtime.child.id();
            let live_exited = !matches!(runtime.child.try_wait(), Ok(None));
            runtime_generation_matches(
                &process.start_nonce,
                process.tracked_pid,
                &live_nonce,
                live_pid,
                live_exited,
            )
        });
        if !unchanged {
            return Err("managed runtime changed while status probes were in flight".into());
        }
    }

    for snapshot in &exited {
        let process = &snapshot.process;
        if let Some(record) = records
            .iter_mut()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&process.key.pubkey))
        {
            record.updated_at = snapshot.record.updated_at.clone();
            record.last_stopped_at = snapshot.record.last_stopped_at.clone();
            record.last_exit_code = process.exit_code;
            record.last_error = process.error.clone();
            record.last_error_code = process.exit_code.map(i64::from);
        }
    }
    save_managed_agents(app, records)?;
    for snapshot in exited {
        runtimes.remove(&snapshot.process.key);
        super::remove_agent_runtime_receipt(app, &snapshot.process.key);
        state.clear_agent_session_cache(&snapshot.process.key);
    }
    Ok(())
}

fn list_managed_agent_runtimes_blocking(
    app: AppHandle,
) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    let state = app.state::<AppState>();
    let (store_generation, process_snapshots) = collect_runtime_process_snapshots(&state)?;

    // Phase 3: all filesystem reads, command discovery, and authentication
    // probes happen after every runtime-management lock has been released.
    let personas = load_personas(&app).unwrap_or_default();
    let global = load_global_agent_config(&app).unwrap_or_default();
    let mut records = load_managed_agents(&app)?;
    ensure_store_generation_unchanged(
        store_generation,
        super::managed_agent_store_generation(),
        "during status snapshot",
    )?;
    let snapshots: Vec<_> = process_snapshots
        .into_iter()
        .filter_map(|process| {
            let record = records
                .iter()
                .find(|record| record.pubkey.eq_ignore_ascii_case(&process.key.pubkey))?;
            let mut record = record.clone();
            if process.exited {
                record.updated_at = crate::util::now_iso();
                record.last_stopped_at = Some(record.updated_at.clone());
            }
            Some(RuntimeStatusSnapshot { process, record })
        })
        .collect();

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

    persist_exited_runtime_snapshots(&app, &state, store_generation, &snapshots, &mut records)?;

    for (snapshot, status) in snapshots.iter().zip(&statuses) {
        if snapshot.process.exited {
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
    let personas = load_personas(&app).unwrap_or_default();
    let global = load_global_agent_config(&app).unwrap_or_default();
    let readiness_records = load_managed_agents(&app)?;
    let readiness_record = readiness_records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(&pubkey))
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    let readiness_updated_at = readiness_record.updated_at.clone();
    // Authentication readiness may launch a CLI. Resolve it before runtime
    // locks, then fence the result against the record generation below.
    let local_setup = local_setup_for(
        readiness_record,
        StatusInputs {
            personas: &personas,
            global: &global,
        },
    );
    let key = ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    let state = app.state::<AppState>();
    let owner = state
        .keys
        .lock()
        .ok()
        .map(|keys| keys.public_key().to_hex());

    // Phase A: validate and reserve this exact pair while holding locks only
    // long enough to copy the immutable record snapshot. The reservation, not
    // a held mutex, prevents a concurrent start from spawning a duplicate.
    let (record_snapshot, _reservation) = {
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
        let records = load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&readiness_record.pubkey))
            .ok_or_else(|| format!("agent {} not found", readiness_record.pubkey))?;
        if record.updated_at != readiness_updated_at {
            return Err("managed agent changed while readiness was in flight".into());
        }
        if record.backend != BackendKind::Local {
            return Err("managed runtime pairs require a local agent".into());
        }
        if expected_updated_at.is_some_and(|expected| record.updated_at != expected) {
            return Err("managed agent changed while runtime reconciliation was in flight".into());
        }
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        if runtimes
            .get_mut(&key)
            .is_some_and(|runtime| runtime.child.try_wait().ok().flatten().is_none())
        {
            return Ok(status_for_with_local_setup(
                &app,
                &key,
                runtimes.get(&key),
                None,
                local_setup,
            ));
        }
        runtimes.remove(&key);
        let reservation = reserve_managed_agent_start(&state, &key)?;
        (record.clone(), reservation)
    };

    // Phase B: command discovery, readiness, log setup, and both the login
    // probe and buzz-acp spawn happen with every runtime lock released.
    terminate_untracked_pair_runtime(&app, &key)?;
    let spawned = spawn_agent_child(
        &app,
        &record_snapshot,
        &key.relay_url,
        lazy,
        owner.as_deref(),
    )?;
    let mut spawned = Some(spawned);
    let mut wrote_receipt = false;

    // Phase C: generation-fence the unlocked result and register it briefly.
    // Any shutdown, record edit, or competing live runtime wins; the newly
    // spawned child is then terminated only after these guards are dropped.
    let registration = (|| -> Result<ManagedAgentRuntimeStatus, String> {
        let _transition = state
            .managed_agent_runtime_transition
            .lock()
            .map_err(|e| e.to_string())?;
        if state.shutdown_started.load(Ordering::Acquire) {
            return Err("desktop shutdown started while managed runtime was spawning".into());
        }
        let _store = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let record = find_managed_agent_mut(&mut records, &record_snapshot.pubkey)?;
        if record.updated_at != record_snapshot.updated_at {
            return Err("managed agent changed while runtime was spawning".into());
        }
        if record.backend != BackendKind::Local {
            return Err("managed runtime pairs require a local agent".into());
        }
        if expected_updated_at.is_some_and(|expected| record.updated_at != expected) {
            return Err("managed agent changed while runtime reconciliation was in flight".into());
        }
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        if runtimes
            .get_mut(&key)
            .is_some_and(|runtime| runtime.child.try_wait().ok().flatten().is_none())
        {
            return Ok(status_for_with_local_setup(
                &app,
                &key,
                runtimes.get(&key),
                None,
                local_setup,
            ));
        }
        runtimes.remove(&key);

        let process = spawned
            .as_ref()
            .ok_or_else(|| "managed runtime spawn result was already consumed".to_string())?;
        let now = crate::util::now_iso();
        let receipt = ManagedAgentRuntimeReceipt {
            key: key.clone(),
            pid: process.child.id(),
            process_identity: managed_process_identity(process),
            desktop_instance_id: current_instance_id(&app),
            started_at: now.clone(),
        };
        write_agent_runtime_receipt(&app, &receipt)?;
        wrote_receipt = true;
        record.runtime_pid = None;
        record.updated_at = now.clone();
        record.last_started_at = Some(now);
        record.last_stopped_at = None;
        record.last_error = None;
        let Some(process) = spawned.take() else {
            return Err("managed runtime spawn result was already consumed".into());
        };
        runtimes.insert(key.clone(), ManagedAgentPairRuntime::starting(process));
        let status = status_for_with_local_setup(&app, &key, runtimes.get(&key), None, local_setup);
        if let Err(error) = save_managed_agents(&app, &records) {
            spawned = runtimes.remove(&key).map(|runtime| runtime.process);
            return Err(error);
        }
        Ok(status)
    })();

    if let Some(mut process) = spawned {
        let _ = terminate_managed_process(&mut process);
        let _ = process.child.wait();
        if wrote_receipt {
            super::remove_agent_runtime_receipt(&app, &key);
        }
    }
    if let Ok(status) = &registration {
        emit_status(&app, status);
    }
    registration
}

#[tauri::command]
pub fn stop_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let personas = load_personas(&app).unwrap_or_default();
    let global = load_global_agent_config(&app).unwrap_or_default();
    let readiness_records = load_managed_agents(&app)?;
    let readiness_record = readiness_records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(&pubkey))
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    let readiness_updated_at = readiness_record.updated_at.clone();
    let local_setup = local_setup_for(
        readiness_record,
        StatusInputs {
            personas: &personas,
            global: &global,
        },
    );
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
    if record.updated_at != readiness_updated_at {
        return Err("managed agent changed while readiness was in flight".into());
    }
    let key = ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    if let Some(mut runtime) = runtimes.remove(&key) {
        let stop_result = terminate_managed_process(&mut runtime.process)
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
    let status = status_for_with_local_setup(&app, &key, None, None, local_setup);
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

    #[test]
    fn stale_runtime_discovery_is_rejected_after_store_generation_changes() {
        use std::sync::{
            atomic::{AtomicU64, Ordering},
            Arc, Barrier,
        };

        let generation = Arc::new(AtomicU64::new(41));
        let discovery_started = Arc::new(Barrier::new(2));
        let allow_persist = Arc::new(Barrier::new(2));
        let worker = std::thread::spawn({
            let generation = Arc::clone(&generation);
            let discovery_started = Arc::clone(&discovery_started);
            let allow_persist = Arc::clone(&allow_persist);
            move || {
                let captured = generation.load(Ordering::SeqCst);
                discovery_started.wait();
                allow_persist.wait();
                ensure_store_generation_unchanged(
                    captured,
                    generation.load(Ordering::SeqCst),
                    "while a delayed runtime result was in flight",
                )
            }
        });
        discovery_started.wait();
        generation.store(42, Ordering::SeqCst);
        allow_persist.wait();
        let result = worker.join().expect("delayed discovery worker");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("managed-agent store changed"));
    }

    #[test]
    fn stale_runtime_discovery_is_rejected_after_process_generation_changes() {
        assert!(runtime_generation_matches(
            "generation-a",
            100,
            "generation-a",
            100,
            true
        ));
        assert!(!runtime_generation_matches(
            "generation-a",
            100,
            "generation-b",
            100,
            true
        ));
        assert!(!runtime_generation_matches(
            "generation-a",
            100,
            "generation-a",
            101,
            true
        ));
        assert!(!runtime_generation_matches(
            "generation-a",
            100,
            "generation-a",
            100,
            false
        ));
    }

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

    #[test]
    fn runtime_lifecycle_locks_are_available_while_readiness_is_blocked() {
        use std::sync::{mpsc, Arc};

        let state = Arc::new(crate::app_state::build_app_state());
        let (probe_started_tx, probe_started_rx) = mpsc::channel();
        let (release_probe_tx, release_probe_rx) = mpsc::channel();
        let worker_state = Arc::clone(&state);
        let worker = std::thread::spawn(move || {
            let _snapshot = collect_runtime_process_snapshots(&worker_state)
                .expect("runtime snapshot should succeed");
            let record = record_with_relay("");
            let personas = Vec::new();
            let global = super::super::GlobalAgentConfig::default();
            local_setup_for_with(
                &record,
                StatusInputs {
                    personas: &personas,
                    global: &global,
                },
                |_| {
                    probe_started_tx.send(()).expect("announce fake probe");
                    release_probe_rx.recv().expect("release fake probe");
                    AgentReadiness::Ready
                },
            )
        });

        probe_started_rx.recv().expect("fake readiness started");
        assert!(state.managed_agent_runtime_transition.try_lock().is_ok());
        assert!(state.managed_agents_store_lock.try_lock().is_ok());
        assert!(state.managed_agent_processes.try_lock().is_ok());
        release_probe_tx.send(()).expect("release fake readiness");
        assert!(worker.join().expect("snapshot worker"));
    }

    #[test]
    fn runtime_start_reservation_spans_unlocked_spawn_without_holding_lifecycle_locks() {
        use std::sync::{mpsc, Arc};

        let state = Arc::new(crate::app_state::build_app_state());
        let key = ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://localhost:3000")
            .expect("valid runtime key");
        let reservation =
            reserve_managed_agent_start(&state, &key).expect("first start reserves the pair");
        let (spawn_started_tx, spawn_started_rx) = mpsc::channel();
        let (release_spawn_tx, release_spawn_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            spawn_started_tx.send(()).expect("announce fake spawn");
            release_spawn_rx.recv().expect("release fake spawn");
        });

        spawn_started_rx.recv().expect("fake spawn started");
        assert!(state.managed_agent_runtime_transition.try_lock().is_ok());
        assert!(state.managed_agents_store_lock.try_lock().is_ok());
        assert!(state.managed_agent_processes.try_lock().is_ok());
        assert!(reserve_managed_agent_start(&state, &key).is_err());

        release_spawn_tx.send(()).expect("release fake spawn");
        worker.join().expect("fake spawn worker");
        drop(reservation);
        assert!(reserve_managed_agent_start(&state, &key).is_ok());
    }

    #[test]
    fn windows_recovery_teardown_never_launches_an_external_helper() {
        let source = include_str!("process_lifecycle.rs");
        assert!(!source.contains("Command::new"));
        assert!(!source.to_ascii_lowercase().contains("taskkill"));
    }

    #[test]
    fn windows_recovery_teardown_finds_descendants_by_generation() {
        let entries = [
            (10, 1),
            (11, 10),
            (12, 10),
            (13, 11),
            (14, 13),
            (99, 1),
            (11, 10),
        ];
        assert_eq!(
            super::super::runtime::descendant_process_waves(&entries, 10),
            vec![vec![11, 12], vec![13], vec![14]]
        );
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
