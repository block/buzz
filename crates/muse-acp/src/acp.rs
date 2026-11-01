//! The ACP agent loop: newline-delimited JSON-RPC over stdio.
//!
//! `buzz-acp` speaks to this binary as an ACP agent. The methods it sends
//! are:
//!
//! * `initialize` — protocol version negotiation (we answer 2).
//! * `session/new` — allocate a session id; the browser tab is created
//!   lazily on the first prompt, so sessions that never run a turn cost no
//!   browser.
//! * `session/prompt` — run one turn through the driver, streaming the
//!   reply as `session/update` `agent_message_chunk` notifications, then
//!   answering with `stopReason` (`end_turn` / `cancelled`).
//! * `session/cancel` (notification) — set the in-flight turn's cancel
//!   flag. A cancel with no in-flight turn is discarded: the next prompt
//!   installs a fresh flag, so a stale cancel can never kill a later turn.
//!
//! Everything else answers `-32601` (method not found). In particular the
//! bridge advertises no `loadSession` capability and no permission or
//! filesystem requests: it has no tools of its own, it relays to Muse.
//!
//! The loop is parameterized over its input stream and an outbound frame
//! channel so tests can drive it without a process or a browser.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, warn};

use crate::driver::{Driver, TurnOutcome};
use crate::BridgeError;

const PROTOCOL_VERSION: u64 = 2;
const AGENT_NAME: &str = "muse-acp";

/// Per-session ACP state. Sessions are never removed: a session id that
/// was allocated stays valid for the bridge's lifetime.
struct SessionState {
    /// Standing prompt from `session/new`, if any.
    system_prompt: Option<String>,
    /// Whether the standing prompt was already prepended to a turn. It is
    /// sent only on the first turn: the tab holds the conversation after
    /// that, and repeating the framing every turn would spam Muse with
    /// the same instructions.
    system_prompt_sent: bool,
    /// Cancel flag for the currently in-flight turn (fresh per prompt).
    cancel: Arc<AtomicBool>,
    /// Whether a turn is currently running for this session. A second
    /// concurrent prompt is rejected as "session busy" rather than
    /// interleaved into the same tab.
    in_flight: bool,
}

type Sessions = Arc<Mutex<HashMap<String, SessionState>>>;

/// Run the agent loop until the input stream ends.
///
/// `frames_out` receives every outbound JSON-RPC frame as a single-line
/// string (responses, errors, and `session/update` notifications). The
/// caller owns writing them to stdout. On input EOF, in-flight turns are
/// cancelled and the loop returns; spawned turn tasks finish on their own.
pub async fn run_agent<R, D>(reader: R, frames_out: mpsc::UnboundedSender<String>, driver: Arc<D>)
where
    R: AsyncBufRead + Unpin,
    D: Driver + 'static,
{
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
    let mut lines = reader.lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break, // EOF
            Err(e) => {
                warn!("stdin read failed: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(e) => {
                warn!("ignoring invalid JSON-RPC line: {e}");
                continue;
            }
        };
        dispatch(value, &sessions, &frames_out, &driver).await;
    }
    // Input is gone: stop anything still running so a lingering process
    // does not keep driving the browser after the harness went away.
    let sessions_guard = sessions.lock().await;
    for session in sessions_guard.values() {
        session.cancel.store(true, Ordering::SeqCst);
    }
}

async fn dispatch<D>(
    value: Value,
    sessions: &Sessions,
    frames_out: &mpsc::UnboundedSender<String>,
    driver: &Arc<D>,
) where
    D: Driver + 'static,
{
    // Own the method name: the request arm moves `value` into the handler,
    // so the match scrutinee must not borrow from it.
    let method = value
        .get("method")
        .and_then(|m| m.as_str())
        .map(ToOwned::to_owned);
    let id = value.get("id").cloned();
    match (method.as_deref(), id) {
        (Some(method), Some(id)) => {
            handle_request(method, id, value, sessions, frames_out, driver).await;
        }
        (Some("session/cancel"), None) => {
            let session_id = value
                .get("params")
                .and_then(|p| p.get("sessionId"))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let sessions_guard = sessions.lock().await;
            if let Some(session) = sessions_guard.get(session_id) {
                session.cancel.store(true, Ordering::SeqCst);
                debug!(session_id, "turn cancelled by client");
            }
        }
        (Some(_), None) => {
            // Any other notification: nothing to do.
        }
        (None, Some(_)) => {
            // A response to a request we never sent: ignore.
        }
        (None, None) => {
            warn!("ignoring JSON-RPC frame with neither method nor id");
        }
    }
}

