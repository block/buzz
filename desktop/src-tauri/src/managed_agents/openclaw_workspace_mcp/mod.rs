//! Hula Buzz: persist short-lived OpenClaw workspace MCP grants and upsert
//! Claude `mcpServers.openclaw-workspace` for UI-managed Claude agents.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::app_state::keyring_service;
use crate::managed_agents::config_bridge::claude::{
    remove_mcp_server, upsert_http_mcp_server, OPENCLAW_WORKSPACE_MCP_NAME,
};
use crate::managed_agents::{load_managed_agents, BackendKind, ManagedAgentRecord};
use crate::secret_store::SecretStore;

const GRANT_KEY: &str = "openclaw-workspace-mcp-grant";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawWorkspaceGrant {
    pub url: String,
    pub authorization: String,
    pub expires_at: String,
    /// Extra HTTP headers for Claude mcpServers (CF Access, etc.). Authorization
    /// is always taken from `authorization` at apply time.
    #[serde(default)]
    pub headers: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub relay: Option<String>,
    #[serde(default)]
    pub connected_via_relay: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawWorkspaceStatus {
    pub connected: bool,
    pub expires_at: Option<String>,
    pub url: Option<String>,
    pub connected_via_relay: bool,
    /// Never includes the raw JWT.
    pub agents_updated: usize,
}

fn store() -> Result<&'static SecretStore, String> {
    Ok(SecretStore::shared(keyring_service()))
}

pub fn load_grant() -> Result<Option<OpenClawWorkspaceGrant>, String> {
    let raw = store()?.load(GRANT_KEY)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let grant: OpenClawWorkspaceGrant =
        serde_json::from_str(&raw).map_err(|e| format!("corrupt openclaw workspace grant: {e}"))?;
    Ok(Some(grant))
}

pub fn save_grant(grant: &OpenClawWorkspaceGrant) -> Result<(), String> {
    let raw = serde_json::to_string(grant).map_err(|e| e.to_string())?;
    store()?.store(GRANT_KEY, &raw)
}

pub fn clear_grant() -> Result<(), String> {
    store()?.delete(GRANT_KEY)?;
    Ok(())
}

fn is_claude_agent(record: &ManagedAgentRecord) -> bool {
    if record.backend != BackendKind::Local {
        return false;
    }
    let runtime = record.runtime.as_deref().unwrap_or("").to_ascii_lowercase();
    if runtime == "claude" || runtime.contains("claude") {
        return true;
    }
    let cmd = record
        .agent_command_override
        .as_deref()
        .unwrap_or(record.agent_command.as_str())
        .to_ascii_lowercase();
    cmd.contains("claude")
}

fn claude_config_dir_for(record: &ManagedAgentRecord) -> Option<PathBuf> {
    record
        .env_vars
        .get("CLAUDE_CONFIG_DIR")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Apply grant to secret store + Claude mcpServers for managed Claude agents
/// and the default home `.claude.json`.
pub fn apply_grant(
    app: &tauri::AppHandle,
    grant: OpenClawWorkspaceGrant,
) -> Result<OpenClawWorkspaceStatus, String> {
    save_grant(&grant)?;
    let mut agents_updated = 0usize;

    // Always upsert the user-default Claude MCP config.
    upsert_http_mcp_server(
        None,
        OPENCLAW_WORKSPACE_MCP_NAME,
        &grant.url,
        &grant.authorization,
        grant.headers.as_ref(),
    )?;

    if let Ok(records) = load_managed_agents(app) {
        for record in records {
            if !is_claude_agent(&record) {
                continue;
            }
            let dir = claude_config_dir_for(&record);
            upsert_http_mcp_server(
                dir.as_deref(),
                OPENCLAW_WORKSPACE_MCP_NAME,
                &grant.url,
                &grant.authorization,
                grant.headers.as_ref(),
            )?;
            agents_updated += 1;
        }
    }

    Ok(OpenClawWorkspaceStatus {
        connected: true,
        expires_at: Some(grant.expires_at),
        url: Some(grant.url),
        connected_via_relay: grant.connected_via_relay,
        agents_updated,
    })
}

pub fn disconnect(app: &tauri::AppHandle) -> Result<OpenClawWorkspaceStatus, String> {
    let _ = clear_grant();
    let _ = remove_mcp_server(None, OPENCLAW_WORKSPACE_MCP_NAME);
    if let Ok(records) = load_managed_agents(app) {
        for record in records {
            if !is_claude_agent(&record) {
                continue;
            }
            let dir = claude_config_dir_for(&record);
            let _ = remove_mcp_server(dir.as_deref(), OPENCLAW_WORKSPACE_MCP_NAME);
        }
    }
    Ok(OpenClawWorkspaceStatus {
        connected: false,
        expires_at: None,
        url: None,
        connected_via_relay: false,
        agents_updated: 0,
    })
}

pub fn status() -> Result<OpenClawWorkspaceStatus, String> {
    match load_grant()? {
        Some(grant) => Ok(OpenClawWorkspaceStatus {
            connected: true,
            expires_at: Some(grant.expires_at),
            url: Some(grant.url),
            connected_via_relay: grant.connected_via_relay,
            agents_updated: 0,
        }),
        None => Ok(OpenClawWorkspaceStatus {
            connected: false,
            expires_at: None,
            url: None,
            connected_via_relay: false,
            agents_updated: 0,
        }),
    }
}
