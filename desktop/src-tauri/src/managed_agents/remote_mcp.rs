//! Per-agent remote MCP connections.
//!
//! Metadata is persisted separately from managed-agent definitions so remote
//! credentials never enter the relay-synced agent event or the React model.
//! Credential values live only in the OS keyring and are materialized into the
//! harness through a one-shot stdin pipe at process startup.

use std::collections::BTreeSet;
use std::fs;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use zeroize::{Zeroize, Zeroizing};

use crate::app_state::AppState;
use crate::secret_store::SecretStore;

use super::{
    atomic_write_json_restricted, load_managed_agents, managed_agent_runtime_keys,
    managed_agents_base_dir, restart_managed_agent_runtime,
};

const PATINA_CONNECTION_ID: &str = "patina";
const PATINA_ORIGIN: &str = "https://patina.so";
const REMOTE_MCP_STORE_FILE: &str = "remote-mcp.json";
const REQUIRED_PATINA_TOOLS: [&str; 4] = [
    "artifact_index",
    "artifact_search",
    "artifact_bundle",
    "artifact_read",
];
// Pinned to Patina's legacy, registry, and semantic command surfaces. The
// connection probe fails closed if any command tool is visible to the key.
const PATINA_WRITE_TOOLS: &[&str] = &[
    "annotate",
    "feedback",
    "record_session_read",
    "cancel_session",
    "revert_session",
    "revert_session_manual",
    "revert_window",
    "report_bug",
    "extract_memory",
    "evaluate_schedule_triggers",
    "record_metric_outcome",
    "retract_node",
    "link_record",
    "apply_lens",
    "fire_trigger",
    "generate_outcome_candidates",
    "open_session",
    "record_action_decision",
    "record_action_outcome",
    "submit_session",
    "define_record_type",
    "create_record",
    "update_record",
    "artifact_put",
    "artifact_revise",
    "artifact_revert",
    "artifact_move",
    "artifact_archive",
    "source_record",
    "source_reread",
    "source_archive",
    "work_propose_promotion",
    "knowledge_ratify",
    "knowledge_request_changes",
    "knowledge_reject",
    "knowledge_flag_contradiction",
    "decision_propose",
    "decision_reverse",
    "decision_reaffirm",
    "decision_ratify",
    "decision_request_changes",
    "decision_reject",
    "decision_run_record",
    "decision_run_materialize",
    "production_submit",
    "production_request_changes",
    "production_approve",
    "production_prepare_publication",
    "production_confirm_publication",
    "production_confirm_retraction",
    "citation_attach",
    "citation_detach",
    "external_ref_attach",
    "external_ref_refresh",
    "external_ref_detach",
    "session_open",
    "session_record_read",
    "session_submit",
    "session_comment_add",
    "session_rebase",
    "session_cancel",
    "session_revert",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StoredRemoteMcpConnection {
    id: String,
    agent_pubkey: String,
    provider: String,
    name: String,
    url: String,
    workspace_slug: String,
    workspace_name: Option<String>,
    principal_name: Option<String>,
    enabled: bool,
    secret_ref: String,
    created_at: String,
    updated_at: String,
    last_verified_at: String,
}

/// Credential-free connection details returned to the webview.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMcpConnectionSummary {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub url: String,
    pub workspace_slug: String,
    pub workspace_name: Option<String>,
    pub principal_name: Option<String>,
    pub enabled: bool,
    pub status: String,
    pub last_verified_at: String,
}

