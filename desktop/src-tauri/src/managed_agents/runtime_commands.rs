use std::sync::atomic::Ordering;

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

fn status_for(
    app: &AppHandle,
    record: &super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    runtime: Option<&ManagedAgentPairRuntime>,
    requested_relay_url: Option<String>,
) -> ManagedAgentRuntimeStatus {
    let personas = load_personas(app).unwrap_or_default();
    let global = load_global_agent_config(app).unwrap_or_default();
    let command = record_agent_command(record, &personas);
    let metadata = super::known_acp_runtime(&command);
    let effective = resolve_effective_agent_env(record, &personas, metadata, &global);
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
    if outer_pubkey.to_ascii_lowercase() != payload.pubkey.to_ascii_lowercase() {
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
pub fn list_managed_agent_runtimes(
    app: AppHandle,
) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    let state = app.state::<AppState>();
    let records = load_managed_agents(&app)?;
    let runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    Ok(runtimes
        .iter()
        .filter_map(|(key, runtime)| {
            let record = records.iter().find(|record| record.pubkey == key.pubkey)?;
            Some(status_for(&app, record, key, Some(runtime), None))
        })
        .collect())
}

#[tauri::command]
pub fn start_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair(pubkey, relay_url, true, app)
}

fn start_pair(
    pubkey: String,
    relay_url: String,
    lazy: bool,
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
        if process_is_running(runtime.child.id()) {
            terminate_process(runtime.child.id())?;
        }
        let status = runtime.child.wait().map_err(|e| e.to_string())?;
        record.last_exit_code = status.code();
        let _ = append_log_marker(&runtime.log_path, "=== stopped pair runtime ===");
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
    start_pair(pubkey, relay_url, true, app)
}

async fn discover_agent_membership(
    state: &AppState,
    record: super::ManagedAgentRecord,
    requested_relay_url: String,
) -> Result<Option<(super::ManagedAgentRecord, ManagedAgentRuntimeKey, String)>, String> {
    let key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), &requested_relay_url)?;
    let keys = nostr::Keys::parse(record.private_key_nsec.trim())
        .map_err(|error| format!("invalid managed-agent key: {error}"))?;
    let api_base = crate::relay::relay_http_base_url(&key.relay_url);
    let events = tokio::time::timeout(
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
    .map_err(|_| "membership discovery timed out".to_string())??;
    Ok((!events.is_empty()).then_some((record, key, requested_relay_url)))
}

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
            .filter(|record| record.backend == BackendKind::Local)
        {
            jobs.push((record.clone(), community.relay_url.clone()));
        }
    }
    let discoveries: Vec<_> = stream::iter(jobs)
        .map(|(record, requested)| {
            let state = app.state::<AppState>();
            async move {
                let fallback_record = record.clone();
                let fallback_requested = requested.clone();
                discover_agent_membership(&state, record, requested)
                    .await
                    .map_err(|error| (fallback_record, fallback_requested, error))
            }
        })
        .buffer_unordered(6)
        .collect()
        .await;

    let mut rows = Vec::new();
    for discovery in discoveries {
        match discovery {
            Ok(Some((record, key, requested))) => {
                match start_pair(
                    record.pubkey.clone(),
                    key.relay_url.clone(),
                    true,
                    app.clone(),
                ) {
                    Ok(mut status) => {
                        status.requested_relay_url = Some(requested);
                        rows.push(status);
                    }
                    Err(error) => {
                        let mut status = status_for(&app, &record, &key, None, Some(requested));
                        status.lifecycle = ManagedAgentRuntimeLifecycle::Failed;
                        status.error = Some(error);
                        rows.push(status);
                    }
                }
            }
            Ok(None) => {}
            Err((record, requested, error)) => {
                let key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), &requested)?;
                let mut status = status_for(&app, &record, &key, None, Some(requested));
                status.lifecycle = ManagedAgentRuntimeLifecycle::Failed;
                status.error = Some(error);
                rows.push(status);
            }
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(
        relay_url: &str,
        lifecycle: ManagedAgentRuntimeLifecycle,
        error: Option<&str>,
    ) -> super::super::ManagedAgentRuntimeLifecycleObserverPayload {
        super::super::ManagedAgentRuntimeLifecycleObserverPayload {
            pubkey: "aa".repeat(32),
            relay_url: relay_url.into(),
            lifecycle,
            error: error.map(str::to_owned),
        }
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
