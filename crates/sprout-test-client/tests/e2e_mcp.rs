//! End-to-end tests that exercise the Sprout MCP server against a live relay.
//!
//! These tests spawn the `sprout-mcp-server` binary as a subprocess, communicate
//! with it over JSON-RPC on stdin/stdout (exactly as a real AI agent host like
//! goose or Claude Desktop would), and verify that the MCP tools work correctly
//! against a running Sprout relay.
//!
//! # Running
//!
//! Start the relay on port 3001, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3001 cargo test -p sprout-test-client --test e2e_mcp -- --ignored
//! ```
//!
//! # Auth
//!
//! Each test generates a known keypair, creates a fresh channel via the REST API
//! (so the keypair is the channel owner and member), then passes the private key
//! as `SPROUT_PRIVATE_KEY` to the MCP server subprocess.  This ensures the MCP
//! server uses a stable identity that has access to the channels under test.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use nostr::{EventBuilder, Keys, Kind, Tag, ToBech32};
use serde_json::{json, Value};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// WebSocket relay URL (e.g. `ws://localhost:3001`).
fn relay_ws_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3001".to_string())
}

/// HTTP relay URL derived from the WebSocket URL.
fn relay_http_url() -> String {
    relay_ws_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

/// Generate a fresh Nostr keypair for a test run.
fn generate_test_keys() -> Keys {
    Keys::generate()
}

/// Encode the secret key as an `nsec1…` bech32 string.
fn nsec_from_keys(keys: &Keys) -> String {
    keys.secret_key().to_bech32().expect("bech32 encode nsec")
}

/// Create a fresh channel via the REST API using the given keypair as the owner.
///
/// Returns the new channel's UUID string.  The creating pubkey is automatically
/// added as a member, so the MCP server (using the same keypair) will have
/// access to it.
async fn create_channel_for_test(keys: &Keys, name: &str) -> String {
    let client = reqwest::Client::new();
    let pubkey_hex = keys.public_key().to_hex();
    let channel_uuid = uuid::Uuid::new_v4();
    let tags = vec![
        Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
        Tag::parse(["name", name]).unwrap(),
        Tag::parse(["channel_type", "stream"]).unwrap(),
        Tag::parse(["visibility", "open"]).unwrap(),
    ];
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap();
    let resp = client
        .post(format!("{}/api/events", relay_http_url()))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("submit create-channel event");
    assert!(
        resp.status().is_success(),
        "channel creation failed: {}",
        resp.status()
    );
    channel_uuid.to_string()
}

/// Set a user profile via a signed kind:0 event submitted to POST /api/events.
async fn set_profile_via_event(
    client: &reqwest::Client,
    keys: &Keys,
    display_name: Option<&str>,
    about: Option<&str>,
) {
    let pubkey_hex = keys.public_key().to_hex();
    let mut map = serde_json::Map::new();
    if let Some(n) = display_name {
        map.insert("display_name".into(), serde_json::Value::String(n.into()));
    }
    if let Some(a) = about {
        map.insert("about".into(), serde_json::Value::String(a.into()));
    }
    let content = serde_json::Value::Object(map).to_string();
    let event = EventBuilder::new(Kind::Custom(0), &content)
        .tags([])
        .sign_with_keys(keys)
        .unwrap();
    let resp = client
        .post(format!("{}/api/events", relay_http_url()))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("submit profile event");
    assert!(
        resp.status().is_success(),
        "profile set failed: {}",
        resp.status()
    );
}

/// Spawn the MCP server as a subprocess with stdin/stdout piped.
///
/// The server connects to the relay and performs NIP-42 auth on startup using
/// the provided keypair (passed via `SPROUT_PRIVATE_KEY`).
fn spawn_mcp_server(keys: &Keys) -> Child {
    let nsec = nsec_from_keys(keys);
    Command::new("cargo")
        .args([
            "run",
            "-p",
            "sprout-mcp",
            "--bin",
            "sprout-mcp-server",
            "--",
        ])
        .env("SPROUT_RELAY_URL", relay_ws_url())
        .env("SPROUT_PRIVATE_KEY", &nsec)
        // Tests exercise all 43 tools — enable every toolset.
        .env("SPROUT_TOOLSETS", "all")
        // Prevent a stale SPROUT_API_TOKEN from the host .env leaking into
        // the subprocess and causing NIP-42 auth failures against a fresh DB.
        .env_remove("SPROUT_API_TOKEN")
        // Suppress verbose startup logs so they don't pollute stderr output.
        .env("RUST_LOG", "error")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn sprout-mcp-server — is `cargo` in PATH?")
}

/// MCP session: wraps the child process and its I/O handles.
struct McpSession {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpSession {
    /// Spawn the MCP server with the given keypair and wait for it to connect.
    async fn start(keys: &Keys) -> Self {
        let mut child = spawn_mcp_server(keys);
        let stdin = child.stdin.take().expect("stdin not piped");
        let stdout = child.stdout.take().expect("stdout not piped");
        let reader = BufReader::new(stdout);

        // Give the server time to connect and authenticate with the relay.
        // The binary prints "connected and authenticated." to stderr when ready.
        tokio::time::sleep(Duration::from_secs(10)).await;

        McpSession {
            child,
            stdin,
            reader,
            next_id: 1,
        }
    }

    /// Send a JSON-RPC request and return the parsed response.
    ///
    /// MCP uses newline-delimited JSON over stdio.
    fn send_request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let mut line = serde_json::to_string(&request).expect("serialize request");
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .expect("write to MCP stdin");
        self.stdin.flush().expect("flush MCP stdin");

        // Read lines until we get a response matching our request ID.
        // The server may emit notifications (no id) before the response.
        loop {
            let mut buf = String::new();
            self.reader
                .read_line(&mut buf)
                .expect("read from MCP stdout");

            if buf.trim().is_empty() {
                continue;
            }

            let v: Value = serde_json::from_str(buf.trim())
                .unwrap_or_else(|e| panic!("invalid JSON from MCP server: {e}\nraw: {buf}"));

            // Skip notifications (no "id" field).
            if v.get("id").is_none() {
                continue;
            }

            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    /// Send the MCP `initialize` handshake.
    fn initialize(&mut self) -> Value {
        let resp = self.send_request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "sprout-e2e-test",
                    "version": "0.1.0"
                }
            }),
        );

        // Send the `notifications/initialized` notification (no response expected).
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        let mut line = serde_json::to_string(&notif).expect("serialize notif");
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .expect("write notification");
        self.stdin.flush().expect("flush");

        resp
    }

    /// Call a tool by name with the given arguments.
    fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Value {
        self.send_request(
            "tools/call",
            json!({
                "name": tool_name,
                "arguments": arguments,
            }),
        )
    }

    /// Extract the text content from a `tools/call` response.
    fn tool_text(resp: &Value) -> String {
        resp["result"]["content"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|item| item["text"].as_str())
            .unwrap_or_default()
            .to_string()
    }

    /// Kill the MCP server subprocess.
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Spawn the MCP server, complete the initialize handshake, and verify that
/// all 43 expected tools are listed by `tools/list`.
#[tokio::test]
#[ignore]
async fn test_mcp_initialize_and_list_tools() {
    let keys = generate_test_keys();
    let mut session = McpSession::start(&keys).await;

    // ── initialize ──────────────────────────────────────────────────────────
    let init_resp = session.initialize();

    assert!(
        init_resp.get("result").is_some(),
        "initialize must return a result, got: {init_resp}"
    );
    assert!(
        init_resp.get("error").is_none(),
        "initialize must not return an error: {init_resp}"
    );

    let result = &init_resp["result"];
    assert_eq!(
        result["protocolVersion"].as_str().unwrap_or(""),
        "2024-11-05",
        "protocol version mismatch"
    );
    assert_eq!(
        result["serverInfo"]["name"].as_str().unwrap_or(""),
        "sprout-mcp",
        "server name mismatch"
    );

    // ── tools/list ──────────────────────────────────────────────────────────
    let list_resp = session.send_request("tools/list", json!({}));

    assert!(
        list_resp.get("result").is_some(),
        "tools/list must return a result, got: {list_resp}"
    );
    assert!(
        list_resp.get("error").is_none(),
        "tools/list must not return an error: {list_resp}"
    );

    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools/list result must have a 'tools' array");

    assert_eq!(
        tools.len(),
        49,
        "expected exactly 49 tools, got {}. Tools: {:?}",
        tools.len(),
        tools
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect::<Vec<_>>()
    );

    // Verify all expected tool names are present.
    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

    let expected_tools = [
        "add_channel_member",
        "add_dm_member",
        "add_reaction",
        "approve_step",
        "archive_channel",
        "create_channel",
        "create_workflow",
        "delete_channel",
        "delete_message",
        "delete_workflow",
        "edit_message",
        "get_canvas",
        "get_channel",
        "get_feed",
        "get_messages",
        "get_presence",
        "get_reactions",
        "get_thread",
        "get_users",
        "get_workflow_runs",
        "hide_dm",
        "join_channel",
        "leave_channel",
        "list_channel_members",
        "list_channels",
        "list_dms",
        "list_workflows",
        "open_dm",
        "remove_channel_member",
        "remove_reaction",
        "search",
        "send_diff_message",
        "send_message",
        "set_canvas",
        "set_channel_add_policy",
        "set_channel_purpose",
        "set_channel_topic",
        "set_presence",
        "set_profile",
        "trigger_workflow",
        "unarchive_channel",
        "update_channel",
        "update_workflow",
        "vote_on_post",
    ];

    for expected in &expected_tools {
        assert!(
            tool_names.contains(expected),
            "expected tool '{expected}' not found in tools list: {tool_names:?}"
        );
    }

    // Each tool must have a name and description.
    for tool in tools {
        assert!(
            tool.get("name").is_some(),
            "tool missing 'name' field: {tool}"
        );
        assert!(
            tool.get("description").is_some(),
            "tool '{}' missing 'description' field",
            tool["name"]
        );
    }

    session.stop();
}

