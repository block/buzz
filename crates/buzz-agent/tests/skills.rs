//! Proves `load_skill` and `AGENTS.md` hints survive the goose swap.
//!
//! Both were briefly lost in the port. Skills are now goose's `skills`
//! platform extension rather than a buzz reimplementation, and goose declares
//! it `unprefixed_tools: true`, so the model sees plain `load_skill` — not
//! `skills__load_skill`. buzz only renders the index, and an index naming a
//! tool that is not in the tool list is a silent failure, so this pins both
//! halves against each other.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

mod approve;

/// Fake OpenAI provider that asks for `load_skill` on the first turn, then
/// answers normally. Sends every observed `system` prompt and tool list back.
type Observed = (String, Vec<String>, String);

/// `(system prompt, tool names, whole request body)` for each provider call.
fn spawn_provider() -> (String, mpsc::Receiver<Observed>) {
    spawn_provider_requesting("widget-maker")
}

/// As [`spawn_provider`], but the first turn asks for `skill_name`. Used to
/// drive `load_skill` at a name the client is expected *not* to know.
fn spawn_provider_requesting(skill_name: &str) -> (String, mpsc::Receiver<Observed>) {
    let skill_name = skill_name.to_string();
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
                let _ = tx.send((system, tools, req.to_string()));
            }

            call_count += 1;
            let sse = if call_count == 1 {
                // Ask for the skill by its namespaced name.
                // goose never resolves this and the turn hangs.
                let c = json!({
                    "id":"c","object":"chat.completion.chunk","created":1,"model":"fake-model",
                    "choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{
                        "index":0,"id":"call_1","type":"function",
                        "function":{"name":"load_skill","arguments": format!("{{\"name\":\"{skill_name}\"}}")}
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
            // Answer the authorization gate so the tool under test actually
            // runs. This suite's subject is not the permission boundary (see
            // `permission_boundary.rs`), so approval is automatic.
            if approve::is_permission_request(&msg) {
                let response = approve::approve(&msg);
                writeln!(self.stdin, "{response}").expect("write approval");
                self.stdin.flush().expect("flush approval");
                continue;
            }
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
    // goose's hint loader walks up to the git root; give it one so the walk
    // terminates inside the temp workspace rather than in the real repo.
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
    let mut bodies = Vec::new();
    while let Ok((sys, tools, body)) = seen.recv_timeout(Duration::from_millis(500)) {
        systems.push(sys);
        tool_lists.push(tools);
        bodies.push(body);
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
        tool_lists[0].iter().any(|t| t == "load_skill"),
        "load_skill not advertised: {:?}",
        tool_lists[0]
    );
    assert!(
        systems.len() >= 2,
        "expected a second round after the tool result"
    );

    // The turn completing is not evidence the skill loaded. A `load_skill`
    // answering "Skill not found." also reaches `end_turn` with a second
    // round, so `stopReason` alone passes on a broken skills path. Assert on
    // the tool result the model was actually handed.
    let second = &bodies[1];
    assert!(
        second.contains("Step 2: cut the flange."),
        "skill body never reached the model as a tool result:\n{second}"
    );
    assert!(
        !second.contains("not found"),
        "load_skill failed to resolve the skill:\n{second}"
    );
}

/// goose's `SkillsClient` ships two skills compiled into the crate —
/// `goose-doc-guide` and `web-search` (`goose/src/skills/builtins/`). Neither
/// is a Buzz skill, and `web-search` tells the model to shell out to `uvx
/// ddgs`, a capability Buzz does not provide. buzz-agent on `main` had no such
/// thing, so registering the client with builtins on would have widened every
/// Buzz agent's advertised surface as a side effect of the goose swap.
///
/// This pins both halves: absent from the prompt index, and unresolvable
/// through the tool. Index-only would pass while the tool still served them.
#[test]
fn goose_builtin_skills_are_not_offered_to_buzz_agents() {
    let (base_url, seen) = spawn_provider_requesting("web-search");
    let home = tempfile::tempdir().expect("home");
    let ws = workspace();
    let mut h = Harness::start(&base_url, home.path());

    h.call("initialize", json!({"protocolVersion": 2}));
    let r = h.call(
        "session/new",
        json!({"cwd": ws.path().to_str().unwrap(), "mcpServers": []}),
    );
    let sid = r["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("session/new failed: {r}"))
        .to_string();

    let r = h.call(
        "session/prompt",
        json!({"sessionId": sid, "prompt": [{"type":"text","text":"search the web"}]}),
    );
    assert_eq!(r["result"]["stopReason"], "end_turn", "turn stalled: {r}");

    let mut systems = Vec::new();
    let mut bodies = Vec::new();
    while let Ok((sys, _tools, body)) = seen.recv_timeout(Duration::from_millis(500)) {
        systems.push(sys);
        bodies.push(body);
    }
    assert!(!systems.is_empty(), "provider was never called");

    let first = &systems[0];
    for builtin in ["web-search", "goose-doc-guide"] {
        assert!(
            !first.contains(builtin),
            "goose builtin {builtin:?} is advertised in the system prompt:\n{first}"
        );
    }
    // The filesystem skill still has to be there — an empty index would pass
    // the assertions above for the wrong reason.
    assert!(
        first.contains("widget-maker"),
        "filesystem skills were lost along with the builtins:\n{first}"
    );

    assert!(
        bodies.len() >= 2,
        "expected a second round carrying the tool result"
    );
    let second = &bodies[1];
    assert!(
        second.contains("not found"),
        "load_skill served a goose builtin:\n{second}"
    );
}
