//! Native replay must remain consistent with the real ACP loop's recovery limits.
mod common;

use common::{spawn_capturing_llm, Harness};
use serde_json::json;

async fn session(h: &mut Harness) -> String {
    let id = h
        .send(
            "initialize",
            json!({"protocolVersion":1,"clientCapabilities":{}}),
        )
        .await;
    h.recv_until(|v| v["id"] == id).await;
    let id = h
        .send(
            "session/new",
            json!({"cwd":"/tmp","mcpServers":[{
                "name":"probe", "command":env!("CARGO_BIN_EXE_fake-mcp"), "args":[], "env":[]
            }]}),
        )
        .await;
    h.recv_until(|v| v["id"] == id).await["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn native_harness(url: &str) -> Harness {
    // Native Anthropic avoids Databricks discovery consuming response fixtures;
    // the exact UC dispatcher and effort contract are tested in llm_fqn_tests.
    Harness::spawn_with_env(
        url,
        &[
            ("BUZZ_AGENT_PROVIDER", "anthropic"),
            ("ANTHROPIC_API_KEY", "test"),
            ("ANTHROPIC_BASE_URL", url),
            ("BUZZ_AGENT_MODEL", "claude-opus-5-5"),
            ("BUZZ_AGENT_THINKING_EFFORT", "high"),
        ],
    )
    .await
}

#[tokio::test]
async fn oversized_native_turn_fails_before_tools_execute() {
    let mut content = vec![json!({"type":"thinking", "thinking":"", "signature":"signed"})];
    content.extend((0..65).map(|n| {
        json!({"type":"tool_use", "id":format!("call-{n}"),
        "name":"probe__tool_0", "input":{}})
    }));
    let llm = spawn_capturing_llm(vec![json!({"stop_reason":"tool_use", "content":content})]).await;
    let mut h = native_harness(&llm.url).await;
    let sid = session(&mut h).await;
    let id = h
        .send(
            "session/prompt",
            json!({"sessionId":sid,"prompt":[{"type":"text","text":"go"}]}),
        )
        .await;
    let mut tool_started = false;
    let reply = h
        .recv_until_approving(|v| {
            tool_started |= v["params"]["update"]["sessionUpdate"] == "tool_call";
            v["id"] == id
        })
        .await;
    assert!(
        reply["error"]
            .to_string()
            .contains("cannot truncate native content"),
        "{reply}"
    );
    assert!(
        !tool_started,
        "oversized native turn must execute no partial subset"
    );
    assert_eq!(llm.captured.lock().await.len(), 1);
    h.shutdown().await;
}

#[tokio::test]
async fn native_max_tokens_recovers_without_signed_state_or_partial_tools() {
    let llm = spawn_capturing_llm(vec![
        json!({"stop_reason":"max_tokens", "content":[
            {"type":"thinking", "thinking":"", "signature":"incomplete-signature"},
            {"type":"text", "text":"partial-text"},
            {"type":"tool_use", "id":"partial-call", "name":"probe__tool_0", "input":"{broken"},
            {"type":"redacted_thinking", "data":"incomplete-redaction"}
        ]}),
        json!({"stop_reason":"end_turn", "content":[{"type":"text", "text":"done"}]}),
    ])
    .await;
    let mut h = native_harness(&llm.url).await;
    let sid = session(&mut h).await;
    let id = h
        .send(
            "session/prompt",
            json!({"sessionId":sid,"prompt":[{"type":"text","text":"go"}]}),
        )
        .await;
    let mut tool_started = false;
    let reply = h
        .recv_until_approving(|v| {
            tool_started |= v["params"]["update"]["sessionUpdate"] == "tool_call";
            v["id"] == id
        })
        .await;
    assert_eq!(reply["result"]["stopReason"], "end_turn", "{reply}");
    assert!(!tool_started);
    let requests = llm.captured.lock().await;
    assert_eq!(requests.len(), 2);
    let retry = &requests[1]["messages"];
    let assistant = retry
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "assistant")
        .unwrap();
    let blocks = assistant["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "partial-text");
    let serialized = retry.to_string();
    for forbidden in [
        "incomplete-signature",
        "incomplete-redaction",
        "partial-call",
        "tool_result",
    ] {
        assert!(!serialized.contains(forbidden), "{retry}");
    }
    assert!(serialized.contains("output token limit"));
    drop(requests);
    h.shutdown().await;
}
