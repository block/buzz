//! Real HTTP/SSE -> agent subprocess -> ACP streaming, with causal gates rather
//! than timing assertions: the provider cannot finish until ACP shows its delta.
mod common;

use common::Harness;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};

struct Reply {
    prefix: String,
    suffix: String,
    gate: Option<Arc<Notify>>,
}

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(replies: Vec<Reply>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let queue = queue.clone();
            let captured = captured.clone();
            connections.spawn(async move {
                let mut request = Vec::new();
                let mut chunk = [0u8; 8192];
                let header_end = loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 { return; }
                    request.extend_from_slice(&chunk[..n]);
                    assert!(request.len() < 4 * 1024 * 1024);
                    if let Some(end) = request.windows(4).position(|s| s == b"\r\n\r\n") { break end + 4; }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                if headers.starts_with("GET ") {
                    let body = r#"{"endpoints":[],"data":[],"models":[]}"#;
                    let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(reply.as_bytes()).await;
                    return;
                }
                let length: usize = headers.lines().find_map(|s| s.to_ascii_lowercase()
                    .strip_prefix("content-length:").and_then(|n| n.trim().parse().ok())).unwrap_or(0);
                while request.len() < header_end + length {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 { return; }
                    request.extend_from_slice(&chunk[..n]);
                }
                captured.lock().await.push(serde_json::from_slice(&request[header_end..header_end + length]).unwrap());
                let Some(reply) = queue.lock().await.pop_front() else { return; };
                let header = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.prefix.len() + reply.suffix.len());
                if socket.write_all(header.as_bytes()).await.is_err() { return; }
                if socket.write_all(reply.prefix.as_bytes()).await.is_err() { return; }
                if let Some(gate) = reply.gate { gate.notified().await; }
                let _ = socket.write_all(reply.suffix.as_bytes()).await;
            });
        }
    });
    Server {
        url,
        requests,
        task,
    }
}

fn event(value: Value) -> String {
    format!("data: {value}\r\n\r\n")
}

fn chat_delta(text: &str) -> String {
    event(json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}))
}