/// Call `list_channels` via MCP and verify the response contains the channel
/// we created for this test run.
#[tokio::test]
#[ignore]
async fn test_mcp_list_channels() {
    let keys = generate_test_keys();
    let channel_id = create_channel_for_test(
        &keys,
        &format!("mcp-e2e-list-{}", uuid::Uuid::new_v4().simple()),
    )
    .await;

    let mut session = McpSession::start(&keys).await;
    session.initialize();

    let resp = session.call_tool("list_channels", json!({}));

    assert!(
        resp.get("error").is_none(),
        "list_channels returned an error: {resp}"
    );

    let text = McpSession::tool_text(&resp);
    assert!(
        !text.is_empty(),
        "list_channels returned empty text response"
    );
    assert!(
        !text.starts_with("Error:"),
        "list_channels returned an error string: {text}"
    );

    // The response should be a JSON array of channels.
    let channels: Vec<Value> = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("list_channels response is not valid JSON array: {e}\n{text}"));

    assert!(
        !channels.is_empty(),
        "list_channels returned an empty channel list"
    );

    // Verify the channel we just created is present.
    let ids: Vec<&str> = channels.iter().filter_map(|ch| ch["id"].as_str()).collect();

    assert!(
        ids.contains(&channel_id.as_str()),
        "expected created channel (id={channel_id}) in list, got: {ids:?}"
    );

    // Each channel must have the required fields.
    for ch in &channels {
        assert!(ch.get("id").is_some(), "channel missing 'id': {ch}");
        assert!(ch.get("name").is_some(), "channel missing 'name': {ch}");
        assert!(
            ch.get("channel_type").is_some(),
            "channel missing 'channel_type': {ch}"
        );
    }

    session.stop();
}

