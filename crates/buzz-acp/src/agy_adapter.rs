//! ACP compatibility bridge for the Antigravity CLI.
//!
//! Antigravity does not currently expose a native ACP transport. This module
//! presents the small ACP surface that `buzz-acp` needs and executes each turn
//! through AGY's official non-interactive `--print` mode.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use futures_util::{Stream, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::process::{Child, Command};
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};
use uuid::Uuid;

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_HOOK_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PROCESS_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_PRINT_TIMEOUT: &str = "2h";
const HOOK_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MODEL_LIST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct Session {
    cwd: PathBuf,
    system_prompt: Option<String>,
    conversation_id: Option<String>,
}

struct AgyOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

enum TurnResult {
    Completed(AgyOutput),
    Cancelled,
    InputClosed,
}

/// Receive one documented Antigravity hook payload.
///
/// Hook commands must always emit valid JSON on stdout. Logging is therefore
/// intentionally best-effort: a missing or malformed observer directory must
/// never break an AGY tool call.
pub(crate) async fn run_hook() -> Result<()> {
    let event = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "unknown".to_string());
    let mut input = Vec::new();
    tokio::io::stdin()
        .take((MAX_HOOK_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .await
        .context("failed reading Antigravity hook input")?;

    let hook_dir = non_empty_env("BUZZ_AGY_HOOK_DIR").map(PathBuf::from);
    if input.len() <= MAX_HOOK_INPUT_BYTES {
        if let (Some(hook_dir), Ok(payload)) =
            (hook_dir.as_deref(), serde_json::from_slice::<Value>(&input))
        {
            let _ = persist_hook_event(hook_dir, &event, payload).await;
        }
    }

    let response = hook_response(&event, hook_dir.is_some());
    let mut stdout = BufWriter::new(tokio::io::stdout());
    send_hook_json(&mut stdout, &response).await
}

fn hook_response(event: &str, observer_attached: bool) -> Value {
    match event {
        "pre-tool-use" if observer_attached => json!({ "decision": "allow" }),
        "pre-tool-use" => json!({
            "decision": "ask",
            "reason": "Buzz hook observer is not attached to a managed AGY turn"
        }),
        // Antigravity's Stop hook contract requires a decision. Any value
        // other than "continue" allows the completed execution to stop.
        "stop" => json!({ "decision": "stop" }),
        _ => json!({}),
    }
}

/// Run the Antigravity ACP compatibility bridge over stdin/stdout.
pub(crate) async fn run() -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut frames = FramedRead::new(stdin, LinesCodec::new_with_max_length(MAX_FRAME_BYTES));
    let mut stdout = BufWriter::new(tokio::io::stdout());
    let mut sessions = HashMap::<String, Session>::new();

    while let Some(frame) = frames.next().await {
        let frame = match frame {
            Ok(frame) => frame,
            Err(error) => {
                send_error(
                    &mut stdout,
                    Value::Null,
                    -32700,
                    &format!("invalid JSON-RPC frame: {error}"),
                )
                .await?;
                continue;
            }
        };
        let request = match serde_json::from_str::<Value>(&frame) {
            Ok(request) => request,
            Err(error) => {
                send_error(
                    &mut stdout,
                    Value::Null,
                    -32700,
                    &format!("invalid JSON: {error}"),
                )
                .await?;
                continue;
            }
        };

        let Some(method) = request.get("method").and_then(Value::as_str) else {
            if request.get("id").is_some() {
                send_error(
                    &mut stdout,
                    request.get("id").cloned().unwrap_or(Value::Null),
                    -32600,
                    "JSON-RPC request is missing method",
                )
                .await?;
            }
            continue;
        };
        let id = request.get("id").cloned();
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        match (method, id) {
            ("initialize", Some(id)) => {
                let requested_version = params
                    .get("protocolVersion")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                send_result(
                    &mut stdout,
                    id,
                    json!({
                        "protocolVersion": requested_version.min(2),
                        "agentCapabilities": {
                            "loadSession": false,
                            "promptCapabilities": {
                                "image": false,
                                "audio": false,
                                "embeddedContext": false
                            },
                            "mcpCapabilities": {
                                "http": false,
                                "sse": false
                            }
                        },
                        "agentInfo": {
                            "name": "Antigravity",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }),
                )
                .await?;
            }
            ("session/new", Some(id)) => {
                let Some(cwd) = params.get("cwd").and_then(Value::as_str) else {
                    send_error(
                        &mut stdout,
                        id,
                        -32602,
                        "session/new requires an absolute cwd",
                    )
                    .await?;
                    continue;
                };
                let cwd = PathBuf::from(cwd);
                if !cwd.is_absolute() {
                    send_error(
                        &mut stdout,
                        id,
                        -32602,
                        "session/new requires an absolute cwd",
                    )
                    .await?;
                    continue;
                }

                let session_id = Uuid::new_v4().to_string();
                sessions.insert(
                    session_id.clone(),
                    Session {
                        cwd,
                        system_prompt: params
                            .get("systemPrompt")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        conversation_id: None,
                    },
                );
                send_result(&mut stdout, id, json!({ "sessionId": session_id })).await?;
            }
            ("session/prompt", Some(id)) => {
                let Some(session_id) = params.get("sessionId").and_then(Value::as_str) else {
                    send_error(&mut stdout, id, -32602, "session/prompt requires sessionId")
                        .await?;
                    continue;
                };
                let Some(mut session) = sessions.get(session_id).cloned() else {
                    send_error(&mut stdout, id, -32602, "unknown sessionId").await?;
                    continue;
                };
                let prompt = match render_prompt(&session, &params) {
                    Ok(prompt) => prompt,
                    Err(error) => {
                        send_error(&mut stdout, id, -32602, &error.to_string()).await?;
                        continue;
                    }
                };

                let turn_result =
                    run_turn(&mut frames, &mut stdout, session_id, &mut session, prompt).await?;
                sessions.insert(session_id.to_string(), session);

                match turn_result {
                    TurnResult::Completed(output) if output.status.success() => {
                        if !output.stdout.is_empty() {
                            send_json(
                                &mut stdout,
                                &json!({
                                    "jsonrpc": "2.0",
                                    "method": "session/update",
                                    "params": {
                                        "sessionId": session_id,
                                        "update": {
                                            "sessionUpdate": "agent_message_chunk",
                                            "content": {
                                                "type": "text",
                                                "text": output.stdout
                                            }
                                        }
                                    }
                                }),
                            )
                            .await?;
                        }
                        send_result(&mut stdout, id, json!({ "stopReason": "end_turn" })).await?;
                    }
                    TurnResult::Completed(output) => {
                        let message = process_failure_message(&output);
                        send_error(&mut stdout, id, -32000, &message).await?;
                    }
                    TurnResult::Cancelled => {
                        send_result(&mut stdout, id, json!({ "stopReason": "cancelled" })).await?;
                    }
                    TurnResult::InputClosed => return Ok(()),
                }
            }
            ("session/cancel", _) => {
                // There is no active process while handling requests in this loop.
            }
            (_, Some(id)) => {
                send_error(&mut stdout, id, -32601, "method not found").await?;
            }
            (_, None) => {}
        }
    }

    Ok(())
}

fn render_prompt(session: &Session, params: &Value) -> Result<String> {
    let blocks = params
        .get("prompt")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("session/prompt requires a prompt array"))?;
    let user_prompt = blocks
        .iter()
        .filter_map(|block| {
            (block.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| block.get("text").and_then(Value::as_str))
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    if user_prompt.trim().is_empty() {
        return Err(anyhow!("session/prompt requires at least one text block"));
    }

    match session.system_prompt.as_deref().map(str::trim) {
        Some(system_prompt) if !system_prompt.is_empty() => Ok(format!(
            "System instructions:\n{system_prompt}\n\nBuzz conversation and request:\n{user_prompt}"
        )),
        _ => Ok(user_prompt),
    }
}

fn agy_args(conversation_id: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--dangerously-skip-permissions".to_string(),
        "--print-timeout".to_string(),
        std::env::var("BUZZ_AGY_PRINT_TIMEOUT")
            .unwrap_or_else(|_| DEFAULT_PRINT_TIMEOUT.to_string()),
    ];
    if let Some(model) = non_empty_env("BUZZ_AGY_MODEL") {
        args.extend(["--model".to_string(), model]);
    }
    if let Some(effort) = non_empty_env("BUZZ_AGY_EFFORT") {
        args.extend(["--effort".to_string(), effort]);
    }
    if let Some(conversation_id) = conversation_id {
        args.extend(["--conversation".to_string(), conversation_id.to_string()]);
    }
    args
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Whether a lightweight `buzz-acp` helper is targeting the bundled AGY bridge.
///
/// Model discovery receives the resolved executable path from Desktop, so this
/// must recognize both a bare `buzz-acp` command and an absolute sidecar path.
pub(crate) fn is_bridge_invocation(command: &str, args: &[String]) -> bool {
    let executable = command.rsplit(['/', '\\']).next().unwrap_or(command);
    let executable = executable.to_ascii_lowercase();
    let executable = executable.strip_suffix(".exe").unwrap_or(&executable);

    executable == "buzz-acp"
        && args
            .first()
            .is_some_and(|arg| arg.eq_ignore_ascii_case("agy-acp"))
}

/// Query AGY's native model catalog.
///
/// The compatibility bridge cannot advertise ACP live model switching because
/// AGY applies `--model` when each print process starts. Its native `models`
/// command is nevertheless authoritative for configuration-time selection.
pub(crate) async fn discover_models() -> Result<Vec<String>> {
    let command = non_empty_env("BUZZ_AGY_COMMAND").unwrap_or_else(|| "agy".to_string());
    let mut command_builder = Command::new(&command);
    command_builder
        .arg("models")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(path) = path_with_current_executable() {
        command_builder.env("PATH", path);
    }

    let mut child = command_builder
        .spawn()
        .with_context(|| format!("failed to start Antigravity CLI `{command} models`"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Antigravity model-list stdout pipe was unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Antigravity model-list stderr pipe was unavailable"))?;
    let stdout_task = tokio::spawn(read_process_output(stdout));
    let stderr_task = tokio::spawn(read_process_output(stderr));

    let status = match tokio::time::timeout(MODEL_LIST_TIMEOUT, child.wait()).await {
        Ok(status) => status.context("failed waiting for Antigravity model list")?,
        Err(_) => {
            stop_child(&mut child).await;
            let _ = finish_output_tasks(stdout_task, stderr_task).await;
            return Err(anyhow!(
                "Antigravity model discovery timed out after {MODEL_LIST_TIMEOUT:?}"
            ));
        }
    };
    let (stdout, stderr) = finish_output_tasks(stdout_task, stderr_task).await?;
    if !status.success() {
        return Err(anyhow!(process_failure_message(&AgyOutput {
            status,
            stdout,
            stderr,
        })));
    }

    let models = parse_model_list(&stdout);
    if models.is_empty() {
        return Err(anyhow!("Antigravity CLI returned no models"));
    }
    Ok(models)
}

fn parse_model_list(output: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    output
        .lines()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .filter(|model| seen.insert((*model).to_string()))
        .map(str::to_string)
        .collect()
}

async fn persist_hook_event(hook_dir: &Path, event: &str, payload: Value) -> Result<()> {
    let metadata = tokio::fs::metadata(hook_dir)
        .await
        .with_context(|| format!("hook directory {} is unavailable", hook_dir.display()))?;
    if !metadata.is_dir() {
        return Err(anyhow!(
            "hook event target {} is not a directory",
            hook_dir.display()
        ));
    }

    let event_id = Uuid::new_v4();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let filename = format!("{timestamp:020}-{event_id}");
    let pending_path = hook_dir.join(format!(".{filename}.tmp"));
    let completed_path = hook_dir.join(format!("{filename}.json"));
    let encoded = serde_json::to_vec(&json!({
        "event": event,
        "payload": payload,
    }))
    .context("failed serializing Antigravity hook event")?;
    tokio::fs::write(&pending_path, encoded)
        .await
        .with_context(|| format!("failed writing {}", pending_path.display()))?;
    tokio::fs::rename(&pending_path, &completed_path)
        .await
        .with_context(|| format!("failed publishing hook event {}", completed_path.display()))
}

async fn run_turn<S, W>(
    frames: &mut S,
    writer: &mut W,
    session_id: &str,
    session: &mut Session,
    prompt: String,
) -> Result<TurnResult>
where
    S: Stream<Item = std::result::Result<String, LinesCodecError>> + Unpin,
    W: AsyncWrite + Unpin,
{
    let command = non_empty_env("BUZZ_AGY_COMMAND").unwrap_or_else(|| "agy".to_string());
    let trajectories_before = snapshot_trajectory_ids().await;
    let hook_dir = std::env::temp_dir().join(format!("buzz-agy-hooks-{}", Uuid::new_v4()));
    tokio::fs::create_dir(&hook_dir)
        .await
        .with_context(|| format!("failed creating hook directory {}", hook_dir.display()))?;

    let mut command_builder = Command::new(&command);
    command_builder
        .args(agy_args(session.conversation_id.as_deref()))
        // AGY print mode does not infer workspace customizations from the
        // process cwd alone. Register the ACP workspace explicitly so
        // `.agents/hooks.json`, skills, and rules are loaded.
        .arg("--add-dir")
        .arg(&session.cwd)
        .arg("--print")
        .arg(prompt)
        .current_dir(&session.cwd)
        .env("BUZZ_AGY_HOOK_DIR", &hook_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(path) = path_with_current_executable() {
        command_builder.env("PATH", path);
    }
    let mut child = command_builder
        .spawn()
        .with_context(|| format!("failed to start Antigravity CLI `{command}`"))?;

    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Antigravity stdout pipe was unavailable"))?;
    let child_stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Antigravity stderr pipe was unavailable"))?;
    let stdout_task = tokio::spawn(read_process_output(child_stdout));
    let stderr_task = tokio::spawn(read_process_output(child_stderr));
    let mut hook_interval = tokio::time::interval(HOOK_POLL_INTERVAL);
    hook_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut seen_hook_events = HashSet::new();
    let mut active_tool_calls = HashSet::new();

    let status = loop {
        tokio::select! {
            status = child.wait() => break status.context("failed waiting for Antigravity CLI")?,
            _ = hook_interval.tick() => {
                emit_hook_events(
                    writer,
                    session_id,
                    &hook_dir,
                    &mut seen_hook_events,
                    &mut active_tool_calls,
                    &mut session.conversation_id,
                )
                .await?;
            }
            frame = frames.next() => {
                match frame {
                    Some(Ok(frame)) if is_cancel_for_session(&frame, session_id) => {
                        stop_child(&mut child).await;
                        emit_hook_events(
                            writer,
                            session_id,
                            &hook_dir,
                            &mut seen_hook_events,
                            &mut active_tool_calls,
                            &mut session.conversation_id,
                        )
                        .await?;
                        finish_output_tasks(stdout_task, stderr_task).await?;
                        remove_hook_dir(&hook_dir).await;
                        return Ok(TurnResult::Cancelled);
                    }
                    Some(Ok(frame)) => {
                        reject_while_busy(writer, &frame).await?;
                    }
                    Some(Err(error)) => {
                        send_error(
                            writer,
                            Value::Null,
                            -32700,
                            &format!("invalid JSON-RPC frame: {error}"),
                        )
                        .await?;
                    }
                    None => {
                        stop_child(&mut child).await;
                        emit_hook_events(
                            writer,
                            session_id,
                            &hook_dir,
                            &mut seen_hook_events,
                            &mut active_tool_calls,
                            &mut session.conversation_id,
                        )
                        .await?;
                        finish_output_tasks(stdout_task, stderr_task).await?;
                        remove_hook_dir(&hook_dir).await;
                        return Ok(TurnResult::InputClosed);
                    }
                }
            }
        }
    };

    emit_hook_events(
        writer,
        session_id,
        &hook_dir,
        &mut seen_hook_events,
        &mut active_tool_calls,
        &mut session.conversation_id,
    )
    .await?;
    if session.conversation_id.is_none() {
        capture_new_trajectory(trajectories_before.as_ref(), &mut session.conversation_id).await;
    }
    let (stdout, stderr) = finish_output_tasks(stdout_task, stderr_task).await?;
    remove_hook_dir(&hook_dir).await;
    Ok(TurnResult::Completed(AgyOutput {
        status,
        stdout,
        stderr,
    }))
}

fn path_with_current_executable() -> Option<std::ffi::OsString> {
    let executable_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let mut paths = vec![executable_dir];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).ok()
}

async fn emit_hook_events<W>(
    writer: &mut W,
    session_id: &str,
    hook_dir: &Path,
    seen: &mut HashSet<PathBuf>,
    active_tool_calls: &mut HashSet<String>,
    conversation_id: &mut Option<String>,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut directory = tokio::fs::read_dir(hook_dir)
        .await
        .with_context(|| format!("failed reading hook directory {}", hook_dir.display()))?;
    let mut paths = Vec::new();
    while let Some(entry) = directory
        .next_entry()
        .await
        .context("failed reading Antigravity hook directory entry")?
    {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("json")
            && !seen.contains(&path)
        {
            paths.push(path);
        }
    }
    paths.sort();

    for path in paths {
        let encoded = tokio::fs::read(&path)
            .await
            .with_context(|| format!("failed reading hook event {}", path.display()))?;
        let hook = serde_json::from_slice::<Value>(&encoded)
            .with_context(|| format!("invalid hook event {}", path.display()))?;
        capture_completed_conversation(&hook, conversation_id);
        for update in hook_session_updates(session_id, &hook, active_tool_calls) {
            send_json(writer, &update).await?;
        }
        seen.insert(path);
    }
    Ok(())
}

fn capture_completed_conversation(hook: &Value, conversation_id: &mut Option<String>) {
    if hook.get("event").and_then(Value::as_str) != Some("stop") {
        return;
    }
    let payload = &hook["payload"];
    if payload.get("fullyIdle").and_then(Value::as_bool) != Some(true) {
        return;
    }
    if let Some(id) = payload
        .get("conversationId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        *conversation_id = Some(id.to_string());
    }
}

fn agy_brain_dir() -> Option<PathBuf> {
    non_empty_env("BUZZ_AGY_APP_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".gemini").join("antigravity-cli")))
        .map(|app_data| app_data.join("brain"))
}

async fn snapshot_trajectory_ids() -> Option<HashSet<String>> {
    let brain_dir = agy_brain_dir()?;
    let mut entries = match tokio::fs::read_dir(&brain_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(HashSet::new());
        }
        Err(error) => {
            tracing::debug!(
                %error,
                path = %brain_dir.display(),
                "failed reading AGY trajectory directory"
            );
            return None;
        }
    };
    let mut ids = HashSet::new();
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(error) => {
                tracing::debug!(
                    %error,
                    path = %brain_dir.display(),
                    "failed reading AGY trajectory directory entry"
                );
                return None;
            }
        };
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if Uuid::parse_str(&id).is_err() {
            continue;
        }
        let transcript = entry
            .path()
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        if tokio::fs::metadata(transcript)
            .await
            .is_ok_and(|metadata| metadata.is_file())
        {
            ids.insert(id);
        }
    }
    Some(ids)
}

async fn capture_new_trajectory(
    before: Option<&HashSet<String>>,
    conversation_id: &mut Option<String>,
) {
    let (Some(before), Some(after)) = (before, snapshot_trajectory_ids().await) else {
        return;
    };
    let mut new_ids = after.difference(before);
    let Some(id) = new_ids.next() else {
        tracing::debug!("AGY did not publish a new trajectory transcript");
        return;
    };
    if new_ids.next().is_some() {
        tracing::debug!(
            "AGY published multiple new trajectories; refusing to guess which one to resume"
        );
        return;
    }
    *conversation_id = Some(id.clone());
}

fn hook_session_updates(
    session_id: &str,
    hook: &Value,
    active_tool_calls: &mut HashSet<String>,
) -> Vec<Value> {
    let event = hook
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let payload = hook.get("payload").cloned().unwrap_or(Value::Null);
    let conversation_id = payload
        .get("conversationId")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let step = payload.get("stepIdx").and_then(Value::as_u64).unwrap_or(0);
    let tool_call_id = format!("agy-{conversation_id}-{step}");
    let metadata = json!({
        "antigravity": {
            "event": event,
            "conversationId": payload.get("conversationId"),
            "stepIdx": payload.get("stepIdx"),
            "transcriptPath": payload.get("transcriptPath"),
            "artifactDirectoryPath": payload.get("artifactDirectoryPath")
        }
    });

    let update = match event {
        "pre-tool-use" => {
            active_tool_calls.insert(tool_call_id.clone());
            let tool_call = payload.get("toolCall").cloned().unwrap_or(Value::Null);
            let title = tool_call
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Antigravity tool");
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": tool_call_id,
                "title": title,
                "kind": acp_tool_kind(title),
                "status": "in_progress",
                "rawInput": tool_call.get("args").cloned().unwrap_or(Value::Null),
                "_meta": metadata
            })
        }
        "post-tool-use" => {
            // AGY 1.1.x can emit PostToolUse payloads with `toolCall: null`
            // for internal model steps. Only complete calls for which Buzz
            // observed the matching PreToolUse event.
            if !active_tool_calls.remove(&tool_call_id) {
                return Vec::new();
            }
            let error = payload.get("error").and_then(Value::as_str).unwrap_or("");
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": tool_call_id,
                "status": if error.is_empty() { "completed" } else { "failed" },
                "rawOutput": if error.is_empty() {
                    json!({ "isError": false })
                } else {
                    json!({ "isError": true, "error": error })
                },
                "_meta": metadata
            })
        }
        _ => {
            json!({
                "sessionUpdate": "session_info_update",
                "_meta": metadata
            })
        }
    };

    vec![json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": update
        }
    })]
}