fn chat_end(reason: &str) -> String {
    event(json!({"choices":[{"index":0,"delta":{},"finish_reason":reason}]}))
        + &event(
            json!({"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}),
        )
        + "data: [DONE]\r\n\r\n"
}

async fn session(h: &mut Harness, tools: bool) -> String {
    let id = h
        .send(
            "initialize",
            json!({"protocolVersion":2,"clientCapabilities":{}}),
        )
        .await;
    assert!(h
        .recv_until(|v| v["id"] == id)
        .await
        .get("result")
        .is_some());
    let servers = if tools {
        json!([{"name":"probe", "command":env!("CARGO_BIN_EXE_fake-mcp"),"args":[],"env":[]}])
    } else {
        json!([])
    };
    let id = h
        .send("session/new", json!({"cwd":"/tmp","mcpServers":servers}))
        .await;
    h.recv_until(|v| v["id"] == id).await["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn prompt(h: &mut Harness, sid: &str) -> i64 {
    h.send(
        "session/prompt",
        json!({"sessionId":sid,"prompt":[{"type":"text","text":"go"}]}),
    )
    .await
}

fn text(v: &Value) -> Option<&str> {
    (v["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .then(|| v["params"]["update"]["content"]["text"].as_str())
        .flatten()
}

#[tokio::test]
async fn providers_stream_before_completion_without_duplicate_text_and_keep_usage() {
    for provider in [
        "openai",
        "openrouter",
        "responses",
        "anthropic",
        "databricks",
        "db-responses",
        "db-anthropic",
        "db-chat",
    ] {
        let gate = Arc::new(Notify::new());
        let (prefix, suffix) = match provider {
            "responses" | "db-responses" => (
                event(json!({"type":"response.output_text.delta","output_index":0,"delta":"one "})),
                event(
                    json!({"type":"response.output_text.delta","output_index":0,"delta":"two 🐝"}),
                ) + &event(json!({
                    "type":"response.completed", "response":{"status":"completed", "output":[{"type":"message","content":[{"type":"output_text","text":"one two 🐝"}]}],
                    "usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}
                })),
            ),
            "anthropic" | "db-anthropic" => (
                event(
                    json!({"type":"message_start","message":{"content":[],"usage":{"input_tokens":7}}}),
                ) + &event(
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                ) + &event(
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"one "}}),
                ),
                event(
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"two 🐝"}}),
                ) + &event(json!({"type":"content_block_stop","index":0}))
                    + &event(
                        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}),
                    )
                    + &event(json!({"type":"message_stop"})),
            ),
            _ => (chat_delta("one "), chat_delta("two 🐝") + &chat_end("stop")),
        };
        let server = server(vec![Reply {
            prefix,
            suffix,
            gate: Some(gate.clone()),
        }])
        .await;
        let extra: Vec<(&str, &str)> = match provider {
            "databricks" | "db-responses" | "db-anthropic" | "db-chat" => vec![
                (
                    "BUZZ_AGENT_PROVIDER",
                    if provider == "databricks" {
                        "databricks"
                    } else {
                        "databricks_v2"
                    },
                ),
                ("DATABRICKS_TOKEN", "test"),
                ("DATABRICKS_HOST", &server.url),
                (
                    "BUZZ_AGENT_MODEL",
                    match provider {
                        "db-responses" => "databricks-gpt-5-4",
                        "db-anthropic" => "databricks-claude-opus-4-6",
                        _ => "fake-model",
                    },
                ),
            ],
            "anthropic" => vec![
                ("BUZZ_AGENT_PROVIDER", "anthropic"),
                ("ANTHROPIC_API_KEY", "test"),
                ("ANTHROPIC_BASE_URL", &server.url),
                ("BUZZ_AGENT_MODEL", "claude-sonnet-4-5"),
            ],
            "openrouter" => vec![
                ("BUZZ_AGENT_PROVIDER", "openrouter"),
                ("OPENROUTER_API_KEY", "test"),
                ("OPENROUTER_BASE_URL", &server.url),
                ("OPENROUTER_MODEL", "test-model"),
            ],
            "responses" => vec![("OPENAI_COMPAT_API", "responses")],
            _ => vec![("OPENAI_COMPAT_API", "chat")],
        };
        let mut h = Harness::spawn_with_env(&server.url, &extra).await;
        let sid = session(&mut h, false).await;
        let id = prompt(&mut h, &sid).await;
        let first = h.recv_until(|v| text(v).is_some() || v["id"] == id).await;
        assert_eq!(
            text(&first),
            Some("one "),
            "{provider}: {first}; {}",
            h.stderr_text()
        );
        let message_id = first["params"]["update"]["messageId"].clone();
        assert!(message_id.is_string());
        let request = server.requests.lock().await[0].clone();
        assert_eq!(request["stream"], true, "{provider}: {request}");
        gate.notify_one();
        let mut output = String::from("one ");
        let mut usage = None;
        loop {
            let v = h.recv().await;
            if let Some(part) = text(&v) {
                assert_eq!(v["params"]["update"]["messageId"], message_id);
                output.push_str(part);
            }
            if v["params"]["update"]["sessionUpdate"] == "usage_update" {
                usage = Some(v["params"]["update"].clone());
            }
            if v["id"] == id {
                assert_eq!(v["result"]["stopReason"], "end_turn", "{provider}: {v}");
                break;
            }
        }
        assert_eq!(output, "one two 🐝", "{provider}");
        let usage = usage.expect("streaming response must retain usage");
        assert_eq!(usage["accumulatedInputTokens"], 7, "{provider}: {usage}");
        assert_eq!(usage["accumulatedOutputTokens"], 3, "{provider}: {usage}");
        assert_eq!(server.requests.lock().await.len(), 1);
        h.shutdown().await;
    }
}

#[tokio::test]
async fn terminal_event_finishes_even_if_the_http_body_stays_open() {
    let server = server(vec![Reply {
        prefix: chat_delta("done") + &chat_end("stop"),
        suffix: " ".into(),
        gate: Some(Arc::new(Notify::new())),
    }])
    .await;
    let mut h = Harness::spawn_with_env(&server.url, &[("OPENAI_COMPAT_API", "chat")]).await;
    let sid = session(&mut h, false).await;
    let id = prompt(&mut h, &sid).await;
    let reply = h.recv_until(|v| v["id"] == id).await;
    assert_eq!(reply["result"]["stopReason"], "end_turn", "{reply}");
    h.shutdown().await;
}

