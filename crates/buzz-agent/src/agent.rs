//! ACP wire emission for buzz-agent's turn loop.
//!
//! The loop itself lives in [`crate::loop_drive`] — buzz-agent owns it and
//! calls goose for the model, the tools, the prompt and compaction. What stays
//! here is the part goose knows nothing about: the mapping from goose
//! `MessageContent` onto the exact `session/update` notifications `buzz-acp`
//! consumes (`acp.rs:1528-1627`), and the `keepalive` ticker that resets the
//! harness idle clock.

use serde_json::{json, Value};

use goose_provider_types::conversation::message::MessageContent;
use rmcp::model::CallToolRequestParams;

use crate::types::ContentBlock;
use crate::wire::{self, WireSender};

/// How often to emit `keepalive` while waiting on the provider.
///
/// NOT part of ACP. `buzz-acp` runs an idle-timeout clock that is reset on
/// every line of valid JSON (`acp.rs:1197-1199`); a silent agent is killed.
/// buzz-agent emitted this from inside its provider `select!`
/// (`agent.rs:122-127`) and `buzz-acp` treats it as a no-op that only resets
/// the clock (`acp.rs:1623`).
///
/// Goose streams, so during generation the chunks themselves keep the clock
/// alive — but there is no traffic during a long *pre-first-token* wait (big
/// prompt processing, provider queueing, reasoning models). So the ticker
/// stays.
pub(crate) const KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Per-turn token accounting, mirroring buzz-agent's contract.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TurnTokens {
    pub input: Option<u64>,
    pub output: Option<u64>,
    /// Cache-served input tokens, a subset of `input`.
    pub cached_input: crate::types::CacheTotalState,
    /// Cache-written input tokens, a subset of `input` but billed separately.
    pub cache_write: crate::types::CacheTotalState,
    /// Provider-reported total. A missing value on any usage-bearing response
    /// poisons the turn rather than silently under-reporting it.
    pub total: crate::types::TurnTotalState,
    /// Proven publisher billing identity while every usage-bearing response
    /// agrees; `Some(None)` is permanently poisoned for this turn.
    pub pricing_identity: Option<Option<crate::types::PricingIdentity>>,
}

impl TurnTokens {
    pub fn observed(&self) -> bool {
        self.input.is_some() || self.output.is_some()
    }
}

/// Flatten ACP prompt blocks into the text Goose's `Message` carries.
pub fn prompt_to_text(blocks: &[ContentBlock]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            ContentBlock::Text { text } => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
            ContentBlock::ResourceLink { uri } => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(uri);
            }
            ContentBlock::Unsupported => {}
        }
    }
    out
}

/// Reset the harness idle clock during provider silence.
pub(crate) async fn emit_keepalive(session_id: &str, wire_tx: &WireSender) {
    wire::send(
        wire_tx,
        wire::session_update(session_id, json!({ "sessionUpdate": "keepalive" })),
    )
    .await;
}

/// Announce a tool call as in-progress.
///
/// Every announcement must be matched by exactly one
/// [`emit_tool_call_update`], or the desktop renders a spinner that never
/// resolves. buzz-agent's loop guarantees the pairing including on cancel.
pub(crate) async fn emit_tool_call(
    wire_tx: &WireSender,
    session_id: &str,
    tool_call_id: &str,
    call: &CallToolRequestParams,
) {
    let raw = serde_json::to_value(&call.arguments).unwrap_or(Value::Null);
    wire::send(
        wire_tx,
        wire::session_update(
            session_id,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": tool_call_id,
                "title": call.name.to_string(),
                "kind": "other",
                "status": "in_progress",
                "rawInput": raw,
            }),
        ),
    )
    .await;
}

/// Move an announced tool call to its terminal state.
pub(crate) async fn emit_tool_call_update(
    wire_tx: &WireSender,
    session_id: &str,
    tool_call_id: &str,
    failed: bool,
) {
    wire::send(
        wire_tx,
        wire::session_update(
            session_id,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": tool_call_id,
                "status": if failed { "failed" } else { "completed" },
            }),
        ),
    )
    .await;
}

/// Emit streamed assistant content onto the ACP wire.
///
/// Text and reasoning only — tool lifecycle is emitted by [`crate::tools`],
/// which is the only place that knows when a call actually starts and ends.
pub(crate) async fn emit_content(content: &MessageContent, session_id: &str, wire_tx: &WireSender) {
    match content {
        MessageContent::Text(t) if !t.text.is_empty() => {
            wire::send(
                wire_tx,
                wire::session_update(
                    session_id,
                    json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": t.text },
                    }),
                ),
            )
            .await;
        }

        MessageContent::Thinking(t) if !t.thinking.is_empty() => {
            wire::send(
                wire_tx,
                wire::session_update(
                    session_id,
                    json!({
                        "sessionUpdate": "agent_thought_chunk",
                        "content": { "type": "text", "text": t.thinking },
                    }),
                ),
            )
            .await;
        }

        // Tool lifecycle is NOT emitted from here. buzz-agent's loop owns
        // it: `crate::tools` announces each call immediately before dispatch
        // and emits the terminal update immediately after, including for calls
        // a cancelled turn never ran. Emitting from the streamed content too
        // would double-announce every tool.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_flattens_text_blocks() {
        let blocks = vec![
            ContentBlock::Text {
                text: "hello".into(),
            },
            ContentBlock::Text {
                text: "world".into(),
            },
        ];
        assert_eq!(prompt_to_text(&blocks), "hello\nworld");
    }

    #[test]
    fn prompt_includes_resource_links() {
        let blocks = vec![
            ContentBlock::Text { text: "see".into() },
            ContentBlock::ResourceLink {
                uri: "file:///a".into(),
            },
        ];
        assert_eq!(prompt_to_text(&blocks), "see\nfile:///a");
    }

    #[test]
    fn prompt_skips_unsupported() {
        let blocks = vec![
            ContentBlock::Unsupported,
            ContentBlock::Text {
                text: "only".into(),
            },
        ];
        assert_eq!(prompt_to_text(&blocks), "only");
    }

    #[test]
    fn turn_tokens_observed_requires_a_count() {
        assert!(!TurnTokens::default().observed());
        assert!(TurnTokens {
            input: Some(1),
            ..Default::default()
        }
        .observed());
        assert!(TurnTokens {
            output: Some(1),
            ..Default::default()
        }
        .observed());
    }
}
