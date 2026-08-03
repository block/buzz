use super::*;
use std::collections::HashSet;

/// Which MCP revision an origin speaks. Detected once by the modern-first
/// probe in [`connect_mcp_app_server`] and cached on the connection: the era
/// is a property of the origin, not of individual requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum McpEra {
    /// Revision `2026-07-28`: no handshake, no sessions, per-request headers.
    Modern,
    /// Revision `2025-11-25`: `initialize` handshake and `mcp-session-id`.
    Legacy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct McpParamHeader {
    pub(super) path: Vec<String>,
    pub(super) name: String,
}

#[derive(Debug, Clone)]
pub(super) struct McpServerConnection {
    pub(super) endpoint: Url,
    pub(super) client: Client,
    pub(super) era: McpEra,
    pub(super) protocol_version: String,
    pub(super) session_id: Option<String>,
    pub(super) next_request_id: Arc<AtomicU64>,
    pub(super) tools: Vec<McpAppTool>,
    pub(super) resources: Vec<McpAppResource>,
}

#[derive(Debug, Clone)]
pub(super) struct ViewPolicy {
    pub(super) server_id: String,
    pub(super) csp: String,
}

/// Runtime state for reviewed MCP servers and isolated app views.
pub struct McpAppHostState {
    pub(super) servers: AsyncMutex<HashMap<String, McpServerConnection>>,
    pub(super) views: Mutex<HashMap<String, ViewPolicy>>,
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
    pub(super) name: String,
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    pub(super) input_schema: Value,
    pub(super) output_schema: Option<Value>,
    pub(super) annotations: Option<Value>,
    pub(super) meta: Value,
    pub(super) ui_resource_uri: Option<String>,
    pub(super) visibility: Vec<String>,
    #[serde(skip)]
    pub(super) param_headers: Vec<McpParamHeader>,
}

/// Host-authored references for one channel app tool call.
///
/// These values identify the local Buzz binding only. They are context, not
/// authorization, and are written after any caller-supplied metadata merge.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppInvocationContext {
    pub(super) community_ref: Option<String>,
    pub(super) channel_ref: Option<String>,
    pub(super) installation_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppResource {
    pub(super) uri: String,
    pub(super) name: Option<String>,
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    pub(super) mime_type: Option<String>,
    pub(super) meta: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppServerDescriptor {
    pub(super) server_id: String,
    pub(super) endpoint: String,
    pub(super) name: String,
    pub(super) version: Option<String>,
    pub(super) protocol_version: String,
    pub(super) tools: Vec<McpAppTool>,
    pub(super) resources: Vec<McpAppResource>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppResourceCsp {
    #[serde(default)]
    pub(super) connect_domains: Vec<String>,
    #[serde(default)]
    pub(super) resource_domains: Vec<String>,
    #[serde(default)]
    pub(super) frame_domains: Vec<String>,
    #[serde(default)]
    pub(super) base_uri_domains: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppResourcePermissions {
    pub(super) camera: Option<Value>,
    pub(super) microphone: Option<Value>,
    pub(super) geolocation: Option<Value>,
    pub(super) clipboard_write: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppResourcePolicy {
    pub(super) csp: McpAppResourceCsp,
    pub(super) requested_permissions: McpAppResourcePermissions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedMcpAppView {
    pub(super) view_id: String,
    pub(super) sandbox_url: String,
    pub(super) html: String,
    pub(super) csp: McpAppResourceCsp,
    /// Permissions are reported for review but not granted by this host layer.
    pub(super) requested_permissions: McpAppResourcePermissions,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAppToolCaller {
    Host,
    App,
}

#[derive(Debug)]
pub(super) struct McpWireResponse {
    pub(super) value: Option<Value>,
    pub(super) session_id: Option<String>,
}

/// Raw HTTP-level MCP reply that preserves the status code and a leniently
/// parsed body, so the era probe can inspect non-2xx responses.
#[derive(Debug)]
pub(super) struct McpHttpReply {
    pub(super) status: reqwest::StatusCode,
    pub(super) value: Option<Value>,
    pub(super) session_id: Option<String>,
}

pub(super) fn text(value: Option<&Value>) -> Option<String> {
    value?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn ui_resource_uri(meta: &Value) -> Option<String> {
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

pub(super) fn tool_visibility(meta: &Value) -> Vec<String> {
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

pub(super) fn parse_tools(value: &Value) -> Result<Vec<McpAppTool>, String> {
    let tools = value
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .ok_or_else(|| "MCP tools/list response is missing result.tools".to_string())?;
    let mut parsed = Vec::new();
    for tool in tools.iter().take(MAX_TOOLS) {
        let name =
            text(tool.get("name")).ok_or_else(|| "MCP tool is missing a valid name".to_string())?;
        let input_schema = tool
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        let param_headers = match parse_param_headers(&input_schema) {
            Ok(headers) => headers,
            Err(reason) => {
                tracing::warn!(
                    tool = %name,
                    reason = %reason,
                    "excluding MCP tool with invalid x-mcp-header annotation"
                );
                continue;
            }
        };
        let meta = tool.get("_meta").cloned().unwrap_or_else(|| json!({}));
        parsed.push(McpAppTool {
            name,
            title: text(tool.get("title")),
            description: text(tool.get("description")),
            input_schema,
            output_schema: tool.get("outputSchema").cloned(),
            annotations: tool.get("annotations").cloned(),
            ui_resource_uri: ui_resource_uri(&meta),
            visibility: tool_visibility(&meta),
            meta,
            param_headers,
        });
    }
    Ok(parsed)
}

fn valid_header_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

pub(super) fn parse_param_headers(input_schema: &Value) -> Result<Vec<McpParamHeader>, String> {
    fn visit(
        schema: &Value,
        path: &mut Vec<String>,
        seen: &mut HashSet<String>,
        headers: &mut Vec<McpParamHeader>,
    ) -> Result<(), String> {
        if let Some(annotation) = schema.get("x-mcp-header") {
            let name = annotation
                .as_str()
                .ok_or_else(|| "x-mcp-header must be a string".to_string())?;
            if !valid_header_token(name) {
                return Err(format!("x-mcp-header {name:?} is not a valid HTTP token"));
            }
            if !seen.insert(name.to_ascii_lowercase()) {
                return Err(format!(
                    "x-mcp-header {name:?} is not case-insensitively unique"
                ));
            }
            if !matches!(
                schema.get("type").and_then(Value::as_str),
                Some("string" | "integer" | "boolean")
            ) {
                return Err(format!(
                    "x-mcp-header {name:?} must annotate a string, integer, or boolean"
                ));
            }
            headers.push(McpParamHeader {
                path: path.clone(),
                name: name.to_string(),
            });
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (property, child) in properties {
                path.push(property.clone());
                visit(child, path, seen, headers)?;
                path.pop();
            }
        }
        Ok(())
    }

    let mut headers = Vec::new();
    visit(
        input_schema,
        &mut Vec::new(),
        &mut HashSet::new(),
        &mut headers,
    )?;
    Ok(headers)
}

pub(super) fn parse_resources(value: &Value) -> Result<Vec<McpAppResource>, String> {
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
