use super::*;
use std::borrow::Cow;

pub(super) fn app_tool_allowed(tool: &McpAppTool, caller: McpAppToolCaller) -> bool {
    match caller {
        McpAppToolCaller::Host => tool.visibility.iter().any(|value| value == "model"),
        McpAppToolCaller::App => tool.visibility.iter().any(|value| value == "app"),
    }
}

/// Connect to a reviewed Streamable HTTP MCP server and discover its Apps.
///
/// Dual-era: the connection probes the modern (`2026-07-28`) revision first
/// and falls back to the legacy (`2025-11-25`) `initialize` handshake only
/// when the probe classifies the origin as legacy.
#[tauri::command]
pub async fn connect_mcp_app_server(
    endpoint: String,
    state: State<'_, McpAppHostState>,
) -> Result<McpAppServerDescriptor, String> {
    let endpoint = validate_mcp_endpoint(&endpoint)?;
    let client = build_pinned_client(&endpoint).await?;
    let (connection, server_name, server_version, tools_response) =
        match probe_modern(&client, &endpoint).await? {
            ModernProbe::Modern {
                response,
                protocol_version,
            } => {
                let connection = McpServerConnection {
                    endpoint: endpoint.clone(),
                    client,
                    era: McpEra::Modern,
                    protocol_version,
                    session_id: None,
                    next_request_id: Arc::new(AtomicU64::new(3)),
                    tools: Vec::new(),
                    resources: Vec::new(),
                };
                (connection, endpoint.to_string(), None, response)
            }
            ModernProbe::Legacy => {
                let initialize = post_mcp(
                    &client,
                    &endpoint,
                    McpEra::Legacy,
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
                let protocol_version =
                    text(value.pointer("/result/protocolVersion")).ok_or_else(|| {
                        "MCP initialize response is missing protocolVersion".to_string()
                    })?;
                let server_name = text(value.pointer("/result/serverInfo/name"))
                    .unwrap_or_else(|| endpoint.to_string());
                let server_version = text(value.pointer("/result/serverInfo/version"));
                let connection = McpServerConnection {
                    endpoint: endpoint.clone(),
                    client,
                    era: McpEra::Legacy,
                    protocol_version,
                    session_id: initialize.session_id,
                    next_request_id: Arc::new(AtomicU64::new(2)),
                    tools: Vec::new(),
                    resources: Vec::new(),
                };
                let _ = post_mcp(
                    &connection.client,
                    &connection.endpoint,
                    McpEra::Legacy,
                    Some(&connection.protocol_version),
                    connection.session_id.as_deref(),
                    &json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/initialized",
                        "params": {}
                    }),
                )
                .await?;
                let tools_response = request(&connection, "tools/list", json!({})).await?;
                (connection, server_name, server_version, tools_response)
            }
        };
    let tools = parse_tools(&tools_response)?;
    let resources = request(&connection, "resources/list", json!({}))
        .await
        .and_then(|value| parse_resources(&value))
        .unwrap_or_default();
    let protocol_version = connection.protocol_version.clone();
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
    .and_then(|value| extract_result(&value, "tools/call"))
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
        .and_then(|value| extract_result(&value, "resources/read"))
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

pub(super) fn sandbox_proxy_html() -> Cow<'static, str> {
    #[cfg(debug_assertions)]
    {
        Cow::Owned(SANDBOX_PROXY_HTML.replace(
            "    /* BUZZ_MCP_APP_DEV_ORIGINS */",
            ",\n    \"http://localhost:1420\",\n    \"http://127.0.0.1:1420\"",
        ))
    }
    #[cfg(not(debug_assertions))]
    {
        Cow::Borrowed(SANDBOX_PROXY_HTML)
    }
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
    html_response(200, &sandbox_proxy_html(), Some(&view.csp))
}
