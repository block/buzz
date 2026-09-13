//! What a turn does when the provider misbehaves.
//!
//! `stdio_turn.rs` proves the happy path end-to-end against a real socket. It
//! only ever answers `200`, so nothing there covers the unhappy half: a
//! rate-limited turn, an expired key, a hard server error, a stream that dies
//! mid-sentence.
//!
//! buzz used to cover that in `llm.rs`, against its own HTTP client. goose owns
//! that client now, and these tests are deliberately **not** a port of those:
//! they assert nothing about backoff timing, retry counts or `Retry-After`
//! parsing, because that is goose's logic to test and re-asserting it here
//! would rebuild the duplication this PR exists to delete.
//!
//! What they pin is the property buzz is responsible for and a user actually
//! feels: **a misbehaving provider ends the turn.** Every one of these
//! scenarios must produce a JSON-RPC response — a `stopReason` or an `error` —
//! and never a hang, a dropped request, or a crashed agent. A turn that never
//! answers is the worst failure mode available to us: `buzz-acp` waits on this
//! response, so silence means an agent that has visibly stopped replying in a
//! channel with nothing in the log to explain it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

mod approve;

/// How a scripted provider should answer one request.
///
/// Named rather than a bare status code so a test reads as the scenario it
/// describes, and so `TruncatedStream` — which is not expressible as a status —
/// sits in the same vocabulary as the others.
#[derive(Clone, Copy, Debug)]
enum Reply {
    /// A complete, well-formed SSE turn.
    Ok,
    /// Rate limited, with `Retry-After: 0` so a retrying client comes straight
    /// back and the test does not pay a real backoff.
    RateLimited,
    /// Expired or invalid credentials.
    Unauthorized,
    /// A hard server error, the non-retryable-looking kind.
    ServerError,
    /// The submitted conversation exceeds the model context window.
    ContextExceeded,
    /// Headers and a first chunk, then the connection dies with no
    /// `finish_reason` and no `[DONE]`. Models a provider or proxy dropping
    /// mid-stream, which is not the same as any HTTP status.
    TruncatedStream,
    /// A well-formed turn whose `finish_reason` is `length`: the model hit
    /// its output-token ceiling and the response text was cut off.
    OutputTokenLimit,
}

/// A provider that answers from a script, one entry per request.
///
/// Requests past the end of the script get `Reply::Ok`, so a test states only
/// the interesting prefix. The returned counter is the number of requests that
/// actually arrived, which is how a test distinguishes "the client retried"
/// from "the client gave up" without reaching into goose.
fn spawn_scripted_provider(script: Vec<Reply>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_for_thread = Arc::clone(&hits);

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
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(v) = trimmed
                    .strip_prefix("content-length: ")
                    .or_else(|| trimmed.strip_prefix("Content-Length: "))
                {
                    content_length = v.parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; content_length];
            let _ = reader.read_exact(&mut body);

            // The catalog endpoint always succeeds: these tests are about
            // completion failures, and a broken /models would fail the turn
            // earlier for an unrelated reason.
            if request_line.contains("/models") {
                let payload =
                    json!({ "object": "list", "data": [{ "id": "fake-model", "object": "model" }] })
                        .to_string();
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        payload.len(),
                        payload
                    )
                    .as_bytes(),
                );
                let _ = stream.flush();
                continue;
            }

            // Count only completion requests, so the number a test asserts on
            // is not inflated by catalog lookups.
            let n = hits_for_thread.fetch_add(1, Ordering::SeqCst);
            let reply = script.get(n).copied().unwrap_or(Reply::Ok);

            let response = match reply {
                Reply::Ok => sse_ok(),
                Reply::OutputTokenLimit => sse_output_token_limit(),
                Reply::RateLimited => error_response(
                    429,
                    "Too Many Requests",
                    Some("retry-after: 0\r\n"),
                    r#"{"error":{"message":"slow down","type":"rate_limit_exceeded"}}"#,
                ),
                Reply::Unauthorized => error_response(
                    401,
                    "Unauthorized",
                    None,
                    r#"{"error":{"message":"invalid api key","type":"invalid_request_error"}}"#,
                ),
                Reply::ServerError => error_response(
                    500,
                    "Internal Server Error",
                    None,
                    r#"{"error":{"message":"boom","type":"server_error"}}"#,
                ),
                Reply::ContextExceeded => error_response(
                    400,
                    "Bad Request",
                    None,
                    r#"{"error":{"message":"maximum context length exceeded","type":"invalid_request_error","code":"context_length_exceeded"}}"#,
                ),
                Reply::TruncatedStream => {
                    // Deliberately no content-length: a length would let the
                    // client detect a short body as a framing error rather
                    // than an ended stream. Dropping the socket after one
                    // chunk is what a real mid-stream death looks like.
                    let chunk = json!({
                        "id": "chatcmpl-truncated",
                        "object": "chat.completion.chunk",
                        "created": 1,
                        "model": "fake-model",
                        "choices": [{
                            "index": 0,
                            "delta": { "role": "assistant", "content": "I was saying" },
                            "finish_reason": Value::Null,
                        }],
                    });
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\ndata: {chunk}\n\n"
                    )
                }
            };

            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            // Dropping `stream` here ends the body, which is what makes
            // `TruncatedStream` truncated.
        }
    });

    (format!("http://{addr}"), hits)
}

