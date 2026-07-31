//! buzz-agent's `_Stop` / `_PostCompact` lifecycle hooks, driven from our own
//! turn loop rather than from goose's.
//!
//! # Why not goose's hook system
//!
//! Goose has a blocking Stop hook with exactly buzz-agent's semantics
//! (`agent.rs:2891-2917`): on `HookDecision::Deny` the objection is pushed
//! into the conversation as an agent-visible / user-invisible message and the
//! loop continues, bounded by a block cap. But it is unreachable for us:
//!
//! * `Agent::hook_manager` is private, and the only setter is
//!   `#[cfg(test)]`-gated (`agent.rs:430`).
//! * Hooks are otherwise discovered as *subprocesses* from
//!   `<root>/.goose/plugins/*/hooks/hooks.json` (`hooks/mod.rs:238-253`).
//!
//! The subprocess route looked viable until you notice where the answer lives:
//! `buzz-dev-mcp`'s todo list is **in-process state** (`todo.rs:49`, a
//! `Mutex<Vec<Item>>`), and that process's stdio is owned by goose. A separate
//! hook binary spawned by goose cannot see it. Writing a `hooks.json` would
//! give us a hook that always answers "no objection" — worse than no hook,
//! because it would look like it worked.
//!
//! # What we do instead
//!
//! We own the outer loop, so we ask the tool ourselves.
//! `Agent::dispatch_tool_call` is public (`agent.rs:1059`), so between rounds
//! we call `_Stop` on the same extension the model uses, and if it objects we
//! feed the objection back via `Agent::steer` — which goose drains at the
//! round boundary (`agent.rs:1951-1974`) and which also prevents the turn from
//! ending while a steer is pending (`agent.rs:2876-2878`).
//!
//! Net effect is buzz-agent's behaviour, with no goose changes: the agent does
//! not get to stop while its todo list has open items.

use std::sync::Arc;

use goose::agents::Agent;
use goose::session::session_manager::Session;
use rmcp::model::CallToolRequestParams;

/// Hard cap on consecutive vetoes, mirroring goose's own `stop_hook_block_cap`
/// (`agent.rs:108-119`). A tool that always objects must not trap the turn.
pub const MAX_STOP_BLOCKS: u32 = 3;

/// Tools are advertised to the model prefixed with their extension name
/// (`extension_manager.rs:1425`), and dispatch resolves the same prefixed
/// form. `_Stop` is hidden from the model but still callable by us.
fn qualified(extension: &str, tool: &str) -> String {
    format!("{extension}__{tool}")
}

/// Ask `_Stop` whether the turn may end.
///
/// Returns `Some(objection)` when the agent should keep working. Any failure —
/// no such extension, tool error, malformed result — is `None`. A broken hook
/// must never trap a turn; that is also goose's rule for its own hooks
/// ("a misbehaving hook MUST NOT block", `hooks/mod.rs:361`).
pub async fn stop_objection(
    agent: &Arc<Agent>,
    session: &Session,
    extension: &str,
) -> Option<String> {
    let text = call_hook(agent, session, extension, "_Stop").await?;
    let trimmed = text.trim();
    // Empty response is the documented "no objection" signal
    // (`buzz-dev-mcp/src/todo.rs:100`).
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Ask `_PostCompact` for state to re-inject after goose compacted history.
///
/// buzz-agent called this so the todo list survives truncation; goose emits
/// `AgentEvent::HistoryReplaced` when it compacts, which is our trigger.
pub async fn post_compact_state(
    agent: &Arc<Agent>,
    session: &Session,
    extension: &str,
) -> Option<String> {
    let text = call_hook(agent, session, extension, "_PostCompact").await?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Dispatch a zero-argument hook tool and flatten its result to text.
async fn call_hook(
    agent: &Arc<Agent>,
    session: &Session,
    extension: &str,
    tool: &str,
) -> Option<String> {
    let params = CallToolRequestParams::new(qualified(extension, tool))
        .with_arguments(serde_json::Map::new());

    let (_id, result) = agent
        .dispatch_tool_call(
            params,
            format!("buzz_hook_{tool}"),
            // Deliberately not cancellable: this runs at a turn boundary and
            // must resolve either way.
            None,
            session,
        )
        .await;

    let call = match result {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(tool, error = %e, "hook tool unavailable");
            return None;
        }
    };

    let out = match call.result.await {
        Ok(o) => o,
        Err(e) => {
            tracing::debug!(tool, error = ?e, "hook tool failed");
            return None;
        }
    };

    if out.is_error.unwrap_or(false) {
        tracing::debug!(tool, "hook tool returned is_error");
        return None;
    }

    let mut text = String::new();
    for content in out.content.iter() {
        if let Some(raw) = content.as_text() {
            text.push_str(&raw.text);
        }
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualifies_tool_names_with_the_extension_prefix() {
        // Must match extension_manager.rs:1425 or dispatch cannot resolve it.
        assert_eq!(qualified("developer", "_Stop"), "developer___Stop");
    }
}
