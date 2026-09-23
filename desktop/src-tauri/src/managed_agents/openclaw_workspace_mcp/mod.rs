//! Hula Buzz: persist short-lived OpenClaw workspace MCP grants and upsert
//! Claude `mcpServers.openclaw-workspace` for **opted-in** UI-managed Claude agents.
//!
//! Per-agent policy: `ManagedAgentRecord.use_openclaw_workspace` (default OFF).
//! When ON + grant present → attach MCP and inject standing skill-pack instructions.
//! When OFF → local Mac FS / local skills; no forced MCP attach for that agent.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::app_state::keyring_service;
use crate::app_state::AppState;
use crate::managed_agents::config_bridge::claude::{
    remove_mcp_server, upsert_http_mcp_server, OPENCLAW_WORKSPACE_MCP_NAME,
};
use crate::managed_agents::{
    current_instance_id, load_managed_agents, save_managed_agents, stop_managed_agent_process,
    sync_managed_agent_processes, BackendKind, ManagedAgentRecord,
};
use crate::secret_store::SecretStore;
use tauri::Manager;

const GRANT_KEY: &str = "openclaw-workspace-mcp-grant";

/// Standing instructions injected into `BUZZ_ACP_SYSTEM_PROMPT` when an agent
/// has OpenClaw workspace mode enabled and a grant is applied at spawn.
pub const OPENCLAW_WORKSPACE_STANDING_INSTRUCTIONS: &str = "\
## OpenClaw workspace (Hula)

Project files for this agent live on the **OpenClaw remote workspace** via the \
`openclaw-workspace` MCP server. Do **not** assume a local Mac checkout of Hula \
(or any laptop path) is the project filesystem. Use MCP filesystem tools on \
`openclaw-workspace` as the only project FS.

**Primary skills source:** before domain work, call `skills_list` then \
`skills_get` on `openclaw-workspace`. Do not treat local `~/.claude/skills` as \
the source of truth — the skill pack is under the OpenClaw workspace \
(`SKILL_ROOTS`).
";

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