#[tokio::test]
async fn cancelled_and_broken_streams_do_not_execute_partial_tools_or_retry() {
    for cancel in [false, true] {
        let gate = Arc::new(Notify::new());
        let partial = chat_delta("partial")
            + &event(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
            "index":0,"id":"partial-call","type":"function","function":{"name":"probe__tool_0","arguments":"{\"x\":"}
        }]},"finish_reason":null}]}));
        let server = server(vec![
            Reply {
                prefix: partial,
                suffix: if cancel {
                    chat_end("tool_calls")
                } else {
                    String::new()
                },
                gate: cancel.then(|| gate.clone()),
            },
            Reply {
                prefix: chat_delta("recovered"),
                suffix: chat_end("stop"),
                gate: None,
            },
        ])
        .await;
        let mut h = Harness::spawn_with_env(&server.url, &[("OPENAI_COMPAT_API", "chat")]).await;
        let sid = session(&mut h, true).await;
        let id = prompt(&mut h, &sid).await;
        let first = h.recv_until(|v| text(v).is_some() || v["id"] == id).await;
        assert_eq!(text(&first), Some("partial"));
        if cancel {
            h.notify("session/cancel", json!({"sessionId":sid})).await;
        }
        loop {
            let v = h.recv().await;
            assert_ne!(
                v["method"], "session/request_permission",
                "partial tool reached permission broker"
            );
            assert_ne!(v["params"]["update"]["sessionUpdate"], "tool_call");
            if v["id"] == id {
                if cancel {
                    assert_eq!(v["result"]["stopReason"], "cancelled");
                } else {
                    assert!(v.get("error").is_some(), "{v}");
                }
                break;
            }
        }
        assert_eq!(server.requests.lock().await.len(), 1, "stream was retried");
        gate.notify_one();
        let id = prompt(&mut h, &sid).await;
        let reply = h.recv_until(|v| v["id"] == id).await;
        assert_eq!(reply["result"]["stopReason"], "end_turn", "{reply}");
        let requests = server.requests.lock().await;
        let history = requests[1]["messages"].as_array().unwrap();
        assert!(
            !history
                .iter()
                .any(|m| m["role"] == "assistant" || m["role"] == "tool"),
            "partial assistant/tool state entered history: {history:?}"
        );
        drop(requests);
        h.shutdown().await;
    }
}