fn acp_tool_kind(name: &str) -> &'static str {
    match name {
        "write_to_file" | "replace_file_content" | "multi_replace_file_content" => "edit",
        "view_file" | "list_dir" => "read",
        "grep_search" | "find_by_name" | "codebase_search" | "search_web" => "search",
        "run_command" => "execute",
        name if name.starts_with("browser_") => "fetch",
        _ => "other",
    }
}

async fn remove_hook_dir(path: &Path) {
    if let Err(error) = tokio::fs::remove_dir_all(path).await {
        tracing::debug!(%error, path = %path.display(), "failed removing AGY hook directory");
    }
}

fn is_cancel_for_session(frame: &str, session_id: &str) -> bool {
    serde_json::from_str::<Value>(frame).is_ok_and(|request| {
        request.get("method").and_then(Value::as_str) == Some("session/cancel")
            && request
                .get("params")
                .and_then(|params| params.get("sessionId"))
                .and_then(Value::as_str)
                == Some(session_id)
    })
}

async fn reject_while_busy<W>(writer: &mut W, frame: &str) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    match serde_json::from_str::<Value>(frame) {
        Ok(request) => {
            if let Some(id) = request.get("id").cloned() {
                send_error(
                    writer,
                    id,
                    -32000,
                    "Antigravity is already processing a turn",
                )
                .await?;
            }
        }
        Err(error) => {
            send_error(
                writer,
                Value::Null,
                -32700,
                &format!("invalid JSON: {error}"),
            )
            .await?;
        }
    }
    Ok(())
}