/// A complete SSE turn: two content chunks, then `finish_reason` with usage.
fn sse_ok() -> String {
    let chunk = |delta: Value, finish: Value, usage: Value| {
        let mut c = json!({
            "id": "chatcmpl-test",
            "object": "chat.completion.chunk",
            "created": 1,
            "model": "fake-model",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
        });
        if !usage.is_null() {
            c["usage"] = usage;
        }
        format!("data: {c}\n\n")
    };
    let body = format!(
        "{}{}data: [DONE]\n\n",
        chunk(
            json!({ "role": "assistant", "content": "Recovered." }),
            Value::Null,
            Value::Null
        ),
        chunk(
            json!({}),
            json!("stop"),
            json!({ "prompt_tokens": 11, "completion_tokens": 2, "total_tokens": 13 })
        ),
    );
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

/// A turn cut off at the output-token limit: one content chunk, then a
/// `finish_reason: "length"` terminator with usage.
fn sse_output_token_limit() -> String {
    let chunk = |delta: Value, finish: Value, usage: Value| {
        let mut c = json!({
            "id": "chatcmpl-truncated-output",
            "object": "chat.completion.chunk",
            "created": 1,
            "model": "fake-model",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
        });
        if !usage.is_null() {
            c["usage"] = usage;
        }
        format!("data: {c}\n\n")
    };
    let body = format!(
        "{}{}data: [DONE]\n\n",
        chunk(
            json!({ "role": "assistant", "content": "The full solution is: step one," }),
            Value::Null,
            Value::Null
        ),
        chunk(
            json!({}),
            json!("length"),
            json!({ "prompt_tokens": 11, "completion_tokens": 9, "total_tokens": 20 })
        ),
    );
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn error_response(status: u16, reason: &str, extra_headers: Option<&str>, payload: &str) -> String {
    format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{}connection: close\r\n\r\n{payload}",
        payload.len(),
        extra_headers.unwrap_or(""),
    )
}

struct Harness {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Harness {
    fn start(base_url: &str, home: &std::path::Path) -> Self {
        Self::start_with_env(base_url, home, &[])
    }

    fn start_with_env(base_url: &str, home: &std::path::Path, env: &[(&str, &str)]) -> Self {
        let exe = env!("CARGO_BIN_EXE_buzz-agent");
        let mut command = Command::new(exe);
        command
            .env("BUZZ_AGENT_PROVIDER", "openai-compat")
            .env("BUZZ_AGENT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_API_KEY", "test-key")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("BUZZ_AGENT_MAX_ROUNDS", "2")
            // Load-bearing, not just impatience. A provider that holds the
            // socket open without finishing the stream is only rescued by the
            // request timeout, and this is the knob that sets it — verified by
            // making the fake provider sleep on a half-written stream: with
            // this line the turn still ends, without it the prompt never
            // answers. It also depends on the projection in `config.rs` that
            // carries this onto goose's per-provider variable; before that
            // projection existed this env var reached nothing.
            .env("BUZZ_AGENT_LLM_TIMEOUT_SECS", "10")
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("GOOSE_DISABLE_KEYRING", "1")
            .env("RUST_LOG", "warn")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // Test-specific overrides win over the defaults above.
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("spawn buzz-agent");

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
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{msg}").expect("write");
        self.stdin.flush().expect("flush");
        id
    }

    fn await_response(&mut self, id: i64) -> Value {
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert_ne!(n, 0, "agent closed stdout before responding to id={id}");
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

    /// `initialize` + `session/new`, returning the session id.
    fn open_session(&mut self, cwd: &std::path::Path) -> String {
        let id = self.request("initialize", json!({ "protocolVersion": 2 }));
        let _ = self.await_response(id);
        let id = self.request(
            "session/new",
            json!({ "cwd": cwd.to_str().unwrap(), "mcpServers": [] }),
        );
        let resp = self.await_response(id);
        resp["result"]["sessionId"]
            .as_str()
            .unwrap_or_else(|| panic!("no sessionId: {resp}"))
            .to_string()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run one prompt against a scripted provider and return the response plus the
/// number of completion requests the provider saw.
///
/// The turn runs on a worker thread so the test can impose its own deadline: a
/// hang is the failure mode most worth catching here, and without a deadline it
/// would present as the whole suite stalling rather than as one failed test.
fn prompt_once(script: Vec<Reply>) -> (Value, usize) {
    prompt_once_with_env(script, &[])
}

fn prompt_once_with_env(script: Vec<Reply>, env: &'static [(&str, &str)]) -> (Value, usize) {
    let (base_url, hits) = spawn_scripted_provider(script);
    let hits_for_assert = Arc::clone(&hits);

    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let home = tempfile::tempdir().expect("tempdir");
        let cwd = tempfile::tempdir().expect("cwd");
        let mut h = Harness::start_with_env(&base_url, home.path(), env);
        let session_id = h.open_session(cwd.path());

        let id = h.request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": "say hello" }],
            }),
        );
        let resp = h.await_response(id);
        // `home`/`cwd` must outlive the child: dropping them first would pull
        // the session store out from under a still-running agent.
        let _ = tx.send(resp);
        drop(h);
    });

    let resp = rx
        .recv_timeout(Duration::from_secs(90))
        .expect("session/prompt never answered — a turn that never returns hangs buzz-acp");
    worker.join().expect("harness thread panicked");
    (resp, hits_for_assert.load(Ordering::SeqCst))
}

