//! Isolated, tool-free `buzz-agent` run for thread catch-up.
//!
//! The bundled `buzz-agent` is spawned over ACP stdio in local-only mode
//! (`BUZZ_AGENT_LOCAL_ONLY=1`, see `crates/buzz-agent/src/config.rs`) with:
//!
//! - a cleared environment carrying only the local endpoint, the model, and
//!   fixed bounds — no inherited provider keys, proxies, or agent defaults;
//! - a fresh empty working directory (also its `HOME`/`TMPDIR`), no hints or
//!   skills, and zero MCP servers, so the model has no tools;
//! - one prompt, one model round, no output-truncation retries;
//! - bounded stdout lines, total stdout, reply text, and stderr capture;
//! - every agent-to-client request refused (permissions are cancelled);
//! - a hard deadline and caller cancellation, both of which tear down the
//!   whole process tree (process group on Unix, kill-on-close Job Object on
//!   Windows). Containment setup failures abort the run.
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const INITIALIZE_ID: u64 = 1;
const SESSION_NEW_ID: u64 = 2;
const PROMPT_ID: u64 = 3;
const ACP_PROTOCOL_VERSION: u32 = 1;
/// Agent-to-client requests tolerated before the run is treated as hostile.
const MAX_AGENT_REQUESTS: usize = 8;
/// Characters of stderr surfaced in an error message.
const STDERR_DETAIL_CHARS: usize = 300;

/// Where the local model is served and which model to ask for. Both values
/// were validated by settings before they reach a launch.
#[derive(Clone, Debug)]
pub(super) struct AcpLaunch {
    pub program: PathBuf,
    pub base_url: String,
    pub model: String,
}

/// Resource bounds for one run.
#[derive(Clone, Debug)]
pub(super) struct AcpLimits {
    pub deadline: Duration,
    pub max_line_bytes: usize,
    pub max_stdout_bytes: usize,
    pub max_reply_bytes: usize,
    pub max_stderr_bytes: usize,
    pub max_output_tokens: u32,
    pub max_context_tokens: u32,
    pub llm_timeout_secs: u64,
}