async fn stop_child(child: &mut Child) {
    if let Err(error) = child.kill().await {
        tracing::debug!(%error, "Antigravity process had already exited");
    }
    let _ = child.wait().await;
}

async fn read_process_output<R>(reader: R) -> Result<String>
where
    R: AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    reader
        .take((MAX_PROCESS_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .context("failed reading Antigravity process output")?;
    if bytes.len() > MAX_PROCESS_OUTPUT_BYTES {
        return Err(anyhow!(
            "Antigravity process output exceeded {} bytes",
            MAX_PROCESS_OUTPUT_BYTES
        ));
    }
    String::from_utf8(bytes).context("Antigravity process output was not UTF-8")
}

async fn finish_output_tasks(
    stdout_task: tokio::task::JoinHandle<Result<String>>,
    stderr_task: tokio::task::JoinHandle<Result<String>>,
) -> Result<(String, String)> {
    let stdout = stdout_task
        .await
        .context("Antigravity stdout reader task failed")??;
    let stderr = stderr_task
        .await
        .context("Antigravity stderr reader task failed")??;
    Ok((stdout, stderr))
}

fn process_failure_message(output: &AgyOutput) -> String {
    let stderr = output.stderr.trim();
    if stderr.is_empty() {
        format!("Antigravity CLI exited with {}", output.status)
    } else {
        format!("Antigravity CLI exited with {}: {stderr}", output.status)
    }
}

async fn send_result<W>(writer: &mut W, id: Value, result: Value) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    send_json(
        writer,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    )
    .await
}

