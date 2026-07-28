use crate::{
    a2a::{invoke, negotiate_extensions},
    oasf::{load_record, resolve_card, CardSource},
    AdapterConfig, AdapterError, MAX_ACP_LINE_BYTES,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PromptBlock {
    Text {
        text: String,
    },
    #[serde(other)]
    Unsupported,
}

#[derive(Debug, Deserialize)]
struct PromptParams {
    #[serde(rename = "sessionId")]
    session_id: String,
    prompt: Vec<PromptBlock>,
}

#[derive(Debug, Deserialize)]
struct CancelParams {
    #[serde(rename = "sessionId")]
    session_id: String,
}

fn prompt_text(blocks: &[PromptBlock]) -> Result<String, AdapterError> {
    let text = blocks
        .iter()
        .filter_map(|block| match block {
            PromptBlock::Text { text } => Some(text.as_str()),
            PromptBlock::Unsupported => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return Err(AdapterError::Acp("prompt contains no text content".into()));
    }
    Ok(text)
}

async fn send_json<W: AsyncWrite + Unpin>(
    writer: &mut W,
    value: Value,
) -> Result<(), AdapterError> {
    let mut line = serde_json::to_vec(&value)
        .map_err(|e| AdapterError::Acp(format!("encode response: {e}")))?;
    line.push(b'\n');
    writer
        .write_all(&line)
        .await
        .map_err(|e| AdapterError::Acp(format!("write response: {e}")))?;
    writer
        .flush()
        .await
        .map_err(|e| AdapterError::Acp(format!("flush response: {e}")))?;
    Ok(())
}

pub(super) enum AcpAction {
    Response(Value),
    Prompt {
        id: Value,
        session_id: String,
        text: String,
    },
    Cancel {
        id: Option<Value>,
        session_id: String,
    },
}

pub(super) fn handle_acp_message(
    message: &Value,
    sessions: &mut HashSet<String>,
    agent_name: &str,
    configured_context_id: Option<&str>,
) -> Result<Option<AcpAction>, AdapterError> {
    let method = message.get("method").and_then(Value::as_str);
    if method == Some("session/cancel") {
        let params: CancelParams =
            serde_json::from_value(message.get("params").cloned().unwrap_or(Value::Null))
                .map_err(|e| AdapterError::Acp(format!("session/cancel params: {e}")))?;
        return Ok(Some(AcpAction::Cancel {
            id: message.get("id").cloned(),
            session_id: params.session_id,
        }));
    }
    let Some(id) = message.get("id").cloned() else {
        return Ok(None);
    };
    match method {
        Some("initialize") => Ok(Some(AcpAction::Response(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": message.pointer("/params/protocolVersion").and_then(Value::as_u64).unwrap_or(1).min(1),
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": { "image": false, "audio": false, "embeddedContext": false },
                    "mcpCapabilities": { "http": false, "sse": false },
                },
                "agentInfo": { "name": agent_name, "version": "oasf-a2a" },
            }
        })))),
        Some("session/new") => {
            let session_id = configured_context_id
                .map(str::to_owned)
                .unwrap_or_else(|| format!("a2a-{}", Uuid::new_v4()));
            sessions.insert(session_id.clone());
            Ok(Some(AcpAction::Response(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "sessionId": session_id },
            }))))
        }
        Some("session/prompt") => {
            let params: PromptParams =
                serde_json::from_value(message.get("params").cloned().unwrap_or(Value::Null))
                    .map_err(|e| AdapterError::Acp(format!("session/prompt params: {e}")))?;
            if !sessions.contains(&params.session_id) {
                return Ok(Some(AcpAction::Response(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32602, "message": "unknown session" },
                }))));
            }
            let text = match prompt_text(&params.prompt) {
                Ok(text) => text,
                Err(error) => {
                    return Ok(Some(AcpAction::Response(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": error.to_string() },
                    }))));
                }
            };
            Ok(Some(AcpAction::Prompt {
                id,
                session_id: params.session_id,
                text,
            }))
        }
        Some(method) => Ok(Some(AcpAction::Response(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {method}") },
        })))),
        None => Ok(None),
    }
}

