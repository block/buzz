use tauri::AppHandle;

use super::super::{
    agent_readiness, load_global_agent_config, load_personas, managed_agent_runtime_log_path,
    record_agent_command, resolve_effective_agent_env, AgentReadiness, ManagedAgentPairRuntime,
    ManagedAgentRuntimeKey, ManagedAgentRuntimeLifecycle, ManagedAgentRuntimeStatus,
};

pub(super) fn migration_status(
    app: &AppHandle,
    record: &super::super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    lifecycle: ManagedAgentRuntimeLifecycle,
    error: &str,
) -> ManagedAgentRuntimeStatus {
    let mut status = status_for(app, record, key, None, None);
    status.lifecycle = lifecycle;
    status.error = Some(error.to_string());
    status
}

pub(super) fn status_for(
    app: &AppHandle,
    record: &super::super::ManagedAgentRecord,
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
pub(super) struct StatusInputs<'a> {
    pub(super) personas: &'a [super::super::AgentDefinition],
    pub(super) global: &'a super::super::GlobalAgentConfig,
}

pub(super) fn status_for_with(
    app: &AppHandle,
    record: &super::super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    runtime: Option<&ManagedAgentPairRuntime>,
    requested_relay_url: Option<String>,
    inputs: StatusInputs<'_>,
) -> ManagedAgentRuntimeStatus {
    let StatusInputs { personas, global } = inputs;
    let command = record_agent_command(record, personas);
    let metadata = super::super::known_acp_runtime(&command);
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
        pid: runtime.map(ManagedAgentPairRuntime::pid),
        error: runtime.and_then(|runtime| runtime.error.clone()),
        log_path: runtime
            .and_then(|runtime| runtime.log_path().map(|path| path.display().to_string()))
            .or_else(|| {
                managed_agent_runtime_log_path(app, key)
                    .ok()
                    .map(|path| path.display().to_string())
            }),
        active_assignment: runtime.and_then(|runtime| runtime.active_assignment.clone()),
        active_job: runtime.and_then(|runtime| runtime.active_job.clone()),
    }
}
