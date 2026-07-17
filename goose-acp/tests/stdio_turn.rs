use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const RESPONSE: &str = r#"data: {"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}

data: {"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{"content":"2"},"finish_reason":null}]}

data: {"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: {"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":1,"total_tokens":11}}

data: [DONE]"#;

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct RpcClient {
    stdin: ChildStdin,
    lines: Receiver<String>,
    notifications: Vec<Value>,
}

impl RpcClient {
    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        writeln!(self.stdin, "{request}").expect("write ACP request");
        self.stdin.flush().expect("flush ACP request");

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(remaining)
                .unwrap_or_else(|error| panic!("timed out waiting for {method}: {error}"));
            let message: Value = serde_json::from_str(&line)
                .unwrap_or_else(|error| panic!("invalid ACP JSON ({error}): {line}"));

            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    panic!("{method} failed: {error}");
                }
                return message
                    .get("result")
                    .cloned()
                    .unwrap_or_else(|| panic!("{method} response omitted result: {message}"));
            }
            self.notifications.push(message);
        }
    }
}

#[test]
fn bundled_sidecar_completes_a_full_acp_turn() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake OpenAI server");
    let address = listener.local_addr().expect("fake OpenAI address");
    let (request_tx, request_rx) = mpsc::channel();
    let server = thread::spawn(move || serve_openai(listener, request_tx));

    let state = tempfile::tempdir().expect("temporary Goose state");
    let mut child = Command::new(env!("CARGO_BIN_EXE_goose-acp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .env("GOOSE_PATH_ROOT", state.path())
        .env("GOOSE_PROVIDER", "openai")
        .env("GOOSE_MODEL", "test-model")
        .env("GOOSE_MODE", "auto")
        .env("GOOSE_DISABLE_KEYRING", "true")
        .env("GOOSE_DISABLE_SESSION_NAMING", "true")
        .env("GOOSE_TELEMETRY_OFF", "true")
        .env("OPENAI_API_KEY", "test-key")
        .env("OPENAI_BASE_URL", format!("http://{address}/v1"))
        .spawn()
        .expect("spawn bundled Goose sidecar");

    let stdin = child.stdin.take().expect("sidecar stdin");
    let stdout = child.stdout.take().expect("sidecar stdout");
    let (line_tx, line_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if line_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let child = ChildGuard(child);
    let mut rpc = RpcClient {
        stdin,
        lines: line_rx,
        notifications: Vec::new(),
    };

    let initialized = rpc.request(
        1,
        "initialize",
        json!({
            "protocolVersion": 2,
            "clientCapabilities": {
                "_meta": { "goose": { "customNotifications": true } }
            },
            "clientInfo": { "name": "buzz-acp", "version": "test" }
        }),
    );
    assert_eq!(
        initialized
            .pointer("/agentInfo/name")
            .and_then(Value::as_str),
        Some("goose")
    );
    assert_eq!(
        initialized
            .pointer("/agentInfo/version")
            .and_then(Value::as_str),
        Some("1.43.0")
    );

    let session = rpc.request(
        2,
        "session/new",
        json!({ "cwd": state.path(), "mcpServers": [] }),
    );
    let session_id = session
        .get("sessionId")
        .and_then(Value::as_str)
        .expect("session/new sessionId")
        .to_string();
    let developer_ready = session
        .pointer("/_meta/extensionResults")
        .and_then(Value::as_array)
        .is_some_and(|results| {
            results.iter().any(|result| {
                result.get("name").and_then(Value::as_str) == Some("developer")
                    && result.get("success").and_then(Value::as_bool) == Some(true)
            })
        });
    assert!(
        developer_ready,
        "bundled developer tools did not initialize: {session}"
    );

    let prompt = rpc.request(
        3,
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "what is 1+1" }]
        }),
    );
    assert_eq!(
        prompt.get("stopReason").and_then(Value::as_str),
        Some("end_turn")
    );

    let assistant_text: String = rpc
        .notifications
        .iter()
        .filter(|message| message.get("method").and_then(Value::as_str) == Some("session/update"))
        .filter(|message| {
            message
                .pointer("/params/update/sessionUpdate")
                .and_then(Value::as_str)
                == Some("agent_message_chunk")
        })
        .filter_map(|message| {
            message
                .pointer("/params/update/content/text")
                .and_then(Value::as_str)
        })
        .collect();
    assert_eq!(assistant_text, "2");

    let provider_request = request_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("captured OpenAI-compatible request");
    assert!(provider_request.contains("POST /v1/chat/completions"));
    assert!(provider_request.contains("what is 1+1"));
    assert!(provider_request
        .to_ascii_lowercase()
        .contains("authorization: bearer test-key"));

    drop(rpc);
    drop(child);
    reader.join().expect("ACP stdout reader");
    server.join().expect("fake OpenAI server");
}

fn serve_openai(listener: TcpListener, request_tx: mpsc::Sender<String>) {
    listener
        .set_nonblocking(true)
        .expect("configure fake OpenAI listener");
    let deadline = Instant::now() + Duration::from_secs(30);

    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let request = read_http_request(&mut stream);
                if request.starts_with("GET /v1/models ") {
                    write_http_response(
                        &mut stream,
                        "application/json",
                        r#"{"object":"list","data":[{"id":"test-model","object":"model","created":0,"owned_by":"test"}]}"#,
                    );
                    continue;
                }
                if request.starts_with("POST /v1/chat/completions ") {
                    request_tx.send(request).expect("record provider request");
                    write_http_response(&mut stream, "text/event-stream", RESPONSE);
                    return;
                }
                panic!("unexpected provider request: {request}");
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("fake OpenAI accept failed: {error}"),
        }
    }
    panic!("Goose never called the fake OpenAI provider");
}

fn read_http_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("provider request timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut expected_len = None;

    loop {
        let count = stream.read(&mut buffer).expect("read provider request");
        assert!(count > 0, "provider closed request early");
        bytes.extend_from_slice(&buffer[..count]);

        if let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n") {
            let body_start = header_end + 4;
            let content_length = *expected_len.get_or_insert_with(|| {
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0)
            });
            if bytes.len() >= body_start + content_length {
                return String::from_utf8(bytes).expect("UTF-8 provider request");
            }
        }
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn write_http_response(stream: &mut TcpStream, content_type: &str, body: &str) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write fake provider response");
    stream.flush().expect("flush fake provider response");
}