impl From<&StoredRemoteMcpConnection> for RemoteMcpConnectionSummary {
    fn from(connection: &StoredRemoteMcpConnection) -> Self {
        Self {
            id: connection.id.clone(),
            provider: connection.provider.clone(),
            name: connection.name.clone(),
            url: connection.url.clone(),
            workspace_slug: connection.workspace_slug.clone(),
            workspace_name: connection.workspace_name.clone(),
            principal_name: connection.principal_name.clone(),
            enabled: connection.enabled,
            status: if connection.enabled {
                "connected".into()
            } else {
                "disabled".into()
            },
            last_verified_at: connection.last_verified_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatinaKeyInfo {
    workspace_name: Option<String>,
    workspace_slug: Option<String>,
    owner_type: String,
    agent: Option<PatinaAgentInfo>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatinaAgentInfo {
    name: String,
}

#[derive(Debug, Clone)]
struct PatinaProbeResult {
    workspace_name: Option<String>,
    principal_name: Option<String>,
}

/// Secret-bearing ACP server payload. This type is serialized only into the
/// private stdin pipe consumed by `buzz-acp`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteMcpWireServer {
    #[serde(rename = "type")]
    transport: &'static str,
    name: String,
    url: String,
    headers: Vec<RemoteMcpWireHeader>,
}

impl RemoteMcpWireServer {
    /// Clear credential-bearing header values once the startup payload has
    /// been serialized into its independently zeroized byte buffer.
    pub(crate) fn zeroize_secrets(&mut self) {
        use zeroize::Zeroize;

        for header in &mut self.headers {
            header.value.zeroize();
        }
    }
}

#[derive(Serialize)]
pub(crate) struct RemoteMcpWireHeader {
    name: String,
    value: String,
}

pub(crate) struct RemoteMcpStartupPayload(Option<Vec<u8>>);

impl RemoteMcpStartupPayload {
    /// Consume the startup payload through the child pipe, then clear its
    /// bytes whether the write succeeds or fails.
    pub(crate) fn write_to(mut self, child: &mut std::process::Child) -> Result<(), String> {
        let Some(mut payload) = self.0.take() else {
            return Ok(());
        };
        use std::io::Write;

        let result = child
            .stdin
            .take()
            .ok_or_else(|| "remote MCP startup pipe was not created".to_string())
            .and_then(|mut stdin| {
                stdin
                    .write_all(&payload)
                    .and_then(|()| stdin.flush())
                    .map_err(|error| format!("failed to send remote MCP startup payload: {error}"))
            });
        payload.zeroize();
        result
    }
}

impl Drop for RemoteMcpStartupPayload {
    fn drop(&mut self) {
        if let Some(payload) = &mut self.0 {
            payload.zeroize();
        }
    }
}

fn store_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    managed_agents_base_dir(app).map(|directory| directory.join(REMOTE_MCP_STORE_FILE))
}

fn load_store(app: &AppHandle) -> Result<Vec<StoredRemoteMcpConnection>, String> {
    let path = store_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read remote MCP store: {error}"))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse remote MCP store: {error}"))
}

fn save_store(app: &AppHandle, connections: &[StoredRemoteMcpConnection]) -> Result<(), String> {
    let path = store_path(app)?;
    let payload = serde_json::to_vec_pretty(connections)
        .map_err(|error| format!("failed to serialize remote MCP store: {error}"))?;
    atomic_write_json_restricted(&path, &payload)
}

fn integration_secret_store() -> &'static SecretStore {
    SecretStore::shared(crate::app_state::keyring_service())
}

fn secret_ref(agent_pubkey: &str, connection_id: &str) -> String {
    format!(
        "integration:mcp:{}:{}",
        agent_pubkey.to_ascii_lowercase(),
        connection_id
    )
}

fn validate_workspace_slug(input: &str) -> Result<String, String> {
    let slug = input.trim().to_ascii_lowercase();
    let valid = !slug.is_empty()
        && slug.len() <= 80
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !slug.starts_with('-')
        && !slug.ends_with('-');
    if !valid {
        return Err(
            "Patina workspace slug must use lowercase letters, numbers, and internal hyphens"
                .into(),
        );
    }
    Ok(slug)
}

fn validate_api_key(input: &str) -> Result<&str, String> {
    let key = input.trim();
    if !key.starts_with("pk_") || key.len() < 12 {
        return Err("Patina agent key must begin with pk_".into());
    }
    Ok(key)
}

