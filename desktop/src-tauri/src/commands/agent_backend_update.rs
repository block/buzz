use std::collections::HashMap;

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        build_managed_agent_summary, load_managed_agents, load_personas,
        stop_managed_agent_process, BackendKind, ManagedAgentPairRuntime, ManagedAgentRecord,
        ManagedAgentRuntimeKey, ManagedAgentSummary,
    },
};

pub(super) fn requested_backend_update(
    current: &BackendKind,
    backend_agent_id: Option<&str>,
    requested: Option<BackendKind>,
) -> Result<Option<BackendKind>, String> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    if requested == BackendKind::Local
        && *current != BackendKind::Local
        && backend_agent_id.is_some()
    {
        return Err(
            "cannot move a deployed provider agent to local: the provider protocol does not support undeploy"
                .to_string(),
        );
    }
    Ok(Some(requested))
}

pub(super) fn apply_backend_update(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    requested: Option<BackendKind>,
) -> Result<Option<BackendKind>, String> {
    let Some(backend) = requested_backend_update(
        &record.backend,
        record.backend_agent_id.as_deref(),
        requested,
    )?
    else {
        return Ok(None);
    };
    if record.backend == BackendKind::Local && backend != BackendKind::Local {
        stop_managed_agent_process(app, record, runtimes)?;
    }
    record.provider_binary_path = match &backend {
        BackendKind::Provider { config, id } => {
            crate::managed_agents::validate_provider_config(config)?;
            Some(
                crate::managed_agents::resolve_provider_binary(id)?
                    .display()
                    .to_string(),
            )
        }
        BackendKind::Local => None,
    };
    if backend != record.backend {
        record.backend_agent_id = None;
    }
    record.start_on_app_launch = backend == BackendKind::Local;
    record.backend = backend;
    Ok(match &record.backend {
        BackendKind::Provider { .. } => Some(record.backend.clone()),
        BackendKind::Local => None,
    })
}

pub(super) async fn deploy_updated_backend(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    backend: Option<BackendKind>,
    fallback: ManagedAgentSummary,
) -> Result<ManagedAgentSummary, String> {
    let Some(BackendKind::Provider { id, config }) = backend else {
        return Ok(fallback);
    };
    let agent_json = {
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(app)?;
        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        super::agents::build_deploy_payload(app, state, record)?
    };
    super::agents::deploy_to_provider(app, state, pubkey, &id, &config, agent_json, None).await?;
    let _guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let records = load_managed_agents(app)?;
    let runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let record = records
        .iter()
        .find(|r| r.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    build_managed_agent_summary(
        app,
        record,
        &runtimes,
        &load_personas(app).unwrap_or_default(),
    )
}
