//! Observer-authored runtime-lifecycle capability command (§3.3a).
//!
//! Extracted here to keep `runtime_commands.rs` under the 1000-line size
//! ratchet; `#[path]`-included from there. Behavior is identical to the inline
//! definition — this is a mechanical move.

use super::*;

pub(super) fn observer_lifecycle_key(
    outer_pubkey: &str,
    payload: &crate::managed_agents::ManagedAgentRuntimeLifecycleObserverPayload,
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
    payload: crate::managed_agents::ManagedAgentRuntimeLifecycleObserverPayload,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    put_managed_agent_runtime_lifecycle_for(outer_pubkey, payload, &app)
}

/// Runtime-generic core so the sibling capability check is testable under `MockRuntime` (§3.3a / §7).
pub(crate) fn put_managed_agent_runtime_lifecycle_for<R: tauri::Runtime>(
    outer_pubkey: String,
    payload: crate::managed_agents::ManagedAgentRuntimeLifecycleObserverPayload,
    app: &tauri::AppHandle<R>,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let key = observer_lifecycle_key(&outer_pubkey, &payload)?;
    let state = app.state::<AppState>();
    let records = load_managed_agents(app)?;
    let record = records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey))
        .ok_or_else(|| format!("agent {} not found", key.pubkey))?;
    // Capture the active scope BEFORE taking the runtime lock so a concurrent
    // workspace switch cannot slip a stale-scope frame past the check.
    let current_scope_id = state.capture_active_scope().map(|scope| scope.scope_id);
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
    if runtime.scope_id != current_scope_id {
        return Err("lifecycle frame does not match the current workspace scope".into());
    }
    let exited = runtime
        .child
        .try_wait()
        .map_err(|e| e.to_string())?
        .is_some();
    if exited {
        return Err("lifecycle frame arrived after process exit".into());
    }
    runtime.lifecycle = payload.lifecycle;
    runtime.error = payload.error;
    let status = status_for(app, record, &key, Some(runtime), None);
    emit_status(app, &status);
    Ok(status)
}
