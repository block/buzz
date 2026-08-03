use tauri::AppHandle;

use crate::managed_agents::project_connections::{
    self, CreateProjectConnectionRequest, ProjectConnection, ProjectConnectionScope,
    UpdateProjectConnectionRequest,
};

#[tauri::command]
pub fn list_project_connections(
    app: AppHandle,
    project_scope: ProjectConnectionScope,
) -> Result<Vec<ProjectConnection>, String> {
    project_connections::list_project_connections(&app, &project_scope)
}

#[tauri::command]
pub fn create_project_connection(
    app: AppHandle,
    input: CreateProjectConnectionRequest,
) -> Result<ProjectConnection, String> {
    project_connections::create_project_connection(&app, input)
}

#[tauri::command]
pub fn update_project_connection(
    app: AppHandle,
    input: UpdateProjectConnectionRequest,
) -> Result<ProjectConnection, String> {
    project_connections::update_project_connection(&app, input)
}

#[tauri::command]
pub async fn test_project_connection(
    app: AppHandle,
    project_scope: ProjectConnectionScope,
    connection_id: String,
) -> Result<ProjectConnection, String> {
    tauri::async_runtime::spawn_blocking(move || {
        project_connections::test_project_connection(&app, &project_scope, &connection_id)
    })
    .await
    .map_err(|error| format!("Project connection test task failed: {error}"))?
}

#[tauri::command]
pub fn delete_project_connection(
    app: AppHandle,
    project_scope: ProjectConnectionScope,
    connection_id: String,
) -> Result<(), String> {
    project_connections::delete_project_connection(&app, &project_scope, &connection_id)
}
