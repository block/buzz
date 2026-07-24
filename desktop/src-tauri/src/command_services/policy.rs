use crate::command_services::ssh::ProtectedFile;
use crate::secret_store::SecretStore;
use chrono::Utc;
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;
use url::Url;

const MAXIMUM_CANONICAL_BYTES: usize = 4 * 1024 * 1024;
const MAXIMUM_JSON_DEPTH: usize = 64;
const MAXIMUM_JSON_NODES: usize = 10_000;
const MAXIMUM_MCP_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAXIMUM_CONFIG_BYTES: u64 = 64 * 1024;
const MCP_TIMEOUT: Duration = Duration::from_secs(3);
const ADMISSION_CACHE_TTL: Duration = Duration::from_secs(30);

pub(crate) const MEMORY_READ_ONLY_TOOLS: &[&str] = &[
    "get_entity",
    "get_wiki_page",
    "list_entities",
    "memory_graph",
    "recall_for_entity",
    "search_events",
    "search_wiki",
    "timeline",
];
pub(crate) const MEMORY_WORKFLOW_WRITE_TOOLS: &[&str] =
    &["link_entities", "record_event", "upsert_entity"];
pub(crate) const MEMORY_CATALOG_TOOLS: &[&str] = &[
    "get_entity",
    "get_wiki_page",
    "link_entities",
    "list_entities",
    "memory_graph",
    "memory_metrics",
    "recall_for_entity",
    "record_event",
    "search_events",
    "search_wiki",
    "timeline",
    "upsert_entity",
];
pub(crate) const RAG_CATALOG_TOOLS: &[&str] = &[
    "get_document",
    "get_snapshot_status",
    "list_collections",
    "search_knowledge_base",
];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum KnowledgeServiceKind {
    Memory,
    Rag,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct VerifiedService {
    pub(crate) kind: KnowledgeServiceKind,
    pub(crate) server_identity: String,
    pub(crate) endpoint: String,
    pub(crate) bearer_token: String,
    pub(crate) active_identity: String,
    pub(crate) advertised_tools: Vec<String>,
    pub(crate) verified_at: String,
}

impl std::fmt::Debug for VerifiedService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedService")
            .field("kind", &self.kind)
            .field("server_identity", &self.server_identity)
            .field("endpoint", &self.endpoint)
            .field("bearer_token", &"[REDACTED]")
            .field("active_identity", &self.active_identity)
            .field("advertised_tools", &self.advertised_tools)
            .field("verified_at", &self.verified_at)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmissionError {
    EndpointNotLiteralLoopback,
    AuthenticationUnavailable,
    ServiceUnavailable,
    InvalidResponse,
    ResponseTooLarge,
    ServerIdentityMismatch,
    ActiveIdentityMismatch,
    MissingRequiredTool,
    UnexpectedToolCatalog,
    InvalidAttestation,
}

pub(crate) struct ServiceAdmissionPolicy {
    kind: KnowledgeServiceKind,
    expected_server_identity: String,
    expected_active_identity: String,
    expected_tools: BTreeSet<String>,
}

impl ServiceAdmissionPolicy {
    pub(crate) fn for_service(
        kind: KnowledgeServiceKind,
        expected_server_identity: &str,
        expected_active_identity: &str,
        expected_tools: &[&str],
    ) -> Self {
        Self {
            kind,
            expected_server_identity: expected_server_identity.to_string(),
            expected_active_identity: expected_active_identity.to_string(),
            expected_tools: expected_tools
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
        }
    }

    pub(crate) fn verify(&self, candidate: &VerifiedService) -> Result<(), AdmissionError> {
        validate_literal_loopback_mcp_endpoint(&candidate.endpoint)?;
        if !valid_bearer_token(&candidate.bearer_token) {
            return Err(AdmissionError::AuthenticationUnavailable);
        }
        if candidate.kind != self.kind || candidate.server_identity != self.expected_server_identity
        {
            return Err(AdmissionError::ServerIdentityMismatch);
        }
        if candidate.active_identity != self.expected_active_identity {
            return Err(AdmissionError::ActiveIdentityMismatch);
        }
        if chrono::DateTime::parse_from_rfc3339(&candidate.verified_at).is_err() {
            return Err(AdmissionError::InvalidAttestation);
        }
        let observed = tool_set(&candidate.advertised_tools)?;
        if self
            .expected_tools
            .iter()
            .any(|tool| !observed.contains(tool))
        {
            return Err(AdmissionError::MissingRequiredTool);
        }
        let catalog = catalog_tools(self.kind);
        if observed
            .iter()
            .any(|tool| !catalog.iter().any(|candidate| candidate == &tool.as_str()))
        {
            return Err(AdmissionError::UnexpectedToolCatalog);
        }
        Ok(())
    }
}

fn tool_set(tools: &[String]) -> Result<BTreeSet<String>, AdmissionError> {
    if tools.is_empty() || tools.len() > 32 {
        return Err(AdmissionError::UnexpectedToolCatalog);
    }
    let mut result = BTreeSet::new();
    for tool in tools {
        if tool.is_empty()
            || tool.len() > 128
            || !tool
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || !result.insert(tool.clone())
        {
            return Err(AdmissionError::UnexpectedToolCatalog);
        }
    }
    Ok(result)
}

fn catalog_tools(kind: KnowledgeServiceKind) -> &'static [&'static str] {
    match kind {
        KnowledgeServiceKind::Memory => MEMORY_CATALOG_TOOLS,
        KnowledgeServiceKind::Rag => RAG_CATALOG_TOOLS,
    }
}

