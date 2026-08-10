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

/// Build the `usage_update` payload for a `_goose/unstable/session/update`.
///
/// Shared by the two places that report usage — after each LLM round inside a
/// turn, and once more when the turn completes — so the wire shape cannot drift
/// between them. A consumer takes the high-water mark per session, so the
/// mid-turn payloads are supersets of each other and the final one wins; a
/// divergence in field names or units between the two call sites would instead
/// show up as tokens silently vanishing, which is the failure this reporting
/// exists to prevent.
///
/// All counts are SESSION-cumulative, matching goose, so buzz-acp's
/// `UsageTracker` can compute per-turn deltas symmetrically for both agents.
///
/// ## `_goose/unstable/session/update` contract (ACP)
///
/// | Field | Type | Semantics |
/// |---|---|---|
/// | `sessionUpdate` | `"usage_update"` | Discriminant |
/// | `used` | `u64` | `input + output`; context-usage proxy |
/// | `contextLimit` | `u64` | `0` — buzz-agent has no context limit tracking |
/// | `accumulatedInputTokens` | `u64?` | Session-cumulative inclusive input tokens; **absent** when overflow-poisoned |
/// | `accumulatedOutputTokens` | `u64?` | Session-cumulative output tokens; **absent** when overflow-poisoned |
/// | `accumulatedCachedInputTokens` | `u64?` | Session-cumulative cache-read tokens; **absent** when never observed (harness restart, goose, or first turn); `0` when provider confirmed no cache hits |
/// | `accumulatedCacheWriteTokens` | `u64?` | Session-cumulative cache-write tokens; **absent** when never observed (same rules as cached-read) |
/// | `accumulatedTotalTokens` | `u64?` | Session-cumulative provider total; absent unless every turn reported one |
/// | `model` | `string` | Effective model id |
///
/// **Absence vs explicit zero**: both cache fields are omitted (never `null`)
/// when the running session has never observed a value for them. An old-harness
/// consumer that does not recognise these fields ignores them cleanly; a new
/// consumer that receives them absent treats them as unknown, not zero.
///
/// **Overflow poison**: `accumulatedInputTokens` and `accumulatedOutputTokens`
/// are omitted (never `null`, never `u64::MAX`) when the session-cumulative sum
/// has overflowed.  A consumer that receives them absent treats them as
/// incomplete, consistent with the `accumulatedTotalTokens` contract.
///
/// **Monotonic within a session**: each notification carries a cumulative
/// snapshot that can only increase. A consumer that takes the high-water mark
/// always ends up at the final value.
///
/// **Per-category independent**: a session where `accumulatedCacheWriteTokens`
/// is absent but `accumulatedCachedInputTokens` is present is valid — the two
/// categories are tracked independently. Either can become absent (e.g. after
/// a provider switch) without poisoning the other.
pub fn usage_update_payload(
    accumulated_input_tokens: Option<u64>,
    accumulated_output_tokens: Option<u64>,
    accumulated_cached_input_tokens: Option<u64>,
    accumulated_cache_write_tokens: Option<u64>,
    accumulated_total: crate::types::TurnTotalState,
    model: &str,
    pricing_identity: Option<&crate::types::PricingIdentity>,
) -> Value {
    let mut update = json!({
        "sessionUpdate": "usage_update",
        // used: total tokens as a context-usage proxy; saturate when either
        // side is absent or poisoned (display-only, ACP treats it as dead code).
        // contextLimit: 0 (buzz-agent has no context limit tracking).
        "used": accumulated_input_tokens.unwrap_or(0).saturating_add(accumulated_output_tokens.unwrap_or(0)),
        "contextLimit": 0u64,
        "model": model,
    });
    // accumulatedInputTokens / accumulatedOutputTokens: omitted (never null,
    // never u64::MAX) when the session-cumulative sum has overflowed.
    if let Some(input) = accumulated_input_tokens {
        update["accumulatedInputTokens"] = json!(input);
    }
    if let Some(output) = accumulated_output_tokens {
        update["accumulatedOutputTokens"] = json!(output);
    }
    // accumulatedCachedInputTokens: a subset of accumulatedInputTokens, not an
    // addition to it. Extends goose's usage_update shape; a consumer that does
    // not know the field ignores it and prices exactly as it did before.
    // Absent (never emitted as null) when the session has never observed a
    // cached-input value — goose never emits it, so old-harness compat is clean.
    if let Some(cached) = accumulated_cached_input_tokens {
        update["accumulatedCachedInputTokens"] = json!(cached);
    }
    // accumulatedCacheWriteTokens: a subset of accumulatedInputTokens, not an
    // addition to it. Absent when never observed (same provenance rules as
    // accumulatedCachedInputTokens).
    if let Some(written) = accumulated_cache_write_tokens {
        update["accumulatedCacheWriteTokens"] = json!(written);
    }
    // Only when the cumulative is exactly known — never when Unseen (no total
    // ever observed) or Unknown (at least one turn lacked a total). A goose
    // consumer that doesn't recognise the field ignores it.
    if let Some(total) = accumulated_total.exact_value() {
        update["accumulatedTotalTokens"] = json!(total);
    }
    // pricingIdentity: present only when the publisher proved applicability.
    // Absent (never null) when unproven — old consumers ignore the field.
    if let Some(pi) = pricing_identity {
        update["pricingIdentity"] = serde_json::to_value(pi).unwrap_or(serde_json::Value::Null);
    }
    update
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

    // ── usage_update_payload: pricingIdentity emission ───────────────────────

    fn make_pi(authority: &str, model: &str) -> crate::types::PricingIdentity {
        crate::types::PricingIdentity {
            authority: authority.to_string(),
            model: model.to_string(),
            cache_class: None,
        }
    }

    /// When a proven identity is passed, the wire payload MUST include a
    /// `pricingIdentity` object with camelCase keys, and it MUST NOT be null.
    #[test]
    fn usage_update_payload_includes_pricing_identity_when_proven() {
        let pi = make_pi("api.anthropic.com", "claude-opus-4-5");
        let payload = usage_update_payload(
            Some(1000),
            Some(200),
            None,
            None,
            crate::types::TurnTotalState::Unseen,
            "claude-opus-4-5",
            Some(&pi),
        );

        let pi_wire = &payload["pricingIdentity"];
        assert!(
            !pi_wire.is_null(),
            "pricingIdentity must be present when identity is proven"
        );
        assert_eq!(pi_wire["authority"], serde_json::json!("api.anthropic.com"));
        assert_eq!(pi_wire["model"], serde_json::json!("claude-opus-4-5"));
        // cacheClass absent when None (skip_serializing_if)
        assert!(pi_wire.get("cacheClass").is_none() || pi_wire["cacheClass"].is_null());
    }

    /// When `pricing_identity` is `None` (unproven: custom endpoint, mixed
    /// identities, etc.), the field MUST be absent from the wire payload.
    /// It must never appear as `null`.
    #[test]
    fn usage_update_payload_omits_pricing_identity_when_absent() {
        let payload = usage_update_payload(
            Some(500),
            Some(100),
            None,
            None,
            crate::types::TurnTotalState::Unseen,
            "some-model",
            None, // no proven identity
        );

        assert!(
            payload.get("pricingIdentity").is_none(),
            "pricingIdentity must be absent (never null) when identity is unproven"
        );
    }

    /// A custom endpoint (Databricks, corporate proxy) must not produce a
    /// wire `pricingIdentity` — it must be absent.
    ///
    /// This relies on the caller passing `None`; the payload builder must not
    /// inject a default. This test documents the contract.
    #[test]
    fn usage_update_payload_no_pricing_identity_for_custom_endpoint() {
        // Custom endpoint → caller passes None (pricing_authority returned None).
        let payload = usage_update_payload(
            Some(800),
            Some(150),
            Some(200),
            None,
            crate::types::TurnTotalState::Unseen,
            "databricks-llama-4",
            None,
        );

        assert!(
            payload.get("pricingIdentity").is_none(),
            "custom endpoint: pricingIdentity must not appear on the wire"
        );
        // Cache field must still be present (independent).
        assert_eq!(
            payload["accumulatedCachedInputTokens"],
            serde_json::json!(200)
        );
    }

    /// When input is overflow-poisoned (None), the wire payload must omit
    /// `accumulatedInputTokens` entirely — never null, never u64::MAX.
    #[test]
    fn usage_update_payload_omits_input_when_poisoned() {
        let payload = usage_update_payload(
            None, // overflow-poisoned
            Some(150),
            None,
            None,
            crate::types::TurnTotalState::Unseen,
            "model",
            None,
        );
        assert!(
            payload.get("accumulatedInputTokens").is_none(),
            "poisoned accumulatedInputTokens must be absent, not null or MAX"
        );
        // output still present
        assert_eq!(payload["accumulatedOutputTokens"], serde_json::json!(150));
    }

    /// When output is overflow-poisoned (None), the wire payload must omit
    /// `accumulatedOutputTokens` entirely — never null, never u64::MAX.
    #[test]
    fn usage_update_payload_omits_output_when_poisoned() {
        let payload = usage_update_payload(
            Some(800),
            None, // overflow-poisoned
            None,
            None,
            crate::types::TurnTotalState::Unseen,
            "model",
            None,
        );
        assert!(
            payload.get("accumulatedOutputTokens").is_none(),
            "poisoned accumulatedOutputTokens must be absent, not null or MAX"
        );
        // input still present
        assert_eq!(payload["accumulatedInputTokens"], serde_json::json!(800));
    }

    /// When both are present and exact, the wire payload emits both at their
    /// values (unchanged goose-compatible behavior).
    #[test]
    fn usage_update_payload_emits_both_when_exact() {
        let payload = usage_update_payload(
            Some(1000),
            Some(200),
            None,
            None,
            crate::types::TurnTotalState::Unseen,
            "model",
            None,
        );
        assert_eq!(payload["accumulatedInputTokens"], serde_json::json!(1000));
        assert_eq!(payload["accumulatedOutputTokens"], serde_json::json!(200));
    }
}