fn patina_url(origin: &str, workspace_slug: &str) -> String {
    format!("{origin}/mcp/{workspace_slug}")
}

fn parse_mcp_response(body: &str) -> Result<serde_json::Value, String> {
    if let Ok(value) = serde_json::from_str(body) {
        return Ok(value);
    }
    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(value) = serde_json::from_str(data) {
                return Ok(value);
            }
        }
    }
    Err("Patina MCP returned neither JSON nor a JSON SSE data frame".into())
}

fn verify_patina_tools(response: &serde_json::Value) -> Result<(), String> {
    let tools = response
        .pointer("/result/tools")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "Patina tools/list response is missing result.tools".to_string())?;
    let names: BTreeSet<&str> = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(serde_json::Value::as_str))
        .collect();
    let missing: Vec<&str> = REQUIRED_PATINA_TOOLS
        .iter()
        .copied()
        .filter(|name| !names.contains(name))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Patina is missing required read tools: {}",
            missing.join(", ")
        ));
    }
    let writes: Vec<&str> = PATINA_WRITE_TOOLS
        .iter()
        .copied()
        .filter(|name| names.contains(name))
        .collect();
    if !writes.is_empty() {
        return Err(format!(
            "Patina key is not viewer-scoped; write tools were exposed: {}",
            writes.join(", ")
        ));
    }
    Ok(())
}

async fn mcp_call(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let response = client
        .post(endpoint)
        .bearer_auth(key)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("Patina MCP unreachable: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("failed to read Patina MCP response: {error}"))?;
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => "Patina key is unauthorized, expired, or revoked".into(),
            _ => format!("Patina MCP returned {status}"),
        });
    }
    parse_mcp_response(&body)
}

async fn probe_patina_at(
    client: &reqwest::Client,
    origin: &str,
    workspace_slug: &str,
    key: &str,
) -> Result<PatinaProbeResult, String> {
    let key_info_response = client
        .get(format!("{origin}/api/v1/auth/key-info"))
        .bearer_auth(key)
        .send()
        .await
        .map_err(|error| format!("Patina identity endpoint unreachable: {error}"))?;
    let key_info_status = key_info_response.status();
    if !key_info_status.is_success() {
        return Err(match key_info_status.as_u16() {
            401 | 403 => "Patina key is unauthorized, expired, or revoked".into(),
            _ => format!("Patina identity endpoint returned {key_info_status}"),
        });
    }
    let key_info: PatinaKeyInfo = key_info_response
        .json()
        .await
        .map_err(|error| format!("invalid Patina key-info response: {error}"))?;
    if key_info.workspace_slug.as_deref() != Some(workspace_slug) {
        return Err(format!(
            "Patina key belongs to workspace '{}', not '{}'",
            key_info.workspace_slug.as_deref().unwrap_or("unknown"),
            workspace_slug
        ));
    }
    if key_info.owner_type != "agent" {
        return Err("Connect Patina requires an agent-owned viewer key".into());
    }

    let endpoint = patina_url(origin, workspace_slug);
    let initialize = mcp_call(
        client,
        &endpoint,
        key,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "buzz", "version": env!("CARGO_PKG_VERSION") }
            }
        }),
    )
    .await?;
    if initialize
        .pointer("/result/serverInfo/name")
        .and_then(serde_json::Value::as_str)
        != Some("patina")
    {
        return Err("MCP endpoint did not identify itself as Patina".into());
    }
    let tools = mcp_call(
        client,
        &endpoint,
        key,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await?;
    verify_patina_tools(&tools)?;

    Ok(PatinaProbeResult {
        workspace_name: key_info.workspace_name,
        principal_name: key_info.agent.map(|agent| agent.name),
    })
}

async fn restart_running_agent_pairs(app: &AppHandle, pubkey: &str) -> Result<(), String> {
    let keys = {
        let state = app.state::<AppState>();
        let runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;
        managed_agent_runtime_keys(&runtimes, pubkey)
    };
    for key in keys {
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            restart_managed_agent_runtime(key.pubkey, key.relay_url, app)
        })
        .await
        .map_err(|error| format!("agent restart task failed: {error}"))??;
    }
    Ok(())
}

