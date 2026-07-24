use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

struct Harness {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: i64,
}

impl Harness {
    async fn spawn(base_url: &str, integrations: Option<&str>) -> Self {
        Self::spawn_with_env(base_url, integrations, &[]).await
    }

    async fn spawn_with_env(
        base_url: &str,
        integrations: Option<&str>,
        extra_env: &[(&str, &str)],
    ) -> Self {
        let bin = env!("CARGO_BIN_EXE_buzz-lmstudio-agent");
        let mut command = tokio::process::Command::new(bin);
        command
            .env("LM_STUDIO_MODEL", "qwen/test")
            .env("LM_STUDIO_BASE_URL", base_url)
            .env("BUZZ_AGENT_LLM_TIMEOUT_SECS", "2")
            .env("BUZZ_AGENT_MAX_ROUNDS", "4")
            .env("BUZZ_AGENT_NO_HINTS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(integrations) = integrations {
            command.env("LM_STUDIO_MCP_INTEGRATIONS", integrations);
        }
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("spawn buzz-lmstudio-agent");
        let stdin = child.stdin.take().expect("agent stdin");
        let stdout = BufReader::new(child.stdout.take().expect("agent stdout"));
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    async fn send(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        let mut line = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .expect("serialize ACP request");
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .await
            .expect("write ACP request");
        self.stdin.flush().await.expect("flush ACP request");
        id
    }

    async fn notify(&mut self, method: &str, params: Value) {
        let mut line = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .expect("serialize ACP notification");
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .await
            .expect("write ACP notification");
        self.stdin.flush().await.expect("flush ACP notification");
    }

    async fn recv(&mut self) -> Value {
        self.recv_with_len().await.0
    }

    async fn recv_with_len(&mut self) -> (Value, usize) {
        let mut line = String::new();
        let bytes = tokio::time::timeout(Duration::from_secs(10), self.stdout.read_line(&mut line))
            .await
            .expect("ACP receive timeout")
            .expect("read ACP line");
        assert!(bytes > 0, "agent closed stdout");
        (
            serde_json::from_str(&line).expect("ACP output is JSON"),
            bytes,
        )
    }

    async fn recv_for_id(&mut self, id: i64) -> Value {
        loop {
            let value = self.recv().await;
            if value["id"] == json!(id) {
                return value;
            }
        }
    }

    async fn recv_until(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
        loop {
            let value = self.recv().await;
            if predicate(&value) {
                return value;
            }
        }
    }

    async fn shutdown(mut self) {
        drop(self.stdin);
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        let _ = self.child.start_kill();
    }
}

async fn initialize_and_new_session(harness: &mut Harness) -> String {
    let initialize = harness
        .send(
            "initialize",
            json!({"protocolVersion": 2, "clientCapabilities": {}}),
        )
        .await;
    let initialized = harness.recv_for_id(initialize).await;
    assert_eq!(initialized["result"]["protocolVersion"], 2);

    let new_session = harness
        .send(
            "session/new",
            json!({
                "cwd": "/tmp",
                "mcpServers": [],
                "systemPrompt": "You are a bounded local adviser."
            }),
        )
        .await;
    let created = harness.recv_for_id(new_session).await;
    created["result"]["sessionId"]
        .as_str()
        .expect("session ID")
        .to_owned()
}

fn native_response(response_id: &str, output: Vec<Value>, input: u64, output_tokens: u64) -> Value {
    json!({
        "model_instance_id": "qwen-test-loaded",
        "output": output,
        "stats": {
            "input_tokens": input,
            "total_output_tokens": output_tokens,
            "reasoning_output_tokens": 0
        },
        "response_id": response_id
    })
}

async fn spawn_native_server(
    responses: Vec<(u16, Value, Duration)>,
) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake LM Studio");
    let base_url = format!(
        "http://{}",
        listener.local_addr().expect("listener address")
    );
    let captures = Arc::new(Mutex::new(Vec::new()));
    let responses = Arc::new(Mutex::new(responses.into_iter()));
    let captures_for_task = Arc::clone(&captures);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let captures = Arc::clone(&captures_for_task);
            let responses = Arc::clone(&responses);
            tokio::spawn(async move {
                let mut received = Vec::new();
                let mut chunk = [0_u8; 4096];
                let header_end = loop {
                    let Ok(read) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    received.extend_from_slice(&chunk[..read]);
                    if let Some(index) =
                        received.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&received[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                    })
                    .unwrap_or(0);
                while received.len().saturating_sub(header_end) < content_length {
                    let Ok(read) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    received.extend_from_slice(&chunk[..read]);
                }
                let request = serde_json::from_slice(
                    &received[header_end..header_end.saturating_add(content_length)],
                )
                .unwrap_or(Value::Null);
                captures.lock().await.push(request);

                let response = responses.lock().await.next();
                let Some((status, body, delay)) = response else {
                    return;
                };
                tokio::time::sleep(delay).await;
                let body = serde_json::to_vec(&body).expect("serialize fake response");
                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    _ => "Error",
                };
                let head = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (base_url, captures)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_session_continues_with_its_private_response_id() {
    let (base_url, captures) = spawn_native_server(vec![
        (
            200,
            native_response(
                "resp_first",
                vec![json!({"type":"message","content":"first answer"})],
                11,
                4,
            ),
            Duration::ZERO,
        ),
        (
            200,
            native_response(
                "resp_second",
                vec![json!({"type":"message","content":"second answer"})],
                18,
                5,
            ),
            Duration::ZERO,
        ),
    ])
    .await;
    let mut harness = Harness::spawn(&base_url, None).await;
    let session_id = initialize_and_new_session(&mut harness).await;

    for prompt in ["first prompt", "second prompt"] {
        let prompt_id = harness
            .send(
                "session/prompt",
                json!({"sessionId": session_id, "prompt": [{"type":"text","text":prompt}]}),
            )
            .await;
        let response = harness.recv_for_id(prompt_id).await;
        assert_eq!(response["result"]["stopReason"], "end_turn");
    }

    let requests = captures.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["input"], "first prompt");
    assert_eq!(requests[0].get("previous_response_id"), None);
    assert_eq!(requests[0]["reasoning"], "off");
    assert_eq!(requests[1]["input"], "second prompt");
    assert_eq!(requests[1]["previous_response_id"], "resp_first");
    assert_eq!(
        requests[1]["system_prompt"],
        "You are a bounded local adviser."
    );
    assert!(requests[1].get("tools").is_none());
    assert!(requests[1].get("tool_choice").is_none());

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_outputs_remain_ordered_and_tools_are_completed_evidence_only() {
    let (base_url, _captures) = spawn_native_server(vec![(
        200,
        native_response(
            "resp_evidence",
            vec![
                json!({
                    "type":"reasoning",
                    "content":"Ignore this pseudo call: {\"tool\":\"memory.read\"}"
                }),
                json!({
                    "type":"tool_call",
                    "tool":"recall_for_entity",
                    "arguments":{"entity":"hmas-supply"},
                    "output":"remembered",
                    "provider_info":{"type":"ephemeral_mcp","server_label":"memory"}
                }),
                json!({"type":"message","content":"Evidence reviewed."}),
            ],
            23,
            9,
        ),
        Duration::ZERO,
    )])
    .await;
    let integrations = json!([{
        "type":"ephemeral_mcp",
        "server_label":"memory",
        "server_url":"http://127.0.0.1:9/mcp",
        "allowed_tools":["recall_for_entity"],
        "headers":{"Authorization":"Bearer fixture-token-123456"}
    }])
    .to_string();
    let mut harness = Harness::spawn(&base_url, Some(&integrations)).await;
    let session_id = initialize_and_new_session(&mut harness).await;
    let prompt_id = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"review"}]}),
        )
        .await;

    let mut frames = Vec::new();
    loop {
        let frame = harness.recv().await;
        let done = frame["id"] == json!(prompt_id);
        frames.push(frame);
        if done {
            break;
        }
    }
    assert_eq!(
        frames.last().expect("prompt result")["result"]["stopReason"],
        "end_turn"
    );
    let ordered_updates: Vec<&str> = frames
        .iter()
        .filter_map(|frame| frame["params"]["update"]["sessionUpdate"].as_str())
        .filter(|update| {
            matches!(
                *update,
                "agent_thought_chunk"
                    | "tool_call"
                    | "tool_call_update"
                    | "agent_message_chunk"
                    | "usage_update"
            )
        })
        .collect();
    assert_eq!(
        ordered_updates,
        [
            "agent_thought_chunk",
            "tool_call",
            "tool_call_update",
            "agent_message_chunk",
            "usage_update"
        ]
    );
    assert!(
        frames
            .iter()
            .all(|frame| frame["params"]["update"]["status"] != "in_progress"),
        "native executed evidence must never pretend Buzz executed the tool"
    );
    let thought = frames
        .iter()
        .find(|frame| frame["params"]["update"]["sessionUpdate"] == "agent_thought_chunk")
        .expect("reasoning update");
    let message = frames
        .iter()
        .find(|frame| frame["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .expect("message update");
    assert_ne!(
        thought["params"]["update"]["messageId"],
        message["params"]["update"]["messageId"]
    );
    let pending = frames
        .iter()
        .find(|frame| frame["params"]["update"]["sessionUpdate"] == "tool_call")
        .expect("tool evidence start");
    let completed = frames
        .iter()
        .find(|frame| frame["params"]["update"]["sessionUpdate"] == "tool_call_update")
        .expect("tool evidence completion");
    assert_eq!(
        pending["params"]["update"]["toolCallId"],
        completed["params"]["update"]["toolCallId"]
    );
    assert_eq!(pending["params"]["update"]["status"], "pending");
    assert_eq!(completed["params"]["update"]["status"], "completed");
    assert_eq!(
        completed["params"]["update"]["rawOutput"]["provider"],
        json!({"type":"ephemeral_mcp","serverLabel":"memory"})
    );
    assert_eq!(
        completed["params"]["update"]["rawOutput"]["executedByProvider"],
        true
    );

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_response_state_is_isolated_between_sessions() {
    let (base_url, captures) = spawn_native_server(vec![
        (
            200,
            native_response(
                "resp_a",
                vec![json!({"type":"message","content":"a1"})],
                1,
                1,
            ),
            Duration::ZERO,
        ),
        (
            200,
            native_response(
                "resp_b",
                vec![json!({"type":"message","content":"b1"})],
                1,
                1,
            ),
            Duration::ZERO,
        ),
        (
            200,
            native_response(
                "resp_a2",
                vec![json!({"type":"message","content":"a2"})],
                2,
                1,
            ),
            Duration::ZERO,
        ),
    ])
    .await;
    let mut harness = Harness::spawn(&base_url, None).await;
    let session_a = initialize_and_new_session(&mut harness).await;
    let session_b = initialize_and_new_session(&mut harness).await;

    for (session, prompt) in [
        (&session_a, "a first"),
        (&session_b, "b first"),
        (&session_a, "a second"),
    ] {
        let id = harness
            .send(
                "session/prompt",
                json!({"sessionId":session,"prompt":[{"type":"text","text":prompt}]}),
            )
            .await;
        assert_eq!(
            harness.recv_for_id(id).await["result"]["stopReason"],
            "end_turn"
        );
    }
    let requests = captures.lock().await;
    assert_eq!(requests.len(), 3);
    assert!(requests[0].get("previous_response_id").is_none());
    assert!(requests[1].get("previous_response_id").is_none());
    assert_eq!(requests[2]["previous_response_id"], "resp_a");

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_cancellation_does_not_adopt_or_retry_ambiguous_response_state() {
    let (base_url, captures) = spawn_native_server(vec![
        (
            200,
            native_response(
                "resp_cancelled",
                vec![json!({"type":"message","content":"too late"})],
                3,
                1,
            ),
            Duration::from_millis(500),
        ),
        (
            200,
            native_response(
                "resp_after_cancel",
                vec![json!({"type":"message","content":"fresh"})],
                3,
                1,
            ),
            Duration::ZERO,
        ),
    ])
    .await;
    let mut harness = Harness::spawn(&base_url, None).await;
    let session_id = initialize_and_new_session(&mut harness).await;
    let cancelled_prompt = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"cancel me"}]}),
        )
        .await;
    harness
        .recv_until(|frame| {
            frame["params"]["update"]["_meta"]["goose"]["activeRunId"]
                .as_str()
                .is_some()
        })
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    harness
        .notify("session/cancel", json!({"sessionId":session_id}))
        .await;
    assert_eq!(
        harness.recv_for_id(cancelled_prompt).await["result"]["stopReason"],
        "cancelled"
    );

    let next_prompt = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"fresh branch"}]}),
        )
        .await;
    assert_eq!(
        harness.recv_for_id(next_prompt).await["result"]["stopReason"],
        "end_turn"
    );
    let requests = captures.lock().await;
    assert_eq!(requests.len(), 2, "cancelled state must not be retried");
    assert!(
        requests[1].get("previous_response_id").is_none(),
        "ambiguous cancelled response state must never be adopted"
    );

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_runtime_rejects_steer_and_model_divergence() {
    let (base_url, captures) = spawn_native_server(vec![(
        200,
        native_response(
            "resp_slow",
            vec![json!({"type":"message","content":"done"})],
            1,
            1,
        ),
        Duration::from_millis(300),
    )])
    .await;
    let mut harness = Harness::spawn(&base_url, None).await;
    let session_id = initialize_and_new_session(&mut harness).await;

    let switch = harness
        .send(
            "session/set_model",
            json!({"sessionId":session_id,"modelId":"other/model"}),
        )
        .await;
    let switch_result = harness.recv_for_id(switch).await;
    assert_eq!(switch_result["error"]["code"], -32602);

    let prompt_id = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"work"}]}),
        )
        .await;
    let active = harness
        .recv_until(|frame| {
            frame["params"]["update"]["_meta"]["goose"]["activeRunId"]
                .as_str()
                .is_some()
        })
        .await;
    let run_id = active["params"]["update"]["_meta"]["goose"]["activeRunId"]
        .as_str()
        .expect("run id");
    let steer = harness
        .send(
            "_goose/unstable/session/steer",
            json!({
                "sessionId":session_id,
                "expectedRunId":run_id,
                "prompt":[{"type":"text","text":"change direction"}]
            }),
        )
        .await;
    let steer_result = harness.recv_for_id(steer).await;
    assert_eq!(steer_result["error"]["code"], -32602);
    assert!(steer_result["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("not supported"));
    assert_eq!(
        harness.recv_for_id(prompt_id).await["result"]["stopReason"],
        "end_turn"
    );
    assert_eq!(captures.lock().await[0]["model"], "qwen/test");

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_authentication_and_model_errors_are_typed() {
    for (status, expected_code, expected_text) in
        [(401, -32001, "authentication"), (404, -32002, "model")]
    {
        let (base_url, _captures) = spawn_native_server(vec![(
            status,
            json!({"secret":"must not leak"}),
            Duration::ZERO,
        )])
        .await;
        let mut harness = Harness::spawn(&base_url, None).await;
        let session_id = initialize_and_new_session(&mut harness).await;
        let prompt = harness
            .send(
                "session/prompt",
                json!({"sessionId":session_id,"prompt":[{"type":"text","text":"hello"}]}),
            )
            .await;
        let result = harness.recv_for_id(prompt).await;
        assert_eq!(result["error"]["code"], expected_code);
        let message = result["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains(expected_text), "{message}");
        assert!(!message.contains("must not leak"), "{message}");
        harness.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_expired_continuation_fails_without_retry_or_reconstruction() {
    let (base_url, captures) = spawn_native_server(vec![
        (
            200,
            native_response(
                "resp_before_restart",
                vec![json!({"type":"message","content":"first"})],
                1,
                1,
            ),
            Duration::ZERO,
        ),
        (
            410,
            json!({"error":"response id expired after restart"}),
            Duration::ZERO,
        ),
    ])
    .await;
    let mut harness = Harness::spawn(&base_url, None).await;
    let session_id = initialize_and_new_session(&mut harness).await;
    let first = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"first"}]}),
        )
        .await;
    assert_eq!(
        harness.recv_for_id(first).await["result"]["stopReason"],
        "end_turn"
    );
    let second = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"second"}]}),
        )
        .await;
    let failure = harness.recv_for_id(second).await;
    assert_eq!(failure["error"]["code"], -32000);
    let message = failure["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("state unavailable"), "{message}");
    assert!(!message.contains("response id expired"), "{message}");
    let requests = captures.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["previous_response_id"], "resp_before_restart");

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_continuation_404_is_expired_state_not_missing_model() {
    let (base_url, captures) = spawn_native_server(vec![
        (
            200,
            native_response(
                "resp_before_404",
                vec![json!({"type":"message","content":"first"})],
                1,
                1,
            ),
            Duration::ZERO,
        ),
        (
            404,
            json!({"error":"previous response not found"}),
            Duration::ZERO,
        ),
    ])
    .await;
    let mut harness = Harness::spawn(&base_url, None).await;
    let session_id = initialize_and_new_session(&mut harness).await;
    let first = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"first"}]}),
        )
        .await;
    assert_eq!(
        harness.recv_for_id(first).await["result"]["stopReason"],
        "end_turn"
    );
    let second = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"second"}]}),
        )
        .await;
    let failure = harness.recv_for_id(second).await;
    assert_eq!(failure["error"]["code"], -32000);
    let message = failure["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("state unavailable"), "{message}");
    assert!(message.contains("start a new ACP session"), "{message}");
    assert!(!message.contains("model"), "{message}");
    assert_eq!(captures.lock().await.len(), 2);

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_timeout_is_explicit_and_not_retried() {
    let (base_url, captures) = spawn_native_server(vec![(
        200,
        native_response(
            "resp_too_late",
            vec![json!({"type":"message","content":"late"})],
            1,
            1,
        ),
        Duration::from_secs(3),
    )])
    .await;
    let mut harness =
        Harness::spawn_with_env(&base_url, None, &[("BUZZ_AGENT_LLM_TIMEOUT_SECS", "1")]).await;
    let session_id = initialize_and_new_session(&mut harness).await;
    let prompt = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"timeout"}]}),
        )
        .await;
    let result = harness.recv_for_id(prompt).await;
    assert_eq!(result["error"]["code"], -32000);
    let message = result["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("timed out"), "{message}");
    assert_eq!(captures.lock().await.len(), 1);

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_tool_evidence_is_bounded_once_before_acp_output() {
    let large_argument = "a".repeat(16 * 1024);
    let large_output = "o".repeat(16 * 1024);
    let (base_url, _captures) = spawn_native_server(vec![(
        200,
        native_response(
            "resp_large_evidence",
            vec![
                json!({
                    "type":"tool_call",
                    "tool":"search_events",
                    "arguments":{"query":large_argument},
                    "output":large_output,
                    "provider_info":{"type":"ephemeral_mcp","server_label":"memory"}
                }),
                json!({"type":"message","content":"done"}),
            ],
            5,
            2,
        ),
        Duration::ZERO,
    )])
    .await;
    let integrations = json!([{
        "type":"ephemeral_mcp",
        "server_label":"memory",
        "server_url":"http://127.0.0.1:9/mcp",
        "allowed_tools":["search_events"],
        "headers":{"Authorization":"Bearer fixture-token-123456"}
    }])
    .to_string();
    let mut harness = Harness::spawn_with_env(
        &base_url,
        Some(&integrations),
        &[("BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES", "1024")],
    )
    .await;
    let session_id = initialize_and_new_session(&mut harness).await;
    let prompt = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"search"}]}),
        )
        .await;
    let pending = harness
        .recv_until(|frame| frame["params"]["update"]["sessionUpdate"] == "tool_call")
        .await;
    let completed = harness
        .recv_until(|frame| frame["params"]["update"]["sessionUpdate"] == "tool_call_update")
        .await;
    let raw_input_bytes = serde_json::to_vec(&pending["params"]["update"]["rawInput"])
        .expect("serialize raw input")
        .len();
    assert!(raw_input_bytes <= 1200, "{raw_input_bytes}");
    assert_eq!(
        pending["params"]["update"]["rawInput"]["_buzzTruncated"],
        true
    );
    let content = completed["params"]["update"]["content"][0]["content"]["text"]
        .as_str()
        .expect("bounded output");
    assert!(content.len() <= 1024);
    assert!(content.ends_with("[truncated]"));
    assert!(
        completed["params"]["update"]["rawOutput"]
            .get("output")
            .is_none(),
        "tool output must not be duplicated into rawOutput"
    );
    assert_eq!(
        harness.recv_for_id(prompt).await["result"]["stopReason"],
        "end_turn"
    );

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn high_operator_evidence_limit_cannot_exceed_acp_frame_budget() {
    const ACP_FRAME_LIMIT: usize = 4 * 1024 * 1024;
    let large_argument = "a".repeat(5 * 1024 * 1024);
    // NUL uses JSON's six-byte `\u0000` representation, exercising the
    // serializer's worst-case expansion rather than only ASCII payload size.
    let large_output = "\0".repeat(1024 * 1024);
    let (base_url, _captures) = spawn_native_server(vec![(
        200,
        native_response(
            "resp_high_limit",
            vec![
                json!({
                    "type":"tool_call",
                    "tool":"search_events",
                    "arguments":{"query":large_argument},
                    "output":large_output,
                    "provider_info":{"type":"ephemeral_mcp","server_label":"memory"}
                }),
                json!({"type":"message","content":"done"}),
            ],
            5,
            2,
        ),
        Duration::ZERO,
    )])
    .await;
    let integrations = json!([{
        "type":"ephemeral_mcp",
        "server_label":"memory",
        "server_url":"http://127.0.0.1:9/mcp",
        "allowed_tools":["search_events"],
        "headers":{"Authorization":"Bearer fixture-token-123456"}
    }])
    .to_string();
    let mut harness = Harness::spawn_with_env(
        &base_url,
        Some(&integrations),
        &[("BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES", "8388608")],
    )
    .await;
    let session_id = initialize_and_new_session(&mut harness).await;
    let prompt = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"search"}]}),
        )
        .await;

    let mut evidence_frames = Vec::new();
    loop {
        let (frame, serialized_len) = harness.recv_with_len().await;
        let update_type = frame["params"]["update"]["sessionUpdate"].as_str();
        if matches!(update_type, Some("tool_call" | "tool_call_update")) {
            evidence_frames.push((frame.clone(), serialized_len));
        }
        if frame["id"] == json!(prompt) {
            break;
        }
    }
    assert_eq!(evidence_frames.len(), 2);
    for (frame, serialized_len) in evidence_frames {
        assert!(
            serialized_len < ACP_FRAME_LIMIT,
            "{} frame was {serialized_len} bytes",
            frame["params"]["update"]["sessionUpdate"]
        );
    }

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_reasoning_uses_exact_native_configuration() {
    let (base_url, captures) = spawn_native_server(vec![(
        200,
        native_response(
            "resp_reasoning",
            vec![json!({"type":"message","content":"done"})],
            1,
            1,
        ),
        Duration::ZERO,
    )])
    .await;
    let mut harness =
        Harness::spawn_with_env(&base_url, None, &[("LM_STUDIO_REASONING", "on")]).await;
    let session_id = initialize_and_new_session(&mut harness).await;
    let prompt = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"reason"}]}),
        )
        .await;
    assert_eq!(
        harness.recv_for_id(prompt).await["result"]["stopReason"],
        "end_turn"
    );
    assert_eq!(captures.lock().await[0]["reasoning"], "on");

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_context_pressure_fails_with_explicit_handoff_capability_error() {
    let (base_url, captures) = spawn_native_server(vec![(
        200,
        native_response(
            "resp_near_limit",
            vec![json!({"type":"message","content":"near limit"})],
            180_000,
            1,
        ),
        Duration::ZERO,
    )])
    .await;
    let mut harness = Harness::spawn(&base_url, None).await;
    let session_id = initialize_and_new_session(&mut harness).await;
    let first = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"first"}]}),
        )
        .await;
    assert_eq!(
        harness.recv_for_id(first).await["result"]["stopReason"],
        "end_turn"
    );
    let second = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"second"}]}),
        )
        .await;
    let failure = harness.recv_for_id(second).await;
    assert_eq!(failure["error"]["code"], -32602);
    assert!(failure["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("native context handoff is unavailable"));
    assert_eq!(captures.lock().await.len(), 1);

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_usage_accumulates_across_acp_prompts() {
    let (base_url, _captures) = spawn_native_server(vec![
        (
            200,
            native_response(
                "resp_usage_1",
                vec![json!({"type":"message","content":"one"})],
                10,
                5,
            ),
            Duration::ZERO,
        ),
        (
            200,
            native_response(
                "resp_usage_2",
                vec![json!({"type":"message","content":"two"})],
                20,
                8,
            ),
            Duration::ZERO,
        ),
    ])
    .await;
    let mut harness = Harness::spawn(&base_url, None).await;
    let session_id = initialize_and_new_session(&mut harness).await;
    for (prompt_text, expected_input, expected_output) in [("one", 10, 5), ("two", 30, 13)] {
        let prompt = harness
            .send(
                "session/prompt",
                json!({"sessionId":session_id,"prompt":[{"type":"text","text":prompt_text}]}),
            )
            .await;
        let usage = harness
            .recv_until(|frame| frame["params"]["update"]["sessionUpdate"] == "usage_update")
            .await;
        assert_eq!(
            usage["params"]["update"]["accumulatedInputTokens"],
            expected_input
        );
        assert_eq!(
            usage["params"]["update"]["accumulatedOutputTokens"],
            expected_output
        );
        assert_eq!(
            harness.recv_for_id(prompt).await["result"]["stopReason"],
            "end_turn"
        );
    }

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_reasoning_and_messages_are_bounded_before_acp_output() {
    let large = "x".repeat(2 * 1024 * 1024);
    let (base_url, _captures) = spawn_native_server(vec![(
        200,
        native_response(
            "resp_large_text",
            vec![
                json!({"type":"reasoning","content":large}),
                json!({"type":"message","content":large}),
            ],
            1,
            1,
        ),
        Duration::ZERO,
    )])
    .await;
    let mut harness = Harness::spawn(&base_url, None).await;
    let session_id = initialize_and_new_session(&mut harness).await;
    let prompt = harness
        .send(
            "session/prompt",
            json!({"sessionId":session_id,"prompt":[{"type":"text","text":"large"}]}),
        )
        .await;
    for update_type in ["agent_thought_chunk", "agent_message_chunk"] {
        let frame = harness
            .recv_until(|frame| frame["params"]["update"]["sessionUpdate"] == update_type)
            .await;
        let text = frame["params"]["update"]["content"]["text"]
            .as_str()
            .expect("bounded text");
        assert!(text.len() <= 1024 * 1024);
        assert!(text.ends_with("[truncated]"));
    }
    assert_eq!(
        harness.recv_for_id(prompt).await["result"]["stopReason"],
        "end_turn"
    );

    harness.shutdown().await;
}