fn valid_bearer_token(value: &str) -> bool {
    (16..=256).contains(&value.len())
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
}

pub(crate) fn validate_literal_loopback_mcp_endpoint(endpoint: &str) -> Result<(), AdmissionError> {
    let url = Url::parse(endpoint).map_err(|_| AdmissionError::EndpointNotLiteralLoopback)?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none_or(|port| port == 0)
        || url.path() != "/mcp"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AdmissionError::EndpointNotLiteralLoopback);
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct AuthenticatedMcpAttestation {
    pub(crate) server_identity: String,
    pub(crate) tools: Vec<String>,
    pub(crate) status: Option<Value>,
}

struct McpSession<'a> {
    client: Client,
    endpoint: &'a str,
    bearer_token: &'a str,
    session_id: Option<String>,
}

impl McpSession<'_> {
    fn post(&mut self, request: &Value) -> Result<Value, AdmissionError> {
        let mut builder = self
            .client
            .post(self.endpoint)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .header(AUTHORIZATION, format!("Bearer {}", self.bearer_token))
            .json(request);
        if let Some(session_id) = &self.session_id {
            builder = builder.header("mcp-session-id", session_id);
        }
        let response = builder
            .send()
            .map_err(|_| AdmissionError::ServiceUnavailable)?;
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err(AdmissionError::AuthenticationUnavailable);
        }
        if response.status().is_redirection() || !response.status().is_success() {
            return Err(AdmissionError::ServiceUnavailable);
        }
        if self.session_id.is_none() {
            self.session_id = response
                .headers()
                .get("mcp-session-id")
                .and_then(|header| header.to_str().ok())
                .filter(|value| {
                    !value.is_empty()
                        && value.len() <= 256
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'\r' | b'\n'))
                })
                .map(str::to_string);
        }
        parse_mcp_response(response)
    }

    fn notify_initialized(&mut self) -> Result<(), AdmissionError> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {},
        });
        let mut builder = self
            .client
            .post(self.endpoint)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .header(AUTHORIZATION, format!("Bearer {}", self.bearer_token))
            .json(&request);
        if let Some(session_id) = &self.session_id {
            builder = builder.header("mcp-session-id", session_id);
        }
        let response = builder
            .send()
            .map_err(|_| AdmissionError::ServiceUnavailable)?;
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err(AdmissionError::AuthenticationUnavailable);
        }
        if response.status().is_redirection() || !response.status().is_success() {
            return Err(AdmissionError::ServiceUnavailable);
        }
        Ok(())
    }
}

fn parse_mcp_response(response: Response) -> Result<Value, AdmissionError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAXIMUM_MCP_RESPONSE_BYTES as u64)
    {
        return Err(AdmissionError::ResponseTooLarge);
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.split(';').next())
        .ok_or(AdmissionError::InvalidResponse)?
        .to_string();
    if content_type != "application/json" && content_type != "text/event-stream" {
        return Err(AdmissionError::InvalidResponse);
    }
    let mut bytes = Vec::new();
    response
        .take((MAXIMUM_MCP_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| AdmissionError::InvalidResponse)?;
    if bytes.len() > MAXIMUM_MCP_RESPONSE_BYTES {
        return Err(AdmissionError::ResponseTooLarge);
    }
    if content_type == "application/json" {
        return serde_json::from_slice(&bytes).map_err(|_| AdmissionError::InvalidResponse);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| AdmissionError::InvalidResponse)?;
    let payload = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or(AdmissionError::InvalidResponse)?;
    serde_json::from_str(payload).map_err(|_| AdmissionError::InvalidResponse)
}

