//! MCP Apps host transport and sandbox registration.
//!
//! The webview never receives MCP credentials or a general-purpose network
//! primitive. Rust owns the reviewed server connection and exposes only the
//! MCP methods required by the Apps protocol.

use std::{
    collections::HashMap,
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
const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const MAX_MCP_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_MCP_APP_HTML_BYTES: usize = 4 * 1024 * 1024;
const MAX_SERVERS: usize = 16;
const MAX_VIEWS: usize = 32;
const MAX_TOOLS: usize = 256;
const MAX_RESOURCES: usize = 256;

const SANDBOX_PROXY_HTML: &str = r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <style>html,body{width:100%;height:100%;margin:0;overflow:hidden}iframe{width:100%;height:100%;border:0;display:block}</style>
</head>
<body>
<script>
(() => {
  "use strict";
  if (window.self === window.top) throw new Error("MCP App sandbox must be framed");
  try {
    window.top.location.href;
    throw new Error("MCP App sandbox origin isolation failed");
  } catch (error) {
    if (error instanceof Error && error.message === "MCP App sandbox origin isolation failed") throw error;
  }

  const allowedHostOrigins = new Set([
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
    "http://localhost:1420",
    "http://127.0.0.1:1420"
  ]);
  if (document.referrer) {
    try { allowedHostOrigins.add(new URL(document.referrer).origin); } catch {}
  }

  const inner = document.createElement("iframe");
  inner.setAttribute("sandbox", "allow-scripts");
  document.body.appendChild(inner);
  let hostOrigin = null;

  const resourceReady = "ui/notifications/sandbox-resource-ready";
  const proxyReady = "ui/notifications/sandbox-proxy-ready";

  function permissionPolicy(permissions) {
    const values = [];
    if (permissions?.camera) values.push("camera");
    if (permissions?.microphone) values.push("microphone");
    if (permissions?.geolocation) values.push("geolocation");
    if (permissions?.clipboardWrite) values.push("clipboard-write");
    return values.join("; ");
  }

  window.addEventListener("message", (event) => {
    if (event.source === window.parent) {
      if (!allowedHostOrigins.has(event.origin)) return;
      hostOrigin = event.origin;
      if (event.data?.method === resourceReady) {
        const { html, permissions } = event.data.params ?? {};
        const allow = permissionPolicy(permissions);
        if (allow) inner.setAttribute("allow", allow);
        if (typeof html === "string") {
          inner.srcdoc = html;
        }
        return;
      }
      inner.contentWindow?.postMessage(event.data, "*");
      return;
    }
    if (event.source === inner.contentWindow && hostOrigin) {
      window.parent.postMessage(event.data, hostOrigin);
    }
  });

  window.parent.postMessage({
    jsonrpc: "2.0",
    method: proxyReady,
    params: {}
  }, "*");
})();
</script>
</body>
</html>"#;

#[derive(Debug, Clone)]
struct McpServerConnection {
    endpoint: Url,
    client: Client,
    protocol_version: String,
    session_id: Option<String>,
    next_request_id: Arc<AtomicU64>,
    tools: Vec<McpAppTool>,
    resources: Vec<McpAppResource>,
}

#[derive(Debug, Clone)]
struct ViewPolicy {
    server_id: String,
    csp: String,
}

/// Runtime state for reviewed MCP servers and isolated app views.
pub struct McpAppHostState {
    servers: AsyncMutex<HashMap<String, McpServerConnection>>,
    views: Mutex<HashMap<String, ViewPolicy>>,
}

impl Default for McpAppHostState {
    fn default() -> Self {
        Self {
            servers: AsyncMutex::new(HashMap::new()),
            views: Mutex::new(HashMap::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppTool {
    name: String,
    title: Option<String>,
    description: Option<String>,
    input_schema: Value,
    output_schema: Option<Value>,
    annotations: Option<Value>,
    meta: Value,
    ui_resource_uri: Option<String>,
    visibility: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppResource {
    uri: String,
    name: Option<String>,
    title: Option<String>,
    description: Option<String>,
    mime_type: Option<String>,
    meta: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppServerDescriptor {
    server_id: String,
    endpoint: String,
    name: String,
    version: Option<String>,
    protocol_version: String,
    tools: Vec<McpAppTool>,
    resources: Vec<McpAppResource>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppResourceCsp {
    #[serde(default)]
    connect_domains: Vec<String>,
    #[serde(default)]
    resource_domains: Vec<String>,
    #[serde(default)]
    frame_domains: Vec<String>,
    #[serde(default)]
    base_uri_domains: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppResourcePermissions {
    camera: Option<Value>,
    microphone: Option<Value>,
    geolocation: Option<Value>,
    clipboard_write: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedMcpAppView {
    view_id: String,
    sandbox_url: String,
    html: String,
    csp: McpAppResourceCsp,
    /// Permissions are reported for review but not granted by this host layer.
    requested_permissions: McpAppResourcePermissions,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAppToolCaller {
    Host,
    App,
}

#[derive(Debug)]
struct McpWireResponse {
    value: Option<Value>,
    session_id: Option<String>,
}

fn text(value: Option<&Value>) -> Option<String> {
    value?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn ui_resource_uri(meta: &Value) -> Option<String> {
    let nested = meta
        .get("ui")
        .and_then(|ui| ui.get("resourceUri"))
        .and_then(Value::as_str);
    let legacy = meta.get("ui/resourceUri").and_then(Value::as_str);
    nested
        .or(legacy)
        .filter(|uri| uri.starts_with("ui://"))
        .map(ToOwned::to_owned)
}

fn tool_visibility(meta: &Value) -> Vec<String> {
    let visibility = meta
        .get("ui")
        .and_then(|ui| ui.get("visibility"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| matches!(*value, "model" | "app"))
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if visibility.is_empty() {
        vec!["model".to_string(), "app".to_string()]
    } else {
        visibility
    }
}

fn parse_tools(value: &Value) -> Result<Vec<McpAppTool>, String> {
    value
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .ok_or_else(|| "MCP tools/list response is missing result.tools".to_string())?
        .iter()
        .take(MAX_TOOLS)
        .map(|tool| {
            let name = text(tool.get("name"))
                .ok_or_else(|| "MCP tool is missing a valid name".to_string())?;
            let meta = tool.get("_meta").cloned().unwrap_or_else(|| json!({}));
            Ok(McpAppTool {
                name,
                title: text(tool.get("title")),
                description: text(tool.get("description")),
                input_schema: tool
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                output_schema: tool.get("outputSchema").cloned(),
                annotations: tool.get("annotations").cloned(),
                ui_resource_uri: ui_resource_uri(&meta),
                visibility: tool_visibility(&meta),
                meta,
            })
        })
        .collect()
}

fn parse_resources(value: &Value) -> Result<Vec<McpAppResource>, String> {
    value
        .pointer("/result/resources")
        .and_then(Value::as_array)
        .ok_or_else(|| "MCP resources/list response is missing result.resources".to_string())?
        .iter()
        .take(MAX_RESOURCES)
        .map(|resource| {
            let uri = text(resource.get("uri"))
                .ok_or_else(|| "MCP resource is missing a valid URI".to_string())?;
            Ok(McpAppResource {
                uri,
                name: text(resource.get("name")),
                title: text(resource.get("title")),
                description: text(resource.get("description")),
                mime_type: text(resource.get("mimeType")),
                meta: resource.get("_meta").cloned().unwrap_or_else(|| json!({})),
            })
        })
        .collect()
}

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
        "https" if !loopback_host => {}
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

async fn read_capped_response(response: Response) -> Result<McpWireResponse, String> {
    let status = response.status();
    let headers = response.headers().clone();
    if !status.is_success() {
        return Err(format!("MCP server returned HTTP {status}"));
    }
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    if status == reqwest::StatusCode::ACCEPTED {
        return Ok(McpWireResponse {
            value: None,
            session_id,
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MCP_RESPONSE_BYTES as u64)
    {
        return Err("MCP response exceeds the 4 MiB limit".to_string());
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
        if bytes.len().saturating_add(chunk.len()) > MAX_MCP_RESPONSE_BYTES {
            return Err("MCP response exceeds the 4 MiB limit".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = if content_type.starts_with("text/event-stream") {
        let text = String::from_utf8(bytes)
            .map_err(|_| "MCP event stream is not valid UTF-8".to_string())?;
        text.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .find_map(|line| serde_json::from_str::<Value>(line).ok())
            .ok_or_else(|| "MCP event stream did not contain a JSON response".to_string())?
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("MCP response is not valid JSON: {error}"))?
    };
    if let Some(error) = value.get("error") {
        return Err(format!("MCP request failed: {error}"));
    }
    Ok(McpWireResponse {
        value: Some(value),
        session_id,
    })
}

async fn post_mcp(
    client: &Client,
    endpoint: &Url,
    protocol_version: Option<&str>,
    session_id: Option<&str>,
    payload: &Value,
) -> Result<McpWireResponse, String> {
    let mut request = client
        .post(endpoint.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .json(payload);
    if let Some(protocol_version) = protocol_version {
        request = request.header("mcp-protocol-version", protocol_version);
    }
    if let Some(session_id) = session_id {
        request = request.header("mcp-session-id", session_id);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("MCP request failed: {error}"))?;
    read_capped_response(response).await
}

async fn request(
    connection: &McpServerConnection,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let id = connection.next_request_id.fetch_add(1, Ordering::Relaxed);
    let response = post_mcp(
        &connection.client,
        &connection.endpoint,
        Some(&connection.protocol_version),
        connection.session_id.as_deref(),
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }),
    )
    .await?;
    response
        .value
        .ok_or_else(|| format!("MCP {method} returned no response"))
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
    let csp = serde_json::from_value(ui.get("csp").cloned().unwrap_or_else(|| json!({})))
        .map_err(|error| format!("MCP App CSP metadata is invalid: {error}"))?;
    let permissions =
        serde_json::from_value(ui.get("permissions").cloned().unwrap_or_else(|| json!({})))
            .map_err(|error| format!("MCP App permission metadata is invalid: {error}"))?;
    Ok((html, csp, permissions))
}

fn csp_origin(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if let Some(suffix) = raw.strip_prefix("https://*.") {
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
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
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
    let bases = sources(&csp.base_uri_domains, "'none'");
    format!(
        "default-src 'none'; script-src 'self' 'unsafe-inline' {resources}; \
         style-src 'self' 'unsafe-inline' {resources}; img-src 'self' data: blob: {resources}; \
         font-src 'self' data: {resources}; media-src 'self' data: blob: {resources}; \
         connect-src {connects}; frame-src 'self' {frames}; base-uri {bases}; \
         object-src 'none'; form-action 'none'"
    )
}

fn app_tool_allowed(tool: &McpAppTool, caller: McpAppToolCaller) -> bool {
    match caller {
        McpAppToolCaller::Host => tool.visibility.iter().any(|value| value == "model"),
        McpAppToolCaller::App => tool.visibility.iter().any(|value| value == "app"),
    }
}

/// Connect to a reviewed Streamable HTTP MCP server and discover its Apps.
#[tauri::command]
pub async fn connect_mcp_app_server(
    endpoint: String,
    state: State<'_, McpAppHostState>,
) -> Result<McpAppServerDescriptor, String> {
    let endpoint = validate_mcp_endpoint(&endpoint)?;
    let client = build_pinned_client(&endpoint).await?;
    let initialize = post_mcp(
        &client,
        &endpoint,
        None,
        None,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "extensions": {
                        "io.modelcontextprotocol/ui": {
                            "mimeTypes": [MCP_APP_MIME_TYPE]
                        }
                    }
                },
                "clientInfo": {
                    "name": "Buzz Desktop",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
    )
    .await?;
    let value = initialize
        .value
        .ok_or_else(|| "MCP initialize returned no response".to_string())?;
    let protocol_version = text(value.pointer("/result/protocolVersion"))
        .ok_or_else(|| "MCP initialize response is missing protocolVersion".to_string())?;
    let server_name =
        text(value.pointer("/result/serverInfo/name")).unwrap_or_else(|| endpoint.to_string());
    let server_version = text(value.pointer("/result/serverInfo/version"));
    let connection = McpServerConnection {
        endpoint: endpoint.clone(),
        client,
        protocol_version: protocol_version.clone(),
        session_id: initialize.session_id,
        next_request_id: Arc::new(AtomicU64::new(2)),
        tools: Vec::new(),
        resources: Vec::new(),
    };
    let _ = post_mcp(
        &connection.client,
        &connection.endpoint,
        Some(&connection.protocol_version),
        connection.session_id.as_deref(),
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )
    .await?;
    let tools = parse_tools(&request(&connection, "tools/list", json!({})).await?)?;
    let resources = request(&connection, "resources/list", json!({}))
        .await
        .and_then(|value| parse_resources(&value))
        .unwrap_or_default();
    let server_id = Uuid::new_v4().to_string();
    let connection = McpServerConnection {
        tools: tools.clone(),
        resources: resources.clone(),
        ..connection
    };
    let mut servers = state.servers.lock().await;
    if servers.len() >= MAX_SERVERS {
        return Err("Too many MCP App servers are connected".to_string());
    }
    servers.insert(server_id.clone(), connection);
    Ok(McpAppServerDescriptor {
        server_id,
        endpoint: endpoint.to_string(),
        name: server_name,
        version: server_version,
        protocol_version,
        tools,
        resources,
    })
}

/// List the reviewed tools for one connected MCP server.
#[tauri::command]
pub async fn list_mcp_app_tools(
    server_id: String,
    state: State<'_, McpAppHostState>,
) -> Result<Vec<McpAppTool>, String> {
    state
        .servers
        .lock()
        .await
        .get(&server_id)
        .map(|connection| connection.tools.clone())
        .ok_or_else(|| "MCP App server is not connected".to_string())
}

/// List the reviewed resources for one connected MCP server.
#[tauri::command]
pub async fn list_mcp_app_resources(
    server_id: String,
    state: State<'_, McpAppHostState>,
) -> Result<Vec<McpAppResource>, String> {
    state
        .servers
        .lock()
        .await
        .get(&server_id)
        .map(|connection| connection.resources.clone())
        .ok_or_else(|| "MCP App server is not connected".to_string())
}

/// Execute a reviewed MCP tool for the host or the isolated App.
#[tauri::command]
pub async fn call_mcp_app_tool(
    server_id: String,
    name: String,
    arguments: Value,
    caller: McpAppToolCaller,
    state: State<'_, McpAppHostState>,
) -> Result<Value, String> {
    let connection = state
        .servers
        .lock()
        .await
        .get(&server_id)
        .cloned()
        .ok_or_else(|| "MCP App server is not connected".to_string())?;
    let tool = connection
        .tools
        .iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| "MCP App requested an unknown tool".to_string())?;
    if !app_tool_allowed(tool, caller) {
        return Err("MCP App tool is not visible to this caller".to_string());
    }
    request(
        &connection,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
    .await
    .and_then(|value| {
        value
            .get("result")
            .cloned()
            .ok_or_else(|| "MCP tools/call response is missing result".to_string())
    })
}

/// Read a resource for an initialized AppBridge request.
#[tauri::command]
pub async fn read_mcp_app_resource(
    server_id: String,
    uri: String,
    state: State<'_, McpAppHostState>,
) -> Result<Value, String> {
    let connection = state
        .servers
        .lock()
        .await
        .get(&server_id)
        .cloned()
        .ok_or_else(|| "MCP App server is not connected".to_string())?;
    if !connection
        .resources
        .iter()
        .any(|resource| resource.uri == uri)
    {
        return Err("MCP App requested an undiscovered resource".to_string());
    }
    request(&connection, "resources/read", json!({"uri": uri}))
        .await
        .and_then(|value| {
            value
                .get("result")
                .cloned()
                .ok_or_else(|| "MCP resources/read response is missing result".to_string())
        })
}

/// Read and validate one UI resource, then register its CSP-bound sandbox URL.
#[tauri::command]
pub async fn prepare_mcp_app_view(
    server_id: String,
    uri: String,
    state: State<'_, McpAppHostState>,
) -> Result<PreparedMcpAppView, String> {
    let connection = state
        .servers
        .lock()
        .await
        .get(&server_id)
        .cloned()
        .ok_or_else(|| "MCP App server is not connected".to_string())?;
    if !connection
        .tools
        .iter()
        .any(|tool| tool.ui_resource_uri.as_deref() == Some(uri.as_str()))
    {
        return Err("MCP App resource is not declared by a reviewed tool".to_string());
    }
    let response = request(&connection, "resources/read", json!({"uri": uri})).await?;
    let listing = connection
        .resources
        .iter()
        .find(|resource| resource.uri == uri);
    let (html, csp, requested_permissions) = parse_ui_resource(&response, &uri, listing)?;
    let view_id = Uuid::new_v4().to_string();
    let mut views = state
        .views
        .lock()
        .map_err(|_| "MCP App view registry is unavailable".to_string())?;
    if views.len() >= MAX_VIEWS {
        return Err("Too many MCP App views are open".to_string());
    }
    views.insert(
        view_id.clone(),
        ViewPolicy {
            server_id,
            csp: sandbox_csp(&csp),
        },
    );
    Ok(PreparedMcpAppView {
        sandbox_url: format!("buzz-mcp-app://localhost/{view_id}"),
        view_id,
        html,
        csp,
        requested_permissions,
    })
}

/// Release an isolated MCP App view and its CSP policy.
#[tauri::command]
pub fn release_mcp_app_view(
    view_id: String,
    state: State<'_, McpAppHostState>,
) -> Result<(), String> {
    state
        .views
        .lock()
        .map_err(|_| "MCP App view registry is unavailable".to_string())?
        .remove(&view_id);
    Ok(())
}

/// Close an MCP server connection and release all views created from it.
#[tauri::command]
pub async fn disconnect_mcp_app_server(
    server_id: String,
    state: State<'_, McpAppHostState>,
) -> Result<(), String> {
    let connection = state.servers.lock().await.remove(&server_id);
    if let Some((connection, session_id)) =
        connection.and_then(|connection| connection.session_id.clone().map(|id| (connection, id)))
    {
        let _ = connection
            .client
            .delete(connection.endpoint)
            .header("mcp-protocol-version", connection.protocol_version)
            .header("mcp-session-id", session_id)
            .send()
            .await;
    }
    state
        .views
        .lock()
        .map_err(|_| "MCP App view registry is unavailable".to_string())?
        .retain(|_, view| view.server_id != server_id);
    Ok(())
}

fn html_response(status: u16, body: &str, csp: Option<&str>) -> http::Response<Vec<u8>> {
    let mut builder = http::Response::builder()
        .status(status)
        .header("content-type", "text/html; charset=utf-8")
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff")
        .header(
            "permissions-policy",
            "camera=(), microphone=(), geolocation=(), clipboard-write=()",
        );
    if let Some(csp) = csp {
        builder = builder.header("content-security-policy", csp);
    }
    builder
        .body(body.as_bytes().to_vec())
        .unwrap_or_else(|_| http::Response::new(Vec::new()))
}

/// Serve the trusted outer sandbox proxy from a Tauri-owned isolated origin.
pub fn handle_mcp_app_protocol(
    app: &AppHandle,
    request: &http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let view_id = request.uri().path().trim_matches('/');
    if Uuid::parse_str(view_id).is_err() {
        return html_response(404, "not found", None);
    }
    let state = app.state::<McpAppHostState>();
    let views = match state.views.lock() {
        Ok(views) => views,
        Err(_) => return html_response(503, "unavailable", None),
    };
    let Some(view) = views.get(view_id) else {
        return html_response(404, "not found", None);
    };
    html_response(200, SANDBOX_PROXY_HTML, Some(&view.csp))
}

#[cfg(test)]
#[path = "mcp_apps_tests.rs"]
mod tests;
