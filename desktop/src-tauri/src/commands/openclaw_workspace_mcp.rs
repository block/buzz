use tauri::AppHandle;

use crate::managed_agents::openclaw_workspace_mcp::{
    apply_grant, disconnect, status, OpenClawWorkspaceGrant, OpenClawWorkspaceStatus,
};

#[tauri::command]
pub fn get_openclaw_workspace_mcp_status() -> Result<OpenClawWorkspaceStatus, String> {
    status()
}

#[tauri::command]
pub fn apply_openclaw_workspace_mcp_grant(
    url: String,
    authorization: String,
    expires_at: String,
    relay: Option<String>,
    connected_via_relay: Option<bool>,
    app: AppHandle,
) -> Result<OpenClawWorkspaceStatus, String> {
    let url = url.trim().to_string();
    let authorization = authorization.trim().to_string();
    let expires_at = expires_at.trim().to_string();
    if url.is_empty() || authorization.is_empty() || expires_at.is_empty() {
        return Err("url, authorization, and expiresAt are required".into());
    }
    if !authorization.to_ascii_lowercase().starts_with("bearer ") {
        return Err("authorization must be a Bearer token".into());
    }
    apply_grant(
        &app,
        OpenClawWorkspaceGrant {
            url,
            authorization,
            expires_at,
            relay,
            connected_via_relay: connected_via_relay.unwrap_or(true),
        },
    )
}

#[tauri::command]
pub fn disconnect_openclaw_workspace_mcp(app: AppHandle) -> Result<OpenClawWorkspaceStatus, String> {
    disconnect(&app)
}