impl Default for AcpLimits {
    fn default() -> Self {
        Self {
            deadline: Duration::from_secs(90),
            max_line_bytes: 256 * 1024,
            max_stdout_bytes: 1024 * 1024,
            max_reply_bytes: 32 * 1024,
            max_stderr_bytes: 16 * 1024,
            max_output_tokens: 4096,
            max_context_tokens: 32_768,
            llm_timeout_secs: 85,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum AcpRunError {
    Cancelled,
    TimedOut,
    Failed(String),
}

/// Path of the `buzz-agent` sidecar bundled next to the app executable. Only
/// the bundled binary is used: a `buzz-agent` found on `PATH` may predate
/// local-only mode.
pub(super) fn bundled_agent_program() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate Buzz: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "cannot locate Buzz's install directory".to_string())?;
    let name = if cfg!(windows) {
        "buzz-agent.exe"
    } else {
        "buzz-agent"
    };
    let program = dir.join(name);
    if program.is_file() {
        Ok(program)
    } else {
        Err("The local recap helper (buzz-agent) is missing from this Buzz installation.".into())
    }
}

/// The complete child environment. Nothing is inherited.
pub(super) fn child_env(
    launch: &AcpLaunch,
    limits: &AcpLimits,
    home: &Path,
) -> Vec<(String, String)> {
    let home = home.to_string_lossy().into_owned();
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut env: Vec<(String, String)> = [
        ("BUZZ_AGENT_LOCAL_ONLY", "1".to_string()),
        ("BUZZ_AGENT_LOCAL_JSON_OUTPUT", "1".to_string()),
        ("BUZZ_AGENT_THINKING_EFFORT", "none".to_string()),
        ("BUZZ_AGENT_PROVIDER", "openai".to_string()),
        ("OPENAI_COMPAT_API", "chat".to_string()),
        ("OPENAI_COMPAT_BASE_URL", launch.base_url.clone()),
        ("BUZZ_AGENT_MODEL", launch.model.clone()),
        ("BUZZ_AGENT_NO_HINTS", "1".to_string()),
        ("BUZZ_AGENT_MAX_ROUNDS", "1".to_string()),
        ("BUZZ_AGENT_MAX_TOKEN_RECOVERIES", "0".to_string()),
        ("BUZZ_AGENT_MAX_HANDOFFS", "0".to_string()),
        ("BUZZ_AGENT_MAX_SESSIONS", "1".to_string()),
        ("BUZZ_AGENT_STOP_MAX_REJECTIONS", "0".to_string()),
        ("BUZZ_AGENT_REQUIRE_REPLY", "0".to_string()),
        ("BUZZ_AGENT_PROMPT_CACHING", "0".to_string()),
        (
            "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
            limits.max_output_tokens.to_string(),
        ),
        (
            "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
            limits.max_context_tokens.to_string(),
        ),
        (
            "BUZZ_AGENT_LLM_TIMEOUT_SECS",
            limits.llm_timeout_secs.to_string(),
        ),
        ("HOME", home.clone()),
        ("TMPDIR", home.clone()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect();
    #[cfg(windows)]
    {
        // Winsock cannot initialize without SystemRoot; it carries no secret.
        if let Some(root) = std::env::var_os("SystemRoot") {
            env.push(("SystemRoot".into(), root.to_string_lossy().into_owned()));
        }
        env.push(("USERPROFILE".into(), home.clone()));
        env.push(("TEMP".into(), home.clone()));
        env.push(("TMP".into(), home));
    }
    env
}

/// Run one tool-free prompt and return the agent's reply text.
pub(super) async fn run_tool_free_prompt(
    launch: &AcpLaunch,
    system_prompt: &str,
    user_prompt: &str,
    limits: &AcpLimits,
    cancel: &CancellationToken,
) -> Result<String, AcpRunError> {
    if cancel.is_cancelled() {
        return Err(AcpRunError::Cancelled);
    }
    let workdir = tempfile::Builder::new()
        .prefix("buzz-catch-up-")
        .tempdir()
        .map_err(|e| AcpRunError::Failed(format!("could not create a private work folder: {e}")))?;
    let cwd = workdir
        .path()
        .canonicalize()
        .map_err(|e| AcpRunError::Failed(format!("could not resolve the work folder: {e}")))?;

    let mut command = Command::new(&launch.program);
    command
        .env_clear()
        .envs(child_env(launch, limits, &cwd))
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut tree = ProcessTree::spawn(command).map_err(AcpRunError::Failed)?;

    let (stdin, stdout, stderr) = match (
        tree.child.stdin.take(),
        tree.child.stdout.take(),
        tree.child.stderr.take(),
    ) {
        (Some(stdin), Some(stdout), Some(stderr)) => (stdin, stdout, stderr),
        _ => {
            reap(tree).await;
            return Err(AcpRunError::Failed(
                "local recap helper has no stdio".into(),
            ));
        }
    };
    let writer = spawn_writer(stdin);
    let (line_tx, line_rx) = mpsc::channel(64);
    spawn_stdout_reader(
        stdout,
        line_tx,
        limits.max_line_bytes,
        limits.max_stdout_bytes,
    );
    let stderr_tail = spawn_stderr_tail(stderr, limits.max_stderr_bytes);

    let mut session = Session {
        writer,
        lines: line_rx,
        agent_requests: 0,
        stderr_tail: stderr_tail.clone(),
    };
    let outcome = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AcpRunError::Cancelled),
        _ = tokio::time::sleep(limits.deadline) => Err(AcpRunError::TimedOut),
        result = session.drive(&cwd, system_prompt, user_prompt, limits.max_reply_bytes) => result,
    };
    drop(session);
    reap(tree).await;
    drop(workdir);
    outcome
}

async fn reap(tree: ProcessTree) {
    // Killing and reaping blocks briefly; keep it off the async worker.
    let _ = tokio::task::spawn_blocking(move || drop(tree)).await;
}

/// The spawned agent and ownership of its whole process tree. Dropping it
/// kills the tree and reaps the direct child on every exit path.
struct ProcessTree {
    child: Child,
    #[cfg(windows)]
    _job: crate::managed_agents::JobHandle,
}

impl ProcessTree {
    fn spawn(mut command: Command) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_SUSPENDED: u32 = 0x0000_0004;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
        }
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut child = command
            .spawn()
            .map_err(|e| format!("could not start the local recap helper: {e}"))?;

        // Own the frozen child before any of its code runs; fail closed.
        #[cfg(windows)]
        let job = {
            let pid = child.id();
            let Some(job) = crate::managed_agents::create_job_for_child(pid) else {
                let _ = child.kill();
                let _ = child.wait();
                return Err("could not contain the local recap helper".into());
            };
            if !crate::managed_agents::resume_process(pid) {
                drop(job);
                let _ = child.kill();
                let _ = child.wait();
                return Err("could not start the local recap helper".into());
            }
            job
        };

