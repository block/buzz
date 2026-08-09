use hmac::{Hmac, KeyInit, Mac};
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Read;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;
use url::Url;

const MAXIMUM_CANONICAL_BYTES: usize = 4 * 1024 * 1024;
const MAXIMUM_JSON_DEPTH: usize = 64;
const MAXIMUM_JSON_NODES: usize = 10_000;
const MAXIMUM_MCP_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAXIMUM_ATTESTATION_RESPONSE_BYTES: usize = 4 * 1024;
const MCP_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) const MEMORY_READ_ONLY_TOOLS: &[&str] = &["command_memory_context"];
pub(crate) const MEMORY_WORKFLOW_WRITE_TOOLS: &[&str] =
    &["link_entities", "record_event", "upsert_entity"];
pub(crate) const MEMORY_CATALOG_TOOLS: &[&str] = &[
    "command_memory_context",
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

/// One already-admitted local service plus the independent secret required to
/// re-attest it before every fixed source read.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct AuthenticatedSourceService {
    verified: VerifiedService,
    attestation_secret: String,
}

impl AuthenticatedSourceService {
    pub(crate) fn new(
        verified: VerifiedService,
        attestation_secret: &str,
    ) -> Result<Self, AdmissionError> {
        if !valid_attestation_secret(attestation_secret)
            || !admission_secrets_are_independent(&verified.bearer_token, attestation_secret)
        {
            return Err(AdmissionError::AuthenticationUnavailable);
        }
        Ok(Self {
            verified,
            attestation_secret: attestation_secret.to_string(),
        })
    }

    #[allow(
        dead_code,
        reason = "Task 8 installs the production command-brief source backend"
    )]
    pub(crate) fn call(
        &self,
        tool_name: &str,
        arguments: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, AdmissionError> {
        call_authenticated_source_tool(
            &self.verified.endpoint,
            &self.verified.bearer_token,
            &self.attestation_secret,
            &self.verified.server_identity,
            &self.verified.active_identity,
            AuthenticatedSourceToolCall::new(tool_name, arguments),
            cancellation,
        )
    }

    pub(crate) fn active_identity(&self) -> &str {
        &self.verified.active_identity
    }
}

impl std::fmt::Debug for AuthenticatedSourceService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthenticatedSourceService")
            .field("verified", &self.verified)
            .field("attestation_secret", &"[REDACTED]")
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

impl AdmissionError {
    pub(crate) const fn is_authentication_failure(self) -> bool {
        matches!(self, Self::AuthenticationUnavailable)
    }
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
        validate_service_endpoint(self.kind, &candidate.endpoint)?;
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
    validate_literal_loopback_mcp_path(endpoint, "/mcp")
}

pub(crate) fn validate_rag_literal_loopback_mcp_endpoint(
    endpoint: &str,
) -> Result<(), AdmissionError> {
    validate_literal_loopback_mcp_path(endpoint, "/mcp/")
}

fn validate_service_endpoint(
    kind: KnowledgeServiceKind,
    endpoint: &str,
) -> Result<(), AdmissionError> {
    match kind {
        KnowledgeServiceKind::Memory => validate_literal_loopback_mcp_endpoint(endpoint),
        KnowledgeServiceKind::Rag => validate_rag_literal_loopback_mcp_endpoint(endpoint),
    }
}

fn validate_literal_loopback_mcp_path(
    endpoint: &str,
    expected_path: &str,
) -> Result<(), AdmissionError> {
    let url = Url::parse(endpoint).map_err(|_| AdmissionError::EndpointNotLiteralLoopback)?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none_or(|port| port == 0)
        || url.path() != expected_path
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
    cancellation: Option<&'a CancellationToken>,
}

