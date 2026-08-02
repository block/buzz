use crate::{
    net::{pinned_http_client, resolve_network_url, response_bytes, validate_http_url},
    AdapterError, MAX_ARTIFACT_BYTES, MAX_EXTENSIONS, MAX_EXTENSIONS_JSON_BYTES,
    MAX_EXTENSION_URI_BYTES, TASK_POLL_BACKOFF_SECS,
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::atomic::{AtomicU64, Ordering},
};
use url::Url;

#[derive(Debug, Clone, Deserialize)]
/// Public A2A Agent Card fields used to select an invocation interface.
pub struct AgentCard {
    /// Non-standard identifier used only by the vendor compatibility path.
    #[serde(default, rename = "id")]
    pub vendor_id: Option<String>,
    /// Human-readable name, when advertised.
    #[serde(default)]
    pub name: Option<String>,
    /// Human-readable description, when advertised.
    #[serde(default)]
    pub description: Option<String>,
    /// A2A 0.3 card endpoint.
    #[serde(default)]
    pub url: Option<String>,
    /// Non-standard endpoint field used by the vendor compatibility path.
    #[serde(default, rename = "serviceEndpoint")]
    pub service_endpoint: Option<String>,
    /// Current A2A interface declarations.
    #[serde(default, rename = "supportedInterfaces")]
    pub supported_interfaces: Vec<SupportedInterface>,
    /// Optional protocol extensions advertised by the agent.
    #[serde(default)]
    pub capabilities: AgentCapabilities,
}

#[derive(Debug, Clone, Default, Deserialize)]
/// A2A capabilities used by the adapter.
pub struct AgentCapabilities {
    /// Extension declarations from the Agent Card.
    #[serde(default)]
    pub extensions: Vec<AgentExtension>,
}

#[derive(Debug, Clone, Deserialize)]
/// One A2A protocol extension advertised by an Agent Card.
pub struct AgentExtension {
    /// Exact URI used for negotiation and message metadata.
    pub uri: String,
    /// Whether a client must activate the extension to invoke the agent.
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Deserialize)]
/// A protocol endpoint declared by an A2A Agent Card.
pub struct SupportedInterface {
    /// URL to the protocol endpoint.
    #[serde(default)]
    pub url: Option<String>,
    /// Protocol binding name, for example `JSONRPC`.
    #[serde(default, rename = "protocolBinding")]
    pub protocol_binding: Option<String>,
    /// Protocol version declared by the remote agent.
    #[serde(default, rename = "protocolVersion")]
    pub protocol_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolMode {
    /// A2A JSON-RPC interface declared by a current Agent Card.
    JsonRpc {
        endpoint: String,
        protocol_version: Option<String>,
    },
    /// Compatibility with a deployed vendor card and method shape.
    VendorServiceEndpoint { endpoint: String },
}

impl ProtocolMode {
    pub(super) fn endpoint(&self) -> &str {
        match self {
            Self::JsonRpc { endpoint, .. } | Self::VendorServiceEndpoint { endpoint } => endpoint,
        }
    }

    fn a2a_version(&self) -> Option<&'static str> {
        match self {
            Self::JsonRpc {
                protocol_version, ..
            } if protocol_version
                .as_deref()
                .is_some_and(|version| version.starts_with("1.")) =>
            {
                Some("1.0")
            }
            Self::JsonRpc { .. } => Some("0.3"),
            Self::VendorServiceEndpoint { .. } => None,
        }
    }

    fn method(&self, task: bool) -> &str {
        match self {
            Self::JsonRpc {
                protocol_version, ..
            } => {
                if protocol_version
                    .as_deref()
                    .is_some_and(|version| version.starts_with("1."))
                {
                    if task {
                        "GetTask"
                    } else {
                        "SendMessage"
                    }
                } else if task {
                    "tasks/get"
                } else {
                    "message/send"
                }
            }
            Self::VendorServiceEndpoint { .. } => {
                if task {
                    "agent/getTask"
                } else {
                    "agent/sendMessage"
                }
            }
        }
    }
}

/// A resolved public record and its invocation mode.
#[derive(Debug, Clone)]
/// Resolved public metadata and invocation mode for one remote agent.
pub struct ResolvedAgent {
    /// Name from the OASF record.
    pub record_name: Option<String>,
    /// OASF schema version from the record.
    pub record_schema_version: Option<String>,
    /// Public A2A card resolved from the OASF module.
    pub card: AgentCard,
    /// Selected current or compatibility invocation mode.
    pub mode: ProtocolMode,
}