async fn send_error<W>(writer: &mut W, id: Value, code: i32, message: &str) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    send_json(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        }),
    )
    .await
}

async fn send_json<W>(writer: &mut W, value: &Value) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let line = serde_json::to_vec(value).context("failed serializing JSON-RPC response")?;
    writer
        .write_all(&line)
        .await
        .context("failed writing JSON-RPC response")?;
    writer
        .write_all(b"\n")
        .await
        .context("failed terminating JSON-RPC response")?;
    writer
        .flush()
        .await
        .context("failed flushing JSON-RPC response")
}

async fn send_hook_json<W>(writer: &mut W, value: &Value) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let encoded = serde_json::to_vec(value).context("failed serializing hook response")?;
    writer
        .write_all(&encoded)
        .await
        .context("failed writing hook response")?;
    writer
        .flush()
        .await
        .context("failed flushing hook response")
}

#[cfg(test)]
mod tests {
    use super::{
        acp_tool_kind, agy_args, capture_completed_conversation, hook_response,
        hook_session_updates, is_bridge_invocation, parse_model_list, process_failure_message,
        render_prompt, AgyOutput, Session,
    };
    use serde_json::json;
    use std::collections::HashSet;
    use std::path::PathBuf;

    #[test]
    fn renders_system_prompt_and_all_text_blocks() {
        let session = Session {
            cwd: PathBuf::from("/tmp"),
            system_prompt: Some("Be concise.".to_string()),
            conversation_id: None,
        };
        let rendered = render_prompt(
            &session,
            &json!({
                "prompt": [
                    { "type": "text", "text": "First" },
                    { "type": "image", "data": "ignored" },
                    { "type": "text", "text": "Second" }
                ]
            }),
        )
        .expect("prompt should render");

        assert_eq!(
            rendered,
            "System instructions:\nBe concise.\n\nBuzz conversation and request:\nFirst\n\nSecond"
        );
    }

