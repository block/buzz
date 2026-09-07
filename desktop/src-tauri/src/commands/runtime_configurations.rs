//! Local configuration management; no secret values or credentials cross IPC.
use super::{
    desktop_profiles::scope,
    desktop_stop::{local_id, owned_local},
};
use crate::{
    app_state::AppState,
    managed_agents::{
        self,
        runtime_configurations::{self, RuntimeConfigurations},
    },
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfigurationView {
    configurations: RuntimeConfigurations,
    updated_at: String,
    host: String,
    catalog: Vec<runtime_configurations::RuntimeConfigurationSummary>,
    running: Option<runtime_configurations::RuntimeConfigurationRef>,
}

fn view(
    record: &managed_agents::ManagedAgentRecord,
    host: String,
    community: &str,
    owner: &str,
    app: &AppHandle,
) -> Result<RuntimeConfigurationView, String> {
    let personas = managed_agents::load_personas(app)?;
    let global = managed_agents::load_global_agent_config(app)?;
    let catalog =
        runtime_configurations::catalog(record, &personas, &global, &host, owner, community);
    let key = managed_agents::ManagedAgentRuntimeKey::new(&record.pubkey, community)?;
    let state = app.state::<AppState>();
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let running = match runtimes.get_mut(&key) {
        Some(runtime) => {
            if runtime
                .child
                .try_wait()
                .map_err(|_| "Running process state is unavailable")?
                .is_none()
            {
                runtime.spawn_config.runtime_configuration.clone()
            } else {
                None
            }
        }
        None => None,
    };
    Ok(RuntimeConfigurationView {
        configurations: record.runtime_configurations.get(owner, community),
        updated_at: record.updated_at.clone(),
        host,
        catalog,
        running,
    })
}

#[tauri::command]
pub async fn get_runtime_configurations(
    app: AppHandle,
    owner: String,
    community: String,
    agent: String,
) -> Result<RuntimeConfigurationView, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let scope = scope(&app, &state, &owner, &community)?;
        if !owned_local(&app, &state, &owner, &agent)? {
            return Err("Agent ownership is unavailable on this Desktop".into());
        }
        let host = local_id(
            &mut managed_agents::retention::open_retention_db(&scope.db_path)?,
            &scope,
        )?;
        let _store = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = managed_agents::load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|r| r.pubkey == agent)
            .ok_or("Agent is unavailable")?;
        view(record, host, &community, &owner, &app)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn save_runtime_configurations(
    app: AppHandle,
    owner: String,
    community: String,
    agent: String,
    expected_updated_at: String,
    configurations: RuntimeConfigurations,
) -> Result<RuntimeConfigurationView, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _transition = state
            .managed_agent_runtime_transition
            .lock()
            .map_err(|e| e.to_string())?;
        let scope = scope(&app, &state, &owner, &community)?;
        if !owned_local(&app, &state, &owner, &agent)? {
            return Err("Agent ownership is unavailable on this Desktop".into());
        }
        let host = local_id(
            &mut managed_agents::retention::open_retention_db(&scope.db_path)?,
            &scope,
        )?;
        configurations.validate()?;
        let _store = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = managed_agents::load_managed_agents(&app)?;
        let record = managed_agents::find_managed_agent_mut(&mut records, &agent)?;
        if record.updated_at != expected_updated_at {
            return Err("Agent changed; reload configurations before saving".into());
        }
        record
            .runtime_configurations
            .replace(&owner, &community, &host, configurations)?;
        record.updated_at = crate::util::now_iso();
        let result = view(record, host, &community, &owner, &app)?;
        managed_agents::save_managed_agents(&app, &records)?;
        let _ = app.emit("agents-data-changed", ());
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
