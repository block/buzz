//! Runtime-pair start: spawn a child for a bound (agent, relay) pair and
//! record it as the live pair.
//!
//! Split out of `runtime.rs` (which is at the repository file-size ceiling)
//! alongside the other `runtime/` submodules. `spawn_agent_child` — the env
//! assembly this calls — stays there.

use std::collections::HashMap;

use tauri::AppHandle;

use super::{
    bound_runtime_key, current_instance_id, spawn_agent_child, terminate_process, SpawnPolicy,
};
use crate::{
    managed_agents::{ManagedAgentPairRuntime, ManagedAgentRecord, ManagedAgentRuntimeKey},
    util::now_iso,
};

/// Spawn (or adopt) the runtime pair for `record` on the caller's bound
/// workspace relay. `workspace_relay` can only be produced by
/// `bind_expected_relay_scope`, so this spawn consumes — by construction — the
/// exact workspace-relay read the caller's scope assertion passed on; it never
/// re-reads the mutable override (see `relay::scope`). The key comes from
/// [`bound_runtime_key`] — the seam the spawn-key regressions exercise.
///
/// `policy` bounds the spawned child's pool and lifetime; see [`SpawnPolicy`].
pub fn start_managed_agent_process(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    owner_hex: Option<&str>,
    workspace_relay: &crate::relay::ScopedWorkspaceRelay,
    policy: SpawnPolicy,
) -> Result<(), String> {
    let key = bound_runtime_key(record, workspace_relay)?;
    if let Some(runtime) = runtimes.get_mut(&key) {
        if runtime
            .child
            .try_wait()
            .map_err(|error| format!("failed to inspect running process: {error}"))?
            .is_none()
        {
            // A live pair is left exactly as it is, whatever `policy` asked
            // for: a durable start landing on a speculative pair spawns
            // nothing, and the running harness's own bound is what promotes it
            // — the send's message dispatch, not this call.
            return Ok(());
        }

        runtimes.remove(&key);
        crate::managed_agents::remove_agent_runtime_receipt(app, &key);
    }

    // Scalar PIDs are migration-only and never establish pair liveness.
    record.runtime_pid = None;

    let mut process = spawn_agent_child(app, record, &key.relay_url, policy, owner_hex)?;
    let now = now_iso();
    let receipt = crate::managed_agents::ManagedAgentRuntimeReceipt {
        key: key.clone(),
        pid: process.child.id(),
        desktop_instance_id: current_instance_id(app),
        started_at: now.clone(),
    };
    if let Err(error) = crate::managed_agents::write_agent_runtime_receipt(app, &receipt) {
        let _ = terminate_process(process.child.id());
        let _ = process.child.wait();
        return Err(error);
    }

    record.updated_at = now.clone();
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_exit_code = None;
    record.last_error = None;
    record.last_error_code = None;

    runtimes.insert(key, ManagedAgentPairRuntime::starting(process));
    Ok(())
}