    #[test]
    fn agy_arguments_enable_noninteractive_permissions() {
        let args = agy_args(None);

        assert!(args
            .iter()
            .any(|arg| arg == "--dangerously-skip-permissions"));
    }

    #[test]
    fn resumed_conversation_is_passed_before_prompt() {
        let args = agy_args(Some("conversation-1"));
        let conversation_flag = args
            .iter()
            .position(|arg| arg == "--conversation")
            .expect("conversation flag");

        assert_eq!(
            args.get(conversation_flag + 1).map(String::as_str),
            Some("conversation-1")
        );
    }

    #[test]
    fn recognizes_resolved_antigravity_bridge_invocation() {
        assert!(is_bridge_invocation(
            "/Applications/Buzz.app/Contents/MacOS/buzz-acp",
            &["agy-acp".to_string()]
        ));
        assert!(is_bridge_invocation(
            r"C:\Program Files\Buzz\buzz-acp.exe",
            &["AGY-ACP".to_string()]
        ));
        assert!(!is_bridge_invocation(
            "buzz-acp",
            &["other-adapter".to_string()]
        ));
        assert!(!is_bridge_invocation("agy", &["agy-acp".to_string()]));
    }

    #[test]
    fn parses_and_deduplicates_native_model_list() {
        assert_eq!(
            parse_model_list(
                "gemini-3.6-flash-high\n\n claude-sonnet-4-6 \ngemini-3.6-flash-high\n"
            ),
            vec!["gemini-3.6-flash-high", "claude-sonnet-4-6"]
        );
    }

