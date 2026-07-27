use crate::command_services::ssh::ProtectedFile;
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;
use url::{Host, Url};

const MAXIMUM_CONFIG_BYTES: u64 = 64 * 1024;
const MAXIMUM_MCP_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const TRUSTED_MODE: &str = "OFFICIAL_TRUSTED_LAN";
const TRUSTED_LAN_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const TRUSTED_LAN_RETRIEVAL_LIMIT: u32 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrustedLanEndpoint(String);

impl TrustedLanEndpoint {
    pub(crate) fn parse_memory(value: &str) -> Result<Self, TrustedLanError> {
        Self::parse(value, "/mcp")
    }

    pub(crate) fn parse_rag(value: &str) -> Result<Self, TrustedLanError> {
        Self::parse(value, "/mcp/")
    }

    fn parse(value: &str, required_path: &str) -> Result<Self, TrustedLanError> {
        let parsed = Url::parse(value).map_err(|_| TrustedLanError::InvalidEndpoint)?;
        let host = match parsed.host() {
            Some(Host::Ipv4(host)) if host.is_private() => IpAddr::V4(host),
            _ => return Err(TrustedLanError::InvalidEndpoint),
        };
        if parsed.scheme() != "http"
            || parsed.port().is_none()
            || parsed.path() != required_path
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.host_str() != Some(&host.to_string())
        {
            return Err(TrustedLanError::InvalidEndpoint);
        }
        Ok(Self(value.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CloudProviderConfig {
    enabled: bool,
    endpoint: String,
    model: String,
    keychain_key: String,
}

impl CloudProviderConfig {
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn keychain_key(&self) -> &str {
        &self.keychain_key
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TrustedLanConfig {
    memory_url: TrustedLanEndpoint,
    rag_url: TrustedLanEndpoint,
    litellm: CloudProviderConfig,
    openai: CloudProviderConfig,
}

impl TrustedLanConfig {
    pub(crate) fn load(path: &Path) -> Result<Self, TrustedLanError> {
        let bytes = ProtectedFile::open(path, MAXIMUM_CONFIG_BYTES)
            .and_then(|file| file.read_all())
            .map_err(|_| TrustedLanError::UnprotectedConfig)?;
        Self::parse(&bytes)
    }

    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, TrustedLanError> {
        let raw: RawTrustedLanConfig =
            serde_json::from_slice(bytes).map_err(|_| TrustedLanError::InvalidConfig)?;
        if raw.schema_version != 1
            || raw.mode != TRUSTED_MODE
            || !raw.automatic_cloud_fallback_acknowledged
        {
            return Err(TrustedLanError::InvalidConfig);
        }
        let memory_url = TrustedLanEndpoint::parse_memory(&raw.memory_url)?;
        let rag_url = TrustedLanEndpoint::parse_rag(&raw.rag_url)?;
        let litellm = validate_cloud(raw.litellm, CloudKind::LiteLlm)?;
        let openai = validate_cloud(raw.openai, CloudKind::OpenAi)?;
        Ok(Self {
            memory_url,
            rag_url,
            litellm,
            openai,
        })
    }

    #[cfg(test)]
    pub(crate) fn memory_url(&self) -> &TrustedLanEndpoint {
        &self.memory_url
    }

    #[cfg(test)]
    pub(crate) fn rag_url(&self) -> &TrustedLanEndpoint {
        &self.rag_url
    }

    pub(crate) fn litellm(&self) -> &CloudProviderConfig {
        &self.litellm
    }

    pub(crate) fn openai(&self) -> &CloudProviderConfig {
        &self.openai
    }

    pub(crate) fn source_client(&self) -> Result<TrustedLanSourceClient, TrustedLanError> {
        TrustedLanSourceClient::new(self.memory_url.clone(), self.rag_url.clone())
    }
}

pub(crate) fn load_optional(path: &Path) -> Result<Option<TrustedLanConfig>, TrustedLanError> {
    if !path.exists() {
        return Ok(None);
    }
    TrustedLanConfig::load(path).map(Some)
}

#[derive(Clone)]
pub(crate) struct TrustedLanSourceClient {
    http: Client,
    memory_url: TrustedLanEndpoint,
    rag_url: TrustedLanEndpoint,
}

impl std::fmt::Debug for TrustedLanSourceClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TrustedLanSourceClient")
            .field("memory_url", &self.memory_url.as_str())
            .field("rag_url", &self.rag_url.as_str())
            .finish_non_exhaustive()
    }
}

impl TrustedLanSourceClient {
    fn new(
        memory_url: TrustedLanEndpoint,
        rag_url: TrustedLanEndpoint,
    ) -> Result<Self, TrustedLanError> {
        let http = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TRUSTED_LAN_REQUEST_TIMEOUT)
            .build()
            .map_err(|_| TrustedLanError::ServiceUnavailable)?;
        Ok(Self {
            http,
            memory_url,
            rag_url,
        })
    }

    pub(crate) fn catalogue(&self) -> Result<Value, TrustedLanError> {
        self.call(&self.rag_url, "list_collections", json!({}))
    }

    pub(crate) fn search_rag(
        &self,
        query: &str,
        collections: &[String],
    ) -> Result<Value, TrustedLanError> {
        self.call(
            &self.rag_url,
            "search_knowledge_base",
            json!({
                "query": query,
                "collections": collections,
                "top_k": TRUSTED_LAN_RETRIEVAL_LIMIT
            }),
        )
    }

    pub(crate) fn search_memory(&self, query: &str, limit: u32) -> Result<Value, TrustedLanError> {
        self.call(
            &self.memory_url,
            "search_events",
            json!({"query": query, "limit": limit}),
        )
    }

    fn call(
        &self,
        endpoint: &TrustedLanEndpoint,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, TrustedLanError> {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "buzz-command", "version": "1"}
            }
        });
        let response = self
            .http
            .post(endpoint.as_str())
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(&initialize)
            .send()
            .map_err(|_| TrustedLanError::ServiceUnavailable)?;
        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .filter(|value| valid_session_id(value))
            .map(str::to_string);
        let initialized = read_mcp_response(response)?;
        if initialized
            .get("result")
            .and_then(|result| result.get("serverInfo"))
            .and_then(|server| server.get("name"))
            .and_then(Value::as_str)
            .is_none()
        {
            return Err(TrustedLanError::InvalidResponse);
        }

        let mut notification = self
            .http
            .post(endpoint.as_str())
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
                "params": {}
            }));
        if let Some(session_id) = &session_id {
            notification = notification.header("mcp-session-id", session_id);
        }
        let notification = notification
            .send()
            .map_err(|_| TrustedLanError::ServiceUnavailable)?;
        if notification.status().is_redirection() || !notification.status().is_success() {
            return Err(TrustedLanError::ServiceUnavailable);
        }

        let mut call = self
            .http
            .post(endpoint.as_str())
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": tool_name, "arguments": arguments}
            }));
        if let Some(session_id) = &session_id {
            call = call.header("mcp-session-id", session_id);
        }
        mcp_tool_result(&read_mcp_response(
            call.send()
                .map_err(|_| TrustedLanError::ServiceUnavailable)?,
        )?)
    }
}

