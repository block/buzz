//! Proves `load_skill` and `AGENTS.md` hints survive the goose swap.
//!
//! Both were briefly lost in the port. `load_skill` is registered as a goose
//! *platform* extension backed by an `McpClientTrait` (the shape Maple uses),
//! so goose dispatches it on its ordinary tool path. Goose namespaces such
//! tools as `{extension}__{tool}`, which the system prompt has to match — a
//! prompt naming a tool that is not in the tool list is a silent failure.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

/// Fake OpenAI provider that asks for `load_skill` on the first turn, then
/// answers normally. Sends every observed `system` prompt and tool list back.
fn spawn_provider() -> (String, mpsc::Receiver<(String, Vec<String>)>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let mut call_count = 0usize;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));

            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    break;
                }
                let t = line.trim_end();
                if t.is_empty() {
                    break;
                }
                if let Some(v) = t
                    .strip_prefix("content-length: ")
                    .or_else(|| t.strip_prefix("Content-Length: "))
                {
                    len = v.parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len];
            use std::io::Read;
            let _ = reader.read_exact(&mut body);

            let send = |stream: &mut std::net::TcpStream, ct: &str, payload: String| {
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: {ct}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        payload.len(), payload
                    ).as_bytes(),
                );
                let _ = stream.flush();
            };

            if request_line.contains("/models") {
                send(
                    &mut stream,
                    "application/json",
                    json!({"object":"list","data":[{"id":"fake-model"}]}).to_string(),
                );
                continue;
            }

            if let Ok(req) = serde_json::from_slice::<Value>(&body) {
                let system = req["messages"]
                    .as_array()
                    .and_then(|m| {
                        m.iter()
                            .find(|x| x["role"] == "system")
                            .and_then(|x| x["content"].as_str())
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                let tools = req["tools"]
                    .as_array()
                    .map(|ts| {
                        ts.iter()
                            .filter_map(|t| t["function"]["name"].as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = tx.send((system, tools));
            }

            call_count += 1;
            let sse = if call_count == 1 {
                // Ask for the skill by its namespaced name.
                // goose never resolves this and the turn hangs.
                let c = json!({
                    "id":"c","object":"chat.completion.chunk","created":1,"model":"fake-model",
                    "choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{
                        "index":0,"id":"call_1","type":"function",
                        "function":{"name":"buzz__load_skill","arguments":"{\"name\":\"widget-maker\"}"}
                    }]},"finish_reason":null}]
                });
                let d = json!({
                    "id":"c","object":"chat.completion.chunk","created":1,"model":"fake-model",
                    "choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],
                    "usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}
                });
                format!("data: {c}\n\ndata: {d}\n\ndata: [DONE]\n\n")
            } else {
                let c = json!({
                    "id":"c","object":"chat.completion.chunk","created":1,"model":"fake-model",
                    "choices":[{"index":0,"delta":{"role":"assistant","content":"Got the skill."},"finish_reason":null}]
                });
                let d = json!({
                    "id":"c","object":"chat.completion.chunk","created":1,"model":"fake-model",
                    "choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
                    "usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}
                });
                format!("data: {c}\n\ndata: {d}\n\ndata: [DONE]\n\n")
            };
            send(&mut stream, "text/event-stream", sse);
        }
    });

    (format!("http://{addr}"), rx)
}

struct Harness {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    id: i64,
}

impl Harness {
    fn start(base_url: &str, home: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-agent"))
            .env("BUZZ_AGENT_PROVIDER", "openai-compat")
            .env("BUZZ_AGENT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_API_KEY", "k")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("cfg"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("GOOSE_DISABLE_KEYRING", "1")
            .env("RUST_LOG", "warn")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin,
            stdout,
            id: 0,
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let id = self.id;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .expect("write");
        self.stdin.flush().expect("flush");
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert_ne!(n, 0, "agent closed stdout awaiting {method}");
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if msg.get("id").and_then(Value::as_i64) == Some(id) {
                return msg;
            }
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Build a workspace with an AGENTS.md and one skill.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path();
    std::fs::write(root.join("AGENTS.md"), "Always ship the sprocket first.").unwrap();
    // hints.rs walks up to the git root; give it one so the walk terminates here.
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let skill = root.join(".agents/skills/widget-maker");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: widget-maker\ndescription: Makes widgets to order.\n---\n\
         Step 1: measure. Step 2: cut the flange.",
    )
    .unwrap();
    dir
}

#[test]
fn agents_md_and_skill_index_reach_the_model_and_load_skill_resolves() {
    let (base_url, seen) = spawn_provider();
    let home = tempfile::tempdir().expect("home");
    let ws = workspace();
    let mut h = Harness::start(&base_url, home.path());

    let r = h.call("initialize", json!({"protocolVersion": 2}));
    assert_eq!(r["result"]["agentInfo"]["name"], "buzz-agent");

    let r = h.call(
        "session/new",
        json!({"cwd": ws.path().to_str().unwrap(), "mcpServers": [],
               "systemPrompt": "You are Fizz."}),
    );
    let sid = r["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("session/new failed: {r}"))
        .to_string();

    // The turn only returns if load_skill was dispatched. A broken tool
    // path hangs here rather than erroring.
    let r = h.call(
        "session/prompt",
        json!({"sessionId": sid, "prompt": [{"type":"text","text":"use the widget skill"}]}),
    );
    assert_eq!(
        r["result"]["stopReason"], "end_turn",
        "turn did not complete — load_skill was probably never resolved: {r}"
    );

    let mut systems = Vec::new();
    let mut tool_lists = Vec::new();
    while let Ok((sys, tools)) = seen.recv_timeout(Duration::from_millis(500)) {
        systems.push(sys);
        tool_lists.push(tools);
    }
    assert!(!systems.is_empty(), "provider was never called");

    let first = &systems[0];
    assert!(
        first.contains("Always ship the sprocket first."),
        "AGENTS.md hints missing from system prompt:\n{first}"
    );
    assert!(
        first.contains("widget-maker") && first.contains("Makes widgets to order."),
        "skill index (name + description) missing from system prompt:\n{first}"
    );
    assert!(
        !first.contains("Step 2: cut the flange."),
        "skill BODY was inlined — the point of load_skill is that it is not:\n{first}"
    );
    assert!(
        tool_lists[0].iter().any(|t| t == "buzz__load_skill"),
        "load_skill not advertised: {:?}",
        tool_lists[0]
    );
    assert!(
        systems.len() >= 2,
        "expected a second round after the tool result"
    );
}