async fn handle_request<D>(
    method: &str,
    id: Value,
    value: Value,
    sessions: &Sessions,
    frames_out: &mpsc::UnboundedSender<String>,
    driver: &Arc<D>,
) where
    D: Driver + 'static,
{
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let _ = frames_out.send(response(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "agentCapabilities": {
                        "loadSession": false,
                        "promptCapabilities": {
                            "image": false,
                            "audio": false,
                            "embeddedContext": false,
                        },
                    },
                    "agentInfo": {
                        "name": AGENT_NAME,
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
            ));
        }
        "session/new" => {
            let system_prompt = params
                .get("systemPrompt")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());
            if params
                .get("mcpServers")
                .is_some_and(|s| !s.as_array().is_some_and(|a| a.is_empty()))
            {
                debug!("mcpServers are not forwarded: Muse web has its own tools");
            }
            let session_id = uuid::Uuid::new_v4().to_string();
            sessions.lock().await.insert(
                session_id.clone(),
                SessionState {
                    system_prompt,
                    system_prompt_sent: false,
                    cancel: Arc::new(AtomicBool::new(false)),
                    in_flight: false,
                },
            );
            let _ = frames_out.send(response(id, json!({ "sessionId": session_id })));
        }
        "session/prompt" => {
            let session_id = params
                .get("sessionId")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let prompt_text = match extract_prompt_text(&params) {
                Ok(text) => text,
                Err(message) => {
                    let _ = frames_out.send(response_error(id, -32602, &message));
                    return;
                }
            };
            let mut sessions_guard = sessions.lock().await;
            let Some(session) = sessions_guard.get_mut(&session_id) else {
                let _ = frames_out.send(response_error(
                    id,
                    -32602,
                    &format!("unknown session: {session_id}"),
                ));
                return;
            };
            if session.in_flight {
                let _ = frames_out.send(response_error(
                    id,
                    -32000,
                    "session busy: a turn is already running for this session",
                ));
                return;
            }
            session.in_flight = true;
            let cancel = Arc::new(AtomicBool::new(false));
            session.cancel = cancel.clone();
            let (message, _sent_system) = compose_message(
                &mut session.system_prompt,
                &mut session.system_prompt_sent,
                &prompt_text,
            );
            drop(sessions_guard);
            spawn_turn(
                session_id,
                id,
                message,
                cancel,
                sessions.clone(),
                frames_out.clone(),
                driver.clone(),
            );
        }
        _ => {
            let _ = frames_out.send(response_error(id, -32601, "method not found"));
        }
    }
}

/// Combine a session's standing prompt with one turn's text.
///
/// The standing prompt is prepended only on the first turn of a session
/// (and then marked sent); later turns send just the new message, since
/// the tab already holds the conversation. Returns the composed message
/// and whether the standing prompt was consumed by this call.
fn compose_message(
    system_prompt: &mut Option<String>,
    system_prompt_sent: &mut bool,
    prompt_text: &str,
) -> (String, bool) {
    if !*system_prompt_sent {
        if let Some(standing) = system_prompt.take() {
            *system_prompt_sent = true;
            if standing.trim().is_empty() {
                return (prompt_text.to_string(), false);
            }
            return (format!("{standing}\n\n{prompt_text}"), true);
        }
        *system_prompt_sent = true;
    }
    (prompt_text.to_string(), false)
}