/// Materialize enabled remote servers for the harness startup pipe.
pub(crate) fn materialize_remote_mcp_servers(
    app: &AppHandle,
    agent_pubkey: &str,
) -> Result<Vec<RemoteMcpWireServer>, String> {
    let connections = load_store(app)?;
    connections
        .into_iter()
        .filter(|connection| {
            connection.enabled && connection.agent_pubkey.eq_ignore_ascii_case(agent_pubkey)
        })
        .map(|connection| {
            let token = Zeroizing::new(
                integration_secret_store()
                    .load(&connection.secret_ref)?
                    .ok_or_else(|| {
                        format!(
                            "remote MCP credential '{}' is missing from the OS keyring",
                            connection.name
                        )
                    })?,
            );
            Ok(RemoteMcpWireServer {
                transport: "http",
                name: connection.name,
                url: connection.url,
                headers: vec![RemoteMcpWireHeader {
                    name: "Authorization".into(),
                    value: format!("Bearer {}", token.as_str()),
                }],
            })
        })
        .collect()
}

/// Resolve remote secrets, configure the private child pipe, and retain only a
/// zeroizable serialized payload for the post-spawn write.
pub(crate) fn configure_remote_mcp_startup(
    command: &mut std::process::Command,
    app: &AppHandle,
    agent_pubkey: &str,
) -> Result<RemoteMcpStartupPayload, String> {
    let mut servers = materialize_remote_mcp_servers(app, agent_pubkey)?;
    let payload = if servers.is_empty() {
        None
    } else {
        let result = serde_json::to_vec(&servers)
            .map_err(|error| format!("failed to serialize remote MCP startup payload: {error}"));
        for server in &mut servers {
            server.zeroize_secrets();
        }
        Some(result?)
    };
    command.stdin(if payload.is_some() {
        std::process::Stdio::piped()
    } else {
        std::process::Stdio::null()
    });
    if payload.is_some() {
        command.env("BUZZ_ACP_REMOTE_MCP_STDIN", "true");
    } else {
        command.env_remove("BUZZ_ACP_REMOTE_MCP_STDIN");
    }
    Ok(RemoteMcpStartupPayload(payload))
}

#[tauri::command]
pub fn list_remote_mcp_connections(
    pubkey: String,
    app: AppHandle,
) -> Result<Vec<RemoteMcpConnectionSummary>, String> {
    Ok(load_store(&app)?
        .iter()
        .filter(|connection| connection.agent_pubkey.eq_ignore_ascii_case(&pubkey))
        .map(RemoteMcpConnectionSummary::from)
        .collect())
}

/// Remove persisted remote-MCP metadata before deleting its keyring entries.
///
/// Agent deletion uses this best-effort cleanup so an integration credential
/// cannot remain addressable after its owning managed agent is gone.
pub(crate) fn delete_remote_mcp_connections_for_agent(
    app: &AppHandle,
    agent_pubkey: &str,
) -> Result<(), String> {
    let mut connections = load_store(app)?;
    let removed_secret_refs: Vec<String> = connections
        .iter()
        .filter(|connection| connection.agent_pubkey.eq_ignore_ascii_case(agent_pubkey))
        .map(|connection| connection.secret_ref.clone())
        .collect();
    if removed_secret_refs.is_empty() {
        return Ok(());
    }
    connections.retain(|connection| !connection.agent_pubkey.eq_ignore_ascii_case(agent_pubkey));
    save_store(app, &connections)?;

    let failures: Vec<String> = removed_secret_refs
        .into_iter()
        .filter_map(|secret_ref| integration_secret_store().delete(&secret_ref).err())
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "remote MCP metadata was removed, but {} OS-keyring credential(s) could not be deleted",
            failures.len()
        ))
    }
}