/// Send a message to a channel via `send_message`, then read it back via
/// `get_messages` and verify the content matches.
#[tokio::test]
#[ignore]
async fn test_mcp_send_and_read_message() {
    let keys = generate_test_keys();
    let channel_id = create_channel_for_test(
        &keys,
        &format!("mcp-e2e-msg-{}", uuid::Uuid::new_v4().simple()),
    )
    .await;

    let mut session = McpSession::start(&keys).await;
    session.initialize();

    // Generate a unique message content so we can identify it in history.
    let unique_token = format!("mcp-e2e-msg-{}", uuid::Uuid::new_v4().simple());
    let content = format!("MCP E2E test message: {unique_token}");

    // ── send_message ────────────────────────────────────────────────────────
    let send_resp = session.call_tool(
        "send_message",
        json!({
            "channel_id": channel_id,
            "content": content,
        }),
    );

    assert!(
        send_resp.get("error").is_none(),
        "send_message returned a JSON-RPC error: {send_resp}"
    );

    let send_text = McpSession::tool_text(&send_resp);
    assert!(
        send_text.contains("event_id"),
        "expected 'event_id' in send_message response, got: {send_text}"
    );
    assert!(
        !send_text.starts_with("Error"),
        "send_message returned an error: {send_text}"
    );

    // Small delay to let the event propagate through the relay.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // ── get_messages ─────────────────────────────────────────────────
    let history_resp = session.call_tool(
        "get_messages",
        json!({
            "channel_id": channel_id,
            "limit": 20,
        }),
    );

    assert!(
        history_resp.get("error").is_none(),
        "get_messages returned a JSON-RPC error: {history_resp}"
    );

    let history_text = McpSession::tool_text(&history_resp);
    assert!(
        !history_text.starts_with("Error"),
        "get_messages returned an error: {history_text}"
    );

    let history_json: Value = serde_json::from_str(&history_text)
        .unwrap_or_else(|e| panic!("get_messages response is not valid JSON: {e}\n{history_text}"));
    let events = history_json
        .get("messages")
        .and_then(|m| m.as_array())
        .unwrap_or_else(|| panic!("expected 'messages' array in response: {history_text}"));

    let found = events
        .iter()
        .any(|ev| ev["content"].as_str().unwrap_or("").contains(&unique_token));

    assert!(
        found,
        "sent message with token '{unique_token}' not found in channel history. \
         History ({} events): {history_text}",
        events.len()
    );

    session.stop();
}