/// Every scenario must end the turn — with a `stopReason` or a JSON-RPC error,
/// never silence. Returns the `stopReason` if there was one.
fn assert_turn_ended(resp: &Value, scenario: &str) -> Option<String> {
    let has_result = resp["result"]["stopReason"].is_string();
    let has_error = resp["error"].is_object();
    assert!(
        has_result || has_error,
        "{scenario}: turn produced neither stopReason nor error, which hangs buzz-acp: {resp}"
    );
    resp["result"]["stopReason"].as_str().map(str::to_owned)
}

/// A rate limit that clears must not be terminal.
///
/// `Retry-After: 0` keeps the test fast. The assertion that matters is the
/// second request arriving at all: if a 429 ended the turn, the user would see
/// an agent that stopped replying because the provider was briefly busy.
#[test]
fn rate_limited_then_ok_completes_the_turn() {
    let (resp, hits) = prompt_once(vec![Reply::RateLimited, Reply::Ok]);

    let stop = assert_turn_ended(&resp, "429 then 200");
    assert!(
        stop.is_some(),
        "a cleared rate limit must still complete the turn: {resp}"
    );
    assert!(
        hits >= 2,
        "provider saw {hits} completion request(s): the 429 was treated as terminal rather than retried"
    );
}

/// A bad key must fail the turn as an *auth* error specifically.
///
/// `-32001` is not cosmetic: `AgentError::LlmAuth` maps to it
/// (`types.rs:102`) and `buzz-acp` routes on it (`acp.rs:118`,
/// `agent_error_from_json`) to tell "your key is wrong" apart from a generic
/// provider failure. Collapsing it into `-32000` would leave a user with an
/// expired key seeing an unexplained failure instead of a credentials problem.
///
/// The bound matters too: 401 is not transient, so retrying it forever would
/// turn one misconfigured agent into sustained load against the provider.
#[test]
fn unauthorized_ends_the_turn_as_an_auth_error() {
    let (resp, hits) = prompt_once(vec![Reply::Unauthorized; 8]);

    assert_turn_ended(&resp, "401");
    assert_eq!(
        resp["error"]["code"], -32001,
        "401 must surface as LlmAuth (-32001), which buzz-acp routes on, not a generic provider error: {resp}"
    );
    assert!(
        hits <= 4,
        "provider saw {hits} completion requests for an invalid key — 401 is not transient and must not be retried indefinitely"
    );
}

