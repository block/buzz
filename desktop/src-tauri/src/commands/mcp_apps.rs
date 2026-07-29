//! MCP Apps host transport and sandbox registration.
//!
//! The webview never receives MCP credentials or a general-purpose network
//! primitive. Rust owns the reviewed server connection and exposes only the
//! MCP methods required by the Apps protocol.

use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use base64::Engine;
use futures_util::StreamExt;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{http, AppHandle, Manager, State};
use tokio::sync::Mutex as AsyncMutex;
use url::Url;
use uuid::Uuid;

const MCP_APP_MIME_TYPE: &str = "text/html;profile=mcp-app";
/// Legacy MCP revision: `initialize` handshake plus `mcp-session-id` sessions.
const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
/// Modern MCP revision: sessionless, handshake-free, header-routed requests.
const MCP_MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
/// Error codes introduced by the modern (2026-07-28) revision. Used only to
/// classify a server's era. Deliberately excludes the generic JSON-RPC
/// `-32602` (Invalid params), which a legacy server may also return and which
/// would otherwise misclassify it as modern and skip the handshake fallback.
const MODERN_MCP_ERROR_CODES: [i64; 3] = [-32020, -32021, -32022];
const MAX_MCP_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_MCP_ERROR_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_MCP_ERROR_MESSAGE_CHARS: usize = 256;
const MAX_MCP_APP_HTML_BYTES: usize = 4 * 1024 * 1024;
const MAX_SERVERS: usize = 16;
const MAX_VIEWS: usize = 32;
const MAX_TOOLS: usize = 256;
const MAX_RESOURCES: usize = 256;

const SANDBOX_PROXY_HTML: &str = include_str!("mcp_apps_sandbox_proxy.html");

#[path = "mcp_apps_model.rs"]
mod model;
use model::*;
pub use model::{
    McpAppHostState, McpAppResource, McpAppResourceCsp, McpAppResourcePermissions,
    McpAppServerDescriptor, McpAppTool, McpAppToolCaller, PreparedMcpAppView,
};

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
}

fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_private_ipv4(mapped);
    }
    let segments = ip.segments();
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || (segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2..6] == [0, 0, 0, 0])
        || segments[0] == 0x2002
        || (segments[0] == 0x2001 && segments[1] == 0)
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_private_ipv4(ip),
        IpAddr::V6(ip) => is_private_ipv6(ip),
    }
}

fn validate_mcp_endpoint(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|error| format!("invalid MCP server URL: {error}"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("MCP server URL must not include credentials".to_string());
    }
    if url.fragment().is_some() {
        return Err("MCP server URL must not include a fragment".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "MCP server URL is missing a host".to_string())?;
    let loopback_host = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    match url.scheme() {
        "https" => {}
        "http" if loopback_host => {}
        _ => {
            return Err(
                "MCP servers require HTTPS; HTTP is allowed only for loopback development"
                    .to_string(),
            )
        }
    }
    if host.ends_with(".local") || host.contains('%') {
        return Err("MCP server host is not allowed".to_string());
    }
    Ok(url)
}

async fn build_pinned_client(endpoint: &Url) -> Result<Client, String> {
    let host = endpoint
        .host_str()
        .ok_or_else(|| "MCP server URL is missing a host".to_string())?;
    let port = endpoint
        .port_or_known_default()
        .ok_or_else(|| "MCP server URL is missing a port".to_string())?;
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("failed to resolve MCP server: {error}"))?
        .collect::<Vec<SocketAddr>>();
    if addresses.is_empty() {
        return Err("MCP server did not resolve to an address".to_string());
    }
    if loopback {
        if addresses.iter().any(|address| !address.ip().is_loopback()) {
            return Err("loopback MCP server resolved outside loopback".to_string());
        }
    } else if addresses.iter().any(|address| is_private_ip(address.ip())) {
        return Err("MCP server resolved to a private or reserved address".to_string());
    }
    let mut builder = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(2);
    if host.parse::<IpAddr>().is_err() {
        builder = builder.resolve_to_addrs(host, &addresses);
    }
    builder
        .build()
        .map_err(|error| format!("failed to build MCP HTTP client: {error}"))
}