fn mcp_result(response: &Value) -> Result<&Map<String, Value>, AdmissionError> {
    let object = response
        .as_object()
        .ok_or(AdmissionError::InvalidResponse)?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || object.contains_key("error")
    {
        return Err(AdmissionError::InvalidResponse);
    }
    object
        .get("result")
        .and_then(Value::as_object)
        .ok_or(AdmissionError::InvalidResponse)
}

fn verify_mcp_authentication_gate(
    client: &Client,
    endpoint: &str,
    bearer_token: &str,
) -> Result<(), AdmissionError> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "buzz-auth-negative-probe",
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "buzz-command-auth-probe", "version": "1"},
        },
    });
    let invalid_token = if bearer_token == "buzz-invalid-token-0000000000000000" {
        "buzz-invalid-token-1111111111111111"
    } else {
        "buzz-invalid-token-0000000000000000"
    };
    for authorization in [None, Some(format!("Bearer {invalid_token}"))] {
        let mut builder = client
            .post(endpoint)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(&request);
        if let Some(authorization) = authorization {
            builder = builder.header(AUTHORIZATION, authorization);
        }
        let response = builder
            .send()
            .map_err(|_| AdmissionError::ServiceUnavailable)?;
        if !matches!(response.status().as_u16(), 401 | 403) {
            return Err(AdmissionError::AuthenticationUnavailable);
        }
    }
    Ok(())
}