/// A persistent 500 must surface as a bounded, reported failure.
///
/// Two properties, both user-visible. It must *report* — a 500 is not an auth
/// problem, so it takes the generic `-32000` arm and must not be silently
/// converted into a successful empty turn, which would look like an agent that
/// answered with nothing. And it must *stop*: retries are bounded, so one
/// broken provider cannot be hammered indefinitely by a single prompt.
#[test]
fn server_error_ends_the_turn_and_stops_retrying() {
    let (resp, hits) = prompt_once(vec![Reply::ServerError; 8]);

    assert_turn_ended(&resp, "500");
    assert!(
        resp["error"].is_object(),
        "a provider that never succeeded must not report a successful turn: {resp}"
    );
    assert_eq!(
        resp["error"]["code"], -32000,
        "a server error is not an auth error: {resp}"
    );
    assert!(
        hits <= 6,
        "provider saw {hits} completion requests: retries must stay bounded"
    );
}

/// A prompt that alone exceeds the compaction threshold must not spin the
/// proactive compactor.
///
/// Compaction preserves the latest user text verbatim, so when that text by
/// itself trips the byte-estimated occupancy gate, every "successful"
/// compaction leaves the estimate over threshold and the start machine would
/// fire again — an unbounded run of summary requests with no inference
/// between them (reproduced pre-fix: 8 consecutive summaries, zero normal
/// requests). The budget is one proactive compaction per inference: after
/// that, the request goes to the provider, whose acceptance or context-400 is
/// the authoritative verdict the estimate can only approximate.
#[test]
fn oversized_prompt_does_not_spin_proactive_compaction() {
    let (base_url, hits) = spawn_scripted_provider(vec![]);
    let hits_for_assert = Arc::clone(&hits);

    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let home = tempfile::tempdir().expect("tempdir");
        let cwd = tempfile::tempdir().expect("cwd");
        // A 1000-token limit with a ~2000-byte prompt: the conservative
        // one-byte-per-token estimate keeps the conversation over threshold
        // no matter how many times it is summarized.
        let mut h = Harness::start_with_env(
            &base_url,
            home.path(),
            &[
                ("GOOSE_CONTEXT_LIMIT", "1000"),
                ("BUZZ_AGENT_MAX_ROUNDS", "1"),
            ],
        );
        let session_id = h.open_session(cwd.path());

        let id = h.request(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": "x".repeat(2000) }],
            }),
        );
        let resp = h.await_response(id);
        let _ = tx.send(resp);
        drop(h);
    });

    let resp = rx
        .recv_timeout(Duration::from_secs(90))
        .expect("session/prompt never answered — proactive compaction is spinning");
    worker.join().expect("harness thread panicked");
    let hits = hits_for_assert.load(Ordering::SeqCst);

    let stop = assert_turn_ended(&resp, "oversized prompt");
    assert!(
        stop.is_some(),
        "an oversized-but-accepted prompt must still complete the turn: {resp}"
    );
    // At most one summary plus the inference itself (and the retry margin of
    // one more pair). Pre-fix this was 8+ summary requests and no inference.
    assert!(
        (1..=4).contains(&hits),
        "provider saw {hits} completion request(s): proactive compaction must be \
         bounded per inference, not free-running"
    );
}

#[test]
fn context_overflow_compacts_and_retries_the_turn() {
    // First request overflows. The next request is the compaction summary, and
    // the third is the retried agent turn against the compacted conversation.
    let (resp, hits) = prompt_once(vec![Reply::ContextExceeded, Reply::Ok, Reply::Ok]);

    let stop = assert_turn_ended(&resp, "context overflow then recovery");
    assert!(
        stop.is_some(),
        "a recoverable context overflow must complete the same turn: {resp}"
    );
    assert!(
        hits >= 3,
        "provider saw {hits} completion request(s): expected overflow, summary, and retried turn"
    );
}