impl McpSession<'_> {
    fn post(&mut self, request: &Value) -> Result<Value, AdmissionError> {
        ensure_mcp_active(self.cancellation)?;
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
        ensure_mcp_active(self.cancellation)?;
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
    cancellation: Option<&CancellationToken>,
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
    let invalid_token = format!(
        "buzz-invalid-{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    for authorization in [None, Some(format!("Bearer {invalid_token}"))] {
        ensure_mcp_active(cancellation)?;
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

fn verify_service_attestation(
    client: &Client,
    endpoint: &str,
    attestation_secret: &str,
    expected_service: &str,
    expected_identity: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<(), AdmissionError> {
    ensure_mcp_active(cancellation)?;
    if !valid_attestation_secret(attestation_secret)
        || !matches!(expected_service, "memory" | "rag")
        || expected_identity.is_empty()
        || expected_identity.len() > 256
    {
        return Err(AdmissionError::AuthenticationUnavailable);
    }
    let mut attestation_url =
        Url::parse(endpoint).map_err(|_| AdmissionError::EndpointNotLiteralLoopback)?;
    attestation_url.set_path("/attestation");
    attestation_url.set_query(None);
    attestation_url.set_fragment(None);
    let nonce = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let response = client
        .post(attestation_url)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({"nonce": nonce}))
        .send()
        .map_err(|_| AdmissionError::ServiceUnavailable)?;
    ensure_mcp_active(cancellation)?;
    if response.status().is_redirection() || !response.status().is_success() {
        return Err(AdmissionError::AuthenticationUnavailable);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAXIMUM_ATTESTATION_RESPONSE_BYTES as u64)
        || response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            != Some("application/json")
    {
        return Err(AdmissionError::InvalidAttestation);
    }
    let mut bytes = Vec::new();
    response
        .take((MAXIMUM_ATTESTATION_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| AdmissionError::InvalidAttestation)?;
    if bytes.len() > MAXIMUM_ATTESTATION_RESPONSE_BYTES {
        return Err(AdmissionError::ResponseTooLarge);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| AdmissionError::InvalidAttestation)?;
    let object = value
        .as_object()
        .filter(|object| {
            object.len() == 4
                && ["service", "identity", "nonce", "mac"]
                    .iter()
                    .all(|key| object.contains_key(*key))
        })
        .ok_or(AdmissionError::InvalidAttestation)?;
    if object.get("service").and_then(Value::as_str) != Some(expected_service)
        || object.get("identity").and_then(Value::as_str) != Some(expected_identity)
        || object.get("nonce").and_then(Value::as_str) != Some(nonce.as_str())
    {
        return Err(AdmissionError::InvalidAttestation);
    }
    let mac = object
        .get("mac")
        .and_then(Value::as_str)
        .ok_or(AdmissionError::InvalidAttestation)
        .and_then(decode_attestation_mac)?;
    verify_attestation_mac(
        attestation_secret,
        expected_service,
        expected_identity,
        &nonce,
        &mac,
    )
}

fn valid_attestation_secret(value: &str) -> bool {
    (32..=1024).contains(&value.len()) && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn ensure_mcp_active(cancellation: Option<&CancellationToken>) -> Result<(), AdmissionError> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        Err(AdmissionError::ServiceUnavailable)
    } else {
        Ok(())
    }
}

pub(crate) fn admission_secrets_are_independent(
    bearer_token: &str,
    attestation_secret: &str,
) -> bool {
    let bearer_digest: [u8; 32] = Sha256::digest(bearer_token.as_bytes()).into();
    let attestation_digest: [u8; 32] = Sha256::digest(attestation_secret.as_bytes()).into();
    !bool::from(bearer_digest.ct_eq(&attestation_digest))
}

fn decode_attestation_mac(value: &str) -> Result<Vec<u8>, AdmissionError> {
    value
        .strip_prefix("sha256:")
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .and_then(|digest| hex::decode(digest).ok())
        .ok_or(AdmissionError::InvalidAttestation)
}

fn verify_attestation_mac(
    attestation_secret: &str,
    service: &str,
    identity: &str,
    nonce: &str,
    supplied_mac: &[u8],
) -> Result<(), AdmissionError> {
    let transcript = format!("buzz-command-attestation-v1\0{service}\0{identity}\0{nonce}");
    let mut verifier = Hmac::<Sha256>::new_from_slice(attestation_secret.as_bytes())
        .map_err(|_| AdmissionError::AuthenticationUnavailable)?;
    verifier.update(transcript.as_bytes());
    verifier
        .verify_slice(supplied_mac)
        .map_err(|_| AdmissionError::InvalidAttestation)
}

pub(crate) fn probe_authenticated_mcp(
    endpoint: &str,
    bearer_token: &str,
    attestation_secret: &str,
    expected_service: &str,
    expected_identity: &str,
    status_tool: Option<&str>,
) -> Result<AuthenticatedMcpAttestation, AdmissionError> {
    let (mut session, server_identity, tools) = open_authenticated_mcp_session(
        endpoint,
        bearer_token,
        attestation_secret,
        expected_service,
        expected_identity,
        None,
    )?;
    let status = if let Some(status_tool) = status_tool {
        if !tools.iter().any(|tool| tool == status_tool) {
            return Err(AdmissionError::MissingRequiredTool);
        }
        Some(call_tool_in_session(
            &mut session,
            status_tool,
            serde_json::json!({}),
        )?)
    } else {
        None
    };
    Ok(AuthenticatedMcpAttestation {
        server_identity,
        tools,
        status,
    })
}

fn open_authenticated_mcp_session<'a>(
    endpoint: &'a str,
    bearer_token: &'a str,
    attestation_secret: &str,
    expected_service: &str,
    expected_identity: &str,
    cancellation: Option<&'a CancellationToken>,
) -> Result<(McpSession<'a>, String, Vec<String>), AdmissionError> {
    ensure_mcp_active(cancellation)?;
    match expected_service {
        "memory" => validate_literal_loopback_mcp_endpoint(endpoint)?,
        "rag" => validate_rag_literal_loopback_mcp_endpoint(endpoint)?,
        _ => return Err(AdmissionError::EndpointNotLiteralLoopback),
    }
    if !valid_bearer_token(bearer_token) {
        return Err(AdmissionError::AuthenticationUnavailable);
    }
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(MCP_TIMEOUT)
        .build()
        .map_err(|_| AdmissionError::ServiceUnavailable)?;
    verify_service_attestation(
        &client,
        endpoint,
        attestation_secret,
        expected_service,
        expected_identity,
        cancellation,
    )?;
    verify_mcp_authentication_gate(&client, endpoint, cancellation)?;
    let mut session = McpSession {
        client,
        endpoint,
        bearer_token,
        session_id: None,
        cancellation,
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
    Ok((session, server_identity, tools))
}

fn call_tool_in_session(
    session: &mut McpSession<'_>,
    tool_name: &str,
    arguments: Value,
) -> Result<Value, AdmissionError> {
    let response = session.post(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": tool_name, "arguments": arguments},
    }))?;
    let result = mcp_result(&response)?;
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(AdmissionError::InvalidResponse);
    }
    if let Some(value) = result.get("structuredContent") {
        return Ok(value.clone());
    }
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_object)
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .ok_or(AdmissionError::InvalidResponse)?;
    serde_json::from_str(text).map_err(|_| AdmissionError::InvalidResponse)
}