#[tauri::command]
pub async fn connect_patina(
    pubkey: String,
    workspace_slug: String,
    api_key: String,
    app: AppHandle,
) -> Result<RemoteMcpConnectionSummary, String> {
    if !load_managed_agents(&app)?
        .iter()
        .any(|agent| agent.pubkey.eq_ignore_ascii_case(&pubkey))
    {
        return Err(format!("agent {pubkey} not found"));
    }
    let workspace_slug = validate_workspace_slug(&workspace_slug)?;
    let api_key = Zeroizing::new(api_key);
    let api_key = validate_api_key(&api_key)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| format!("failed to build Patina client: {error}"))?;
    let probe = probe_patina_at(&client, PATINA_ORIGIN, &workspace_slug, api_key).await?;

    let mut connections = load_store(&app)?;
    let secret_ref = secret_ref(&pubkey, PATINA_CONNECTION_ID);
    let previous_secret = integration_secret_store()
        .load(&secret_ref)?
        .map(Zeroizing::new);
    integration_secret_store().store(&secret_ref, api_key)?;

    let now = crate::util::now_iso();
    let connection = StoredRemoteMcpConnection {
        id: PATINA_CONNECTION_ID.into(),
        agent_pubkey: pubkey.clone(),
        provider: "patina".into(),
        name: "patina".into(),
        url: patina_url(PATINA_ORIGIN, &workspace_slug),
        workspace_slug,
        workspace_name: probe.workspace_name,
        principal_name: probe.principal_name,
        enabled: true,
        secret_ref: secret_ref.clone(),
        created_at: connections
            .iter()
            .find(|connection| {
                connection.agent_pubkey.eq_ignore_ascii_case(&pubkey)
                    && connection.id == PATINA_CONNECTION_ID
            })
            .map(|connection| connection.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
        last_verified_at: now,
    };
    connections.retain(|existing| {
        !(existing.agent_pubkey.eq_ignore_ascii_case(&pubkey)
            && existing.id == PATINA_CONNECTION_ID)
    });
    connections.push(connection.clone());
    if let Err(error) = save_store(&app, &connections) {
        match previous_secret {
            Some(previous) => {
                let _ = integration_secret_store().store(&secret_ref, previous.as_str());
            }
            None => {
                let _ = integration_secret_store().delete(&secret_ref);
            }
        }
        return Err(error);
    }
    restart_running_agent_pairs(&app, &pubkey).await?;
    Ok(RemoteMcpConnectionSummary::from(&connection))
}

#[tauri::command]
pub async fn test_patina_connection(
    pubkey: String,
    app: AppHandle,
) -> Result<RemoteMcpConnectionSummary, String> {
    let mut connections = load_store(&app)?;
    let connection = connections
        .iter_mut()
        .find(|connection| {
            connection.agent_pubkey.eq_ignore_ascii_case(&pubkey)
                && connection.id == PATINA_CONNECTION_ID
        })
        .ok_or_else(|| "Patina is not connected for this agent".to_string())?;
    let api_key = Zeroizing::new(
        integration_secret_store()
            .load(&connection.secret_ref)?
            .ok_or_else(|| "Patina credential is missing from the OS keyring".to_string())?,
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| format!("failed to build Patina client: {error}"))?;
    let probe =
        probe_patina_at(&client, PATINA_ORIGIN, &connection.workspace_slug, &api_key).await?;
    connection.workspace_name = probe.workspace_name;
    connection.principal_name = probe.principal_name;
    connection.last_verified_at = crate::util::now_iso();
    connection.updated_at = connection.last_verified_at.clone();
    let summary = RemoteMcpConnectionSummary::from(&*connection);
    save_store(&app, &connections)?;
    Ok(summary)
}