async fn read_capped_reply(response: Response) -> Result<McpHttpReply, String> {
    let status = response.status();
    let max_bytes = if status.is_success() {
        MAX_MCP_RESPONSE_BYTES
    } else {
        MAX_MCP_ERROR_RESPONSE_BYTES
    };
    let limit_message = if status.is_success() {
        "MCP response exceeds the 4 MiB limit"
    } else {
        "MCP error response exceeds the 64 KiB limit"
    };
    let headers = response.headers().clone();
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    if status == reqwest::StatusCode::ACCEPTED {
        return Ok(McpHttpReply {
            status,
            value: None,
            session_id,
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(limit_message.to_string());
    }
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("failed to read MCP response: {error}"))?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(limit_message.to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = if content_type.starts_with("text/event-stream") {
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) if status.is_success() => {
                return Err("MCP event stream is not valid UTF-8".to_string())
            }
            Err(_) => String::new(),
        };
        let parsed = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .find_map(|line| serde_json::from_str::<Value>(line).ok());
        if parsed.is_none() && status.is_success() {
            return Err("MCP event stream did not contain a JSON response".to_string());
        }
        parsed
    } else {
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => Some(value),
            Err(error) if status.is_success() => {
                return Err(format!("MCP response is not valid JSON: {error}"))
            }
            Err(_) => None,
        }
    };
    Ok(McpHttpReply {
        status,
        value,
        session_id,
    })
}

fn format_rpc_error(error: &Value) -> String {
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("The server returned an MCP error.");
    let mut truncated = message
        .chars()
        .take(MAX_MCP_ERROR_MESSAGE_CHARS)
        .collect::<String>();
    if message.chars().count() > MAX_MCP_ERROR_MESSAGE_CHARS {
        truncated.push('…');
    }
    format!("code {code}: {truncated}")
}

/// Convert a raw reply into the success-only wire response, preserving the
/// pre-dual-era error messages.
fn wire_response(reply: McpHttpReply) -> Result<McpWireResponse, String> {
    if !reply.status.is_success() {
        return Err(format!("MCP server returned HTTP {}", reply.status));
    }
    if let Some(error) = reply.value.as_ref().and_then(|value| value.get("error")) {
        return Err(format!("MCP request failed: {}", format_rpc_error(error)));
    }
    Ok(McpWireResponse {
        value: reply.value,
        session_id: reply.session_id,
    })
}

/// The modern `_meta` block every modern-era request body must carry:
/// protocol version (must match the `MCP-Protocol-Version` header), client
/// capabilities with the Apps UI extension, and client info.
fn modern_meta(protocol_version: &str) -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": protocol_version,
        "io.modelcontextprotocol/clientCapabilities": {
            "extensions": {
                "io.modelcontextprotocol/ui": {
                    "mimeTypes": [MCP_APP_MIME_TYPE]
                }
            }
        },
        "io.modelcontextprotocol/clientInfo": {
            "name": "Buzz Desktop",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

/// Inject the modern `_meta` block into a request's params, preserving any
/// caller-provided `_meta` entries that do not collide with the required keys.
fn inject_modern_meta(params: Value, protocol_version: &str) -> Value {
    let meta = modern_meta(protocol_version);
    let Value::Object(mut map) = params else {
        return json!({ "_meta": meta });
    };
    let mut merged = match map.remove("_meta") {
        Some(Value::Object(existing)) => existing,
        _ => serde_json::Map::new(),
    };
    if let Value::Object(entries) = meta {
        for (key, value) in entries {
            merged.insert(key, value);
        }
    }
    map.insert("_meta".to_string(), Value::Object(merged));
    Value::Object(map)
}

/// Prepare request params for one era: modern requests carry the required
/// `_meta` block, legacy requests pass through untouched.
fn prepare_params(era: McpEra, params: Value, protocol_version: &str) -> Value {
    match era {
        McpEra::Modern => inject_modern_meta(params, protocol_version),
        McpEra::Legacy => params,
    }
}

/// The `Mcp-Name` value the modern revision requires for name-addressed
/// methods; `None` for every other method.
fn modern_mcp_name(method: &str, params: &Value) -> Option<String> {
    match method {
        "tools/call" | "prompts/get" => text(params.get("name")),
        "resources/read" => text(params.get("uri")),
        _ => None,
    }
}

/// Encode an `Mcp-Name` header value. Plain visible-ASCII values pass through;
/// anything else (non-ASCII, whitespace, or a value that could be mistaken for
/// the sentinel itself) uses the spec's `=?base64?{value}?=` sentinel form.
fn mcp_name_header_value(raw: &str) -> String {
    let plain = !raw.is_empty()
        && !raw.starts_with("=?")
        && raw.bytes().all(|byte| (0x21..=0x7e).contains(&byte));
    if plain {
        raw.to_string()
    } else {
        format!(
            "=?base64?{}?=",
            base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
        )
    }
}

fn mcp_param_header_value(value: &Value) -> Result<Option<String>, String> {
    let raw = match value {
        Value::Null => return Ok(None),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => {
            const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
            let integer = value
                .as_i64()
                .ok_or_else(|| "x-mcp-header parameter must be an integer".to_string())?;
            if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&integer) {
                return Err(
                    "x-mcp-header integer exceeds the JavaScript safe integer range".to_string(),
                );
            }
            integer.to_string()
        }
        _ => {
            return Err(
                "x-mcp-header parameter must be a string, integer, boolean, or null".to_string(),
            )
        }
    };
    let sentinel = raw.starts_with("=?base64?") && raw.ends_with("?=");
    let edge_whitespace = raw
        .as_bytes()
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        || raw
            .as_bytes()
            .last()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'));
    let plain = !sentinel
        && !edge_whitespace
        && raw
            .bytes()
            .all(|byte| (0x20..=0x7e).contains(&byte) && byte != b'\t');
    if plain {
        Ok(Some(raw))
    } else {
        Ok(Some(format!(
            "=?base64?{}?=",
            base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
        )))
    }
}

