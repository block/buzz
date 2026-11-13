use serde_json::json;
use tokio::sync::mpsc;
use tracing::Instrument as _;

use super::RunCtx;
use crate::llm::LlmDelta;
use crate::types::{AgentError, LlmResponse, ToolDef};
use crate::wire;

impl RunCtx<'_> {
    /// Forward bounded model deltas while keeping the complete response private
    /// until its terminal event validates history, usage, and tool calls.
    pub(super) async fn complete_round(
        &mut self,
        tools: &[ToolDef],
        round: u32,
    ) -> Option<Result<LlmResponse, AgentError>> {
        let (tx, mut rx) = mpsc::channel(16);
        let response = self
            .llm
            .complete_streamed(
                self.cfg,
                self.system_prompt,
                self.history,
                tools,
                self.effective_model,
                &tx,
            )
            .instrument(tracing::info_span!("llm", session_id = %self.session_id));
        tokio::pin!(response);
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await;
        let message_id = format!("{}-message-{round}", self.run_id);
        let mut emitted = String::new();
        let result = loop {
            tokio::select! {
                biased;
                _ = self.cancel.changed() => return None,
                Some(LlmDelta::Text(text)) = rx.recv() => {
                    emitted.push_str(&text);
                    // Keep backpressure on the provider rather than accumulating
                    // an unbounded queue behind a slow ACP consumer.
                    tokio::select! {
                        biased;
                        _ = self.cancel.changed() => return None,
                        () = send_text(self.wire, self.session_id, &message_id, &text) => {}
                    }
                }
                result = &mut response => break result,
                _ = interval.tick() => {
                    tokio::select! {
                        biased;
                        _ = self.cancel.changed() => return None,
                        () = wire::send(self.wire, wire::session_update(self.session_id,
                            json!({"sessionUpdate":"keepalive"}))) => {}
                    }
                }
            }
        };
        // A ready HTTP future can enqueue its final deltas and resolve in the
        // same poll. Drain them before handling success or failure, otherwise a
        // fast EOF could hide already-received text or collapse it into a final
        // buffered update.
        while let Ok(LlmDelta::Text(text)) = rx.try_recv() {
            emitted.push_str(&text);
            tokio::select! {
                biased;
                _ = self.cancel.changed() => return None,
                () = send_text(self.wire, self.session_id, &message_id, &text) => {}
            }
        }
        let response = match result {
            Ok(response) => response,
            Err(error) => return Some(Err(error)),
        };
        // JSON-only compatibility endpoints and structured content blocks may
        // supply text only in the final response. Emit just the missing suffix.
        let Some(suffix) = response.text.strip_prefix(&emitted) else {
            return Some(Err(AgentError::Llm(
                "streamed text disagrees with final response".into(),
            )));
        };
        let final_updates = async {
            if !response.reasoning.is_empty() {
                wire::send(
                    self.wire,
                    wire::session_update(
                        self.session_id,
                        json!({
                            "sessionUpdate":"agent_thought_chunk",
                            "messageId":format!("{}-thought-{round}", self.run_id),
                            "content":{"type":"text","text":&response.reasoning}
                        }),
                    ),
                )
                .await;
            }
            if !suffix.is_empty() {
                send_text(self.wire, self.session_id, &message_id, suffix).await;
            }
        };
        tokio::select! {
            biased;
            _ = self.cancel.changed() => return None,
            () = final_updates => {}
        }
        Some(Ok(response))
    }
}

async fn send_text(wire: &wire::WireSender, session: &str, message: &str, text: &str) {
    wire::send(
        wire,
        wire::session_update(
            session,
            json!({
                "sessionUpdate":"agent_message_chunk", "messageId":message,
                "content":{"type":"text","text":text}
            }),
        ),
    )
    .await;
}
