use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    templates::{
        load_channel_templates, save_channel_templates, validate_channel_template_deletion,
        ChannelTemplateRecord, CreateChannelTemplateRequest, TemplateType, TemplateWorktreeConfig,
        UpdateChannelTemplateRequest,
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

fn normalize_project_folders(
    project_folders: Vec<String>,
    project_folder: Option<String>,
) -> Vec<String> {
    let mut normalized = Vec::new();
    for candidate in project_folders.into_iter().chain(project_folder) {
        let trimmed = candidate.trim();
        if !trimmed.is_empty() && !normalized.iter().any(|folder| folder == trimmed) {
            normalized.push(trimmed.to_string());
        }
    }
    normalized
}

fn normalize_worktree(
    worktree: Option<TemplateWorktreeConfig>,
) -> Result<Option<TemplateWorktreeConfig>, String> {
    worktree
        .map(|worktree| {
            Ok(TemplateWorktreeConfig {
                location: trim_required(&worktree.location, "Worktree location")?,
                base_branch: trim_required(&worktree.base_branch, "Base branch")?,
            })
        })
        .transpose()
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

#[tauri::command]
pub async fn pick_channel_template_project_folder(app: AppHandle) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folders(move |paths| {
        let _ = tx.send(paths);
    });

    let Some(folder_paths) = rx.await.map_err(|_| "dialog cancelled".to_string())? else {
        return Ok(Vec::new());
    };
    folder_paths
        .into_iter()
        .map(|folder_path| {
            folder_path
                .as_path()
                .map(|path| path.to_string_lossy().into_owned())
                .ok_or_else(|| "Folder picker returned an invalid path".to_string())
        })
        .collect()
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
        let template_type = input.template_type.unwrap_or(TemplateType::Channel);
        let description = trim_optional(input.description);
        let canvas_template = trim_optional(input.canvas_template);
        let project_folders =
            normalize_project_folders(input.project_folders, input.project_folder);
        let project_folder = project_folders.first().cloned();
        let worktree = normalize_worktree(input.worktree)?;
        let channel_type = input.channel_type.unwrap_or_else(|| "stream".to_string());
        let visibility = input.visibility.unwrap_or_else(|| "open".to_string());
        validate_channel_type(&channel_type)?;
        validate_visibility(&visibility)?;
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
            template_type,
            description,
            channel_type,
            visibility,
            canvas_template,
            project_folders,
            project_folder,
            worktree,
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
        let requested_template_type = input.template_type;
        let description = trim_optional(input.description);
        let canvas_template = trim_optional(input.canvas_template);
        let project_folders =
            normalize_project_folders(input.project_folders, input.project_folder);
        let project_folder = project_folders.first().cloned();
        let worktree = normalize_worktree(input.worktree)?;
        let channel_type = input.channel_type.unwrap_or_else(|| "stream".to_string());
        let visibility = input.visibility.unwrap_or_else(|| "open".to_string());
        validate_channel_type(&channel_type)?;
        validate_visibility(&visibility)?;

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
        let template_type = requested_template_type.unwrap_or(template.template_type);

        template.name = name;
        template.template_type = template_type;
        template.description = description;
        template.channel_type = channel_type;
        template.visibility = visibility;
        template.canvas_template = canvas_template;
        template.project_folders = project_folders;
        template.project_folder = project_folder;
        template.worktree = worktree;
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
