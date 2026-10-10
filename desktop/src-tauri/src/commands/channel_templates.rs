use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    templates::{
        load_channel_templates, save_channel_templates, validate_channel_template_deletion,
        ChannelTemplateRecord, CreateChannelTemplateRequest, UpdateChannelTemplateRequest,
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

fn validate_channel_type(value: &str) -> Result<(), String> {
    match value {
        "stream" | "forum" => Ok(()),
        _ => Err(format!(
            "invalid channel type: {value:?} (expected \"stream\" or \"forum\")"
        )),
    }
}

fn validate_visibility(value: &str) -> Result<(), String> {
    match value {
        "open" | "private" => Ok(()),
        _ => Err(format!(
            "invalid visibility: {value:?} (expected \"open\" or \"private\")"
        )),
    }
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Reject template rosters whose direct-pubkey members are malformed, so an
/// invalid pubkey fails at create/update time rather than being silently
/// skipped when the template is applied to a channel.
fn validate_template_members(roster: &crate::templates::TemplateAgentRoster) -> Result<(), String> {
    for member in &roster.members {
        if !is_hex64(member.pubkey.trim()) {
            let label = member.label.as_deref().unwrap_or("?");
            return Err(format!(
                "template member {label:?} has an invalid pubkey (must be a 64-character hex string)"
            ));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn list_channel_templates(app: AppHandle) -> Result<Vec<ChannelTemplateRecord>, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .channel_templates_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        load_channel_templates(&app)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn create_channel_template(
    input: CreateChannelTemplateRequest,
    app: AppHandle,
) -> Result<ChannelTemplateRecord, String> {
    tokio::task::spawn_blocking(move || {
        let name = trim_required(&input.name, "Template name")?;
        let description = trim_optional(input.description);
        let canvas_template = trim_optional(input.canvas_template);
        let channel_type = input.channel_type.unwrap_or_else(|| "stream".to_string());
        let visibility = input.visibility.unwrap_or_else(|| "open".to_string());
        validate_channel_type(&channel_type)?;
        validate_visibility(&visibility)?;
        validate_template_members(&input.agents)?;
        let now = now_iso();

        let state = app.state::<AppState>();
        let _store_guard = state
            .channel_templates_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut templates = load_channel_templates(&app)?;

        let template = ChannelTemplateRecord {
            id: Uuid::new_v4().to_string(),
            name,
            description,
            channel_type,
            visibility,
            canvas_template,
            agents: input.agents,
            is_builtin: false,
            created_at: now.clone(),
            updated_at: now,
        };

        templates.push(template.clone());
        save_channel_templates(&app, &templates)?;
        Ok(template)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn update_channel_template(
    input: UpdateChannelTemplateRequest,
    app: AppHandle,
) -> Result<ChannelTemplateRecord, String> {
    tokio::task::spawn_blocking(move || {
        let name = trim_required(&input.name, "Template name")?;
        let description = trim_optional(input.description);
        let canvas_template = trim_optional(input.canvas_template);
        let channel_type = input.channel_type.unwrap_or_else(|| "stream".to_string());
        let visibility = input.visibility.unwrap_or_else(|| "open".to_string());
        validate_channel_type(&channel_type)?;
        validate_visibility(&visibility)?;
        validate_template_members(&input.agents)?;

        let state = app.state::<AppState>();
        let _store_guard = state
            .channel_templates_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut templates = load_channel_templates(&app)?;
        let template = templates
            .iter_mut()
            .find(|record| record.id == input.id)
            .ok_or_else(|| format!("template {} not found", input.id))?;

        template.name = name;
        template.description = description;
        template.channel_type = channel_type;
        template.visibility = visibility;
        template.canvas_template = canvas_template;
        template.agents = input.agents;
        template.updated_at = now_iso();

        let updated = template.clone();
        save_channel_templates(&app, &templates)?;
        Ok(updated)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn delete_channel_template(id: String, app: AppHandle) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .channel_templates_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut templates = load_channel_templates(&app)?;
        let template = templates
            .iter()
            .find(|record| record.id == id)
            .ok_or_else(|| format!("template {id} not found"))?;
        validate_channel_template_deletion(template)?;
        templates.retain(|record| record.id != id);
        save_channel_templates(&app, &templates)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn duplicate_channel_template(
    id: String,
    app: AppHandle,
) -> Result<ChannelTemplateRecord, String> {
    tokio::task::spawn_blocking(move || {
        let now = now_iso();

        let state = app.state::<AppState>();
        let _store_guard = state
            .channel_templates_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut templates = load_channel_templates(&app)?;
        let source = templates
            .iter()
            .find(|record| record.id == id)
            .ok_or_else(|| format!("template {id} not found"))?
            .clone();

        let duplicate = ChannelTemplateRecord {
            id: Uuid::new_v4().to_string(),
            name: format!("{} (Copy)", source.name),
            is_builtin: false,
            created_at: now.clone(),
            updated_at: now,
            ..source
        };

        templates.push(duplicate.clone());
        save_channel_templates(&app, &templates)?;
        Ok(duplicate)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