    #[test]
    fn failure_message_includes_stderr() {
        let output = AgyOutput {
            status: std::process::Command::new("sh")
                .args(["-c", "exit 7"])
                .status()
                .expect("shell should run"),
            stdout: String::new(),
            stderr: "login required\n".to_string(),
        };

        let message = process_failure_message(&output);
        assert!(message.contains("status: 7"));
        assert!(message.contains("login required"));
    }

    #[test]
    fn maps_antigravity_edit_hook_to_acp_tool_updates() {
        let mut active_tool_calls = HashSet::new();
        let updates = hook_session_updates(
            "session-1",
            &json!({
                "event": "pre-tool-use",
                "payload": {
                    "conversationId": "conversation-1",
                    "stepIdx": 4,
                    "transcriptPath": "/tmp/transcript.jsonl",
                    "artifactDirectoryPath": "/tmp/artifacts",
                    "toolCall": {
                        "name": "replace_file_content",
                        "args": {
                            "TargetFile": "/workspace/src/lib.rs",
                            "ReplacementContent": "replacement"
                        }
                    }
                }
            }),
            &mut active_tool_calls,
        );

        assert_eq!(updates.len(), 1);
        let update = &updates[0]["params"]["update"];
        assert_eq!(update["sessionUpdate"], "tool_call");
        assert_eq!(update["toolCallId"], "agy-conversation-1-4");
        assert_eq!(update["kind"], "edit");
        assert_eq!(update["rawInput"]["ReplacementContent"], "replacement");
        assert_eq!(
            update["_meta"]["antigravity"]["transcriptPath"],
            "/tmp/transcript.jsonl"
        );
    }

