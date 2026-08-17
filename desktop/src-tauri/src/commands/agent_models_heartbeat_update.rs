//! Command-layer lifecycle wiring for heartbeat-preflight edits.

use std::{collections::HashMap, sync::MutexGuard};

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        apply_heartbeat_preflight_update, stop_managed_agent_process,
        HeartbeatPreflightDesignation, ManagedAgentPairRuntime, ManagedAgentRecord,
        ManagedAgentRuntimeKey,
    },
};

/// Serialize the record edit with every runtime start/stop transition.
///
/// The caller keeps this guard until its store/runtime locks are released, then
/// drops it before awaiting relay I/O. This preserves the repository lock order
/// of transition -> store -> runtimes.
pub(super) fn lock_update_transition(state: &AppState) -> Result<MutexGuard<'_, ()>, String> {
    crate::managed_agents::runtime_transition::lock(state)
}

/// Tracks whether a validated preflight edit invalidated the running process.
pub(super) struct HeartbeatUpdate {
    stop_obsolete_process: bool,
}

impl HeartbeatUpdate {
    /// Apply the ACP-command and designation patch through the consolidated
    /// managed-agent validation boundary.
    pub(super) fn apply(
        record: &mut ManagedAgentRecord,
        acp_command: Option<String>,
        designation: Option<Option<HeartbeatPreflightDesignation>>,
    ) -> Result<Self, String> {
        let stop_obsolete_process =
            apply_heartbeat_preflight_update(record, acp_command, designation)?;
        Ok(Self {
            stop_obsolete_process,
        })
    }

    /// Stop every process using the previous gate before the caller persists
    /// the edited record, and invalidate its cached sessions.
    pub(super) fn stop_obsolete_process(
        self,
        app: &AppHandle,
        state: &AppState,
        record: &mut ManagedAgentRecord,
        runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    ) -> Result<(), String> {
        if !self.stop_obsolete_process {
            return Ok(());
        }
        stop_managed_agent_process(app, record, runtimes)?;
        state.clear_agent_session_caches(&record.pubkey);
        Ok(())
    }
}