pub(super) static REQUEST_ID: AtomicU64 = AtomicU64::new(1);
/// Select the declared JSON-RPC interface, with a named pre-1.0 compatibility path.
pub fn select_protocol_mode(card: &AgentCard) -> Result<ProtocolMode, AdapterError> {
    if let Some(interface) = card.supported_interfaces.iter().find(|i| {
        i.protocol_binding
            .as_deref()
            .is_some_and(|binding| binding.to_ascii_lowercase().contains("jsonrpc"))
    }) {
        if let Some(endpoint) = interface.url.clone() {
            validate_http_url(&endpoint)?;
            return Ok(ProtocolMode::JsonRpc {
                endpoint,
                protocol_version: interface.protocol_version.clone(),
            });
        }
    }
    if let Some(endpoint) = card.service_endpoint.clone() {
        validate_http_url(&endpoint)?;
        return Ok(ProtocolMode::VendorServiceEndpoint { endpoint });
    }
    if let Some(endpoint) = card.url.clone() {
        validate_http_url(&endpoint)?;
        return Ok(ProtocolMode::JsonRpc {
            endpoint,
            protocol_version: Some("0.3".into()),
        });
    }
    Err(AdapterError::MissingEndpoint)
}

pub(super) fn protocol_request(
    client: &Client,
    mode: &ProtocolMode,
    endpoint: &str,
    extensions: &BTreeMap<String, Value>,
) -> reqwest::RequestBuilder {
    let request = client.post(endpoint);
    let request = match mode.a2a_version() {
        Some(version) => request.header("A2A-Version", version),
        None => request,
    };
    if extensions.is_empty() {
        request
    } else {
        request.header(
            "A2A-Extensions",
            extensions.keys().cloned().collect::<Vec<_>>().join(", "),
        )
    }
}

pub(super) fn parse_extensions_json(
    raw: Option<&str>,
) -> Result<BTreeMap<String, Value>, AdapterError> {
    let Some(raw) = raw.filter(|value| !value.trim().is_empty()) else {
        return Ok(BTreeMap::new());
    };
    if raw.len() > MAX_EXTENSIONS_JSON_BYTES {
        return Err(AdapterError::InvalidExtensionConfig(format!(
            "configuration exceeds {MAX_EXTENSIONS_JSON_BYTES} bytes"
        )));
    }
    let extensions: BTreeMap<String, Value> = serde_json::from_str(raw)
        .map_err(|error| AdapterError::InvalidExtensionConfig(error.to_string()))?;
    if extensions.len() > MAX_EXTENSIONS {
        return Err(AdapterError::InvalidExtensionConfig(format!(
            "configuration exceeds {MAX_EXTENSIONS} extensions"
        )));
    }
    for uri in extensions.keys() {
        validate_extension_uri(uri)?;
    }
    Ok(extensions)
}

fn validate_extension_uri(uri: &str) -> Result<(), AdapterError> {
    if uri.is_empty() || uri.len() > MAX_EXTENSION_URI_BYTES {
        return Err(AdapterError::InvalidExtensionConfig(
            "extension URI is empty or too long".into(),
        ));
    }
    Url::parse(uri)
        .map(|_| ())
        .map_err(|_| AdapterError::InvalidExtensionConfig(format!("invalid extension URI: {uri}")))
}

pub(super) fn negotiate_extensions(
    card: &AgentCard,
    configured: &BTreeMap<String, Value>,
    mode: &ProtocolMode,
) -> Result<BTreeMap<String, Value>, AdapterError> {
    if !configured.is_empty() && matches!(mode, ProtocolMode::VendorServiceEndpoint { .. }) {
        return Err(AdapterError::InvalidExtensionConfig(
            "A2A extensions require a standard A2A interface".into(),
        ));
    }
    let mut advertised = HashSet::new();
    for extension in &card.capabilities.extensions {
        validate_extension_uri(&extension.uri)?;
        advertised.insert(extension.uri.as_str());
        if extension.required && !configured.contains_key(&extension.uri) {
            return Err(AdapterError::RequiredExtension(extension.uri.clone()));
        }
    }
    for uri in configured.keys() {
        if !advertised.contains(uri.as_str()) {
            return Err(AdapterError::UnsupportedExtension(uri.clone()));
        }
    }
    Ok(configured.clone())
}