pub fn is_claude_agent(record: &ManagedAgentRecord) -> bool {
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

/// Merge standing OpenClaw instructions into an effective system prompt when
/// the agent opted in. Returns the prompt unchanged when the flag is off.
pub fn maybe_inject_standing_instructions(
    record: &ManagedAgentRecord,
    effective_prompt: Option<String>,
) -> Option<String> {
    if !record.use_openclaw_workspace {
        return effective_prompt;
    }
    let block = OPENCLAW_WORKSPACE_STANDING_INSTRUCTIONS.trim();
    match effective_prompt {
        Some(existing) if existing.trim().is_empty() => Some(block.to_string()),
        Some(existing) => {
            if existing.contains("OpenClaw workspace (Hula)") {
                Some(existing)
            } else {
                Some(format!("{existing}\n\n{block}"))
            }
        }
        None => Some(block.to_string()),
    }
}

/// Ensure MCP is present for one opted-in Claude agent when a grant exists.
/// Returns Ok(true) if MCP was upserted, Ok(false) if skipped (not Claude / no dir needed path still upserts with None).
pub fn ensure_mcp_for_agent(
    record: &ManagedAgentRecord,
    grant: &OpenClawWorkspaceGrant,
) -> Result<(), String> {
    if !record.use_openclaw_workspace || !is_claude_agent(record) {
        return Ok(());
    }
    let dir = claude_config_dir_for(record);
    upsert_http_mcp_server(
        dir.as_deref(),
        OPENCLAW_WORKSPACE_MCP_NAME,
        &grant.url,
        &grant.authorization,
        grant.headers.as_ref(),
    )?;
    Ok(())
}

pub fn remove_mcp_for_agent(record: &ManagedAgentRecord) -> Result<(), String> {
    if !is_claude_agent(record) {
        return Ok(());
    }
    let dir = claude_config_dir_for(record);
    remove_mcp_server(dir.as_deref(), OPENCLAW_WORKSPACE_MCP_NAME)?;
    Ok(())
}

/// Clear error when an opted-in agent is started without a grant.
pub fn missing_grant_error() -> String {
    "OpenClaw workspace is enabled for this agent, but no workspace MCP grant is connected. \
Open Settings → Agents → OpenClaw workspace and join a Hula relay that provisions the grant, \
or turn off “Use OpenClaw workspace (MCP)” for a local-only agent."
        .into()
}

/// Apply grant to secret store + Claude mcpServers for **opted-in** managed
/// Claude agents and the default home `.claude.json` (Hula connection).
pub fn apply_grant(
    app: &tauri::AppHandle,
    grant: OpenClawWorkspaceGrant,
) -> Result<OpenClawWorkspaceStatus, String> {
    save_grant(&grant)?;
    let mut agents_updated = 0usize;

    // Keep user-default Claude MCP config in sync with the Hula grant (CLI /
    // non-managed Claude Code). Managed UI agents only get MCP when opted in.
    upsert_http_mcp_server(
        None,
        OPENCLAW_WORKSPACE_MCP_NAME,
        &grant.url,
        &grant.authorization,
        grant.headers.as_ref(),
    )?;

    if let Ok(records) = load_managed_agents(app) {
        for record in records {
            if !record.use_openclaw_workspace || !is_claude_agent(&record) {
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

/// Stop every running managed agent that has `use_openclaw_workspace` enabled.
/// Does **not** clear the per-agent flag — reconnect + start again is enough.
fn stop_running_openclaw_agents(app: &tauri::AppHandle) -> usize {
    let state = app.state::<AppState>();
    let Ok(_store_guard) = state.managed_agents_store_lock.lock() else {
        return 0;
    };
    let Ok(mut records) = load_managed_agents(app) else {
        return 0;
    };
    let Ok(mut runtimes) = state.managed_agent_processes.lock() else {
        return 0;
    };

    let (sync_changed, exited_pubkeys) =
        sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(app));
    for pubkey in &exited_pubkeys {
        state.clear_agent_session_caches(pubkey);
    }

    let targets: Vec<String> = records
        .iter()
        .filter(|record| {
            record.use_openclaw_workspace
                && record.backend == BackendKind::Local
                && (record.runtime_pid.is_some()
                    || runtimes
                        .keys()
                        .any(|key| key.pubkey.eq_ignore_ascii_case(&record.pubkey)))
        })
        .map(|record| record.pubkey.clone())
        .collect();

    let mut stopped = 0usize;
    for pubkey in targets {
        let Some(record) = records
            .iter_mut()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&pubkey))
        else {
            continue;
        };
        match stop_managed_agent_process(app, record, &mut runtimes) {
            Ok(()) => {
                stopped += 1;
                state.clear_agent_session_caches(&pubkey);
            }
            Err(error) => {
                eprintln!(
                    "buzz-desktop: openclaw disconnect: failed to stop agent {pubkey}: {error}"
                );
            }
        }
    }

    if sync_changed || stopped > 0 {
        let _ = save_managed_agents(app, &records);
    }
    stopped
}

fn grant_is_expired(expires_at: &str) -> bool {
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(expires_at) else {
        // Unparseable expiry — treat as still valid; disconnect is explicit.
        return false;
    };
    parsed < chrono::Utc::now()
}

/// Clear grant, strip MCP, and stop opted-in running agents.
pub fn disconnect(app: &tauri::AppHandle) -> Result<OpenClawWorkspaceStatus, String> {
    let stopped = stop_running_openclaw_agents(app);
    let _ = clear_grant();
    let _ = remove_mcp_server(None, OPENCLAW_WORKSPACE_MCP_NAME);
    // Remove from all Claude managed agents (opted-in or not) so a stale MCP
    // entry cannot linger after disconnect.
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
        agents_updated: stopped,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawWorkspaceTestResult {
    pub ok: bool,
    pub message: String,
    pub http_status: Option<u16>,
}

/// Re-apply the stored grant to Claude `mcpServers` (opted-in agents + home config).
/// Does **not** mint a new JWT — that arrives on the next relay AUTH / HULA frame.
/// Clears nothing; if no grant is stored, returns an actionable error.
pub fn refresh(app: &tauri::AppHandle) -> Result<OpenClawWorkspaceStatus, String> {
    let grant = load_grant()?.ok_or_else(|| {
        "No OpenClaw workspace grant is stored. Join a Hula relay (or wait for the next AUTH) so a capability can be provisioned, then try again.".to_string()
    })?;
    apply_grant(app, grant)
}

/// Authenticated MCP ping: POST JSON-RPC `initialize` to the grant URL with the
/// stored Authorization (+ optional CF Access headers). Does not log secrets.
pub async fn test_connection() -> Result<OpenClawWorkspaceTestResult, String> {
    let grant = load_grant()?
        .ok_or_else(|| "Not connected — no OpenClaw workspace grant is stored.".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let mut req = client
        .post(&grant.url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .header(reqwest::header::AUTHORIZATION, grant.authorization.as_str());

    if let Some(headers) = &grant.headers {
        for (key, value) in headers {
            if key.eq_ignore_ascii_case("authorization") {
                continue;
            }
            req = req.header(key.as_str(), value.as_str());
        }
    }

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "hula-buzz", "version": "0" }
        }
    });

    let response = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("MCP request failed: {e}"))?;

    let status = response.status();
    let http_status = Some(status.as_u16());

    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Ok(OpenClawWorkspaceTestResult {
            ok: false,
            message: "MCP rejected credentials. Try Refresh after the next relay AUTH, or Disconnect and reconnect.".into(),
            http_status,
        });
    }

    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        let snippet: String = text.chars().take(160).collect();
        let detail = if snippet.trim().is_empty() {
            String::new()
        } else {
            format!(": {snippet}")
        };
        return Ok(OpenClawWorkspaceTestResult {
            ok: false,
            message: format!("MCP returned HTTP {}{detail}", status.as_u16()),
            http_status,
        });
    }

    // Body may be JSON or SSE; a 2xx after auth is enough to call the grant healthy.
    let _ = response.bytes().await;
    Ok(OpenClawWorkspaceTestResult {
        ok: true,
        message: "MCP accepted initialize — connection looks healthy.".into(),
        http_status,
    })
}