    #[test]
    fn maps_antigravity_post_hook_error_to_failed_update() {
        let mut active_tool_calls = HashSet::new();
        let pre_hook = json!({
            "event": "pre-tool-use",
            "payload": {
                "conversationId": "conversation-1",
                "stepIdx": 4,
                "toolCall": { "name": "run_command", "args": {} }
            }
        });
        hook_session_updates("session-1", &pre_hook, &mut active_tool_calls);
        let updates = hook_session_updates(
            "session-1",
            &json!({
                "event": "post-tool-use",
                "payload": {
                    "conversationId": "conversation-1",
                    "stepIdx": 4,
                    "error": "exit status 1"
                }
            }),
            &mut active_tool_calls,
        );

        let update = &updates[0]["params"]["update"];
        assert_eq!(update["sessionUpdate"], "tool_call_update");
        assert_eq!(update["status"], "failed");
        assert_eq!(update["rawOutput"]["error"], "exit status 1");
    }

    #[test]
    fn ignores_unmatched_antigravity_post_hook() {
        let updates = hook_session_updates(
            "session-1",
            &json!({
                "event": "post-tool-use",
                "payload": {
                    "conversationId": "conversation-1",
                    "stepIdx": 4,
                    "toolCall": null,
                    "error": ""
                }
            }),
            &mut HashSet::new(),
        );

        assert!(updates.is_empty());
    }