pub(super) fn extract_text(value: &Value) -> Option<String> {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    if let Some(parts) = value.get("parts").and_then(Value::as_array) {
        let joined = parts
            .iter()
            .filter_map(extract_text)
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    if let Some(artifacts) = value.get("artifacts").and_then(Value::as_array) {
        let joined = artifacts
            .iter()
            .rev()
            .filter_map(extract_text)
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    if let Some(history) = value.get("history").and_then(Value::as_array) {
        for item in history.iter().rev() {
            if item.get("role").and_then(Value::as_str) != Some("user") {
                if let Some(text) = extract_text(item) {
                    return Some(text);
                }
            }
        }
    }
    value.get("message").and_then(extract_text)
}

pub(super) async fn invoke(
    resolved: &ResolvedAgent,
    token: Option<&str>,
    token_endpoint: Option<&str>,
    extensions: &BTreeMap<String, Value>,
    task_poll_secs: u64,
    session_id: &str,
    text: &str,
) -> Result<String, AdapterError> {
    validate_endpoint_binding(token, token_endpoint, resolved.mode.endpoint())?;
    let endpoint = Url::parse(resolved.mode.endpoint())
        .map_err(|_| AdapterError::UnsafeEndpoint(resolved.mode.endpoint().to_owned()))?;
    let addresses = resolve_network_url(&endpoint).await?;
    let client = pinned_http_client(&endpoint, &addresses)?;
    let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let payload = request_payload(
        &resolved.mode,
        resolved.card.vendor_id.as_deref(),
        id,
        session_id,
        text,
        extensions,
    );
    let mut request = protocol_request(
        &client,
        &resolved.mode,
        resolved.mode.endpoint(),
        extensions,
    )
    .json(&payload);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|e| AdapterError::Request(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AdapterError::HttpStatus {
            what: "A2A request",
            status,
        });
    }
    let bytes = response_bytes(response, "A2A response", MAX_ARTIFACT_BYTES).await?;
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|e| AdapterError::Request(format!("decode A2A response: {e}")))?;
    if let Some(error) = body.get("error") {
        return Err(AdapterError::InvalidResponse(error.to_string()));
    }
    let result = body.get("result").unwrap_or(&body);
    let result = result
        .get("task")
        .or_else(|| result.get("message"))
        .unwrap_or(result);
    if result.pointer("/status/state").is_some() {
        let task_id = result
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if let Some(text) = task_outcome(result, task_id)? {
            return Ok(text);
        }
        if task_id != "unknown" {
            return poll_task(
                resolved,
                token,
                token_endpoint,
                task_id,
                task_poll_secs,
                &client,
                extensions,
            )
            .await;
        }
        return Err(AdapterError::InvalidResponse(
            "A2A task response has no task id".into(),
        ));
    }
    extract_text(result).ok_or_else(|| {
        AdapterError::InvalidResponse("A2A response contains no message or task state".into())
    })
}

async fn poll_task(
    resolved: &ResolvedAgent,
    token: Option<&str>,
    token_endpoint: Option<&str>,
    task_id: &str,
    task_poll_secs: u64,
    client: &Client,
    extensions: &BTreeMap<String, Value>,
) -> Result<String, AdapterError> {
    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(task_poll_secs);
    let mut poll_attempt = 0usize;
    while started.elapsed() < timeout {
        let remaining = timeout.saturating_sub(started.elapsed());
        let delay = std::time::Duration::from_secs(
            TASK_POLL_BACKOFF_SECS[poll_attempt.min(TASK_POLL_BACKOFF_SECS.len() - 1)],
        )
        .min(remaining);
        tokio::time::sleep(delay).await;
        poll_attempt = poll_attempt.saturating_add(1);
        if started.elapsed() >= timeout {
            break;
        }
        let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let params = match resolved.mode {
            ProtocolMode::JsonRpc { .. } => json!({ "id": task_id }),
            ProtocolMode::VendorServiceEndpoint { .. } => json!({ "taskId": task_id }),
        };
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": resolved.mode.method(true),
            "params": params,
        });
        validate_endpoint_binding(token, token_endpoint, resolved.mode.endpoint())?;
        let mut request =
            protocol_request(client, &resolved.mode, resolved.mode.endpoint(), extensions)
                .json(&payload);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|e| AdapterError::Request(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(AdapterError::HttpStatus {
                what: "A2A task poll",
                status,
            });
        }
        let bytes = response_bytes(response, "A2A task response", MAX_ARTIFACT_BYTES).await?;
        let body: Value = serde_json::from_slice(&bytes)
            .map_err(|e| AdapterError::Request(format!("decode A2A task response: {e}")))?;
        if let Some(error) = body.get("error") {
            return Err(AdapterError::InvalidResponse(error.to_string()));
        }
        let result = body.get("result").unwrap_or(&body);
        let result = result
            .get("task")
            .or_else(|| result.get("message"))
            .unwrap_or(result);
        if let Some(text) = task_outcome(result, task_id)? {
            return Ok(text);
        }
    }
    Err(AdapterError::TaskTimeout(task_poll_secs))
}