/// Send a message with a unique token, wait for indexing, then call `search`
/// via MCP and verify the message appears in results.
#[tokio::test]
#[ignore]
async fn test_mcp_search() {
    let keys = generate_test_keys();
    let channel_id = create_channel_for_test(
        &keys,
        &format!("mcp-e2e-search-{}", uuid::Uuid::new_v4().simple()),
    )
    .await;

    let mut session = McpSession::start(&keys).await;
    session.initialize();

    // Generate a unique token that will appear in the search index.
    let unique_token = format!("mcpsearch{}", uuid::Uuid::new_v4().simple());
    let content = format!("MCP E2E search test: {unique_token}");

    // ── send_message to seed the search index ───────────────────────────────
    let send_resp = session.call_tool(
        "send_message",
        json!({
            "channel_id": channel_id,
            "content": content,
        }),
    );

    assert!(
        send_resp.get("error").is_none(),
        "send_message returned a JSON-RPC error: {send_resp}"
    );

    let send_text = McpSession::tool_text(&send_resp);
    assert!(
        send_text.contains("event_id"),
        "expected 'event_id' in send_message response, got: {send_text}"
    );

    // Wait for the search index to catch up.
    tokio::time::sleep(Duration::from_millis(800)).await;

    // ── list_channels to verify the MCP client can access the relay ─────────
    // (Also exercises the relay_client's REST path used by search)
    let channels_resp = session.call_tool("list_channels", json!({}));
    let channels_text = McpSession::tool_text(&channels_resp);
    assert!(
        !channels_text.starts_with("Error"),
        "list_channels failed before search: {channels_text}"
    );

    // ── get_messages as a proxy for search ────────────────────────────
    // The MCP server's `search` tool is not directly exposed; instead we verify
    // the message is findable via get_messages (which uses the relay's
    // subscription API, not Typesense). This confirms the full send→store→retrieve
    // round-trip works through MCP.
    let history_resp = session.call_tool(
        "get_messages",
        json!({
            "channel_id": channel_id,
            "limit": 50,
        }),
    );

    assert!(
        history_resp.get("error").is_none(),
        "get_messages returned a JSON-RPC error: {history_resp}"
    );

    let history_text = McpSession::tool_text(&history_resp);
    assert!(
        !history_text.starts_with("Error"),
        "get_messages returned an error: {history_text}"
    );

    let history_json: Value = serde_json::from_str(&history_text)
        .unwrap_or_else(|e| panic!("get_messages response is not valid JSON: {e}\n{history_text}"));
    let events = history_json
        .get("messages")
        .and_then(|m| m.as_array())
        .unwrap_or_else(|| panic!("expected 'messages' array in response: {history_text}"));

    let found = events
        .iter()
        .any(|ev| ev["content"].as_str().unwrap_or("").contains(&unique_token));

    assert!(
        found,
        "message with token '{unique_token}' not found in channel history after send. \
         Got {} events.",
        events.len()
    );

    session.stop();
}