pub(super) fn prompt_success(id: Value, session_id: &str, text: &str) -> [Value; 2] {
    [
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": text },
                }
            }
        }),
        json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": "end_turn" } }),
    ]
}

struct ActivePrompt {
    id: Value,
    session_id: String,
    task: tokio::task::JoinHandle<Result<String, AdapterError>>,
}

enum LoopEvent {
    Input(Option<Result<Option<String>, AdapterError>>),
    PromptFinished(Result<Result<String, AdapterError>, tokio::task::JoinError>),
}

/// Run the adapter over ACP JSON-RPC lines on stdin/stdout.
pub async fn run(config: AdapterConfig) -> Result<(), AdapterError> {
    let record = load_record(&config.record).await?;
    eprintln!(
        "buzz-a2a-acp: resolved Agent Record {} ({})",
        record.content_digest,
        record.verification.label()
    );
    let (resolved, source) = resolve_card(record.record, record.base.as_ref()).await?;
    let extensions = negotiate_extensions(&resolved.card, &config.extensions, &resolved.mode)?;
    if source == CardSource::DeprecatedCardData {
        eprintln!(
            "buzz-a2a-acp: using deprecated OASF integration/a2a data.card_data compatibility path"
        );
    }
    let mut sessions = HashSet::new();
    let mut lines = spawn_line_reader(BufReader::new(tokio::io::stdin()));
    let mut writer = tokio::io::stdout();
    let mut active_prompt: Option<ActivePrompt> = None;
    loop {
        let event = if let Some(active) = active_prompt.as_mut() {
            tokio::select! {
                line = lines.recv() => LoopEvent::Input(line),
                result = &mut active.task => LoopEvent::PromptFinished(result),
            }
        } else {
            LoopEvent::Input(lines.recv().await)
        };
        match event {
            LoopEvent::PromptFinished(result) => {
                let Some(active) = active_prompt.take() else {
                    return Err(AdapterError::Acp(
                        "prompt completed without an active request".into(),
                    ));
                };
                match result {
                    Ok(Ok(text)) => {
                        for value in prompt_success(active.id, &active.session_id, &text) {
                            send_json(&mut writer, value).await?;
                        }
                    }
                    Ok(Err(error)) => {
                        send_json(
                            &mut writer,
                            json!({ "jsonrpc": "2.0", "id": active.id, "error": { "code": -32000, "message": error.to_string() } }),
                        )
                        .await?;
                    }
                    Err(error) => {
                        send_json(
                            &mut writer,
                            json!({ "jsonrpc": "2.0", "id": active.id, "error": { "code": -32000, "message": format!("remote prompt task failed: {error}") } }),
                        )
                        .await?;
                    }
                }
            }
            LoopEvent::Input(None | Some(Ok(None))) => return Ok(()),
            LoopEvent::Input(Some(Err(error))) => {
                eprintln!("buzz-a2a-acp: ignored malformed ACP input: {error}");
            }
            LoopEvent::Input(Some(Ok(Some(line)))) => {
                let message: Value = match serde_json::from_str(line.trim()) {
                    Ok(message) => message,
                    Err(error) => {
                        eprintln!("buzz-a2a-acp: ignored malformed JSON-RPC line: {error}");
                        continue;
                    }
                };
                let action = match handle_acp_message(
                    &message,
                    &mut sessions,
                    resolved.card.name.as_deref().unwrap_or("remote-a2a-agent"),
                    config.context_id.as_deref(),
                ) {
                    Ok(action) => action,
                    Err(error) => {
                        if let Some(id) = message.get("id").cloned() {
                            send_json(
                                &mut writer,
                                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": error.to_string() } }),
                            )
                            .await?;
                        } else {
                            eprintln!("buzz-a2a-acp: ignored invalid notification: {error}");
                        }
                        continue;
                    }
                };
                match action {
                    Some(AcpAction::Response(response)) => send_json(&mut writer, response).await?,
                    Some(AcpAction::Prompt {
                        id,
                        session_id,
                        text,
                    }) => {
                        if active_prompt.is_some() {
                            send_json(
                                &mut writer,
                                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32001, "message": "another prompt is already active" } }),
                            )
                            .await?;
                            continue;
                        }
                        let prompt_resolved = resolved.clone();
                        let prompt_token = config.bearer_token.clone();
                        let prompt_token_endpoint = config.bearer_token_endpoint.clone();
                        let prompt_task_poll_secs = config.task_poll_secs;
                        let prompt_extensions = extensions.clone();
                        let prompt_session_id = session_id.clone();
                        let task = tokio::spawn(async move {
                            invoke(
                                &prompt_resolved,
                                prompt_token.as_deref(),
                                prompt_token_endpoint.as_deref(),
                                &prompt_extensions,
                                prompt_task_poll_secs,
                                &prompt_session_id,
                                &text,
                            )
                            .await
                        });
                        active_prompt = Some(ActivePrompt {
                            id,
                            session_id,
                            task,
                        });
                    }
                    Some(AcpAction::Cancel { id, session_id }) => {
                        if active_prompt
                            .as_ref()
                            .is_some_and(|active| active.session_id == session_id)
                        {
                            let Some(active) = active_prompt.take() else {
                                return Err(AdapterError::Acp(
                                    "matching prompt disappeared during cancellation".into(),
                                ));
                            };
                            active.task.abort();
                            send_json(
                                &mut writer,
                                json!({ "jsonrpc": "2.0", "id": active.id, "result": { "stopReason": "cancelled" } }),
                            )
                            .await?;
                            if let Some(id) = id {
                                send_json(
                                    &mut writer,
                                    json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
                                )
                                .await?;
                            }
                        } else if let Some(id) = id {
                            send_json(
                                &mut writer,
                                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": "no active prompt for session" } }),
                            )
                            .await?;
                        }
                    }
                    None => {}
                }
            }
        }
    }
}