pub(super) fn validate_endpoint_binding(
    token: Option<&str>,
    expected_endpoint: Option<&str>,
    actual_endpoint: &str,
) -> Result<(), AdapterError> {
    let endpoints_match = match expected_endpoint {
        Some(expected) => {
            let expected = validate_http_url(expected)?;
            let actual = validate_http_url(actual_endpoint)?;
            expected == actual
        }
        None => token.is_none(),
    };
    if !endpoints_match {
        return Err(AdapterError::UnauthorizedTokenEndpoint(
            actual_endpoint.to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn task_outcome(result: &Value, task_id: &str) -> Result<Option<String>, AdapterError> {
    let wire_state = result
        .get("status")
        .and_then(|status| status.get("state"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AdapterError::InvalidResponse(format!("A2A task {task_id} has no status state"))
        })?;
    let normalized_state = wire_state.trim().to_ascii_lowercase();
    let state = normalized_state
        .strip_prefix("task_state_")
        .unwrap_or(&normalized_state);
    match state {
        "completed" => {
            Ok(Some(extract_text(result).unwrap_or_else(|| {
                format!("A2A task {task_id} completed")
            })))
        }
        "accepted" | "submitted" | "working" | "pending" => Ok(None),
        "failed" | "canceled" | "cancelled" | "rejected" | "input-required" | "input_required" => {
            let detail = extract_text(result)
                .map(|text| format!(": {text}"))
                .unwrap_or_default();
            Err(AdapterError::InvalidResponse(format!(
                "A2A task {task_id} ended in {state}{detail}"
            )))
        }
        other => Err(AdapterError::InvalidResponse(format!(
            "A2A task {task_id} has unknown state {wire_state} (normalized as {other})"
        ))),
    }
}

pub(super) fn request_payload(
    mode: &ProtocolMode,
    agent_id: Option<&str>,
    id: u64,
    session_id: &str,
    text: &str,
    extensions: &BTreeMap<String, Value>,
) -> Value {
    let mut params = match mode {
        ProtocolMode::JsonRpc {
            protocol_version, ..
        } if protocol_version
            .as_deref()
            .is_some_and(|version| version.starts_with("1.")) =>
        {
            json!({
                "message": { "messageId": format!("buzz-{id}"), "role": "ROLE_USER", "contextId": session_id, "parts": [{ "text": text }] },
            })
        }
        ProtocolMode::JsonRpc { .. } => json!({
            "message": { "messageId": format!("buzz-{id}"), "role": "user", "contextId": session_id, "parts": [{ "kind": "text", "text": text }] },
        }),
        ProtocolMode::VendorServiceEndpoint { .. } => json!({
            "agentId": agent_id,
            "message": { "role": "user", "parts": [{ "type": "text", "text": text }] },
            "contextId": session_id,
        }),
    };
    if !extensions.is_empty() && matches!(mode, ProtocolMode::JsonRpc { .. }) {
        if let Some(message) = params.get_mut("message").and_then(Value::as_object_mut) {
            message.insert(
                "extensions".into(),
                Value::Array(extensions.keys().cloned().map(Value::String).collect()),
            );
            message.insert(
                "metadata".into(),
                Value::Object(
                    extensions
                        .iter()
                        .map(|(uri, metadata)| (uri.clone(), metadata.clone()))
                        .collect(),
                ),
            );
        }
    }
    json!({ "jsonrpc": "2.0", "id": id, "method": mode.method(false), "params": params })
}