fn valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'\r' | b'\n'))
}

fn read_mcp_response(response: Response) -> Result<Value, TrustedLanError> {
    if response.status().is_redirection() || !response.status().is_success() {
        return Err(TrustedLanError::ServiceUnavailable);
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAXIMUM_MCP_RESPONSE_BYTES as u64)
    {
        return Err(TrustedLanError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    response
        .take((MAXIMUM_MCP_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| TrustedLanError::InvalidResponse)?;
    if bytes.len() > MAXIMUM_MCP_RESPONSE_BYTES {
        return Err(TrustedLanError::ResponseTooLarge);
    }
    serde_json::from_slice(&bytes).map_err(|_| TrustedLanError::InvalidResponse)
}

pub(crate) fn mcp_tool_result(response: &Value) -> Result<Value, TrustedLanError> {
    let result = response
        .as_object()
        .filter(|object| {
            object.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
                && !object.contains_key("error")
        })
        .and_then(|object| object.get("result"))
        .and_then(Value::as_object)
        .filter(|result| result.get("isError").and_then(Value::as_bool) != Some(true))
        .ok_or(TrustedLanError::InvalidResponse)?;
    if let Some(value) = result
        .get("structuredContent")
        .and_then(|content| content.get("result"))
    {
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
        .ok_or(TrustedLanError::InvalidResponse)?;
    serde_json::from_str(text).map_err(|_| TrustedLanError::InvalidResponse)
}

pub(crate) fn catalogue_fingerprint(value: &Value) -> Result<String, TrustedLanError> {
    let bytes = serde_jcs::to_vec(value).map_err(|_| TrustedLanError::InvalidResponse)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Clone, Copy)]
enum CloudKind {
    LiteLlm,
    OpenAi,
}

fn validate_cloud(
    raw: RawCloudProviderConfig,
    kind: CloudKind,
) -> Result<CloudProviderConfig, TrustedLanError> {
    if !raw.enabled || !valid_identifier(&raw.model, 160) || !valid_keychain_key(&raw.keychain_key)
    {
        return Err(TrustedLanError::InvalidConfig);
    }
    let endpoint = Url::parse(&raw.endpoint).map_err(|_| TrustedLanError::InvalidEndpoint)?;
    let valid_endpoint = match kind {
        CloudKind::LiteLlm => {
            endpoint.scheme() == "http"
                && matches!(endpoint.host(), Some(Host::Ipv4(host)) if host.is_private())
                && endpoint.port().is_some()
                && endpoint.path() == "/v1/chat/completions"
        }
        CloudKind::OpenAi => {
            endpoint.scheme() == "https"
                && endpoint.host_str() == Some("api.openai.com")
                && endpoint.port().is_none()
                && endpoint.path() == "/v1/responses"
        }
    };
    if !valid_endpoint
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err(TrustedLanError::InvalidEndpoint);
    }
    Ok(CloudProviderConfig {
        enabled: raw.enabled,
        endpoint: raw.endpoint,
        model: raw.model,
        keychain_key: raw.keychain_key,
    })
}

fn valid_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/'))
}

fn valid_keychain_key(value: &str) -> bool {
    value.starts_with("command.cloud.") && valid_identifier(value, 128)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrustedLanConfig {
    schema_version: u32,
    mode: String,
    memory_url: String,
    rag_url: String,
    automatic_cloud_fallback_acknowledged: bool,
    litellm: RawCloudProviderConfig,
    openai: RawCloudProviderConfig,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCloudProviderConfig {
    enabled: bool,
    endpoint: String,
    model: String,
    keychain_key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrustedLanError {
    InvalidConfig,
    InvalidEndpoint,
    InvalidResponse,
    ResponseTooLarge,
    ServiceUnavailable,
    UnprotectedConfig,
}