/// A response truncated at the output-token limit must be retried, and the
/// retry completes the turn.
///
/// goose marks such responses (`finish_reason: length` →
/// `output_token_limit_reached`) but leaves the recovery policy to the
/// embedder. Main recovered with `BUZZ_AGENT_MAX_TOKEN_RECOVERIES`; this pins
/// the rebuilt equivalent: the truncated text is preserved, a continue nudge
/// is injected, and the next inference finishes the turn — rather than the
/// truncated fragment silently ending it.
#[test]
fn output_token_limit_is_recovered_within_the_turn() {
    // The harness default of 2 rounds exists for the failure suites; recovery
    // legitimately consumes a round per retry, so give this turn headroom.
    let (resp, hits) = prompt_once_with_env(
        vec![Reply::OutputTokenLimit, Reply::Ok],
        &[("BUZZ_AGENT_MAX_ROUNDS", "8")],
    );

    let stop = assert_turn_ended(&resp, "output-token limit then recovery");
    assert_eq!(
        stop.as_deref(),
        Some("end_turn"),
        "a recovered truncation must complete the turn normally: {resp}"
    );
    assert!(
        hits >= 2,
        "provider saw {hits} completion request(s): the truncated response was \
         not retried, so the turn ended mid-thought"
    );
}

/// A model that hits the output-token limit on every attempt must stop with a
/// bounded budget, and report `max_tokens` rather than a normal end of turn.
#[test]
fn persistent_output_token_limit_is_bounded_and_reported() {
    // Rounds deliberately exceed the recovery budget so the stop reason below
    // is proven to come from the recovery bound, not the round gate.
    let (resp, hits) = prompt_once_with_env(
        vec![Reply::OutputTokenLimit; 12],
        &[("BUZZ_AGENT_MAX_ROUNDS", "10")],
    );

    let stop = assert_turn_ended(&resp, "persistent output-token limit");
    assert_eq!(
        stop.as_deref(),
        Some("max_tokens"),
        "an exhausted recovery budget must surface as max_tokens: {resp}"
    );
    // Initial attempt + default budget of 3 recoveries.
    assert!(
        (1..=4).contains(&hits),
        "provider saw {hits} completion request(s): truncation recovery must be \
         bounded by BUZZ_AGENT_MAX_TOKEN_RECOVERIES, not free-running"
    );
}

/// A stream that dies mid-sentence must still end the turn.
///
/// This is the scenario least likely to be handled by status-code logic: the
/// response was a valid `200` and the failure is the *absence* of a terminator.
/// A client waiting for `[DONE]` on a closed socket is exactly how a turn hangs
/// forever.
#[test]
fn truncated_stream_ends_the_turn() {
    let (resp, hits) = prompt_once(vec![Reply::TruncatedStream; 4]);

    let stop = assert_turn_ended(&resp, "truncated stream");
    assert!(
        stop.is_some() || resp["error"].is_object(),
        "a stream that died mid-sentence must resolve one way or the other: {resp}"
    );
    assert!(
        hits >= 1,
        "the request never reached the provider, so nothing was tested"
    );
}

/// A transient failure must not poison the session.
///
/// Recovery within one turn (`rate_limited_then_ok_completes_the_turn`) and
/// recovery across turns are different properties: goose holds per-session
/// state, so a turn that failed hard could leave a session that never works
/// again. That would present to a user as an agent permanently silent after one
/// bad moment, which is worse than the original failure.
#[test]
fn a_session_survives_a_failed_turn() {
    let (base_url, hits) = spawn_scripted_provider(vec![Reply::ServerError; 4]);

    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let home = tempfile::tempdir().expect("tempdir");
        let cwd = tempfile::tempdir().expect("cwd");
        let mut h = Harness::start(&base_url, home.path());
        let session_id = h.open_session(cwd.path());

        let id = h.request(
            "session/prompt",
            json!({ "sessionId": session_id, "prompt": [{ "type": "text", "text": "first" }] }),
        );
        let first = h.await_response(id);

        // The script is exhausted, so this turn is served `Reply::Ok`.
        let id = h.request(
            "session/prompt",
            json!({ "sessionId": session_id, "prompt": [{ "type": "text", "text": "second" }] }),
        );
        let second = h.await_response(id);

        let _ = tx.send((first, second));
        drop(h);
    });

    let (first, second) = rx
        .recv_timeout(Duration::from_secs(120))
        .expect("a prompt never answered after a failed turn");
    worker.join().expect("harness thread panicked");

    assert_turn_ended(&first, "failing first turn");
    let recovered = assert_turn_ended(&second, "recovering second turn");
    assert!(
        recovered.is_some(),
        "the session did not recover once the provider did — a transient failure must not leave an agent permanently silent: {second}"
    );
    assert!(
        hits.load(Ordering::SeqCst) >= 2,
        "the second prompt never reached the provider"
    );
}