pub(crate) fn probe_authenticated_mcp(
    endpoint: &str,
    bearer_token: &str,
    status_tool: Option<&str>,
) -> Result<AuthenticatedMcpAttestation, AdmissionError> {
    validate_literal_loopback_mcp_endpoint(endpoint)?;
    if !valid_bearer_token(bearer_token) {
        return Err(AdmissionError::AuthenticationUnavailable);
    }
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(MCP_TIMEOUT)
        .build()
        .map_err(|_| AdmissionError::ServiceUnavailable)?;
    verify_mcp_authentication_gate(&client, endpoint, bearer_token)?;
    let mut session = McpSession {
        client,
        endpoint,
        bearer_token,
        session_id: None,
    };
    let initialize = session.post(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "buzz-command", "version": "1"},
        },
    }))?;
    let initialize_result = mcp_result(&initialize)?;
    let server_identity = initialize_result
        .get("serverInfo")
        .and_then(Value::as_object)
        .and_then(|server| server.get("name"))
        .and_then(Value::as_str)
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
        .ok_or(AdmissionError::InvalidResponse)?
        .to_string();
    session.notify_initialized()?;

    let tools_response = session.post(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {},
    }))?;
    let tools_result = mcp_result(&tools_response)?;
    let tool_values = tools_result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or(AdmissionError::InvalidResponse)?;
    let mut tools = Vec::with_capacity(tool_values.len());
    for tool in tool_values {
        let name = tool
            .as_object()
            .and_then(|tool| tool.get("name"))
            .and_then(Value::as_str)
            .ok_or(AdmissionError::InvalidResponse)?;
        tools.push(name.to_string());
    }
    tool_set(&tools)?;

    let status = if let Some(status_tool) = status_tool {
        if !tools.iter().any(|tool| tool == status_tool) {
            return Err(AdmissionError::MissingRequiredTool);
        }
        let status_response = session.post(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": status_tool, "arguments": {}},
        }))?;
        let result = mcp_result(&status_response)?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(AdmissionError::InvalidResponse);
        }
        let value = if let Some(value) = result.get("structuredContent") {
            value.clone()
        } else {
            let text = result
                .get("content")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_object)
                .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str)
                .ok_or(AdmissionError::InvalidResponse)?;
            serde_json::from_str(text).map_err(|_| AdmissionError::InvalidResponse)?
        };
        Some(value)
    } else {
        None
    };
    Ok(AuthenticatedMcpAttestation {
        server_identity,
        tools,
        status,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandKnowledgeWorkflow {
    Adviser,
    CommandMemory,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub(crate) struct NativeMcpIntegration {
    #[serde(rename = "type")]
    integration_type: &'static str,
    pub(crate) server_label: String,
    pub(crate) server_url: String,
    pub(crate) allowed_tools: Vec<String>,
    headers: BTreeMap<String, String>,
}

impl std::fmt::Debug for NativeMcpIntegration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeMcpIntegration")
            .field("integration_type", &self.integration_type)
            .field("server_label", &self.server_label)
            .field("server_url", &self.server_url)
            .field("allowed_tools", &self.allowed_tools)
            .field("headers", &"[REDACTED]")
            .finish()
    }
}

pub(crate) fn build_catalog_integrations(
    services: &[VerifiedService],
    workflow: CommandKnowledgeWorkflow,
) -> Result<Vec<NativeMcpIntegration>, AdmissionError> {
    if services.len() > 2 {
        return Err(AdmissionError::UnexpectedToolCatalog);
    }
    let mut ordered = services.to_vec();
    ordered.sort_by_key(|service| service.kind);
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(ordered.len());
    for service in ordered {
        if !seen.insert(service.kind) {
            return Err(AdmissionError::UnexpectedToolCatalog);
        }
        validate_literal_loopback_mcp_endpoint(&service.endpoint)?;
        if !valid_bearer_token(&service.bearer_token) {
            return Err(AdmissionError::AuthenticationUnavailable);
        }
        let expected_identity = match service.kind {
            KnowledgeServiceKind::Memory => "memory",
            KnowledgeServiceKind::Rag => "rag",
        };
        if service.server_identity != expected_identity || service.active_identity.is_empty() {
            return Err(AdmissionError::ServerIdentityMismatch);
        }
        let observed = tool_set(&service.advertised_tools)?;
        if observed.iter().any(|tool| {
            !catalog_tools(service.kind)
                .iter()
                .any(|candidate| candidate == &tool.as_str())
        }) {
            return Err(AdmissionError::UnexpectedToolCatalog);
        }
        let allowed: Vec<String> = match service.kind {
            KnowledgeServiceKind::Rag => RAG_CATALOG_TOOLS
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
            KnowledgeServiceKind::Memory => MEMORY_READ_ONLY_TOOLS
                .iter()
                .chain(
                    (workflow == CommandKnowledgeWorkflow::CommandMemory)
                        .then_some(MEMORY_WORKFLOW_WRITE_TOOLS)
                        .into_iter()
                        .flatten(),
                )
                .filter(|tool| observed.contains(**tool))
                .map(|tool| (*tool).to_string())
                .collect(),
        };
        if allowed.is_empty() || allowed.iter().any(|tool| !observed.contains(tool)) {
            return Err(AdmissionError::MissingRequiredTool);
        }
        result.push(NativeMcpIntegration {
            integration_type: "ephemeral_mcp",
            server_label: expected_identity.to_string(),
            server_url: service.endpoint,
            allowed_tools: allowed,
            headers: BTreeMap::from([(
                "Authorization".to_string(),
                format!("Bearer {}", service.bearer_token),
            )]),
        });
    }
    Ok(result)
}

struct CachedAdmissions {
    expires_at: Instant,
    services: Vec<VerifiedService>,
}

static ADMISSION_CACHE: LazyLock<Mutex<Option<CachedAdmissions>>> =
    LazyLock::new(|| Mutex::new(None));

pub(crate) fn cache_verified_service(service: VerifiedService) {
    let Ok(mut cache) = ADMISSION_CACHE.lock() else {
        return;
    };
    let now = Instant::now();
    let mut services = cache
        .take()
        .filter(|cached| cached.expires_at > now)
        .map(|cached| cached.services)
        .unwrap_or_default();
    services.retain(|candidate| candidate.kind != service.kind);
    services.push(service);
    *cache = Some(CachedAdmissions {
        expires_at: now + ADMISSION_CACHE_TTL,
        services,
    });
}

pub(crate) fn clear_cached_service(kind: KnowledgeServiceKind) {
    let Ok(mut cache) = ADMISSION_CACHE.lock() else {
        return;
    };
    if let Some(cached) = cache.as_mut() {
        cached.services.retain(|service| service.kind != kind);
        if cached.services.is_empty() {
            *cache = None;
        }
    }
}

pub(crate) fn catalog_integrations_json(workflow: CommandKnowledgeWorkflow) -> String {
    let services = ADMISSION_CACHE
        .lock()
        .ok()
        .and_then(|mut cache| {
            let now = Instant::now();
            if cache
                .as_ref()
                .is_some_and(|cached| cached.expires_at <= now)
            {
                *cache = None;
            }
            cache.as_ref().map(|cached| cached.services.clone())
        })
        .unwrap_or_default();
    build_catalog_integrations(&services, workflow)
        .and_then(|integrations| {
            serde_json::to_string(&integrations).map_err(|_| AdmissionError::InvalidAttestation)
        })
        .unwrap_or_else(|_| "[]".to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryCredentialKeys {
    local_read: String,
    local_replicate: String,
    remote_read: String,
    remote_replicate: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryAdmissionConfig {
    schema_version: u32,
    local_port: u16,
    home_host_alias: String,
    home_user: String,
    pinned_host_fingerprint: String,
    known_hosts_path: std::path::PathBuf,
    identity_file: std::path::PathBuf,
    remote_loopback_port: u16,
    local_node_id: String,
    home_node_id: String,
    sync_interval_minutes: u32,
    tool_allowlist: Vec<String>,
    credential_keys: MemoryCredentialKeys,
}

fn valid_memory_credential_key(value: &str) -> bool {
    value.starts_with("memory.")
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_memory_admission_config(config: &MemoryAdmissionConfig) -> Result<(), AdmissionError> {
    let tools = tool_set(&config.tool_allowlist)?;
    let credential_keys = [
        config.credential_keys.local_read.as_str(),
        config.credential_keys.local_replicate.as_str(),
        config.credential_keys.remote_read.as_str(),
        config.credential_keys.remote_replicate.as_str(),
    ];
    if config.schema_version != 1
        || config.local_port == 0
        || config.remote_loopback_port == 0
        || config.local_node_id == config.home_node_id
        || config.local_node_id.len() > 128
        || !config.local_node_id.starts_with("node:")
        || config.home_node_id.len() > 128
        || !config.home_node_id.starts_with("node:")
        || !(5..=1440).contains(&config.sync_interval_minutes)
        || tools.iter().any(|tool| {
            !MEMORY_CATALOG_TOOLS
                .iter()
                .any(|candidate| candidate == &tool.as_str())
        })
        || !MEMORY_READ_ONLY_TOOLS
            .iter()
            .any(|required| tools.contains(*required))
        || credential_keys
            .iter()
            .any(|key| !valid_memory_credential_key(key))
        || credential_keys
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != credential_keys.len()
        || config.home_host_alias.is_empty()
        || config.home_user.is_empty()
        || config.pinned_host_fingerprint.is_empty()
        || !config.known_hosts_path.is_absolute()
        || !config.identity_file.is_absolute()
    {
        return Err(AdmissionError::InvalidAttestation);
    }
    Ok(())
}

pub(crate) fn admit_memory_for_catalog(
    app: &tauri::AppHandle,
    readiness_node_id: &str,
    observed_at: &str,
) -> Result<VerifiedService, AdmissionError> {
    let path = app
        .path()
        .app_config_dir()
        .map_err(|_| AdmissionError::InvalidAttestation)?
        .join("command-memory.json");
    let bytes = ProtectedFile::open(&path, MAXIMUM_CONFIG_BYTES)
        .and_then(|file| file.read_all())
        .map_err(|_| AdmissionError::InvalidAttestation)?;
    let config: MemoryAdmissionConfig =
        serde_json::from_slice(&bytes).map_err(|_| AdmissionError::InvalidAttestation)?;
    validate_memory_admission_config(&config)?;
    if config.local_node_id != readiness_node_id {
        return Err(AdmissionError::ActiveIdentityMismatch);
    }
    let store = SecretStore::shared(crate::app_state::keyring_service());
    let bearer_token = store
        .load(&config.credential_keys.local_read)
        .map_err(|_| AdmissionError::AuthenticationUnavailable)?
        .ok_or(AdmissionError::AuthenticationUnavailable)?;
    let endpoint = format!("http://127.0.0.1:{}/mcp", config.local_port);
    let attestation = probe_authenticated_mcp(&endpoint, &bearer_token, None)?;
    let advertised = tool_set(&attestation.tools)?;
    // Memory may expose replication/admin tools for native operators. Admit
    // only the protected-config subset and never copy the broader server
    // catalogue into the model-facing integration.
    if config
        .tool_allowlist
        .iter()
        .any(|tool| !advertised.contains(tool))
    {
        return Err(AdmissionError::UnexpectedToolCatalog);
    }
    let service = VerifiedService {
        kind: KnowledgeServiceKind::Memory,
        server_identity: attestation.server_identity,
        endpoint,
        bearer_token,
        active_identity: config.local_node_id.clone(),
        advertised_tools: config.tool_allowlist.clone(),
        verified_at: observed_at.to_string(),
    };
    let expected: Vec<&str> = config.tool_allowlist.iter().map(String::as_str).collect();
    ServiceAdmissionPolicy::for_service(
        KnowledgeServiceKind::Memory,
        "memory",
        &config.local_node_id,
        &expected,
    )
    .verify(&service)?;
    Ok(service)
}

mod context;
pub(crate) use context::{canonical_json_bytes, sha256_hex};
#[allow(
    unused_imports,
    reason = "Phase 4 consumes these sealed context-validation APIs"
)]
pub(crate) use context::{
    validate_apple_context, validate_memory_context, validate_rag_context, verify_memory_revision,
    verify_replication_envelope, AdviserContextPolicy, ContextRejection, IntegrityError,
};
pub(crate) mod status;

#[cfg(test)]
mod tests;