pub fn status() -> Result<OpenClawWorkspaceStatus, String> {
    match load_grant()? {
        Some(grant) if grant_is_expired(&grant.expires_at) => Ok(OpenClawWorkspaceStatus {
            connected: false,
            expires_at: Some(grant.expires_at),
            url: Some(grant.url),
            connected_via_relay: grant.connected_via_relay,
            agents_updated: 0,
        }),
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

/// If the stored grant is expired (or missing), treat the workspace as down:
/// stop opted-in running agents and clear the grant. Returns the post-check status.
pub fn reconcile_expired_grant(app: &tauri::AppHandle) -> Result<OpenClawWorkspaceStatus, String> {
    let Some(grant) = load_grant()? else {
        return status();
    };
    if !grant_is_expired(&grant.expires_at) {
        return status();
    }
    // Expired grant is "down" — same stop behavior as an explicit Disconnect.
    disconnect(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn bare_record(use_oc: bool) -> ManagedAgentRecord {
        ManagedAgentRecord {
            description: None,
            pubkey: "p".into(),
            name: "n".into(),
            persona_id: None,
            private_key_nsec: "nsec1fake".into(),
            auth_tag: None,
            relay_url: "ws://localhost:3000".into(),
            avatar_url: None,
            acp_command: "buzz-acp".into(),
            agent_command: "claude".into(),
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 320,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            model: None,
            provider: None,
            persona_source_version: None,
            env_vars: BTreeMap::new(),
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            use_openclaw_workspace: use_oc,
            runtime_pid: None,
            backend: BackendKind::Local,
            backend_agent_id: None,
            provider_policy_pending: false,
            provider_binary_path: None,
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: "now".into(),
            updated_at: "now".into(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: Default::default(),
            respond_to_allowlist: vec![],
            display_name: None,
            slug: None,
            runtime: Some("claude".into()),
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            definition_respond_to: None,
            definition_respond_to_allowlist: Vec::new(),
            definition_parallelism: None,
            relay_mesh: None,
            effort_level: None,
        }
    }

    #[test]
    fn inject_skips_when_flag_off() {
        let rec = bare_record(false);
        assert_eq!(
            maybe_inject_standing_instructions(&rec, Some("hello".into())),
            Some("hello".into())
        );
        assert_eq!(maybe_inject_standing_instructions(&rec, None), None);
    }

    #[test]
    fn inject_appends_when_flag_on() {
        let rec = bare_record(true);
        let out =
            maybe_inject_standing_instructions(&rec, Some("You are helpful.".into())).unwrap();
        assert!(out.starts_with("You are helpful."));
        assert!(out.contains("skills_list"));
        assert!(out.contains("openclaw-workspace"));
    }

    #[test]
    fn inject_idempotent() {
        let rec = bare_record(true);
        let once = maybe_inject_standing_instructions(&rec, Some("base".into())).unwrap();
        let twice = maybe_inject_standing_instructions(&rec, Some(once.clone())).unwrap();
        assert_eq!(once, twice);
    }
}
