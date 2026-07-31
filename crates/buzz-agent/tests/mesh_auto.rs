//! Proves the relay-mesh `auto` policy end to end, over the real stdio wire.
//!
//! The unit tests in `src/mesh.rs` cover the pure decisions (catalog parsing,
//! error classification). This covers the part that actually matters: does the
//! request the provider sends carry `"model":"mesh"` instead of `"auto"`?
//!
//! Hysteresis makes this a two-turn test, deliberately. `ENABLE_OBSERVATIONS`
//! is 2 and one observation is cached for `CATALOG_TTL` (5s), so:
//!
//!   turn 1 → first observation → still `auto`
//!   (wait out the TTL)
//!   turn 2 → second observation → `mesh`
//!
//! That two-strike rule is the whole point — it stops a briefly-flapping peer
//! from bouncing a long-running agent in and out of MoA.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use serde_json::{json, Value};

/// Fake mesh-llm router.
///
/// `/models` reports the virtual `mesh` model plus `physical_models` real ones.
/// `/chat/completions` records the requested model and answers "ok" — unless
/// `moa_503` is set, in which case a `mesh` request gets mesh-llm's real
/// under-provisioned 503 so the contraction fallback can be exercised.
fn spawn_router(
    physical_models: usize,
    moa_503: bool,
) -> (String, mpsc::Receiver<String>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    let catalog_hits = Arc::new(AtomicUsize::new(0));
    let hits = catalog_hits.clone();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));

            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut content_length = 0usize;
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
                    content_length = v.parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; content_length];
            use std::io::Read;
            let _ = reader.read_exact(&mut body);

            let respond = |stream: &mut std::net::TcpStream,
                           status: &str,
                           ct: &str,
                           payload: String| {
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 {status}\r\ncontent-type: {ct}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        payload.len(),
                        payload
                    )
                    .as_bytes(),
                );
                let _ = stream.flush();
            };

            if request_line.contains("/models") {
                hits.fetch_add(1, Ordering::SeqCst);
                let mut ids: Vec<Value> = vec![json!({"id": "mesh"}), json!({"id": "auto"})];
                for i in 0..physical_models {
                    ids.push(json!({ "id": format!("model-{i}") }));
                }
                respond(
                    &mut stream,
                    "200 OK",
                    "application/json",
                    json!({ "object": "list", "data": ids }).to_string(),
                );
                continue;
            }

            // Record which model the agent actually asked for.
            let requested = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|v| v["model"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "<none>".into());
            let _ = tx.send(requested.clone());

            if moa_503 && requested == "mesh" {
                // mesh-llm's real under-provisioned response.
                respond(
                    &mut stream,
                    "503 Service Unavailable",
                    "application/json",
                    json!({"error": {"message": "MoA requires ≥2 models available in the mesh"}})
                        .to_string(),
                );
                continue;
            }

            let usage = json!({"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7});
            let text = json!({
                "id": "c", "object": "chat.completion.chunk", "created": 1, "model": requested,
                "choices": [{"index": 0, "delta": {"role": "assistant", "content": "ok"},
                             "finish_reason": null}],
            });
            let done = json!({
                "id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                "usage": usage,
            });
            respond(
                &mut stream,
                "200 OK",
                "text/event-stream",
                format!("data: {text}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
            );
        }
    });

    (format!("http://{addr}"), rx, catalog_hits)
}

struct Harness {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Harness {
    fn start(base_url: &str, home: &std::path::Path, prefer_mesh: bool) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_buzz-agent"));
        cmd.env("BUZZ_AGENT_PROVIDER", "relay-mesh")
            .env("BUZZ_AGENT_MODEL", "auto")
            .env("OPENAI_COMPAT_API_KEY", "k")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("GOOSE_DISABLE_KEYRING", "1")
            .env("RUST_LOG", "warn")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if prefer_mesh {
            cmd.env("BUZZ_AGENT_PREFER_MESH_FOR_AUTO", "1");
        }
        let mut child = cmd.spawn().expect("spawn");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .expect("write");
        self.stdin.flush().expect("flush");
        id
    }