    #[test]
    fn maps_known_antigravity_tool_kinds() {
        assert_eq!(acp_tool_kind("view_file"), "read");
        assert_eq!(acp_tool_kind("run_command"), "execute");
        assert_eq!(acp_tool_kind("browser_click"), "fetch");
        assert_eq!(acp_tool_kind("invoke_subagent"), "other");
    }

    #[test]
    fn emits_documented_hook_decisions() {
        assert_eq!(hook_response("pre-tool-use", true)["decision"], "allow");
        assert_eq!(hook_response("pre-tool-use", false)["decision"], "ask");
        assert_eq!(hook_response("stop", true)["decision"], "stop");
        assert_eq!(hook_response("post-tool-use", true), json!({}));
    }

    #[test]
    fn captures_only_fully_idle_stop_conversation() {
        let mut conversation_id = None;
        capture_completed_conversation(
            &json!({
                "event": "pre-tool-use",
                "payload": { "conversationId": "wrong", "fullyIdle": true }
            }),
            &mut conversation_id,
        );
        capture_completed_conversation(
            &json!({
                "event": "stop",
                "payload": { "conversationId": "also-wrong", "fullyIdle": false }
            }),
            &mut conversation_id,
        );
        assert_eq!(conversation_id, None);

        capture_completed_conversation(
            &json!({
                "event": "stop",
                "payload": { "conversationId": "conversation-1", "fullyIdle": true }
            }),
            &mut conversation_id,
        );
        assert_eq!(conversation_id.as_deref(), Some("conversation-1"));
    }
}
