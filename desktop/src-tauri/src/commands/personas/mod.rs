use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        load_managed_agents, load_personas, load_teams, save_personas, try_regenerate_nest,
        validate_persona_activation_change, AgentDefinition,
    },
    util::now_iso,
};

fn trim_required(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    Ok(trimmed.to_string())
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value.and_then(|candidate| {
        let trimmed = candidate.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

mod pending;
pub(in crate::commands) use pending::retain_persona_pending;
pub(super) use pending::tombstone_persona_pending;
mod create;
pub use create::create_persona;
mod sharing;
pub use sharing::set_persona_shared;
pub use sharing::update_persona_and_publish;
mod update;
pub use update::update_persona;
mod inbound;
pub use inbound::reconcile_inbound_persona_event;

#[tauri::command]
pub async fn list_personas(app: AppHandle) -> Result<Vec<AgentDefinition>, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut personas = load_personas(&app)?;
        pending::project_active_persona_sharing(&app, &state, &mut personas);
        Ok(personas)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

mod delete;
pub use delete::delete_persona;

#[tauri::command]
pub async fn set_persona_active(
    id: String,
    active: bool,
    app: AppHandle,
) -> Result<AgentDefinition, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut personas = load_personas(&app)?;
        let persona = personas
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| format!("agent {id} not found"))?;

        let referenced_by_managed_agent = !active
            && load_managed_agents(&app)?
                .iter()
                .any(|agent| agent.persona_id.as_deref() == Some(id.as_str()));
        let referenced_by_team = !active
            && load_teams(&app)?.iter().any(|team| {
                team.persona_ids
                    .iter()
                    .any(|persona_id| persona_id == id.as_str())
            });

        validate_persona_activation_change(
            persona,
            active,
            referenced_by_managed_agent,
            referenced_by_team,
        )?;

        if persona.is_active == active {
            return Ok(persona.clone());
        }

        persona.is_active = active;
        persona.updated_at = now_iso();

        let updated = persona.clone();
        save_personas(&app, &personas)?;
        try_regenerate_nest(&app);
        Ok(updated)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

pub(crate) const PNG_MAGIC: [u8; 4] = [0x89, 0x50, 0x4E, 0x47];
mod snapshot;
pub use snapshot::encode_agent_snapshot_for_send;
pub use snapshot::export_agent_snapshot;
pub(crate) use snapshot::import::{
    decode_snapshot_from_bytes, resolve_snapshot_import_behavior, MAX_SNAPSHOT_JSON_BYTES,
    MAX_SNAPSHOT_PNG_BYTES,
};
pub use snapshot::{confirm_agent_snapshot_import, preview_agent_snapshot_import};
