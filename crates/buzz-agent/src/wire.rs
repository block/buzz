use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
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
    /// A bare JSON-RPC response (id present, no method) — the client's answer
    /// to a request buzz-agent issued. Today the only such request is
    /// `session/request_permission`. `result` carries the JSON-RPC `result`
    /// field ONLY when the frame is a structurally valid response — no `method`
    /// member and exactly one of `result`/`error`. Any malformed shape (present
    /// non-string `method`, both `result` and `error`, or neither) is normalized
    /// to `Null` so a possibly-`selected` payload is never laundered into an
    /// approval; every non-`selected` shape fails the broker's authorization
    /// predicate and denies.
    Response {
        id: Value,
        result: Value,
    },
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
        // Bare responses (id present, no method) answer a request buzz-agent
        // issued — today only `session/request_permission`. Route to the
        // permission broker, which matches a live correlation id or ignores an
        // unknown one. Forward the `result` ONLY when the frame is a
        // structurally valid response — the exactly-one-of invariant: no
        // `method` member at all, and `result` present with `error` absent. A
        // present non-string `method` (which `as_str` above collapsed to
        // `None`), both `result` and `error`, or neither is malformed; forward
        // `Null` so the broker fails closed (deny) rather than laundering a
        // possibly-`selected` payload into an approval.
        (None, Some(id)) => {
            let well_formed = msg.get("method").is_none()
                && msg.get("result").is_some()
                && msg.get("error").is_none();
            let result = if well_formed {
                msg.get("result").cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            Inbound::Response { id, result }
        }
        (None, None) => Inbound::Invalid {
            id: Value::Null,
            code: INVALID_REQUEST,
            message: "jsonrpc: missing method and id".into(),
        },
    }
}

/// `optionId`/`kind` of the single allow option offered on every
/// `session/request_permission`. buzz-acp's answering side selects the option
/// whose `kind == "allow_once"` (never by hardcoded `optionId`), and the
/// authorization predicate on this side requires the returned `optionId` to
/// equal exactly this value. Keeping option id and kind identical means both
/// sides agree without a separate lookup table.
pub const ALLOW_OPTION_ID: &str = "allow_once";

/// The two options offered on every permission request: allow-once and
/// reject-once. First cut ships only these (no session-scoped grant), so every
/// offered option is already in the desktop card's exact actionable allowlist.
fn permission_options() -> Value {
    json!([
        { "optionId": ALLOW_OPTION_ID, "name": "Allow", "kind": ALLOW_OPTION_ID },
        { "optionId": "reject_once", "name": "Deny", "kind": "reject_once" },
    ])
}

/// Build `session/request_permission` params for the negotiated protocol
/// version. No hybrid shapes — the request must match exactly what the client
/// negotiated at `initialize`, or a strict client can reject it before policy
/// is applied.
///
/// - **v2** (what buzz-agent negotiates with current buzz-acp): tool context
///   lives under `subject: {type: "tool_call", toolCall}` with top-level
///   `title` and `options`.
/// - **v1** (still negotiated when a client requests it): the legacy shape with
///   `toolCall` (carrying `kind`) directly at the params level.
pub fn request_permission_params(
    version: u32,
    session_id: &str,
    tool_call_id: &str,
    title: &str,
    raw_input: &Value,
) -> Value {
    if version >= 2 {
        json!({
            "sessionId": session_id,
            "title": title,
            "subject": {
                "type": "tool_call",
                "toolCall": {
                    "toolCallId": tool_call_id,
                    "title": title,
                    "rawInput": raw_input,
                },
            },
            "options": permission_options(),
        })
    } else {
        json!({
            "sessionId": session_id,
            "toolCall": {
                "toolCallId": tool_call_id,
                "title": title,
                "kind": "other",
                "rawInput": raw_input,
            },
            "options": permission_options(),
        })
    }
}

/// Build an outbound JSON-RPC request `session/request_permission` frame.
pub fn request_permission(id: Value, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "session/request_permission",
        "params": params,
    })
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
    let _ = send_checked(wire, msg).await;
}