fn nested_value<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter()
        .try_fold(root, |value, key| value.as_object()?.get(key))
}

fn tool_param_headers(
    tools: &[McpAppTool],
    method: &str,
    params: &Value,
) -> Result<Vec<(String, String)>, String> {
    if method != "tools/call" {
        return Ok(Vec::new());
    }
    let Some(tool_name) = params.get("name").and_then(Value::as_str) else {
        return Ok(Vec::new());
    };
    let Some(tool) = tools.iter().find(|tool| tool.name == tool_name) else {
        return Ok(Vec::new());
    };
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    tool.param_headers
        .iter()
        .filter_map(|header| {
            nested_value(arguments, &header.path).map(|value| {
                mcp_param_header_value(value).map(|encoded| {
                    encoded.map(|value| (format!("Mcp-Param-{}", header.name), value))
                })
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|values| values.into_iter().flatten().collect())
}

/// Build the per-request MCP headers for one era.
///
/// Legacy requests carry the negotiated `mcp-protocol-version` plus the
/// server-issued `mcp-session-id`, exactly as before dual-era support. Modern
/// requests instead carry `MCP-Protocol-Version`, `Mcp-Method`, and — for
/// `tools/call`, `resources/read`, and `prompts/get` — `Mcp-Name`; a session
/// header is never sent in the modern era.
fn build_mcp_headers(
    era: McpEra,
    protocol_version: Option<&str>,
    session_id: Option<&str>,
    payload: &Value,
    param_headers: &[(String, String)],
) -> Vec<(String, String)> {
    match era {
        McpEra::Legacy => {
            let mut headers = Vec::new();
            if let Some(protocol_version) = protocol_version {
                headers.push((
                    "mcp-protocol-version".to_string(),
                    protocol_version.to_string(),
                ));
            }
            if let Some(session_id) = session_id {
                headers.push(("mcp-session-id".to_string(), session_id.to_string()));
            }
            headers
        }
        McpEra::Modern => {
            let mut headers = vec![(
                "MCP-Protocol-Version".to_string(),
                protocol_version
                    .unwrap_or(MCP_MODERN_PROTOCOL_VERSION)
                    .to_string(),
            )];
            if let Some(method) = payload.get("method").and_then(Value::as_str) {
                headers.push(("Mcp-Method".to_string(), method.to_string()));
                if let Some(name) = payload
                    .get("params")
                    .and_then(|params| modern_mcp_name(method, params))
                {
                    headers.push(("Mcp-Name".to_string(), mcp_name_header_value(&name)));
                }
            }
            headers.extend(param_headers.iter().cloned());
            headers
        }
    }
}

/// True when a JSON-RPC error means resource-not-found. The modern revision
/// moved this code from `-32002` to `-32602`; both are accepted.
fn is_resource_not_found(error: &Value) -> bool {
    matches!(
        error.get("code").and_then(Value::as_i64),
        Some(-32002 | -32602)
    )
}

/// Completion state of a JSON-RPC result. The modern revision may attach
/// `resultType`; an absent value MUST be read as `"complete"`.
fn result_completion(result: &Value) -> &str {
    result
        .get("resultType")
        .and_then(Value::as_str)
        .unwrap_or("complete")
}

/// Extract the JSON-RPC `result`, tolerating the modern advisory fields
/// (`resultType`, `ttlMs`, `cacheScope`) rather than failing on them.
fn extract_result(response: &Value, method: &str) -> Result<Value, String> {
    let result = response
        .get("result")
        .ok_or_else(|| format!("MCP {method} response is missing result"))?;
    if result_completion(result).is_empty() {
        return Err(format!("MCP {method} result declared an empty resultType"));
    }
    Ok(result.clone())
}

fn recognized_modern_error(body: Option<&Value>) -> Option<&Value> {
    let error = body?.get("error")?;
    let code = error.get("code").and_then(Value::as_i64)?;
    MODERN_MCP_ERROR_CODES.contains(&code).then_some(error)
}

fn advertised_supported_versions(error: &Value) -> Vec<String> {
    error
        .pointer("/data/supported")
        .and_then(Value::as_array)
        .map(|versions| {
            versions
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Classification of the modern-first era probe.
#[derive(Debug, PartialEq, Eq)]
enum ProbeOutcome {
    /// The server answered the modern request.
    Modern,
    /// The server is modern but rejected our protocol version (`-32022`);
    /// retry with a mutually supported version from this list.
    ModernRetry { supported: Vec<String> },
    /// The server is modern and rejected the request for a non-version reason.
    ModernError { message: String },
    /// The server does not speak the modern revision; use the legacy handshake.
    Legacy,
}

/// Classify a modern-probe response per the spec's Backward Compatibility
/// rule: a recognized modern JSON-RPC error means the server speaks modern
/// (retry or correct, never fall back); an empty or unrecognized `400` body
/// and HTTP `404`/`405` mean the legacy handshake is required.
fn classify_probe(status: reqwest::StatusCode, body: Option<&Value>) -> ProbeOutcome {
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
    {
        return ProbeOutcome::Legacy;
    }
    if status.is_success() && body.is_some_and(|value| value.get("result").is_some()) {
        return ProbeOutcome::Modern;
    }
    if let Some(error) = recognized_modern_error(body) {
        if error.get("code").and_then(Value::as_i64) == Some(-32022) {
            return ProbeOutcome::ModernRetry {
                supported: advertised_supported_versions(error),
            };
        }
        return ProbeOutcome::ModernError {
            message: format!(
                "MCP server rejected the modern request: {}",
                format_rpc_error(error)
            ),
        };
    }
    ProbeOutcome::Legacy
}

/// POST one JSON-RPC payload with era-appropriate headers, preserving the
/// HTTP status and leniently parsed body for era-probe inspection.
async fn post_mcp_raw(
    client: &Client,
    endpoint: &Url,
    era: McpEra,
    protocol_version: Option<&str>,
    session_id: Option<&str>,
    payload: &Value,
    param_headers: &[(String, String)],
) -> Result<McpHttpReply, String> {
    let mut request = client
        .post(endpoint.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .json(payload);
    for (name, value) in
        build_mcp_headers(era, protocol_version, session_id, payload, param_headers)
    {
        request = request.header(name.as_str(), value.as_str());
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("MCP request failed: {error}"))?;
    read_capped_reply(response).await
}

/// POST one JSON-RPC payload and require a successful, error-free response.
async fn post_mcp(
    client: &Client,
    endpoint: &Url,
    era: McpEra,
    protocol_version: Option<&str>,
    session_id: Option<&str>,
    payload: &Value,
) -> Result<McpWireResponse, String> {
    wire_response(
        post_mcp_raw(
            client,
            endpoint,
            era,
            protocol_version,
            session_id,
            payload,
            &[],
        )
        .await?,
    )
}

async fn request(
    connection: &McpServerConnection,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let id = connection.next_request_id.fetch_add(1, Ordering::Relaxed);
    let param_headers = if connection.era == McpEra::Modern {
        tool_param_headers(&connection.tools, method, &params)?
    } else {
        Vec::new()
    };
    let params = prepare_params(connection.era, params, &connection.protocol_version);
    let reply = post_mcp_raw(
        &connection.client,
        &connection.endpoint,
        connection.era,
        Some(&connection.protocol_version),
        connection.session_id.as_deref(),
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }),
        &param_headers,
    )
    .await?;
    if !reply.status.is_success() {
        return Err(format!("MCP server returned HTTP {}", reply.status));
    }
    if let Some(error) = reply.value.as_ref().and_then(|value| value.get("error")) {
        if method == "resources/read" && is_resource_not_found(error) {
            return Err(format!(
                "MCP resource not found: {}",
                format_rpc_error(error)
            ));
        }
        return Err(format!("MCP request failed: {}", format_rpc_error(error)));
    }
    reply
        .value
        .ok_or_else(|| format!("MCP {method} returned no response"))
}

/// Result of the modern-first probe against a new origin.
enum ModernProbe {
    /// The origin speaks the modern revision; carries the probe's `tools/list`
    /// JSON-RPC response and the negotiated protocol version.
    Modern {
        response: Value,
        protocol_version: String,
    },
    /// The origin requires the legacy `initialize` handshake.
    Legacy,
}

async fn modern_probe_once(
    client: &Client,
    endpoint: &Url,
    protocol_version: &str,
    id: u64,
) -> Result<McpHttpReply, String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": prepare_params(McpEra::Modern, json!({}), protocol_version),
    });
    post_mcp_raw(
        client,
        endpoint,
        McpEra::Modern,
        Some(protocol_version),
        None,
        &payload,
        &[],
    )
    .await
}

/// Probe an origin with a modern `tools/list` request and decide its era. On
/// `-32022` the probe retries once with a mutually supported version from the
/// server's advertised `error.data.supported` list, and falls back to the
/// legacy handshake when only the legacy version is mutually supported.
async fn probe_modern(client: &Client, endpoint: &Url) -> Result<ModernProbe, String> {
    let reply = modern_probe_once(client, endpoint, MCP_MODERN_PROTOCOL_VERSION, 1).await?;
    match classify_probe(reply.status, reply.value.as_ref()) {
        ProbeOutcome::Modern => Ok(ModernProbe::Modern {
            response: reply
                .value
                .ok_or_else(|| "MCP modern probe returned no response".to_string())?,
            protocol_version: MCP_MODERN_PROTOCOL_VERSION.to_string(),
        }),
        ProbeOutcome::Legacy => Ok(ModernProbe::Legacy),
        ProbeOutcome::ModernError { message } => Err(message),
        ProbeOutcome::ModernRetry { supported } => {
            if supported
                .iter()
                .any(|version| version == MCP_MODERN_PROTOCOL_VERSION)
            {
                let retry =
                    modern_probe_once(client, endpoint, MCP_MODERN_PROTOCOL_VERSION, 2).await?;
                match classify_probe(retry.status, retry.value.as_ref()) {
                    ProbeOutcome::Modern => Ok(ModernProbe::Modern {
                        response: retry
                            .value
                            .ok_or_else(|| "MCP modern probe returned no response".to_string())?,
                        protocol_version: MCP_MODERN_PROTOCOL_VERSION.to_string(),
                    }),
                    _ => Err("MCP server rejected the retried modern protocol version".to_string()),
                }
            } else if supported
                .iter()
                .any(|version| version == MCP_PROTOCOL_VERSION)
            {
                Ok(ModernProbe::Legacy)
            } else if supported.is_empty() {
                Err(
                    "MCP server rejected the protocol version without advertising alternatives"
                        .to_string(),
                )
            } else {
                Err(format!(
                    "MCP server supports no mutual protocol version (offered: {})",
                    supported.join(", ")
                ))
            }
        }
    }
}

fn resource_meta(content: &Value, listing: Option<&McpAppResource>) -> Value {
    content
        .get("_meta")
        .or_else(|| content.get("meta"))
        .cloned()
        .or_else(|| listing.map(|resource| resource.meta.clone()))
        .unwrap_or_else(|| json!({}))
}

fn parse_ui_resource(
    response: &Value,
    requested_uri: &str,
    listing: Option<&McpAppResource>,
) -> Result<(String, McpAppResourceCsp, McpAppResourcePermissions), String> {
    let contents = response
        .pointer("/result/contents")
        .and_then(Value::as_array)
        .ok_or_else(|| "MCP resources/read response is missing result.contents".to_string())?;
    if contents.len() != 1 {
        return Err("MCP App resource must contain exactly one document".to_string());
    }
    let content = &contents[0];
    if text(content.get("uri")).as_deref() != Some(requested_uri) {
        return Err("MCP App resource URI does not match the request".to_string());
    }
    if text(content.get("mimeType")).as_deref() != Some(MCP_APP_MIME_TYPE) {
        return Err(format!("MCP App resource must use {MCP_APP_MIME_TYPE}"));
    }
    let html = if let Some(text) = content.get("text").and_then(Value::as_str) {
        text.to_string()
    } else if let Some(blob) = content.get("blob").and_then(Value::as_str) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(blob)
            .map_err(|_| "MCP App resource blob is not valid base64".to_string())?;
        String::from_utf8(bytes)
            .map_err(|_| "MCP App resource blob is not valid UTF-8".to_string())?
    } else {
        return Err("MCP App resource has no text or blob content".to_string());
    };
    if html.len() > MAX_MCP_APP_HTML_BYTES {
        return Err("MCP App HTML exceeds the 4 MiB limit".to_string());
    }
    let meta = resource_meta(content, listing);
    let ui = meta.get("ui").cloned().unwrap_or_else(|| json!({}));
    let csp = sanitize_csp(
        serde_json::from_value(ui.get("csp").cloned().unwrap_or_else(|| json!({})))
            .map_err(|error| format!("MCP App CSP metadata is invalid: {error}"))?,
    );
    let permissions =
        serde_json::from_value(ui.get("permissions").cloned().unwrap_or_else(|| json!({})))
            .map_err(|error| format!("MCP App permission metadata is invalid: {error}"))?;
    Ok((html, csp, permissions))
}

fn csp_origin(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let wildcard_suffix = raw
        .strip_prefix("https://*.")
        .or_else(|| raw.strip_prefix("wss://*."));
    if let Some(suffix) = wildcard_suffix {
        if !suffix.is_empty()
            && !suffix.contains('/')
            && suffix
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, '.' | '-'))
        {
            return Some(raw.to_string());
        }
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return None;
    }
    let host = url.host_str()?;
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if !(matches!(url.scheme(), "https" | "wss")
        || matches!(url.scheme(), "http" | "ws") && loopback)
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn sanitize_csp(csp: McpAppResourceCsp) -> McpAppResourceCsp {
    fn sanitize(values: Vec<String>) -> Vec<String> {
        values
            .into_iter()
            .filter_map(|value| csp_origin(&value))
            .collect()
    }
    McpAppResourceCsp {
        connect_domains: sanitize(csp.connect_domains),
        resource_domains: sanitize(csp.resource_domains),
        frame_domains: sanitize(csp.frame_domains),
        base_uri_domains: sanitize(csp.base_uri_domains),
    }
}