        Ok(Self {
            child,
            #[cfg(windows)]
            _job: job,
        })
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // The child leads its own group (`process_group(0)`), so its pid
            // is the group id; this reaches every descendant still in it.
            if let Ok(pgid) = i32::try_from(self.child.id()) {
                unsafe {
                    libc::killpg(pgid, libc::SIGKILL);
                }
            }
        }
        // On Windows `_job` closes after this body, killing the whole tree.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_writer(mut stdin: std::process::ChildStdin) -> std_mpsc::Sender<Vec<u8>> {
    let (tx, rx) = std_mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        for line in rx {
            if stdin.write_all(&line).is_err() || stdin.flush().is_err() {
                return;
            }
        }
    });
    tx
}

#[derive(Debug)]
enum StdoutEvent {
    Line(String),
    Overflow(&'static str),
}

fn spawn_stdout_reader(
    stdout: std::process::ChildStdout,
    tx: mpsc::Sender<StdoutEvent>,
    max_line_bytes: usize,
    max_stdout_bytes: usize,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut total = 0usize;
        loop {
            let mut line = Vec::new();
            let limit = (max_line_bytes + 1) as u64;
            match (&mut reader).take(limit).read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    total = total.saturating_add(read);
                    let event = if line.len() > max_line_bytes {
                        StdoutEvent::Overflow("a protocol line exceeded its size limit")
                    } else if total > max_stdout_bytes {
                        StdoutEvent::Overflow("output exceeded its size limit")
                    } else {
                        StdoutEvent::Line(String::from_utf8_lossy(&line).into_owned())
                    };
                    let stop = matches!(event, StdoutEvent::Overflow(_));
                    if tx.blocking_send(event).is_err() || stop {
                        return;
                    }
                }
            }
        }
    });
}

/// Keep the last `max_bytes` of stderr for diagnostics.
fn spawn_stderr_tail(
    mut stderr: std::process::ChildStderr,
    max_bytes: usize,
) -> Arc<Mutex<Vec<u8>>> {
    let tail = Arc::new(Mutex::new(Vec::new()));
    let sink = tail.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match stderr.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    let Ok(mut buf) = sink.lock() else { return };
                    buf.extend_from_slice(&chunk[..read]);
                    if buf.len() > max_bytes {
                        let excess = buf.len() - max_bytes;
                        buf.drain(..excess);
                    }
                }
            }
        }
    });
    tail
}

struct Session {
    writer: std_mpsc::Sender<Vec<u8>>,
    lines: mpsc::Receiver<StdoutEvent>,
    agent_requests: usize,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
}

impl Session {
    async fn drive(
        &mut self,
        cwd: &Path,
        system_prompt: &str,
        user_prompt: &str,
        max_reply_bytes: usize,
    ) -> Result<String, AcpRunError> {
        let mut reply = Reply::new(max_reply_bytes);
        self.send(json!({
            "jsonrpc": "2.0",
            "id": INITIALIZE_ID,
            "method": "initialize",
            "params": {
                "protocolVersion": ACP_PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false,
                },
            },
        }))?;
        self.await_result(INITIALIZE_ID, None, &mut reply).await?;