/// Extract the text of a `session/prompt` request: the concatenation of
/// its text blocks. Non-text blocks are skipped (the bridge relays text
/// only); a prompt with no text at all is an error, not an empty turn.
fn extract_prompt_text(params: &Value) -> Result<String, String> {
    let Some(blocks) = params.get("prompt").and_then(|p| p.as_array()) else {
        return Err("session/prompt params.prompt must be an array".into());
    };
    let mut text = String::new();
    for block in blocks {
        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(part) = block.get("text").and_then(|t| t.as_str()) {
                text.push_str(part);
            }
        }
    }
    if text.trim().is_empty() {
        return Err("session/prompt contains no text blocks".into());
    }
    Ok(text)
}

#[allow(clippy::too_many_arguments)]
fn spawn_turn<D>(
    session_id: String,
    request_id: Value,
    message: String,
    cancel: Arc<AtomicBool>,
    sessions: Sessions,
    frames_out: mpsc::UnboundedSender<String>,
    driver: Arc<D>,
) where
    D: Driver + 'static,
{
    tokio::spawn(async move {
        let (chunk_tx, mut chunk_rx) = mpsc::channel::<String>(256);
        let run = driver.run_turn(&session_id, &message, cancel, chunk_tx);
        tokio::pin!(run);
        let outcome: Result<TurnOutcome, BridgeError>;
        loop {
            tokio::select! {
                biased;
                result = &mut run => {
                    outcome = result;
                    break;
                }
                chunk = chunk_rx.recv() => {
                    if let Some(text) = chunk {
                        let _ = frames_out.send(session_update(&session_id, &text));
                    }
                }
            }
        }
        // Drain any chunks the driver sent just before finishing, so the
        // final text is never lost to a race with the outcome.
        while let Ok(text) = chunk_rx.try_recv() {
            let _ = frames_out.send(session_update(&session_id, &text));
        }
        {
            let mut sessions_guard = sessions.lock().await;
            if let Some(session) = sessions_guard.get_mut(&session_id) {
                session.in_flight = false;
            }
        }
        match outcome {
            Ok(TurnOutcome::Finished) => {
                let _ = frames_out.send(response(request_id, json!({ "stopReason": "end_turn" })));
            }
            Ok(TurnOutcome::Cancelled) | Err(BridgeError::Cancelled) => {
                let _ = frames_out.send(response(request_id, json!({ "stopReason": "cancelled" })));
            }
            Err(e) => {
                let _ = frames_out.send(response_error(request_id, -32000, &e.to_string()));
            }
        }
    });
}