fn sources(values: &[String], fallback: &str) -> String {
    let values = values.iter().filter_map(|value| csp_origin(value));
    let collected = values.collect::<Vec<_>>();
    if collected.is_empty() {
        fallback.to_string()
    } else {
        collected.join(" ")
    }
}

fn sandbox_csp(csp: &McpAppResourceCsp) -> String {
    let resources = sources(&csp.resource_domains, "");
    let connects = sources(&csp.connect_domains, "'none'");
    let frames = sources(&csp.frame_domains, "");
    let bases = sources(&csp.base_uri_domains, "'self'");
    format!(
        "default-src 'none'; script-src 'self' 'unsafe-inline' {resources}; \
         style-src 'self' 'unsafe-inline' {resources}; img-src 'self' data: blob: {resources}; \
         font-src 'self' data: {resources}; media-src 'self' data: blob: {resources}; \
         connect-src {connects}; frame-src 'self' {frames}; base-uri {bases}; \
         object-src 'none'; form-action 'none'"
    )
}

#[path = "mcp_apps_host.rs"]
mod host;
#[cfg(test)]
use host::{app_tool_allowed, sandbox_proxy_html};
pub use host::{
    call_mcp_app_tool, connect_mcp_app_server, disconnect_mcp_app_server, handle_mcp_app_protocol,
    list_mcp_app_resources, list_mcp_app_tools, prepare_mcp_app_view, read_mcp_app_resource,
    release_mcp_app_view,
};

#[cfg(test)]
#[path = "mcp_apps_tests.rs"]
mod tests;