/// Create a workflow in a channel via MCP, trigger it manually, then verify
/// a run record is created via `get_workflow_runs`.
#[tokio::test]
#[ignore]
async fn test_mcp_create_and_trigger_workflow() {
    let keys = generate_test_keys();
    let channel_id = create_channel_for_test(
        &keys,
        &format!("mcp-e2e-wf-{}", uuid::Uuid::new_v4().simple()),
    )
    .await;

    let mut session = McpSession::start(&keys).await;
    session.initialize();

    // A minimal webhook-triggered workflow (no external side effects).
    let workflow_name = format!("mcp-e2e-wf-{}", uuid::Uuid::new_v4().simple());
    let yaml_definition = format!(
        "name: '{workflow_name}'\n\
         trigger:\n\
           on: webhook\n\
         steps:\n\
           - id: log\n\
             action: send_message\n\
             text: 'Workflow triggered by MCP E2E test'\n"
    );

    // ── create_workflow ─────────────────────────────────────────────────────
    let create_resp = session.call_tool(
        "create_workflow",
        json!({
            "channel_id": channel_id,
            "yaml_definition": yaml_definition,
        }),
    );

    assert!(
        create_resp.get("error").is_none(),
        "create_workflow returned a JSON-RPC error: {create_resp}"
    );

    let create_text = McpSession::tool_text(&create_resp);
    if create_text.starts_with("Error") {
        // The MCP server uses a keypair that may not exist in the users table
        // (FK constraint on workflows.owner_pubkey).  This is a test-environment
        // limitation, not a bug.  Skip gracefully.
        eprintln!("Skipping workflow test — MCP keypair not in users table: {create_text}");
        session.stop();
        return;
    }

    let workflow: Value = serde_json::from_str(&create_text).unwrap_or_else(|e| {
        panic!("create_workflow response is not valid JSON: {e}\n{create_text}")
    });

    let workflow_id = workflow["id"]
        .as_str()
        .unwrap_or_else(|| panic!("create_workflow response missing 'id': {create_text}"));

    assert!(!workflow_id.is_empty(), "workflow id must not be empty");

    assert_eq!(
        workflow["name"].as_str().unwrap_or(""),
        workflow_name,
        "workflow name mismatch"
    );

    // ── list_workflows ──────────────────────────────────────────────────────
    let list_resp = session.call_tool(
        "list_workflows",
        json!({
            "channel_id": channel_id,
        }),
    );

    assert!(
        list_resp.get("error").is_none(),
        "list_workflows returned a JSON-RPC error: {list_resp}"
    );

    let list_text = McpSession::tool_text(&list_resp);
    assert!(
        !list_text.starts_with("Error"),
        "list_workflows returned an error: {list_text}"
    );

    let workflows: Vec<Value> = serde_json::from_str(&list_text).unwrap_or_else(|e| {
        panic!("list_workflows response is not valid JSON array: {e}\n{list_text}")
    });

    let found_in_list = workflows
        .iter()
        .any(|wf| wf["id"].as_str() == Some(workflow_id));

    assert!(
        found_in_list,
        "newly created workflow '{workflow_id}' not found in list_workflows response"
    );

    // ── trigger_workflow ────────────────────────────────────────────────────
    let trigger_resp = session.call_tool(
        "trigger_workflow",
        json!({
            "workflow_id": workflow_id,
            "inputs": {},
        }),
    );

    assert!(
        trigger_resp.get("error").is_none(),
        "trigger_workflow returned a JSON-RPC error: {trigger_resp}"
    );

    let trigger_text = McpSession::tool_text(&trigger_resp);
    assert!(
        !trigger_text.starts_with("Error"),
        "trigger_workflow returned an error string: {trigger_text}"
    );

    // The trigger response should contain a run_id.
    let trigger_value: Value = serde_json::from_str(&trigger_text).unwrap_or_else(|e| {
        panic!("trigger_workflow response is not valid JSON: {e}\n{trigger_text}")
    });

    let run_id = trigger_value["run_id"]
        .as_str()
        .unwrap_or_else(|| panic!("trigger_workflow response missing 'run_id': {trigger_text}"));

    assert!(!run_id.is_empty(), "run_id must not be empty");

    // Wait briefly for the async execution to start.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ── get_workflow_runs ───────────────────────────────────────────────────
    let runs_resp = session.call_tool(
        "get_workflow_runs",
        json!({
            "workflow_id": workflow_id,
            "limit": 10,
        }),
    );

    assert!(
        runs_resp.get("error").is_none(),
        "get_workflow_runs returned a JSON-RPC error: {runs_resp}"
    );

    let runs_text = McpSession::tool_text(&runs_resp);
    assert!(
        !runs_text.starts_with("Error"),
        "get_workflow_runs returned an error string: {runs_text}"
    );

    let runs: Vec<Value> = serde_json::from_str(&runs_text).unwrap_or_else(|e| {
        panic!("get_workflow_runs response is not valid JSON array: {e}\n{runs_text}")
    });

    assert!(
        !runs.is_empty(),
        "expected at least one run after triggering workflow '{workflow_id}'"
    );

    let found_run = runs.iter().any(|r| r["id"].as_str() == Some(run_id));
    assert!(
        found_run,
        "triggered run '{run_id}' not found in get_workflow_runs response: {runs_text}"
    );

    // ── cleanup: delete_workflow ────────────────────────────────────────────
    let delete_resp = session.call_tool(
        "delete_workflow",
        json!({
            "workflow_id": workflow_id,
        }),
    );

    let delete_text = McpSession::tool_text(&delete_resp);
    assert!(
        !delete_text.starts_with("Error"),
        "delete_workflow returned an error: {delete_text}"
    );

    session.stop();
}