pub(crate) struct AuthenticatedSourceToolCall<'a> {
    name: &'a str,
    arguments: Value,
}

impl<'a> AuthenticatedSourceToolCall<'a> {
    pub(crate) const fn new(name: &'a str, arguments: Value) -> Self {
        Self { name, arguments }
    }
}

/// Executes one exact source-read tool through the same fully authenticated
/// MCP session gates used by readiness admission.
pub(crate) fn call_authenticated_source_tool(
    endpoint: &str,
    bearer_token: &str,
    attestation_secret: &str,
    expected_service: &str,
    expected_identity: &str,
    call: AuthenticatedSourceToolCall<'_>,
    cancellation: &CancellationToken,
) -> Result<Value, AdmissionError> {
    ensure_mcp_active(Some(cancellation))?;
    let expected_tool_service = match call.name {
        "command_memory_context" => "memory",
        "search_knowledge_base" | "get_snapshot_status" => "rag",
        _ => return Err(AdmissionError::UnexpectedToolCatalog),
    };
    if expected_service != expected_tool_service
        || !call.arguments.is_object()
        || serde_json::to_vec(&call.arguments)
            .ok()
            .is_none_or(|bytes| bytes.len() > 64 * 1024)
    {
        return Err(AdmissionError::UnexpectedToolCatalog);
    }
    let (mut session, _, tools) = open_authenticated_mcp_session(
        endpoint,
        bearer_token,
        attestation_secret,
        expected_service,
        expected_identity,
        Some(cancellation),
    )?;
    if !tools.iter().any(|tool| tool == call.name) {
        return Err(AdmissionError::MissingRequiredTool);
    }
    call_tool_in_session(&mut session, call.name, call.arguments)
}

#[path = "policy/catalog.rs"]
mod catalog;
pub(crate) use catalog::*;

mod context;
#[cfg(test)]
pub(crate) use context::canonical_json_bytes;
pub(crate) use context::sha256_hex;
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
