use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        config_bridge::{
            reader::read_config_surface,
            types::{
                AcpConfigOptionEntry, AcpConfigOptionValue, AcpModelEntry, ConfigWriteMechanism,
                RuntimeConfigSurface, SessionConfigCache, WriteConfigFieldRequest,
                WriteConfigResult, WriteConfigTarget,
            },
            writer::plan_config_write,
        },
        known_acp_runtime, load_managed_agents, save_managed_agents, sync_managed_agent_processes,
    },
};

/// Get the full config surface for a managed agent.
///
/// Returns normalized + advanced config from all available tiers.
/// Pre-spawn agents show config file values with ACP tiers marked as pending.
#[tauri::command]
pub async fn get_agent_config_surface(
    pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuntimeConfigSurface, String> {
    let record = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        if sync_managed_agent_processes(&mut records, &mut runtimes) {
            save_managed_agents(&app, &records)?;
        }
        records
            .into_iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?
    };

    let runtime_meta = known_acp_runtime(&record.agent_command);
    let session_cache = state.get_session_cache(&pubkey);

    Ok(read_config_surface(
        &record,
        runtime_meta,
        session_cache.as_ref(),
    ))
}

/// Write a config field value for a managed agent.
///
/// Plans the write mechanism based on the current config surface, then
/// executes: either updating the record (for env var respawn) or returning
/// the mechanism for the frontend to send via observer control (for ACP writes).
#[tauri::command]
pub async fn write_agent_config_field(
    request: WriteConfigFieldRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WriteConfigResult, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;

    let record = records
        .iter()
        .find(|r| r.pubkey == request.pubkey)
        .cloned()
        .ok_or_else(|| format!("agent {} not found", request.pubkey))?;

    let runtime_meta = known_acp_runtime(&record.agent_command);
    let session_cache = state.get_session_cache(&request.pubkey);
    let surface = read_config_surface(&record, runtime_meta, session_cache.as_ref());

    let mut result = plan_config_write(&surface, &request.field);

    if !result.success {
        return Ok(result);
    }

    if let ConfigWriteMechanism::RespawnWithEnvVar { ref env_key } = result.mechanism_used {
        let record = records
            .iter_mut()
            .find(|r| r.pubkey == request.pubkey)
            .ok_or_else(|| format!("agent {} not found", request.pubkey))?;

        match request.value {
            Some(ref val) if !val.is_empty() => {
                record.env_vars.insert(env_key.clone(), val.clone());
            }
            _ => {
                record.env_vars.remove(env_key);
            }
        }

        if matches!(request.field, WriteConfigTarget::Model) {
            record.model = request.value.clone();
        }

        record.updated_at = crate::util::now_iso();
        save_managed_agents(&app, &records)?;
        result.requires_restart = true;
    }

    Ok(result)
}

/// Store a `session_config_captured` observer event payload into the session cache.
///
/// Called by the TypeScript observer relay when it decrypts a `session_config_captured`
/// event from a running agent. The payload contains raw ACP session/new fields.
#[tauri::command]
pub fn put_agent_session_config(
    pubkey: String,
    payload: serde_json::Value,
    app: AppHandle,
    state: State<'_, AppState>,
) {
    {
        let _guard = match state.managed_agents_store_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match load_managed_agents(&app) {
            Ok(records) if records.iter().any(|r| r.pubkey == pubkey) => {}
            _ => return,
        }
    }

    let config_options = parse_config_options(payload.get("configOptions"));
    let available_modes = parse_modes(&config_options, payload.get("modes"));
    let (available_models, current_model) = parse_models(payload.get("models"));

    let cache = SessionConfigCache {
        config_options,
        available_modes,
        available_models,
        current_model,
        goose_native_config: None,
        captured_at: crate::util::now_iso(),
    };

    state.put_session_cache(&pubkey, cache);
}

fn parse_config_options(raw: Option<&serde_json::Value>) -> Vec<AcpConfigOptionEntry> {
    let arr = match raw.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|opt| {
            let config_id = opt
                .get("id")
                .or_else(|| opt.get("configId"))?
                .as_str()?
                .to_string();
            Some(AcpConfigOptionEntry {
                config_id,
                category: opt
                    .get("category")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                display_name: opt
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                current_value: opt
                    .get("value")
                    .or_else(|| opt.get("currentValue"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                options: parse_option_values(opt.get("options")),
            })
        })
        .collect()
}

fn parse_option_values(raw: Option<&serde_json::Value>) -> Vec<AcpConfigOptionValue> {
    let arr = match raw.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|o| {
            let value = o.get("value").and_then(|v| v.as_str())?.to_string();
            Some(AcpConfigOptionValue {
                value,
                display_name: o
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect()
}

fn parse_modes(
    config_options: &[AcpConfigOptionEntry],
    raw: Option<&serde_json::Value>,
) -> Vec<String> {
    if let Some(arr) = raw.and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|m| m.as_str().map(str::to_string))
            .collect();
    }
    // Fall back: extract mode options from configOptions with category "mode".
    config_options
        .iter()
        .filter(|o| o.category.as_deref() == Some("mode"))
        .flat_map(|o| o.options.iter().map(|v| v.value.clone()))
        .collect()
}

fn parse_models(raw: Option<&serde_json::Value>) -> (Vec<AcpModelEntry>, Option<String>) {
    let raw = match raw {
        Some(v) => v,
        None => return (Vec::new(), None),
    };

    // Object shape: { currentModelId, availableModels: [...] }
    if let Some(obj) = raw.as_object() {
        let current_model = obj
            .get("currentModelId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let models = obj
            .get("availableModels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let model_id = m
                            .get("modelId")
                            .or_else(|| m.get("id"))
                            .and_then(|v| v.as_str())?
                            .to_string();
                        Some(AcpModelEntry {
                            model_id,
                            name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                            description: m
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        return (models, current_model);
    }

    // Array shape: [{ modelId, isCurrent, ... }]
    let arr = match raw.as_array() {
        Some(a) => a,
        None => return (Vec::new(), None),
    };
    let mut current_model = None;
    let models = arr
        .iter()
        .filter_map(|m| {
            let model_id = m
                .get("modelId")
                .or_else(|| m.get("id"))
                .and_then(|v| v.as_str())?
                .to_string();
            if m.get("isCurrent")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                current_model = Some(model_id.clone());
            }
            Some(AcpModelEntry {
                model_id,
                name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                description: m
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect();
    (models, current_model)
}