        self.send(json!({
            "jsonrpc": "2.0",
            "id": SESSION_NEW_ID,
            "method": "session/new",
            "params": {
                "cwd": cwd.to_string_lossy(),
                "mcpServers": [],
                "systemPrompt": system_prompt,
            },
        }))?;
        let created = self.await_result(SESSION_NEW_ID, None, &mut reply).await?;
        let session_id = created
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| AcpRunError::Failed("local recap helper returned no session".into()))?
            .to_string();

        self.send(json!({
            "jsonrpc": "2.0",
            "id": PROMPT_ID,
            "method": "session/prompt",
            "params": {
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": user_prompt }],
            },
        }))?;
        let done = self
            .await_result(PROMPT_ID, Some(&session_id), &mut reply)
            .await?;
        match done.get("stopReason").and_then(Value::as_str) {
            Some("end_turn") => {}
            Some("cancelled") => return Err(AcpRunError::Cancelled),
            Some("max_tokens") => {
                return Err(AcpRunError::Failed(
                    "the local model's reply was cut off; try a model with a larger output limit"
                        .into(),
                ))
            }
            Some("max_turn_requests") => {
                return Err(AcpRunError::Failed(
                    "the local model tried to use tools, which catch-up does not allow".into(),
                ))
            }
            other => {
                return Err(AcpRunError::Failed(format!(
                    "the local model stopped unexpectedly ({})",
                    other.unwrap_or("no reason")
                )))
            }
        }
        if reply.text.trim().is_empty() {
            return Err(AcpRunError::Failed(
                "the local model returned no reply".into(),
            ));
        }
        Ok(reply.text)
    }

    fn send(&self, message: Value) -> Result<(), AcpRunError> {
        let mut line = serde_json::to_vec(&message)
            .map_err(|e| AcpRunError::Failed(format!("could not encode a request: {e}")))?;
        line.push(b'\n');
        self.writer
            .send(line)
            .map_err(|_| self.exited("stopped accepting input"))
    }

    /// Process inbound traffic until the response to `id` arrives.
    async fn await_result(
        &mut self,
        id: u64,
        session_id: Option<&str>,
        reply: &mut Reply,
    ) -> Result<Value, AcpRunError> {
        loop {
            let line = match self.lines.recv().await {
                Some(StdoutEvent::Line(line)) => line,
                Some(StdoutEvent::Overflow(why)) => {
                    return Err(AcpRunError::Failed(format!("local recap helper {why}")))
                }
                None => return Err(self.exited("exited")),
            };
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&line).map_err(|_| {
                AcpRunError::Failed("local recap helper sent malformed protocol data".into())
            })?;
            let method = message.get("method").and_then(Value::as_str);
            let message_id = message.get("id").filter(|value| !value.is_null());
            match (method, message_id) {
                (Some(method), Some(request_id)) => {
                    self.refuse_request(method, request_id.clone())?;
                }
                (Some("session/update"), None) => {
                    if let Some(session_id) = session_id {
                        reply.accept_update(session_id, &message)?;
                    }
                }
                (Some(_), None) => {}
                (None, Some(response_id)) => {
                    if response_id.as_u64() != Some(id) {
                        continue;
                    }
                    if let Some(error) = message.get("error") {
                        let detail = error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("request failed");
                        return Err(AcpRunError::Failed(format!(
                            "local recap helper: {}",
                            sanitize_detail(detail)
                        )));
                    }
                    return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                }
                (None, None) => {}
            }
        }
    }

    /// Answer an agent-to-client request without granting anything.
    fn refuse_request(&mut self, method: &str, request_id: Value) -> Result<(), AcpRunError> {
        self.agent_requests += 1;
        if self.agent_requests > MAX_AGENT_REQUESTS {
            return Err(AcpRunError::Failed(
                "local recap helper made too many requests".into(),
            ));
        }
        let response = if method == "session/request_permission" {
            json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "result": { "outcome": { "outcome": "cancelled" } },
            })
        } else {
            json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "error": { "code": -32601, "message": "not available during catch-up" },
            })
        };
        self.send(response)
    }

    fn exited(&self, what: &str) -> AcpRunError {
        let tail = self
            .stderr_tail
            .lock()
            .map(|buf| String::from_utf8_lossy(&buf).into_owned())
            .unwrap_or_default();
        let last = tail
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(sanitize_detail);
        match last {
            Some(detail) => AcpRunError::Failed(format!("local recap helper {what}: {detail}")),
            None => AcpRunError::Failed(format!("local recap helper {what}")),
        }
    }
}

/// Accumulated `agent_message_chunk` text for one session.
struct Reply {
    text: String,
    max_bytes: usize,
}

impl Reply {
    fn new(max_bytes: usize) -> Self {
        Self {
            text: String::new(),
            max_bytes,
        }
    }

    fn accept_update(&mut self, session_id: &str, message: &Value) -> Result<(), AcpRunError> {
        let params = &message["params"];
        if params["sessionId"].as_str() != Some(session_id) {
            return Ok(());
        }
        let update = &params["update"];
        if update["sessionUpdate"].as_str() != Some("agent_message_chunk")
            || update["content"]["type"].as_str() != Some("text")
        {
            return Ok(());
        }
        let chunk = update["content"]["text"].as_str().unwrap_or_default();
        if self.text.len() + chunk.len() > self.max_bytes {
            return Err(AcpRunError::Failed(
                "the local model's reply exceeded its size limit".into(),
            ));
        }
        self.text.push_str(chunk);
        Ok(())
    }
}

fn sanitize_detail(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(STDERR_DETAIL_CHARS)
        .collect()
}

#[cfg(all(test, unix))]
#[path = "thread_catch_up_process_tests.rs"]
mod tests;