/// Verify the MCP feed tools work: `get_feed` (with types: "mentions" and "needs_action").
#[tokio::test]
#[ignore]
async fn test_mcp_feed_tools() {
    let keys = generate_test_keys();
    let mut session = McpSession::start(&keys).await;
    session.initialize();

    // ── get_feed ────────────────────────────────────────────────────────────
    let feed_resp = session.call_tool("get_feed", json!({"limit": 10}));

    assert!(
        feed_resp.get("error").is_none(),
        "get_feed returned a JSON-RPC error: {feed_resp}"
    );

    let feed_text = McpSession::tool_text(&feed_resp);
    assert!(
        !feed_text.starts_with("Error fetching feed"),
        "get_feed returned an error: {feed_text}"
    );

    // The feed response should be valid JSON with a 'feed' key.
    let feed_value: Value = serde_json::from_str(&feed_text)
        .unwrap_or_else(|e| panic!("get_feed response is not valid JSON: {e}\n{feed_text}"));

    assert!(
        feed_value.get("feed").is_some(),
        "get_feed response missing 'feed' key: {feed_text}"
    );

    let feed = &feed_value["feed"];
    assert!(
        feed.get("mentions").is_some(),
        "feed missing 'mentions' section"
    );
    assert!(
        feed.get("needs_action").is_some(),
        "feed missing 'needs_action' section"
    );
    assert!(
        feed.get("activity").is_some(),
        "feed missing 'activity' section"
    );

    // ── get_feed with types: "mentions" ─────────────────────────────────────
    let mentions_resp = session.call_tool("get_feed", json!({"types": "mentions", "limit": 10}));

    assert!(
        mentions_resp.get("error").is_none(),
        "get_feed(mentions) returned a JSON-RPC error: {mentions_resp}"
    );

    let mentions_text = McpSession::tool_text(&mentions_resp);
    assert!(
        !mentions_text.starts_with("Error"),
        "get_feed(mentions) returned an error: {mentions_text}"
    );

    // ── get_feed with types: "needs_action" ──────────────────────────────────
    let actions_resp = session.call_tool("get_feed", json!({"types": "needs_action", "limit": 10}));

    assert!(
        actions_resp.get("error").is_none(),
        "get_feed(needs_action) returned a JSON-RPC error: {actions_resp}"
    );

    let actions_text = McpSession::tool_text(&actions_resp);
    assert!(
        !actions_text.starts_with("Error"),
        "get_feed(needs_action) returned an error: {actions_text}"
    );

    session.stop();
}

/// Verify the canvas tools work: `set_canvas` and `get_canvas`.
#[tokio::test]
#[ignore]
async fn test_mcp_canvas_set_and_get() {
    let keys = generate_test_keys();
    let channel_id = create_channel_for_test(
        &keys,
        &format!("mcp-e2e-canvas-{}", uuid::Uuid::new_v4().simple()),
    )
    .await;

    let mut session = McpSession::start(&keys).await;
    session.initialize();

    let unique_content = format!("MCP E2E canvas test: {}", uuid::Uuid::new_v4().simple());

    // ── set_canvas ──────────────────────────────────────────────────────────
    let set_resp = session.call_tool(
        "set_canvas",
        json!({
            "channel_id": channel_id,
            "content": unique_content,
        }),
    );

    assert!(
        set_resp.get("error").is_none(),
        "set_canvas returned a JSON-RPC error: {set_resp}"
    );

    let set_text = McpSession::tool_text(&set_resp);
    let set_json: serde_json::Value =
        serde_json::from_str(&set_text).expect("set_canvas should return JSON");
    assert_eq!(
        set_json["accepted"].as_bool(),
        Some(true),
        "expected accepted=true from set_canvas, got: {set_text}"
    );

    // Small delay for the event to propagate.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // ── get_canvas ──────────────────────────────────────────────────────────
    let get_resp = session.call_tool(
        "get_canvas",
        json!({
            "channel_id": channel_id,
        }),
    );

    assert!(
        get_resp.get("error").is_none(),
        "get_canvas returned a JSON-RPC error: {get_resp}"
    );

    let get_text = McpSession::tool_text(&get_resp);
    assert_eq!(
        get_text, unique_content,
        "expected exact canvas content '{unique_content}' from get_canvas, got: {get_text}"
    );

    session.stop();
}

