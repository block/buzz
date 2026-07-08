use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::types::{ContentBlock, McpServerStdio};

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;

pub enum WireMsg {
    Notify(Value),
}

pub type WireSender = mpsc::Sender<WireMsg>;

#[derive(Debug)]
pub enum Inbound {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Ignored,
    Invalid {
        id: Value,
        code: i32,
        message: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(default, rename = "clientCapabilities")]
    pub _client_capabilities: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewParams {
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerStdio>,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPromptParams {
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCancelParams {
    pub session_id: String,
}

/// Params for goose's non-standard `_goose/unstable/session/steer` request:
/// inject user input into the *currently active* prompt without starting a new
/// one. `expected_run_id` must match the run id buzz-agent advertised via
/// `params.update._meta.goose.activeRunId` on a `session/update`, so a steer
/// can't race a turn that already ended or hasn't started.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSteerParams {
    pub session_id: String,
    #[serde(default)]
    pub prompt: Vec<ContentBlock>,
    pub expected_run_id: String,
}

/// Params for `session/set_model`: override the active model for an existing
/// session without respawning. Applied immediately; subsequent prompts on this
/// session use `model_id` instead of the configured `BUZZ_AGENT_MODEL`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSetModelParams {
    pub session_id: String,
    pub model_id: String,
}

pub fn classify(msg: &Value) -> Inbound {
    if !msg.is_object() || msg.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Inbound::Invalid {
            id: msg.get("id").cloned().unwrap_or(Value::Null),
            code: INVALID_REQUEST,
            message: "jsonrpc: missing or invalid version".into(),
        };
    }
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    match (method, id) {
        (Some(m), Some(id)) => Inbound::Request {
            id,
            method: m,
            params,
        },
        (Some(m), None) => Inbound::Notification { method: m, params },
        // Bare responses (id present, no method) are unexpected — buzz-agent
        // does not issue requests to the client. Ignore silently.
        (None, Some(_)) => Inbound::Ignored,
        (None, None) => Inbound::Invalid {
            id: Value::Null,
            code: INVALID_REQUEST,
            message: "jsonrpc: missing method and id".into(),
        },
    }
}

pub fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn err(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

pub fn session_update(sid: &str, update: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": { "sessionId": sid, "update": update },
    })
}

/// A `_goose/unstable/session/update` notification — the separate top-level
/// method goose uses for custom usage and status events.  Used by buzz-agent
/// to emit the `usage_update` payload so buzz-acp's `UsageTracker` can treat
/// buzz-agent and goose symmetrically.
pub fn goose_session_update(sid: &str, update: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "_goose/unstable/session/update",
        "params": { "sessionId": sid, "update": update },
    })
}

/// A `session/update` notification carrying a `update._meta.goose.<key>` field.
/// Used to advertise `activeRunId` (so steer-capable clients can target the
/// in-flight run) and `queuedSteer` (so they can correlate an accepted steer
/// with the chunk that later picks it up) — matching goose's wire layout where
/// `_meta` is nested inside the `update` object (per the ACP `SessionInfoUpdate`
/// schema), not alongside it at the params level.
pub fn session_update_with_goose_meta(sid: &str, update: Value, goose_meta: Value) -> Value {
    let mut update = update;
    update["_meta"] = json!({ "goose": goose_meta });
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": sid,
            "update": update,
        },
    })
}

pub async fn send(wire: &WireSender, msg: Value) {
    let _ = wire.send(WireMsg::Notify(msg)).await;
}

pub async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    stdin: &mut R,
    max: usize,
) -> std::io::Result<Option<String>> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let chunk = stdin.fill_buf().await?;
        if chunk.is_empty() {
            if !buf.is_empty() {
                tracing::error!(
                    "io: unterminated frame at EOF ({} bytes dropped)",
                    buf.len()
                );
            }
            return Ok(None);
        }
        let take = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map_or(chunk.len(), |i| i + 1);
        if buf.len().saturating_add(take) > max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("io: line exceeds max ({max} bytes)"),
            ));
        }
        buf.extend_from_slice(&chunk[..take]);
        stdin.consume(take);
        if buf.ends_with(b"\n") {
            buf.pop();
            if buf.ends_with(b"\r") {
                buf.pop();
            }
            match String::from_utf8(buf) {
                Ok(s) => return Ok(Some(s)),
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "io: frame contains invalid UTF-8",
                    ))
                }
            }
        }
    }
}

pub async fn writer_task(mut rx: mpsc::Receiver<WireMsg>) {
    let mut stdout = tokio::io::stdout();
    while let Some(msg) = rx.recv().await {
        let WireMsg::Notify(v) = msg;
        let mut s = match serde_json::to_string(&v) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("io: serialize: {e}");
                continue;
            }
        };
        s.push('\n');
        if stdout.write_all(s.as_bytes()).await.is_err() {
            return;
        }
        let _ = stdout.flush().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_new_params_deserializes_system_prompt() {
        let json = serde_json::json!({
            "cwd": "/tmp/test",
            "mcpServers": [],
            "systemPrompt": "You are a helpful agent."
        });
        let params: SessionNewParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.cwd, "/tmp/test");
        assert_eq!(
            params.system_prompt.as_deref(),
            Some("You are a helpful agent.")
        );
    }

    #[test]
    fn session_new_params_system_prompt_defaults_to_none() {
        let json = serde_json::json!({
            "cwd": "/tmp/test",
            "mcpServers": []
        });
        let params: SessionNewParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.cwd, "/tmp/test");
        assert!(params.system_prompt.is_none());
    }

    #[test]
    fn session_new_params_ignores_unknown_fields() {
        // Backward compat: old agents with new harness — unknown fields are ignored.
        let json = serde_json::json!({
            "cwd": "/tmp/test",
            "mcpServers": [],
            "unknownField": "should be ignored"
        });
        let params: SessionNewParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.cwd, "/tmp/test");
        assert!(params.system_prompt.is_none());
    }

    #[test]
    fn session_new_params_empty_string_system_prompt() {
        // An explicit empty string is distinct from absent — deserializes to Some("").
        let json = serde_json::json!({
            "cwd": "/tmp/test",
            "mcpServers": [],
            "systemPrompt": ""
        });
        let params: SessionNewParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.system_prompt, Some(String::new()));
    }
}
