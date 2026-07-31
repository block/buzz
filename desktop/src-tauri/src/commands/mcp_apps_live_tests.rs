use super::*;
use axum::{extract::State as AxumState, routing::post, Json, Router};
use tokio::net::TcpListener;

type Calls = Arc<AsyncMutex<Vec<Value>>>;

async fn handle_mcp(AxumState(calls): AxumState<Calls>, Json(payload): Json<Value>) -> Json<Value> {
    calls.lock().await.push(payload.clone());
    let id = payload.get("id").cloned().unwrap_or(Value::Null);
    let result = match payload.get("method").and_then(Value::as_str) {
        Some("tools/list") => json!({
            "tools": [{
                "name": "prepare_brief",
                "title": "Signal reader",
                "inputSchema": {
                    "type": "object",
                    "properties": {"storyId": {"type": "string"}}
                },
                "_meta": {
                    "ui": {
                        "resourceUri": "ui://review/signal-reader",
                        "visibility": ["app", "model"]
                    }
                }
            }]
        }),
        Some("tools/call") => json!({
            "content": [{
                "type": "text",
                "text": "Prepared through the live Streamable HTTP path."
            }]
        }),
        method => panic!("unexpected MCP method: {method:?}"),
    };
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

#[tokio::test]
async fn live_streamable_http_round_trip_preserves_host_context() {
    let calls: Calls = Arc::new(AsyncMutex::new(Vec::new()));
    let app = Router::new()
        .route("/mcp", post(handle_mcp))
        .with_state(calls.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind MCP test server");
    let address = listener.local_addr().expect("read MCP test address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve MCP test endpoint");
    });

    let endpoint = validate_mcp_endpoint(&format!("http://{address}/mcp"))
        .expect("loopback MCP endpoint must be allowed");
    let client = build_pinned_client(&endpoint)
        .await
        .expect("build pinned MCP client");
    let (tools_response, protocol_version) = match probe_modern(&client, &endpoint)
        .await
        .expect("probe modern MCP server")
    {
        ModernProbe::Modern {
            response,
            protocol_version,
        } => (response, protocol_version),
        ModernProbe::Legacy => panic!("test server must negotiate the modern MCP revision"),
    };
    let tools = parse_tools(&tools_response).expect("parse advertised MCP App tool");
    let connection = McpServerConnection {
        endpoint,
        client,
        era: McpEra::Modern,
        protocol_version,
        session_id: None,
        next_request_id: Arc::new(AtomicU64::new(2)),
        tools,
        resources: Vec::new(),
    };
    let result = request(
        &connection,
        "tools/call",
        build_tool_call_params(
            "prepare_brief",
            json!({"storyId": "interactive-tools"}),
            None,
            Some(&McpAppInvocationContext {
                community_ref: Some("community-1".to_string()),
                channel_ref: Some("channel-1".to_string()),
                installation_ref: Some("installation-1".to_string()),
            }),
        ),
    )
    .await
    .expect("call live MCP App tool");

    assert_eq!(
        result.pointer("/result/content/0/text"),
        Some(&json!("Prepared through the live Streamable HTTP path."))
    );
    let observed = calls.lock().await;
    assert_eq!(observed.len(), 2);
    assert_eq!(
        observed[1].pointer("/params/_meta/xyz.block.buzz~1context"),
        Some(&json!({
            "communityRef": "community-1",
            "channelRef": "channel-1",
            "installationRef": "installation-1"
        }))
    );
    assert_eq!(
        observed[1].pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion"),
        Some(&json!(MCP_MODERN_PROTOCOL_VERSION))
    );

    drop(observed);
    server.abort();
}
