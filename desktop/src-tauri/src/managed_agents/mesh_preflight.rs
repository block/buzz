use tauri::AppHandle;

use super::{load_global_agent_config, AgentDefinition, ManagedAgentRecord};

pub(crate) fn verify_relay_mesh_preflight(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    expected_model_id: &Option<String>,
    action: &str,
) -> Result<(), String> {
    let global = load_global_agent_config(app).unwrap_or_default();
    let current_model_id =
        super::effective_config::resolve_effective_relay_mesh_model_id(record, personas, &global);
    if &current_model_id == expected_model_id {
        return Ok(());
    }
    Err(format!(
        "managed-agent mesh configuration changed while {action} preflight was in flight; retry {action}"
    ))
}