/// Enqueue a frame, reporting whether the writer accepted it. Unlike mpsc's
/// non-blocking `try_send`, this awaits channel capacity; it fails only when
/// the writer task has dropped its receiver, which happens exactly when the
/// writer has exited because stdout is closed/broken. A frame that fails here
/// will never be written, so callers that correlate a response — the
/// permission broker — must fail closed immediately rather than wait out a
/// deadline for a reply that can never arrive.
pub async fn send_checked(wire: &WireSender, msg: Value) -> Result<(), ()> {
    wire.send(WireMsg::Notify(msg)).await.map_err(|_| ())
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

pub async fn writer_task(rx: mpsc::Receiver<WireMsg>) {
    write_frames(rx, tokio::io::stdout()).await;
}

/// Drain `rx`, writing each frame to `out` as a newline-terminated JSON line.
/// Generic over the sink so tests can inject an `AsyncWrite` that fails on
/// flush; production passes stdout.
///
/// Both `write_all` and `flush` failure are connection-fatal: they return,
/// dropping `rx` so `async_main`'s writer-death arm cancels every session.
/// Flush must be fatal too — a blocking stdout can report `Ok` from
/// `write_all` when it only schedules the underlying write and surface the
/// real error at `flush`, so ignoring flush failure would leave a dead stdout
/// undetected and strand any correlated ask waiting for a reply that can never
/// be written.
pub(crate) async fn write_frames<W: AsyncWrite + Unpin>(
    mut rx: mpsc::Receiver<WireMsg>,
    mut out: W,
) {
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
        if out.write_all(s.as_bytes()).await.is_err() || out.flush().await.is_err() {
            return;
        }
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

    // ── request_permission_params: version-aware wire shape ──────────────────

    /// v2 (what buzz-agent negotiates with current buzz-acp): tool context is
    /// nested under `subject: {type: "tool_call", toolCall}` with top-level
    /// `title` and `options`, matching the ACP v2 `RequestPermissionRequest`.
    #[test]
    fn request_permission_params_v2_nests_tool_call_under_subject() {
        let raw = json!({ "command": "ls" });
        let p = request_permission_params(2, "ses_1", "fake__shell", "fake__shell", &raw);

        assert_eq!(p["sessionId"], "ses_1");
        assert_eq!(p["title"], "fake__shell");
        assert_eq!(p["subject"]["type"], "tool_call");
        assert_eq!(p["subject"]["toolCall"]["toolCallId"], "fake__shell");
        assert_eq!(p["subject"]["toolCall"]["title"], "fake__shell");
        assert_eq!(p["subject"]["toolCall"]["rawInput"], raw);
        // No hybrid: v2 must NOT carry a top-level `toolCall`.
        assert!(p.get("toolCall").is_none(), "v2 must not use the v1 shape");
        assert_options(&p["options"]);
    }

    /// v1 (still negotiated when a client requests it): the legacy shape with
    /// `toolCall` (carrying `kind`) directly at the params level, no `subject`.
    #[test]
    fn request_permission_params_v1_uses_legacy_top_level_tool_call() {
        let raw = json!({ "command": "ls" });
        let p = request_permission_params(1, "ses_1", "fake__shell", "fake__shell", &raw);

        assert_eq!(p["sessionId"], "ses_1");
        assert_eq!(p["toolCall"]["toolCallId"], "fake__shell");
        assert_eq!(p["toolCall"]["title"], "fake__shell");
        assert_eq!(p["toolCall"]["kind"], "other");
        assert_eq!(p["toolCall"]["rawInput"], raw);
        // No hybrid: v1 must NOT carry the v2 `subject` or top-level `title`.
        assert!(p.get("subject").is_none(), "v1 must not use the v2 shape");
        assert!(p.get("title").is_none(), "v1 has no top-level title");
        assert_options(&p["options"]);
    }

    /// Both offered options are exactly allow-once and reject-once, with
    /// `optionId == kind` so buzz-acp's `kind`-based selector and this side's
    /// `optionId`-based predicate agree without a lookup table.
    fn assert_options(options: &Value) {
        let opts = options.as_array().expect("options is an array");
        assert_eq!(opts.len(), 2, "first cut offers exactly two options");
        assert_eq!(opts[0]["optionId"], ALLOW_OPTION_ID);
        assert_eq!(opts[0]["kind"], ALLOW_OPTION_ID);
        assert_eq!(opts[0]["name"], "Allow");
        assert_eq!(opts[1]["optionId"], "reject_once");
        assert_eq!(opts[1]["kind"], "reject_once");
        assert_eq!(opts[1]["name"], "Deny");
    }

    /// The outbound frame wraps params in a JSON-RPC request whose id echoes
    /// back verbatim so the broker can correlate the response.
    #[test]
    fn request_permission_frame_is_a_correlatable_jsonrpc_request() {
        let params = request_permission_params(2, "ses_1", "t", "t", &json!({}));
        let frame = request_permission(json!("perm-7"), params);
        assert_eq!(frame["jsonrpc"], "2.0");
        assert_eq!(frame["id"], "perm-7");
        assert_eq!(frame["method"], "session/request_permission");
        assert_eq!(frame["params"]["sessionId"], "ses_1");
    }

    // ── classify: bare responses route to the broker ─────────────────────────

    /// A bare JSON-RPC response (id, no method) is the client's answer to a
    /// request buzz-agent issued; it routes to the broker with its `result`.
    #[test]
    fn classify_bare_response_routes_to_broker() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": "perm-3",
            "result": { "outcome": { "outcome": "selected", "optionId": ALLOW_OPTION_ID } },
        });
        match classify(&msg) {
            Inbound::Response { id, result } => {
                assert_eq!(id, json!("perm-3"));
                assert_eq!(result["outcome"]["outcome"], "selected");
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    /// A JSON-RPC error response (id, `error`, no `result`) still routes to the
    /// broker but with `result == Null`, which the authorization predicate
    /// fails closed. buzz-agent never leaves the waiter hanging on an error.
    #[test]
    fn classify_error_response_routes_with_null_result() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": "perm-3",
            "error": { "code": -32601, "message": "method not found" },
        });
        match classify(&msg) {
            Inbound::Response { id, result } => {
                assert_eq!(id, json!("perm-3"));
                assert_eq!(result, Value::Null, "error/absent result → Null → deny");
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    /// Carl's frame #1: a response carrying BOTH `result` and `error` is
    /// structurally ambiguous and must NOT deliver the `result`, even when that
    /// `result` is a well-formed `selected`/`allow_once` payload. The wire layer
    /// normalizes it to `Null` so the broker denies instead of the frame
    /// laundering an approval upstream of every fail-closed check.
    #[test]
    fn classify_response_with_both_result_and_error_denies() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": "perm-3",
            "result": { "outcome": { "outcome": "selected", "optionId": ALLOW_OPTION_ID } },
            "error": { "code": -32603, "message": "internal" },
        });
        match classify(&msg) {
            Inbound::Response { id, result } => {
                assert_eq!(id, json!("perm-3"));
                assert_eq!(
                    result,
                    Value::Null,
                    "result+error is malformed → Null → deny, never forward the allow payload"
                );
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    /// Carl's frame #2: a present but non-string `method` is NOT "method
    /// absent". `as_str` collapses `method: 7` to `None`, which lands the frame
    /// in the response arm, but it is not a valid response and must not forward
    /// its `result` (a well-formed `selected` payload here). The structural
    /// check sees the present `method` member and normalizes to `Null` → deny.
    #[test]
    fn classify_response_with_non_string_method_denies() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": "perm-3",
            "method": 7,
            "result": { "outcome": { "outcome": "selected", "optionId": ALLOW_OPTION_ID } },
        });
        match classify(&msg) {
            Inbound::Response { id, result } => {
                assert_eq!(id, json!("perm-3"));
                assert_eq!(
                    result,
                    Value::Null,
                    "present non-string method → not a valid response → Null → deny"
                );
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    // ── send_checked: observable wire closure ────────────────────────────────

    /// `send_checked` reports `Ok` while the writer's receiver is alive and
    /// `Err` once it is gone (writer task exited on closed/broken stdout). This
    /// is the contract the permission broker relies on to fail an undeliverable
    /// ask closed immediately instead of waiting out its deadline for a reply
    /// that can never be written.
    #[tokio::test]
    async fn send_checked_reports_closure_when_writer_gone() {
        let (tx, rx) = mpsc::channel::<WireMsg>(4);
        assert!(
            send_checked(&tx, json!({ "ok": 1 })).await.is_ok(),
            "send succeeds while the writer receiver is alive"
        );
        drop(rx); // writer exited → receiver dropped
        assert!(
            send_checked(&tx, json!({ "ok": 2 })).await.is_err(),
            "send reports failure once the writer is gone"
        );
    }
}
