use crate::agent::RunCtx;
use crate::config::{
    HANDOFF_MAX_OUTPUT_TOKENS, HANDOFF_MAX_TOOL_NAMES, HANDOFF_ORIGINAL_TASK_MAX_BYTES,
};
use crate::types::HistoryItem;

pub(crate) enum HandoffOutcome {
    Performed,
    Skipped,
    Cancelled,
}

const HANDOFF_SYSTEM_PROMPT: &str = "You are generating a context handoff summary for the next \
turn of an autonomous agent. Be concise but thorough. Cover: what the original task was, what \
you accomplished, key decisions made, what remains, and one concrete next step. Output plain \
text only — no tool calls, no JSON. Stay under 8192 tokens.";

impl RunCtx<'_> {
    pub(crate) async fn maybe_handoff(&mut self) -> HandoffOutcome {
        if !self.should_handoff() {
            return HandoffOutcome::Skipped;
        }
        if *self.handoff_count >= self.cfg.max_handoffs {
            tracing::info!(
                "handoff cap reached ({}); using truncation",
                self.cfg.max_handoffs
            );
            return HandoffOutcome::Skipped;
        }
        let prompt = self.build_handoff_prompt();
        let summary = tokio::select! {
            biased;
            _ = self.cancel.changed() => return HandoffOutcome::Cancelled,
            r = self.llm.summarize(
                self.cfg,
                HANDOFF_SYSTEM_PROMPT,
                &prompt,
                HANDOFF_MAX_OUTPUT_TOKENS,
            ) => match r {
                Ok(s) if !s.trim().is_empty() => s,
                Ok(_) => {
                    tracing::warn!("handoff returned empty summary; truncating");
                    return HandoffOutcome::Skipped;
                }
                Err(e) => {
                    tracing::warn!("handoff failed: {e}; truncating");
                    return HandoffOutcome::Skipped;
                }
            },
        };
        let current_prompt = self.history.iter().rev().find_map(|item| match item {
            HistoryItem::User(s) => Some(s.clone()),
            _ => None,
        });
        let prior = self.history.len();
        // Reset history first; the _PostCompact hook is meant to inject
        // state into the FRESH context, not the old one we're discarding.
        self.history.clear();
        let post_compact = self
            .mcp
            .call_hooks(
                "_PostCompact",
                &serde_json::json!({}),
                self.cfg.hook_timeout,
                &self.cfg.hook_servers,
            )
            .await;
        // Handoff summary and hook output are injected as a synthetic user
        // message in one block. This keeps `_PostCompact` untrusted while also
        // avoiding orphan tool-result messages in the fresh context: OpenAI
        // Chat/Responses require tool outputs to follow an assistant tool call,
        // but handoff reset intentionally discards the old assistant turn.
        let mut handoff_text = format!("[Context Handoff]\n{summary}");
        if !post_compact.is_empty() {
            handoff_text.push_str("\n\n[Post-compact hook output — untrusted]\n");
            handoff_text.push_str(&hook_outputs_text(&post_compact));
        }
        self.history.push(HistoryItem::User(handoff_text));
        if let Some(prompt) = current_prompt {
            self.history.push(HistoryItem::User(prompt));
        }
        *self.handoff_count += 1;
        tracing::info!(
            "handoff #{} (history {prior} -> {} items)",
            *self.handoff_count,
            self.history.len()
        );
        HandoffOutcome::Performed
    }

    fn should_handoff(&self) -> bool {
        match *self.last_request_input_tokens {
            // Token-first: the provider told us exactly how many input tokens
            // the PREVIOUS request used. But history has grown since that
            // measurement — new assistant text, tool results, and the next
            // user prompt are appended before the next `complete()`. The exact
            // count alone would miss "previous request was under threshold, but
            // newly appended content pushes the next one over" (the stale-usage
            // cousin of the original stale-bytes bug). So we add a conservative
            // token estimate of the bytes added since the measurement.
            Some(measured_tokens) => {
                let measured_bytes = self.last_request_history_bytes.unwrap_or(0);
                let current_bytes: usize =
                    self.history.iter().map(HistoryItem::estimated_bytes).sum();
                let grown = current_bytes.saturating_sub(measured_bytes);
                let projected = measured_tokens.saturating_add(estimate_tokens_from_bytes(grown));
                projected
                    >= token_threshold(self.cfg.max_context_tokens, self.cfg.max_output_tokens)
            }
            // No usage yet (first request, or just after a handoff reset).
            // Fall back to the byte heuristic, capped conservatively so a
            // single pre-usage request can't blow the window. We map the token
            // threshold to bytes using a deliberately LOW bytes/token ratio:
            // a low ratio implies more tokens per byte, so the byte cap is
            // small and the handoff fires early rather than late. Never raise
            // the cap above the configured byte budget.
            //
            // Caveat: this can't shrink a single oversized current prompt,
            // since a handoff re-adds the current prompt verbatim — that is a
            // prompt-cap concern (MAX_PROMPT_BYTES), not this gate.
            None => {
                let bytes: usize = self.history.iter().map(HistoryItem::estimated_bytes).sum();
                bytes
                    > byte_fallback_threshold(
                        self.cfg.max_context_tokens,
                        self.cfg.max_output_tokens,
                        self.cfg.max_history_bytes,
                    )
            }
        }
    }

    fn build_handoff_prompt(&self) -> String {
        let mut head = String::new();
        head.push_str(&format!(
            "[Internal handoff #{} — context reset]\n\n",
            *self.handoff_count + 1
        ));
        head.push_str("# Original Task\n");
        let task = self.original_task.as_deref().unwrap_or("(unknown)");
        head.push_str(&clamp_bytes(task, HANDOFF_ORIGINAL_TASK_MAX_BYTES));
        head.push_str("\n\n# Available Tools\n");
        let all_tools = self.mcp.tools();
        let total = all_tools.len();
        if total == 0 {
            head.push_str("(none)\n");
        } else {
            let shown = total.min(HANDOFF_MAX_TOOL_NAMES);
            let names: Vec<&str> = all_tools[..shown].iter().map(|t| t.name.as_str()).collect();
            head.push_str(&names.join(", "));
            if shown < total {
                head.push_str(&format!(", … (+{} more)", total - shown));
            }
            head.push('\n');
        }
        let tail = "\n# Instructions\n\
             Produce a context handoff summary covering: (1) original task, \
             (2) what was accomplished, (3) key decisions, (4) what remains, \
             (5) one concrete next step. Be concise but thorough. Plain text.\n";
        let history_header = "\n# Session History (oldest first)\n";
        let prompt_budget = handoff_prompt_budget_bytes(
            self.cfg.max_context_tokens,
            HANDOFF_MAX_OUTPUT_TOKENS,
            head.len() + history_header.len() + tail.len(),
        );

        let mut snippets: Vec<String> = Vec::new();
        let mut snippets_bytes = 0usize;
        let mut dropped = 0usize;
        for item in self.history.iter().rev() {
            let mut snippet = String::new();
            push_history_snippet(&mut snippet, item);
            let snippet_bytes = snippet.len();
            if snippets_bytes.saturating_add(snippet_bytes) > prompt_budget {
                if snippets.is_empty() {
                    snippets.push(clamp_bytes(&snippet, prompt_budget));
                    snippets_bytes = prompt_budget;
                }
                dropped += 1;
                continue;
            }
            snippets_bytes += snippet_bytes;
            snippets.push(snippet);
        }
        snippets.reverse();
        if dropped > 0 {
            tracing::info!(
                "handoff prompt budget, dropped {dropped} oldest snippets; kept {} bytes",
                snippets_bytes
            );
        }

        let mut out = String::with_capacity(
            head.len()
                + history_header.len()
                + tail.len()
                + snippets_bytes
                + if dropped > 0 { 32 } else { 0 },
        );
        out.push_str(&head);
        out.push_str(history_header);
        if dropped > 0 {
            out.push_str(&format!("(… {dropped} older items omitted)\n"));
        }
        for s in &snippets {
            out.push_str(s);
        }
        out.push_str(tail);
        out
    }
}