    fn await_response(&mut self, id: i64) -> Value {
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert_ne!(n, 0, "agent closed stdout awaiting id={id}");
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if msg.get("id").and_then(Value::as_i64) == Some(id) {
                return msg;
            }
        }
    }

    fn new_session(&mut self, cwd: &std::path::Path) -> String {
        let id = self.request("initialize", json!({"protocolVersion": 2}));
        let _ = self.await_response(id);
        let id = self.request(
            "session/new",
            json!({"cwd": cwd.to_str().unwrap(), "mcpServers": []}),
        );
        let resp = self.await_response(id);
        resp["result"]["sessionId"]
            .as_str()
            .unwrap_or_else(|| panic!("session/new failed: {resp}"))
            .to_string()
    }

    fn prompt(&mut self, session_id: &str, text: &str) -> Value {
        let id = self.request(
            "session/prompt",
            json!({"sessionId": session_id, "prompt": [{"type": "text", "text": text}]}),
        );
        self.await_response(id)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Collect every model the router was asked for.
fn drain(rx: &mpsc::Receiver<String>) -> Vec<String> {
    let mut seen = Vec::new();
    while let Ok(model) = rx.recv_timeout(Duration::from_millis(500)) {
        seen.push(model);
    }
    seen
}

#[test]
fn moa_engages_after_two_confirmations() {
    // 2 physical models + virtual mesh = MoA is viable.
    let (base_url, requests, catalog_hits) = spawn_router(2, false);
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path(), true);
    let session = h.new_session(cwd.path());

    let resp = h.prompt(&session, "one");
    assert!(resp["result"]["stopReason"].is_string(), "turn 1: {resp}");
    let first = drain(&requests);
    assert_eq!(
        first,
        vec!["auto"],
        "first observation must NOT enable MoA — two strikes are required so a \
         flapping peer cannot bounce a long-running agent"
    );

    // Wait out CATALOG_TTL so the next turn re-observes.
    std::thread::sleep(Duration::from_millis(5_200));

    let resp = h.prompt(&session, "two");
    assert!(resp["result"]["stopReason"].is_string(), "turn 2: {resp}");
    let second = drain(&requests);
    assert_eq!(
        second,
        vec!["mesh"],
        "second confirmation must switch to the virtual MoA model"
    );

    assert!(
        catalog_hits.load(Ordering::SeqCst) >= 2,
        "expected the catalog to be re-polled after the TTL — that re-poll is \
         what lets a long-running agent adopt MoA without a restart"
    );
}

#[test]
fn moa_stays_off_with_only_one_physical_model() {
    // MoA over a single model is pointless, and mesh-llm 503s on it.
    let (base_url, requests, _) = spawn_router(1, false);
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path(), true);
    let session = h.new_session(cwd.path());

    h.prompt(&session, "one");
    std::thread::sleep(Duration::from_millis(5_200));
    h.prompt(&session, "two");

    let seen = drain(&requests);
    assert!(
        seen.iter().all(|m| m == "auto"),
        "a single-model mesh must never route to MoA, got {seen:?}"
    );
}

#[test]
fn contraction_mid_request_falls_back_to_auto() {
    // Catalog says MoA is fine, but inference 503s: the mesh shrank in between.
    let (base_url, requests, _) = spawn_router(2, true);
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path(), true);
    let session = h.new_session(cwd.path());

    h.prompt(&session, "one"); // observation 1 → auto
    std::thread::sleep(Duration::from_millis(5_200));

    // Observation 2 enables MoA, the mesh request 503s, and the turn must still
    // complete via a single retry on `auto`.
    let resp = h.prompt(&session, "two");
    assert!(
        resp["result"]["stopReason"].is_string(),
        "a contraction must not fail the turn: {resp}"
    );

    let seen = drain(&requests);
    assert!(
        seen.windows(2).any(|w| w[0] == "mesh" && w[1] == "auto"),
        "expected a mesh attempt immediately followed by an auto retry, got {seen:?}"
    );
}

#[test]
fn policy_is_inert_when_the_desktop_did_not_ask_for_it() {
    // No BUZZ_AGENT_PREFER_MESH_FOR_AUTO: `auto` must be sent untouched, and
    // the catalog must not be probed at all.
    let (base_url, requests, catalog_hits) = spawn_router(2, false);
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path(), false);
    let session = h.new_session(cwd.path());

    // /models is polled for reasons that have nothing to do with this policy:
    // `session/new` populates the desktop model picker via `discover_models`,
    // and goose's own provider does a lazy capability lookup on its first
    // request. So an absolute count proves nothing.
    //
    // The differential does. The policy re-observes once per turn whose cached
    // observation has expired, so measuring across the TTL boundary isolates
    // it: enabled adds a poll, disabled adds none.
    h.prompt(&session, "one");
    let after_first = catalog_hits.load(Ordering::SeqCst);

    std::thread::sleep(Duration::from_millis(5_200));

    h.prompt(&session, "two");
    let after_second = catalog_hits.load(Ordering::SeqCst);

    let seen = drain(&requests);
    assert!(
        seen.iter().all(|m| m == "auto"),
        "without the flag the model must pass through untouched, got {seen:?}"
    );
    assert_eq!(
        after_second, after_first,
        "a disabled policy must not poll /models across the TTL boundary"
    );
}