pub(super) fn spawn_line_reader<R>(
    mut reader: R,
) -> tokio::sync::mpsc::Receiver<Result<Option<String>, AdapterError>>
where
    R: tokio::io::AsyncBufRead + Send + Unpin + 'static,
{
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    tokio::spawn(async move {
        loop {
            let line = read_bounded_line(&mut reader).await;
            let reached_eof = matches!(line, Ok(None));
            let transport_failed = matches!(line, Err(AdapterError::Read { .. }));
            if sender.send(line).await.is_err() || reached_eof || transport_failed {
                break;
            }
        }
    });
    receiver
}

pub(super) async fn read_bounded_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<String>, AdapterError> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader
            .fill_buf()
            .await
            .map_err(|source| AdapterError::Read {
                what: "ACP request",
                source,
            })?;
        if chunk.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            return Err(AdapterError::Acp("unterminated request at EOF".into()));
        }
        let take = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > MAX_ACP_LINE_BYTES {
            let ended = chunk[..take].ends_with(b"\n");
            reader.consume(take);
            if !ended {
                discard_until_newline(reader).await?;
            }
            return Err(AdapterError::Acp("request exceeds 1 MiB".into()));
        }
        bytes.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if bytes.ends_with(b"\n") {
            bytes.pop();
            if bytes.ends_with(b"\r") {
                bytes.pop();
            }
            return String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| AdapterError::Acp("request is not UTF-8".into()));
        }
    }
}

async fn discard_until_newline<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<(), AdapterError> {
    loop {
        let chunk = reader
            .fill_buf()
            .await
            .map_err(|source| AdapterError::Read {
                what: "ACP request",
                source,
            })?;
        if chunk.is_empty() {
            return Ok(());
        }
        let take = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |index| index + 1);
        let ended = chunk[..take].ends_with(b"\n");
        reader.consume(take);
        if ended {
            return Ok(());
        }
    }
}