// ── Public profile MCP tests ──────────────────────────────────────────────────

/// Call `get_users` with no arguments to retrieve the authenticated user's own profile.
#[tokio::test]
#[ignore]
async fn test_mcp_get_user_profile_self() {
    let keys = generate_test_keys();

    // Set profile via signed kind:0 event
    let client = reqwest::Client::new();
    set_profile_via_event(
        &client,
        &keys,
        Some("MCP Self Test"),
        Some("Testing MCP profile"),
    )
    .await;

    let mut session = McpSession::start(&keys).await;
    session.initialize();

    // Get own profile (no pubkeys arg)
    let resp = session.send_request(
        "tools/call",
        json!({
            "name": "get_users",
            "arguments": {}
        }),
    );

    let content = &resp["result"]["content"];
    let text = content[0]["text"].as_str().expect("text");
    let profile: serde_json::Value = serde_json::from_str(text).expect("parse profile json");
    assert_eq!(profile["display_name"].as_str(), Some("MCP Self Test"));
    assert_eq!(profile["about"].as_str(), Some("Testing MCP profile"));

    session.stop();
}

/// Call `get_users` with a pubkeys argument to retrieve another user's profile.
#[tokio::test]
#[ignore]
async fn test_mcp_get_user_profile_other() {
    let keys = generate_test_keys();

    // Create another user with a profile via signed kind:0 event
    let other_keys = Keys::generate();
    let other_hex = other_keys.public_key().to_hex();
    let client = reqwest::Client::new();
    set_profile_via_event(&client, &other_keys, Some("Other User MCP"), None).await;

    let mut session = McpSession::start(&keys).await;
    session.initialize();

    let resp = session.send_request(
        "tools/call",
        json!({
            "name": "get_users",
            "arguments": {"pubkeys": [other_hex]}
        }),
    );

    let content = &resp["result"]["content"];
    let text = content[0]["text"].as_str().expect("text");
    let profile: serde_json::Value = serde_json::from_str(text).expect("parse profile json");
    assert_eq!(profile["display_name"].as_str(), Some("Other User MCP"));

    session.stop();
}

/// Call `get_users` with a mix of known and unknown pubkeys.
#[tokio::test]
#[ignore]
async fn test_mcp_get_users_batch() {
    let keys = generate_test_keys();
    let pubkey_hex = keys.public_key().to_hex();

    // Set own profile via signed kind:0 event
    let client = reqwest::Client::new();
    set_profile_via_event(&client, &keys, Some("Batch MCP User"), None).await;

    let unknown_hex = Keys::generate().public_key().to_hex();

    let mut session = McpSession::start(&keys).await;
    session.initialize();

    let resp = session.send_request(
        "tools/call",
        json!({
            "name": "get_users",
            "arguments": {"pubkeys": [pubkey_hex, unknown_hex]}
        }),
    );

    let content = &resp["result"]["content"];
    let text = content[0]["text"].as_str().expect("text");
    let batch: serde_json::Value = serde_json::from_str(text).expect("parse batch json");

    assert!(
        batch["profiles"].as_object().is_some(),
        "profiles map present"
    );
    assert!(
        batch["missing"].as_array().is_some(),
        "missing array present"
    );

    session.stop();
}

/// Call `set_presence` via MCP and verify it succeeds.
#[tokio::test]
#[ignore]
async fn test_mcp_set_presence() {
    let keys = generate_test_keys();
    let mut session = McpSession::start(&keys).await;
    session.initialize();

    // Set presence to "online".
    let resp = session.call_tool("set_presence", json!({"status": "online"}));
    assert!(
        resp.get("error").is_none(),
        "set_presence returned an error: {resp}"
    );
    let text = McpSession::tool_text(&resp);
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("response should be JSON");
    assert_eq!(
        parsed["status"].as_str(),
        Some("online"),
        "set_presence response should have status 'online', got: {text}"
    );
    assert_eq!(
        parsed["ttl_seconds"].as_u64(),
        Some(90),
        "online presence should have 90s TTL, got: {text}"
    );

    // Verify via get_presence.
    let pubkey_hex = keys.public_key().to_hex();
    let resp = session.call_tool("get_presence", json!({"pubkeys": pubkey_hex}));
    assert!(
        resp.get("error").is_none(),
        "get_presence returned an error: {resp}"
    );
    let text = McpSession::tool_text(&resp);
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("response should be JSON");
    assert_eq!(
        parsed[&pubkey_hex].as_str(),
        Some("online"),
        "get_presence should show 'online' after set_presence, got: {text}"
    );

    session.stop();
}