fn response(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn response_error(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

fn session_update(session_id: &str, text: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": text },
            },
        },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::FakeDriver;
    use tokio::io::BufReader;

    /// Feed lines to the agent loop and collect outbound frames until
    /// `wanted` frames have arrived (or the timeout elapses).
    struct Harness {
        input: tokio::io::DuplexStream,
        frames: mpsc::UnboundedReceiver<String>,
        _task: tokio::task::JoinHandle<()>,
    }

    impl Harness {
        async fn start(driver: FakeDriver) -> Self {
            let (client_input, agent_input) = tokio::io::duplex(65536);
            let (frames_tx, frames) = mpsc::unbounded_channel();
            let task = tokio::spawn(run_agent(
                BufReader::new(agent_input),
                frames_tx,
                Arc::new(driver),
            ));
            Self {
                input: client_input,
                frames,
                _task: task,
            }
        }

        async fn send(&mut self, line: &str) {
            use tokio::io::AsyncWriteExt;
            self.input.write_all(line.as_bytes()).await.unwrap();
            self.input.write_all(b"\n").await.unwrap();
        }

        async fn next_frame(&mut self) -> Value {
            let frame = tokio::time::timeout(Duration::from_secs(5), self.frames.recv())
                .await
                .expect("timed out waiting for a frame")
                .expect("frame channel closed");
            serde_json::from_str(&frame).expect("frame is valid JSON")
        }
    }

    use std::time::Duration;

    #[tokio::test]
    async fn initialize_and_unknown_method() {
        let mut harness = Harness::start(FakeDriver::new(vec![], 0)).await;
        harness
            .send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .await;
        let frame = harness.next_frame().await;
        assert_eq!(frame["id"], 1);
        assert_eq!(frame["result"]["protocolVersion"], 2);
        assert_eq!(frame["result"]["agentInfo"]["name"], "muse-acp");

        harness
            .send(r#"{"jsonrpc":"2.0","id":2,"method":"session/load","params":{}}"#)
            .await;
        let frame = harness.next_frame().await;
        assert_eq!(frame["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn prompt_streams_chunks_then_end_turn() {
        let mut harness =
            Harness::start(FakeDriver::new(vec!["hello ".into(), "world".into()], 0)).await;
        harness
            .send(r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":"/tmp"}}"#)
            .await;
        let frame = harness.next_frame().await;
        let session_id = frame["result"]["sessionId"].as_str().unwrap().to_string();

        harness
            .send(&format!(
                r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"hi"}}]}}}}"#
            ))
            .await;
        let update = harness.next_frame().await;
        assert_eq!(update["method"], "session/update");
        assert_eq!(update["params"]["update"]["content"]["text"], "hello ");
        let update = harness.next_frame().await;
        assert_eq!(update["params"]["update"]["content"]["text"], "world");
        let done = harness.next_frame().await;
        assert_eq!(done["id"], 2);
        assert_eq!(done["result"]["stopReason"], "end_turn");
    }

    #[tokio::test]
    async fn prompt_to_unknown_session_is_an_error() {
        let mut harness = Harness::start(FakeDriver::new(vec![], 0)).await;
        harness
            .send(
                r#"{"jsonrpc":"2.0","id":1,"method":"session/prompt","params":{"sessionId":"nope","prompt":[{"type":"text","text":"hi"}]}}"#,
            )
            .await;
        let frame = harness.next_frame().await;
        assert_eq!(frame["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn cancel_mid_turn_reports_cancelled() {
        let mut harness = Harness::start(FakeDriver::new(
            vec!["a".into(), "b".into(), "c".into()],
            50,
        ))
        .await;
        harness
            .send(r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{}}"#)
            .await;
        let frame = harness.next_frame().await;
        let session_id = frame["result"]["sessionId"].as_str().unwrap().to_string();
        harness
            .send(&format!(
                r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"hi"}}]}}}}"#
            ))
            .await;
        // Cancel before the scripted chunks finish.
        harness
            .send(&format!(
                r#"{{"jsonrpc":"2.0","method":"session/cancel","params":{{"sessionId":"{session_id}"}}}}"#
            ))
            .await;
        // Read until the prompt's response arrives (updates may interleave).
        loop {
            let frame = harness.next_frame().await;
            if frame.get("id") == Some(&json!(2)) {
                assert_eq!(frame["result"]["stopReason"], "cancelled");
                break;
            }
        }
    }

    #[test]
    fn standing_prompt_prepends_only_on_first_turn() {
        let mut standing = Some("Be brief.".to_string());
        let mut sent = false;
        let (message, consumed) = compose_message(&mut standing, &mut sent, "hello");
        assert_eq!(message, "Be brief.\n\nhello");
        assert!(consumed);
        assert!(standing.is_none());
        let (message, consumed) = compose_message(&mut standing, &mut sent, "again");
        assert_eq!(message, "again");
        assert!(!consumed);
    }

    #[test]
    fn prompt_text_extraction_skips_non_text_blocks() {
        let params = json!({
            "prompt": [
                { "type": "text", "text": "hello " },
                { "type": "image", "data": "abc" },
                { "type": "text", "text": "world" },
            ],
        });
        assert_eq!(extract_prompt_text(&params).unwrap(), "hello world");
        let params = json!({ "prompt": [{ "type": "image", "data": "abc" }] });
        assert!(extract_prompt_text(&params).is_err());
    }
}