fn hook_outputs_text(outputs: &[(String, String)]) -> String {
    outputs
        .iter()
        .map(|(name, text)| format!("[{name}]\n{text}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn push_history_snippet(out: &mut String, item: &HistoryItem) {
    match item {
        HistoryItem::User(s) => {
            out.push_str("[user] ");
            out.push_str(s);
            out.push('\n');
        }
        HistoryItem::Assistant { text, tool_calls } => {
            out.push_str("[assistant] ");
            if !text.is_empty() {
                out.push_str(text);
            }
            for c in tool_calls {
                out.push_str(&format!(" tool:{}", c.name));
            }
            out.push('\n');
        }
        HistoryItem::ToolResult(r) => {
            out.push_str(if r.is_error { "[tool_err] " } else { "[tool] " });
            out.push_str(&r.text());
            out.push('\n');
        }
    }
}

/// Byte budget for session-history text inside the handoff prompt. The
/// summarizer uses the same provider/model config as normal completion, so
/// derive the input budget from the model context window instead of applying a
/// separate fixed prompt cap. We keep the same 1 byte/token upper-bound
/// estimate used by the handoff gate, which is conservative: it may drop old
/// history early for unusually large sessions, but it should not build a prompt
/// that exceeds the configured context window.
fn handoff_prompt_budget_bytes(
    max_context_tokens: u64,
    max_output_tokens: u32,
    fixed_prompt_bytes: usize,
) -> usize {
    max_context_tokens
        .saturating_sub(u64::from(max_output_tokens))
        .saturating_mul(CONSERVATIVE_BYTES_PER_TOKEN)
        .saturating_sub(u64::try_from(fixed_prompt_bytes).unwrap_or(u64::MAX))
        .try_into()
        .unwrap_or(usize::MAX)
}

pub(crate) fn clamp_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    if max_bytes < 4 {
        let mut cut = max_bytes.min(s.len());
        while cut > 0 && !s.is_char_boundary(cut) {
            cut -= 1;
        }
        return s[..cut].to_owned();
    }
    let target = max_bytes - "…".len();
    let mut cut = target;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

/// Conservative bytes-per-token ratio used when estimating tokens from raw
/// history bytes. We use 1: a token is always at least one byte, so treating
/// every byte as a whole token is an unconditional UPPER bound on the true
/// token count — it can never undercount, regardless of content density (even
/// the densest real content sits at ~1.4 bytes/token). That over-estimate is
/// exactly what a fail-early preflight gate wants: it hands off sooner rather
/// than risk the next request exceeding the window.
const CONSERVATIVE_BYTES_PER_TOKEN: u64 = 1;

/// Estimate tokens from a byte count at the conservative ratio (rounding up,
/// so a partial token still counts). At a 1:1 ratio this is just the byte
/// count — a guaranteed upper bound on tokens.
fn estimate_tokens_from_bytes(bytes: usize) -> u64 {
    (bytes as u64).div_ceil(CONSERVATIVE_BYTES_PER_TOKEN)
}

/// Input-token count at which to hand off. Caps at the configured fraction of
/// the window and also leaves room for `max_output_tokens`, so input + output
/// can't together exceed the window. Free function so the policy math is unit
/// testable without constructing a [`RunCtx`].
fn token_threshold(max_context_tokens: u64, max_output_tokens: u32) -> u64 {
    // Integer math: handoff threshold is 90%, i.e. window * 9 / 10.
    let fractional = max_context_tokens / 10 * 9;
    let output_reserved = max_context_tokens.saturating_sub(u64::from(max_output_tokens));
    fractional.min(output_reserved)
}

/// Conservative byte cap used only before any usage is known. Maps the token
/// threshold to bytes at the conservative bytes/token ratio (so the cap is
/// small and the handoff fires early), clamped to the configured byte budget
/// so it can only ever be more conservative than the old byte-only behavior.
fn byte_fallback_threshold(
    max_context_tokens: u64,
    max_output_tokens: u32,
    max_history_bytes: usize,
) -> usize {
    let derived = token_threshold(max_context_tokens, max_output_tokens)
        .saturating_mul(CONSERVATIVE_BYTES_PER_TOKEN);
    let byte_cap = max_history_bytes / 10 * 9;
    usize::try_from(derived).unwrap_or(usize::MAX).min(byte_cap)
}

#[cfg(test)]
mod tests {
    use super::{
        byte_fallback_threshold, estimate_tokens_from_bytes, handoff_prompt_budget_bytes,
        token_threshold,
    };

    #[test]
    fn handoff_prompt_budget_reserves_summary_output_and_fixed_prompt() {
        assert_eq!(handoff_prompt_budget_bytes(25_000, 8_192, 1_000), 15_808);
    }

    #[test]
    fn handoff_prompt_budget_saturates_when_fixed_prompt_exceeds_window() {
        assert_eq!(handoff_prompt_budget_bytes(1_000, 2_000, 10_000), 0);
    }

    #[test]
    fn token_threshold_uses_fraction_when_output_is_small() {
        // 200k window, 1k output. fractional = 0.9*200000 = 180000;
        // output_reserved = 200000-1000 = 199000; min = 180000.
        assert_eq!(token_threshold(200_000, 1_000), 180_000);
    }

    #[test]
    fn token_threshold_reserves_output_headroom() {
        // Large output relative to window: the output-reserve term dominates,
        // keeping input+output within the window.
        // 100k window, 40k output: fractional=90k, reserved=60k -> 60k.
        assert_eq!(token_threshold(100_000, 40_000), 60_000);
    }

    #[test]
    fn token_threshold_saturates_when_output_exceeds_window() {
        // Degenerate (config validation forbids this, but math must not panic):
        // reserved saturates to 0, so threshold is 0 -> always hand off.
        assert_eq!(token_threshold(1000, 5000), 0);
    }

    #[test]
    fn byte_fallback_is_conservative_and_capped() {
        // Derived = token_threshold * 1 (1 byte/token upper bound). For
        // 200k/1k: 180000 bytes, well under a 16 MiB byte budget, so derived
        // wins (early handoff).
        let t = byte_fallback_threshold(200_000, 1_000, 16 * 1024 * 1024);
        assert_eq!(t, 180_000);
        // With a tiny byte budget the cap wins -> never exceeds it (window*90%).
        let capped = byte_fallback_threshold(200_000, 1_000, 8192);
        assert_eq!(capped, 8192 / 10 * 9);
    }

    #[test]
    fn estimate_tokens_is_upper_bound_on_tokens() {
        // 1 byte/token: a token is always >= 1 byte, so byte count is an
        // unconditional upper bound on the true token count.
        assert_eq!(estimate_tokens_from_bytes(0), 0);
        assert_eq!(estimate_tokens_from_bytes(1), 1);
        assert_eq!(estimate_tokens_from_bytes(4), 4);
        assert_eq!(estimate_tokens_from_bytes(5), 5);
    }
}