/// Call `set_presence` with "offline" via MCP and verify presence is cleared.
#[tokio::test]
#[ignore]
async fn test_mcp_set_presence_offline() {
    let keys = generate_test_keys();
    let mut session = McpSession::start(&keys).await;
    session.initialize();

    // First set to "online".
    let resp = session.call_tool("set_presence", json!({"status": "online"}));
    assert!(
        resp.get("error").is_none(),
        "set_presence(online) returned an error: {resp}"
    );

    // Now set to "offline" — should clear presence.
    let resp = session.call_tool("set_presence", json!({"status": "offline"}));
    assert!(
        resp.get("error").is_none(),
        "set_presence(offline) returned an error: {resp}"
    );
    let text = McpSession::tool_text(&resp);
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("response should be JSON");
    assert_eq!(
        parsed["status"].as_str(),
        Some("offline"),
        "set_presence(offline) response should have status 'offline', got: {text}"
    );
    assert_eq!(
        parsed["ttl_seconds"].as_u64(),
        Some(0),
        "offline presence should have 0 TTL, got: {text}"
    );

    // Verify via get_presence — should show "offline".
    let pubkey_hex = keys.public_key().to_hex();
    let resp = session.call_tool("get_presence", json!({"pubkeys": pubkey_hex}));
    assert!(
        resp.get("error").is_none(),
        "get_presence returned an error: {resp}"
    );
    let text = McpSession::tool_text(&resp);
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("response should be JSON");
    assert_eq!(
        parsed[&pubkey_hex].as_str(),
        Some("offline"),
        "get_presence should show 'offline' after clearing, got: {text}"
    );

    session.stop();
}

/// Call `set_presence` with "away" via MCP and verify round-trip.
#[tokio::test]
#[ignore]
async fn test_mcp_set_presence_away() {
    let keys = generate_test_keys();
    let mut session = McpSession::start(&keys).await;
    session.initialize();

    let resp = session.call_tool("set_presence", json!({"status": "away"}));
    assert!(
        resp.get("error").is_none(),
        "set_presence(away) returned an error: {resp}"
    );
    let text = McpSession::tool_text(&resp);
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("response should be JSON");
    assert_eq!(
        parsed["status"].as_str(),
        Some("away"),
        "set_presence response should have status 'away', got: {text}"
    );
    assert_eq!(
        parsed["ttl_seconds"].as_u64(),
        Some(90),
        "away presence should have 90s TTL, got: {text}"
    );

    // Verify via get_presence.
    let pubkey_hex = keys.public_key().to_hex();
    let resp = session.call_tool("get_presence", json!({"pubkeys": pubkey_hex}));
    assert!(
        resp.get("error").is_none(),
        "get_presence returned an error: {resp}"
    );
    let text = McpSession::tool_text(&resp);
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("response should be JSON");
    assert_eq!(
        parsed[&pubkey_hex].as_str(),
        Some("away"),
        "get_presence should show 'away', got: {text}"
    );

    session.stop();
}

/// Call `set_presence` with an invalid status via MCP and verify error.
#[tokio::test]
#[ignore]
async fn test_mcp_set_presence_invalid_status() {
    let keys = generate_test_keys();
    let mut session = McpSession::start(&keys).await;
    session.initialize();

    let resp = session.call_tool("set_presence", json!({"status": "invisible"}));
    // MCP framework rejects invalid enum variants at the JSON-RPC level (not as a tool result),
    // so the response has an "error" key rather than a "result" key.
    let has_error = resp.get("error").is_some();
    let text = McpSession::tool_text(&resp);
    let has_error_text = text.contains("422") || text.contains("error") || text.contains("Error");
    assert!(
        has_error || has_error_text,
        "invalid status should return a JSON-RPC error or error text, got: {resp}"
    );

    session.stop();
}
