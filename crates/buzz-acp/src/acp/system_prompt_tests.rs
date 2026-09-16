#[tokio::test]
async fn session_new_full_sends_pi_replacement_prompt_in_meta() {
    let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_pi","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
    let mut client = spawn_script(script).await;
    client
        .initialize()
        .await
        .expect("initialize should succeed");

    let resp = client
        .session_new_full(
            "/tmp",
            vec![],
            Some(SystemPromptTransport::PiMeta("Buzz instructions")),
            Some("Pi · #buzz-dev"),
        )
        .await
        .expect("session_new_full should succeed");

    let received = &resp.raw["_receivedRequest"];
    assert!(received["params"].get("systemPrompt").is_none());
    assert_eq!(
        received["params"]["_meta"]["systemPrompt"].as_str(),
        Some("Buzz instructions")
    );
    assert_eq!(
        received["params"]["_meta"]["sessionTitle"].as_str(),
        Some("Pi · #buzz-dev")
    );
}

#[tokio::test]
async fn session_new_full_sends_claude_meta_system_prompt_when_claude_meta_transport() {
    // When ClaudeMeta transport is requested, the prompt must appear as
    // _meta.systemPrompt: {"append": text} — never as a bare systemPrompt field.
    let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_claude","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
    let mut client = spawn_script(script).await;
    client
        .initialize()
        .await
        .expect("initialize should succeed");

    let resp = client
        .session_new_full(
            "/tmp",
            vec![],
            Some(SystemPromptTransport::ClaudeMeta("Be concise")),
            None,
        )
        .await
        .expect("session_new_full should succeed");

    let received = &resp.raw["_receivedRequest"];
    assert!(
        received["params"].get("systemPrompt").is_none(),
        "bare systemPrompt must not be present for ClaudeMeta transport"
    );
    assert_eq!(
        received["params"]["_meta"]["systemPrompt"]["append"].as_str(),
        Some("Be concise"),
        "_meta.systemPrompt.append must carry the prompt text"
    );
}

#[tokio::test]
async fn session_new_full_merges_claude_meta_and_session_title_into_single_meta_object() {
    // Both ClaudeMeta prompt and session_title must coexist under _meta —
    // the prompt must not clobber sessionTitle or vice versa.
    let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_merged","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
    let mut client = spawn_script(script).await;
    client
        .initialize()
        .await
        .expect("initialize should succeed");

    let resp = client
        .session_new_full(
            "/tmp",
            vec![],
            Some(SystemPromptTransport::ClaudeMeta("Be concise")),
            Some("Fizz · #buzz-dev"),
        )
        .await
        .expect("session_new_full should succeed");

    let received = &resp.raw["_receivedRequest"];
    assert_eq!(
        received["params"]["_meta"]["systemPrompt"]["append"].as_str(),
        Some("Be concise"),
        "_meta.systemPrompt.append must be present"
    );
    assert_eq!(
        received["params"]["_meta"]["sessionTitle"].as_str(),
        Some("Fizz · #buzz-dev"),
        "_meta.sessionTitle must be present alongside systemPrompt"
    );
}

// ── Goose-native steer scaffold (PR follow-up to #1160) ──────────────

/// Helper: spawn an inert `cat` subprocess so we have a real AcpClient
/// to drive `handle_session_update` against. `cat` never writes back,
/// which is fine — these tests don't read from the agent, they just
/// feed JSON into the parser.
async fn spawn_inert_client() -> AcpClient {
    AcpClient::spawn("cat", &[], &[], false)
        .await
        .expect("spawn cat as inert client")
}

/// Build a `session/update` JSON-RPC notification carrying a
/// `session_info_update` with the given `_meta.goose.activeRunId` value.
/// Pass `None` to omit the `activeRunId` field entirely.
///
/// `_meta` is nested inside the `update` object (per the ACP
/// `SessionInfoUpdate` schema), matching what goose and buzz-agent
/// emit on the wire.
fn session_info_update_msg(active_run_id: Option<serde_json::Value>) -> serde_json::Value {
    let mut goose = serde_json::Map::new();
    if let Some(v) = active_run_id {
        goose.insert("activeRunId".to_string(), v);
    }
    let mut meta = serde_json::Map::new();
    meta.insert("goose".to_string(), serde_json::Value::Object(goose));
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": "test-session",
            "update": {
                "sessionUpdate": "session_info_update",
                "_meta": serde_json::Value::Object(meta),
            },
        }
    })
}
