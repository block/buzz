use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use tauri::{AppHandle, Emitter, Manager};

use super::{
    agent_readiness, append_log_marker, find_managed_agent_mut, load_global_agent_config,
    load_managed_agents, load_personas, record_agent_command, resolve_effective_agent_env,
    save_managed_agents, spawn_agent_child, AgentReadiness, BackendKind, LegacyMigrationGate,
    ManagedAgentPairRuntime, ManagedAgentProcess, ManagedAgentRuntimeKey,
    ManagedAgentRuntimeLifecycle, ManagedAgentRuntimeStatus,
};
use crate::app_state::AppState;

const STATUS_EVENT: &str = "managed-agent-runtime-status";
mod status;
use status::{migration_status, status_for, status_for_with, StatusInputs};

fn active_job_status(
    controller: &buzz_runtime_pkg::client::RuntimeClient,
    status: &buzz_runtime_pkg::protocol::RuntimeStatus,
) -> Option<buzz_runtime_pkg::protocol::JobStatus> {
    status
        .active_job
        .and_then(|job_id| super::block_on_runtime_io(controller.jobs_status(job_id)).ok())
}

pub(crate) fn connect_runtime_receipt(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    mut process: Option<ManagedAgentProcess>,
    wait_for_ready: bool,
) -> Result<ManagedAgentPairRuntime, String> {
    let receipt_path = super::managed_agent_runtime_receipt_path(app, key)?;
    let lock_path = super::managed_agent_runtime_lock_path(app, key)?;
    let deadline = Instant::now()
        + if wait_for_ready {
            Duration::from_secs(5)
        } else {
            Duration::ZERO
        };
    let mut last_error = None;
    loop {
        if receipt_path.exists() {
            match super::adopt_schema_v2_runtime(&receipt_path, key) {
                Ok((receipt, controller, status)) => {
                    super::verify_runtime_lock_proof(&receipt, &lock_path)?;
                    let retained_process = match process.take() {
                        Some(process) if process.child.id() == receipt.pid => Some(process),
                        Some(mut losing_process) => {
                            let _ = losing_process.child.try_wait();
                            None
                        }
                        None => None,
                    };
                    let active_job = active_job_status(&controller, &status);
                    return Ok(ManagedAgentPairRuntime::connected(
                        retained_process,
                        receipt,
                        receipt_path,
                        controller,
                        &status,
                        active_job,
                    ));
                }
                Err(error) => last_error = Some(error),
            }
        }
        if !wait_for_ready || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(mut launched) = process {
        let _ = super::terminate_process(launched.child.id());
        let _ = launched.child.wait();
    }
    Err(last_error.unwrap_or_else(|| {
        format!(
            "runtime did not publish an authenticated ready receipt at {}",
            receipt_path.display()
        )
    }))
}

pub(crate) fn connect_legacy_runtime_receipt(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    mut process: ManagedAgentProcess,
) -> Result<ManagedAgentPairRuntime, String> {
    let receipt_path = super::managed_agent_legacy_runtime_receipt_path(app, key)?;
    let lock_path = super::managed_agent_runtime_lock_path(app, key)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_error = None;
    loop {
        if receipt_path.exists() {
            match buzz_runtime_pkg::read_legacy_runtime_receipt(&receipt_path) {
                Ok(receipt) => {
                    let receipt_key = ManagedAgentRuntimeKey::new(
                        receipt.key.pubkey.clone(),
                        &receipt.key.relay_url,
                    );
                    let key_matches = receipt_key.as_ref().ok() == Some(key);
                    let pid_matches = receipt.pid == process.child.id();
                    let protocol_matches =
                        receipt.lock_protocol_version == super::RUNTIME_LOCK_PROTOCOL_VERSION;
                    let lock_hash_matches =
                        receipt.lock_path_hash == super::runtime_lock_path_hash(&lock_path);
                    let marker_matches = buzz_runtime_pkg::process_matches_marker(
                        receipt.pid,
                        &receipt.process_start_marker,
                    );
                    let lock_is_held = super::pair_lock_is_held(app, key)?;
                    let proof_matches = key_matches
                        && pid_matches
                        && protocol_matches
                        && lock_hash_matches
                        && marker_matches
                        && lock_is_held;
                    if !proof_matches {
                        eprintln!(
                            "[DEBUG-receipt-proof] child_pid={} receipt_pid={} key={} protocol={} lock_hash={} marker={} lock_held={}",
                            process.child.id(),
                            receipt.pid,
                            key_matches,
                            protocol_matches,
                            lock_hash_matches,
                            marker_matches,
                            lock_is_held,
                        );
                    }
                    if proof_matches {
                        let receipt = super::LegacyManagedAgentRuntimeReceipt {
                            schema_version: receipt.schema_version,
                            key: key.clone(),
                            pid: receipt.pid,
                            process_start_marker: receipt.process_start_marker,
                            desktop_instance_id: receipt.desktop_instance_id,
                            started_at: receipt.started_at.to_rfc3339(),
                            lock_protocol_version: receipt.lock_protocol_version,
                            lock_path_hash: receipt.lock_path_hash,
                        };
                        return Ok(ManagedAgentPairRuntime::legacy(
                            process,
                            receipt,
                            receipt_path,
                        ));
                    }
                    last_error =
                        Some("schema-v1 runtime receipt proof does not match launch".into());
                }
                Err(error) => {
                    last_error = Some(format!("invalid schema-v1 runtime receipt: {error}"))
                }
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = super::terminate_process(process.child.id());
    let _ = process.child.wait();
    Err(last_error.unwrap_or_else(|| {
        format!(
            "legacy runtime did not publish a lock-proven receipt at {}",
            receipt_path.display()
        )
    }))
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
    if runtime.observer_nonce.as_deref() != Some(payload.start_nonce.as_str()) {
        return Err("lifecycle frame does not match the current harness generation".into());
    }
    runtime.lifecycle = payload.lifecycle;
    runtime.error = payload.error;
    let status = status_for(&app, record, &key, Some(runtime), None);
    emit_status(&app, &status);
    Ok(status)
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
    let records = load_managed_agents(&app)?;
    let probes = {
        let runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        runtimes
            .iter()
            .filter_map(|(key, runtime)| {
                runtime
                    .controller
                    .clone()
                    .map(|controller| (key.clone(), controller))
            })
            .collect::<Vec<_>>()
    };
    let probe_results = probes
        .into_iter()
        .map(|(key, controller)| {
            let result = super::block_on_runtime_io(controller.status());
            let active_job = result
                .as_ref()
                .ok()
                .and_then(|status| active_job_status(&controller, status));
            (key, result, active_job)
        })
        .collect::<Vec<_>>();
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    for (key, result, active_job) in probe_results {
        let Some(runtime) = runtimes.get_mut(&key) else {
            continue;
        };
        match result {
            Ok(status)
                if runtime.receipt.as_ref().is_some_and(|receipt| {
                    status.runtime_id == receipt.runtime_id
                        && status.generation == receipt.generation
                }) =>
            {
                runtime.apply_authenticated_status(&status, active_job);
            }
            Ok(_) => {
                runtime.lifecycle = ManagedAgentRuntimeLifecycle::Failed;
                runtime.error = Some("authenticated runtime status changed identity".into());
            }
            Err(error) => {
                runtime.lifecycle = ManagedAgentRuntimeLifecycle::Failed;
                runtime.error = Some(format!("runtime control unavailable: {error}"));
            }
        }
    }
    let statuses = runtimes
        .iter()
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
        })
        .collect();
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
pub(crate) fn start_pair(
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
    if let Some(runtime) = runtimes.get_mut(&key) {
        if runtime.is_legacy()
            && runtime.legacy_receipt.as_ref().is_some_and(|receipt| {
                buzz_runtime_pkg::process_matches_marker(receipt.pid, &receipt.process_start_marker)
            })
        {
            return Ok(status_for(&app, record, &key, Some(runtime), None));
        }
        if let Some(controller) = runtime.controller.clone() {
            if let Ok(control_status) = super::block_on_runtime_io(controller.status()) {
                let active_job = active_job_status(&controller, &control_status);
                runtime.apply_authenticated_status(&control_status, active_job);
                return Ok(status_for(&app, record, &key, Some(runtime), None));
            }
        }
        runtimes.remove(&key);
    }

    let preferred_launch_mode = super::managed_runtime_feature_gates().launch_mode();
    let receipt_path = super::managed_agent_runtime_receipt_path(&app, &key)?;
    let had_v2_receipt = receipt_path.exists();
    if had_v2_receipt {
        match connect_runtime_receipt(&app, &key, None, false) {
            Ok(runtime) => {
                runtimes.insert(key.clone(), runtime);
                let status = status_for(&app, record, &key, runtimes.get(&key), None);
                emit_status(&app, &status);
                return Ok(status);
            }
            Err(error) if super::pair_lock_is_held(&app, &key)? => {
                return Ok(migration_status(
                    &app,
                    record,
                    &key,
                    ManagedAgentRuntimeLifecycle::Recovering,
                    &format!("runtime receipt is not adoptable while pair lock is held: {error}"),
                ));
            }
            Err(error)
                if matches!(
                    preferred_launch_mode,
                    super::ManagedRuntimeLaunchMode::LegacyPhase0
                ) =>
            {
                return Ok(migration_status(
                    &app,
                    record,
                    &key,
                    ManagedAgentRuntimeLifecycle::Recovering,
                    &format!(
                        "durable runtime recovery required; refusing schema-v1 fallback: {error}"
                    ),
                ));
            }
            Err(_) => {
                super::quarantine_agent_runtime_receipt_path(&receipt_path)?;
            }
        }
    }
    let durable_store_exists = super::managed_agent_runtime_state_path(&app, &key)?
        .join("runtime.sqlite3")
        .exists();
    let needs_phase_zero_decision = !matches!(
        preferred_launch_mode,
        super::ManagedRuntimeLaunchMode::LegacyPhase0
    ) && !had_v2_receipt
        && !durable_store_exists;
    let (proof_exists, migration_gate) = if needs_phase_zero_decision {
        (
            super::managed_agent_legacy_runtime_receipt_path(&app, &key)?.exists(),
            super::legacy_migration_gate(&app, &key, record.runtime_pid)?,
        )
    } else {
        (false, LegacyMigrationGate::Clear)
    };
    let launch_mode = match super::select_rollout_launch_mode(
        preferred_launch_mode,
        had_v2_receipt || durable_store_exists,
        proof_exists,
        migration_gate,
    ) {
        Ok(mode) => mode,
        Err(LegacyMigrationGate::LegacyRuntimeActive) => {
            return Ok(migration_status(
                &app,
                record,
                &key,
                ManagedAgentRuntimeLifecycle::LegacyRuntimeActive,
                "legacy_runtime_active",
            ));
        }
        Err(LegacyMigrationGate::ManualLegacyStopRequired) => {
            return Ok(migration_status(
                &app,
                record,
                &key,
                ManagedAgentRuntimeLifecycle::ManualLegacyStopRequired,
                "manual_legacy_stop_required",
            ));
        }
        Err(LegacyMigrationGate::Clear) => unreachable!("clear migration gate is not blocking"),
    };
    if matches!(launch_mode, super::ManagedRuntimeLaunchMode::LegacyPhase0) && durable_store_exists
    {
        return Ok(migration_status(
            &app,
            record,
            &key,
            ManagedAgentRuntimeLifecycle::Recovering,
            "durable runtime state exists; refusing schema-v1 fallback",
        ));
    }

    let owner = state
        .keys
        .lock()
        .ok()
        .map(|keys| keys.public_key().to_hex());
    let process = match spawn_agent_child(
        &app,
        record,
        &key.relay_url,
        lazy,
        owner.as_deref(),
        launch_mode,
    ) {
        Ok(process) => process,
        Err(spawn_error) => {
            if matches!(
                launch_mode,
                super::ManagedRuntimeLaunchMode::DurableV2 { .. }
            ) {
                if let Ok(runtime) = connect_runtime_receipt(&app, &key, None, true) {
                    runtimes.insert(key.clone(), runtime);
                    let status = status_for(&app, record, &key, runtimes.get(&key), None);
                    emit_status(&app, &status);
                    return Ok(status);
                }
            }
            return Err(spawn_error);
        }
    };
    let runtime = match launch_mode {
        super::ManagedRuntimeLaunchMode::LegacyPhase0 => {
            connect_legacy_runtime_receipt(&app, &key, process)?
        }
        super::ManagedRuntimeLaunchMode::DurableV2 { .. } => {
            connect_runtime_receipt(&app, &key, Some(process), true)?
        }
    };
    let now = crate::util::now_iso();
    record.runtime_pid = runtime.is_legacy().then(|| runtime.pid());
    record.updated_at = now.clone();
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_error = None;
    runtimes.insert(key.clone(), runtime);
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
    let mut runtime = match runtimes.remove(&key) {
        Some(runtime) => runtime,
        None => {
            if let Ok(runtime) = connect_runtime_receipt(&app, &key, None, false) {
                runtime
            } else if super::stop_verified_legacy_runtime(&app, &key, record.runtime_pid)? {
                state.clear_agent_session_cache(&key);
                record.last_exit_code = None;
                record.runtime_pid = None;
                record.updated_at = crate::util::now_iso();
                record.last_stopped_at = Some(record.updated_at.clone());
                let status = status_for(&app, record, &key, None, None);
                drop(runtimes);
                save_managed_agents(&app, &records)?;
                emit_status(&app, &status);
                return Ok(status);
            } else {
                match super::legacy_migration_gate(&app, &key, record.runtime_pid)? {
                    LegacyMigrationGate::LegacyRuntimeActive => {
                        return Ok(migration_status(
                            &app,
                            record,
                            &key,
                            ManagedAgentRuntimeLifecycle::LegacyRuntimeActive,
                            "legacy_runtime_active",
                        ));
                    }
                    LegacyMigrationGate::ManualLegacyStopRequired => {
                        return Ok(migration_status(
                            &app,
                            record,
                            &key,
                            ManagedAgentRuntimeLifecycle::ManualLegacyStopRequired,
                            "manual_legacy_stop_required",
                        ));
                    }
                    LegacyMigrationGate::Clear => {
                        state.clear_agent_session_cache(&key);
                        record.runtime_pid = None;
                        let status = status_for(&app, record, &key, None, None);
                        drop(runtimes);
                        save_managed_agents(&app, &records)?;
                        emit_status(&app, &status);
                        return Ok(status);
                    }
                }
            }
        }
    };

    if runtime.is_legacy() {
        let receipt = runtime
            .legacy_receipt
            .as_ref()
            .ok_or_else(|| "legacy runtime receipt is unavailable".to_string())?;
        if runtime
            .process
            .as_ref()
            .is_none_or(|process| process.child.id() != receipt.pid)
            || !buzz_runtime_pkg::process_matches_marker(receipt.pid, &receipt.process_start_marker)
        {
            runtimes.insert(key.clone(), runtime);
            return Err("legacy runtime identity cannot be verified; refusing PID signal".into());
        }
        super::terminate_process(receipt.pid)?;
        if let Some(process) = runtime.process.as_mut() {
            let _ = process.child.wait();
        }
    } else {
        // One authenticated, generation-fenced shutdown request is the normal
        // schema-v2 path. PID cleanup is a bounded identity-verified fallback.
        let controller = runtime
            .controller
            .as_ref()
            .ok_or_else(|| "runtime has no authenticated controller".to_string())?;
        if let Err(error) = super::block_on_runtime_io(controller.shutdown()) {
            runtimes.insert(key.clone(), runtime);
            return Err(format!(
                "generation-fenced runtime shutdown failed: {error}"
            ));
        }
        let receipt = runtime
            .receipt
            .as_ref()
            .expect("authenticated controller has a schema-v2 receipt");
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if buzz_runtime_pkg::process_start_marker(receipt.pid).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Ok(marker) = buzz_runtime_pkg::process_start_marker(receipt.pid) {
            if marker != receipt.process_start_marker {
                runtimes.insert(key.clone(), runtime);
                return Err(
                    "runtime PID was reused during shutdown; refusing cleanup signal".into(),
                );
            }
            super::terminate_process(receipt.pid)?;
        }
        let _ = super::quarantine_agent_runtime_receipt_path(&runtime.receipt_path);
    }
    if let Some(log_path) = runtime.log_path() {
        let _ = append_log_marker(log_path, "=== stopped pair runtime ===");
    }
    record.last_exit_code = None;
    // Leave a stopped schema-v1 receipt in place as migration proof. The
    // schema-v2 cutover gate quarantines it only after proving the lock is free.
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
        active_assignment: None,
        active_job: None,
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

    // start_pair performs blocking store access, path validation, process
    // spawn, and authenticated receipt adoption, so keep the post-probe start
    // loop off the async workers, matching the restart flows.
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
#[path = "runtime_commands_tests.rs"]
mod tests;