#[tokio::test]
async fn completed_streams_execute_tools_and_replay_provider_state_once() {
    for provider in ["chat", "openrouter", "responses", "anthropic"] {
        let gate = Arc::new(Notify::new());
        let call = json!({"type":"function_call","call_id":"call-a","name":"probe__tool_0","arguments":"{\"n\":1}"});
        let message = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"running"}]});
        let (prefix, suffix, final_reply) = match provider {
            "responses" => (
                event(
                    json!({"type":"response.output_text.delta","output_index":0,"delta":"running"}),
                ) + &event(
                    json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"n\":"}),
                ),
                event(json!({"type":"response.output_item.done","output_index":0,"item":message}))
                    + &event(
                        json!({"type":"response.output_item.done","output_index":1,"item":call}),
                    )
                    + &event(
                        json!({"type":"response.completed","response":{"status":"completed"}}),
                    ),
                event(json!({"type":"response.output_text.delta","output_index":0,"delta":"done"}))
                    + &event(
                        json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"done"}]}]}}),
                    ),
            ),
            "anthropic" => (
                event(
                    json!({"type":"message_start","message":{"content":[],"usage":{"input_tokens":7}}}),
                ) + &event(
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"check","signature":""}}),
                ) + &event(
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed"}}),
                ) + &event(
                    json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
                ) + &event(
                    json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"running"}}),
                ) + &event(
                    json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"call-a","name":"probe__tool_0","input":{}}}),
                ) + &event(
                    json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"n\":"}}),
                ),
                event(
                    json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"1}"}}),
                ) + &event(json!({"type":"content_block_stop","index":2}))
                    + &event(
                        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}),
                    )
                    + &event(json!({"type":"message_stop"})),
                event(
                    json!({"type":"message_start","message":{"content":[],"usage":{"input_tokens":8}}}),
                ) + &event(
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"done"}}),
                ) + &event(
                    json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}),
                ) + &event(json!({"type":"message_stop"})),
            ),
            _ => (
                chat_delta("running")
                    + &event(json!({"choices":[{"index":0,"delta":{"reasoning_details":[
                        {"type":"reasoning.summary","index":0,"id":"summary-a","summary":"first "}
                    ]},"finish_reason":null}]}))
                    + &event(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
                    "index":0,"id":"call-a","type":"function","function":{"name":"probe__tool_0","arguments":"{\"n\":"}
                }]},"finish_reason":null}]})),
                event(
                    json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]},"finish_reason":null}]}),
                ) + &event(json!({"choices":[{"index":0,"delta":{"reasoning_details":[
                    {"type":"reasoning.summary","index":0,"id":"summary-a","summary":"second"}
                ]},"finish_reason":null}]}))
                    + &chat_end("tool_calls"),
                chat_delta("done") + &chat_end("stop"),
            ),
        };
        let server = server(vec![
            Reply {
                prefix,
                suffix,
                gate: Some(gate.clone()),
            },
            Reply {
                prefix: final_reply,
                suffix: String::new(),
                gate: None,
            },
        ])
        .await;
        let extra = match provider {
            "openrouter" => vec![
                ("BUZZ_AGENT_PROVIDER", "openrouter"),
                ("OPENROUTER_API_KEY", "test"),
                ("OPENROUTER_BASE_URL", server.url.as_str()),
                ("OPENROUTER_MODEL", "test-model"),
            ],
            "anthropic" => vec![
                ("BUZZ_AGENT_PROVIDER", "anthropic"),
                ("ANTHROPIC_API_KEY", "test"),
                ("ANTHROPIC_BASE_URL", server.url.as_str()),
                ("BUZZ_AGENT_MODEL", "claude-sonnet-4-5"),
            ],
            _ => vec![(
                "OPENAI_COMPAT_API",
                if provider == "responses" {
                    "responses"
                } else {
                    "chat"
                },
            )],
        };
        let mut h = Harness::spawn_with_env(&server.url, &extra).await;
        let sid = session(&mut h, true).await;
        let id = prompt(&mut h, &sid).await;
        let first = h.recv_until(|v| text(v).is_some() || v["id"] == id).await;
        assert_eq!(text(&first), Some("running"), "{provider}: {first}");
        gate.notify_one();
        let mut permissions = 0;
        loop {
            let v = h.recv().await;
            if v["method"] == "session/request_permission" {
                permissions += 1;
                h.write(common::approve_permission(&v)).await;
            }
            if v["id"] == id {
                assert_eq!(v["result"]["stopReason"], "end_turn", "{provider}: {v}");
                break;
            }
        }
        assert_eq!(
            permissions, 1,
            "{provider}: completed call must execute once"
        );
        let requests = server.requests.lock().await;
        assert_eq!(requests.len(), 2);
        let replay = &requests[1];
        match provider {
            "anthropic" => {
                let assistant = replay["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|v| v["role"] == "assistant")
                    .unwrap();
                assert_eq!(assistant["content"][0]["signature"], "signed");
                assert_eq!(assistant["content"][2]["input"], json!({"n":1}));
                assert_eq!(
                    replay["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|v| v["content"]
                            .as_array()
                            .is_some_and(|a| a.iter().any(|v| v["type"] == "tool_result")))
                        .count(),
                    1
                );
            }
            "responses" => {
                let items = replay["input"].as_array().unwrap();
                assert_eq!(
                    items
                        .iter()
                        .filter(|v| v["type"] == "function_call_output")
                        .count(),
                    1
                );
                assert_eq!(
                    items.iter().find(|v| v["type"] == "function_call").unwrap()["arguments"],
                    "{\"n\":1}"
                );
            }
            _ => {
                let messages = replay["messages"].as_array().unwrap();
                assert_eq!(messages.iter().filter(|v| v["role"] == "tool").count(), 1);
                assert_eq!(
                    messages.iter().find(|v| v["role"] == "assistant").unwrap()["tool_calls"][0]
                        ["function"]["arguments"],
                    "{\"n\":1}"
                );
                if provider == "openrouter" {
                    assert_eq!(
                        messages.iter().find(|v| v["role"] == "assistant").unwrap()
                            ["reasoning_details"][0]["summary"],
                        "first second"
                    );
                }
            }
        }
        drop(requests);
        h.shutdown().await;
    }
}