#[tauri::command]
pub async fn set_remote_mcp_enabled(
    pubkey: String,
    connection_id: String,
    enabled: bool,
    app: AppHandle,
) -> Result<RemoteMcpConnectionSummary, String> {
    let mut connections = load_store(&app)?;
    let connection = connections
        .iter_mut()
        .find(|connection| {
            connection.agent_pubkey.eq_ignore_ascii_case(&pubkey) && connection.id == connection_id
        })
        .ok_or_else(|| format!("remote MCP connection {connection_id} not found"))?;
    if enabled
        && integration_secret_store()
            .load(&connection.secret_ref)?
            .is_none()
    {
        return Err("remote MCP credential is missing from the OS keyring".into());
    }
    connection.enabled = enabled;
    connection.updated_at = crate::util::now_iso();
    let summary = RemoteMcpConnectionSummary::from(&*connection);
    save_store(&app, &connections)?;
    restart_running_agent_pairs(&app, &pubkey).await?;
    Ok(summary)
}

#[tauri::command]
pub async fn disconnect_remote_mcp(
    pubkey: String,
    connection_id: String,
    app: AppHandle,
) -> Result<(), String> {
    let mut connections = load_store(&app)?;
    let index = connections
        .iter()
        .position(|connection| {
            connection.agent_pubkey.eq_ignore_ascii_case(&pubkey) && connection.id == connection_id
        })
        .ok_or_else(|| format!("remote MCP connection {connection_id} not found"))?;
    let removed = connections.remove(index);
    let secret = integration_secret_store()
        .load(&removed.secret_ref)?
        .map(Zeroizing::new);
    integration_secret_store().delete(&removed.secret_ref)?;
    if let Err(error) = save_store(&app, &connections) {
        if let Some(secret) = secret {
            let _ = integration_secret_store().store(&removed.secret_ref, secret.as_str());
        }
        return Err(error);
    }
    restart_running_agent_pairs(&app, &pubkey).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_slug_is_normalized_and_restricted() {
        assert_eq!(validate_workspace_slug(" Acme-Team ").unwrap(), "acme-team");
        assert!(validate_workspace_slug("https://patina.so/mcp/acme").is_err());
        assert!(validate_workspace_slug("../acme").is_err());
    }

    #[test]
    fn persisted_connection_never_contains_credential_value() {
        let connection = StoredRemoteMcpConnection {
            id: "patina".into(),
            agent_pubkey: "abc".into(),
            provider: "patina".into(),
            name: "patina".into(),
            url: "https://patina.so/mcp/acme".into(),
            workspace_slug: "acme".into(),
            workspace_name: Some("Acme".into()),
            principal_name: Some("Buzz Viewer".into()),
            enabled: true,
            secret_ref: "integration:mcp:abc:patina".into(),
            created_at: "2026-07-28T00:00:00Z".into(),
            updated_at: "2026-07-28T00:00:00Z".into(),
            last_verified_at: "2026-07-28T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&connection).unwrap();
        assert!(!json.contains("pk_secret"));
        assert!(!json.contains("apiKey"));
        assert!(json.contains("secretRef"));
    }

    #[test]
    fn streamable_http_sse_response_is_parsed() {
        let response = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"serverInfo\":{\"name\":\"patina\"}}}\n\n";
        let parsed = parse_mcp_response(response).unwrap();
        assert_eq!(parsed["result"]["serverInfo"]["name"], "patina");
    }

    #[test]
    fn viewer_tool_contract_rejects_missing_or_write_tools() {
        let valid = serde_json::json!({
            "result": {
                "tools": REQUIRED_PATINA_TOOLS.map(|name| serde_json::json!({"name": name}))
            }
        });
        assert!(verify_patina_tools(&valid).is_ok());

        let missing = serde_json::json!({
            "result": { "tools": [{"name": "artifact_index"}] }
        });
        assert!(verify_patina_tools(&missing).is_err());

        let mut names = REQUIRED_PATINA_TOOLS.to_vec();
        names.extend(["create_record", "fire_trigger", "artifact_put"]);
        let write = serde_json::json!({
            "result": {
                "tools": names.into_iter().map(|name| serde_json::json!({"name": name})).collect::<Vec<_>>()
            }
        });
        assert!(verify_patina_tools(&write).is_err());
    }
}
