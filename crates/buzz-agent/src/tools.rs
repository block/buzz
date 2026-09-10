//! Tool execution for buzz-agent's own loop.
//!
//! goose's `Agent::dispatch_tool_call` resolves the prefixed tool name, runs
//! the MCP call, and applies goose's own pre/post hooks — so we use it rather
//! than reaching into `ExtensionManager` directly. What we keep is the policy
//! around it: parallel dispatch, the announce→terminal wire invariant,
//! `[Reflect]` appended to failed results, and the client authorization gate.
//!
//! # Authorization
//!
//! Every LLM-issued tool call is put to the client over
//! `session/request_permission` *before* it is dispatched, and only a selected
//! allow option lets it run (`crate::permission`). This is buzz's own gate, not
//! goose's `GooseMode` approval: goose applies its mode inside `Agent::reply`,
//! which buzz-agent does not call, so without this the mode is inert on this
//! path. Lifecycle hooks (`_Stop`, `_PostCompact`) are dispatched by
//! `crate::hooks`, not through here, so they are deliberately not gated — they
//! are the runtime's own calls, not the model's.

use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};
use rmcp::model::{CallToolResult, ContentBlock, ErrorData};
use tokio_util::sync::CancellationToken;

use crate::mcp::McpRegistry;
use goose_provider_types::conversation::message::{ToolRequest, ToolResult};

use crate::permission::{AskSubject, PermissionBroker, PermissionDecision};
use crate::wire::WireSender;

/// One tool's outcome, keyed by the request id the model used.
pub type Outcome = (String, ToolResult<CallToolResult>);

/// Run every tool the model asked for.
///
/// Calls are dispatched concurrently — a turn that reads three files should
/// not pay for them serially — but results are returned in **request order**,
/// because providers require tool results to line up with the requests that
/// produced them.
#[allow(clippy::too_many_arguments)]
pub async fn execute(
    mcp: &Arc<McpRegistry>,
    session_id: &str,
    wire_tx: &WireSender,
    cancel: &CancellationToken,
    permissions: &Arc<PermissionBroker>,
    protocol_version: u32,
    requests: &[ToolRequest],
    reflections: &mut usize,
) -> Vec<Outcome> {
    let mut pending = FuturesUnordered::new();
    for (index, request) in requests.iter().enumerate() {
        pending.push(run_one(
            mcp,
            session_id,
            wire_tx,
            cancel,
            permissions,
            protocol_version,
            index,
            request,
        ));
    }

    let mut collected: Vec<Option<Outcome>> = (0..requests.len()).map(|_| None).collect();
    while let Some((index, outcome)) = pending.next().await {
        collected[index] = Some(outcome);
    }

    let mut results = Vec::with_capacity(collected.len());
    for outcome in collected.into_iter().flatten() {
        let (id, result) = outcome;
        let result = reflect_on_failure(result, reflections);
        emit_terminal(wire_tx, session_id, &id, &result).await;
        results.push((id, result));
    }
    results
}

/// Synthesise results for tools that were never run because the turn was
/// cancelled first.
///
/// Every announced tool call must reach a terminal state; the desktop renders
/// an unresolved one as a spinner that never stops.
pub fn cancelled_results(requests: &[ToolRequest]) -> Vec<Outcome> {
    requests
        .iter()
        .map(|request| {
            (
                request.id.clone(),
                Ok(CallToolResult::error(vec![ContentBlock::text(
                    "Tool call was cancelled before execution",
                )])),
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn run_one(
    mcp: &Arc<McpRegistry>,
    session_id: &str,
    wire_tx: &WireSender,
    cancel: &CancellationToken,
    permissions: &Arc<PermissionBroker>,
    protocol_version: u32,
    index: usize,
    request: &ToolRequest,
) -> (usize, Outcome) {
    let id = request.id.clone();

    // A tool call the provider itself failed to parse never reaches MCP.
    let call = match &request.tool_call {
        Ok(call) => call.clone(),
        Err(e) => return (index, (id, Err(e.clone()))),
    };

    crate::agent::emit_tool_call(wire_tx, session_id, &id, &call).await;

    // Ask the client to authorize this call. Every non-authorizing outcome
    // fails closed: the model sees an ordinary tool failure and the turn
    // continues, exactly as it would for any other refused call.
    let arguments = call
        .arguments
        .clone()
        .map(serde_json::Value::Object)
        .unwrap_or(serde_json::Value::Null);
    match permissions
        .request_permission(
            wire_tx,
            protocol_version,
            session_id,
            AskSubject {
                tool_call_id: &id,
                tool_name: &call.name,
                arguments: &arguments,
            },
            cancel,
        )
        .await
    {
        PermissionDecision::Allowed => {}
        PermissionDecision::Denied(msg) => {
            return (
                index,
                (id, Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))),
            );
        }
        PermissionDecision::Cancelled => {
            return (
                index,
                (
                    id,
                    Ok(CallToolResult::error(vec![ContentBlock::text(
                        "Tool call was cancelled before execution",
                    )])),
                ),
            );
        }
    }

    // A cancel may have landed while we waited for approval; do not start it.
    if cancel.is_cancelled() {
        return (
            index,
            (
                id,
                Ok(CallToolResult::error(vec![ContentBlock::text(
                    "Tool call was cancelled before execution",
                )])),
            ),
        );
    }

    let arguments = call
        .arguments
        .clone()
        .map(serde_json::Value::Object)
        .unwrap_or(serde_json::Value::Null);
    let result = mcp
        .call_rmcp(call.name.as_ref(), &id, &arguments, cancel)
        .await;

    (index, (id, result))
}

/// Append `[Reflect]` to a failed tool result.
///
/// buzz-agent's original behaviour: the model reads the reflection in the same
/// content block as the failure, rather than as a separate steer message
/// arriving at an unrelated point in the conversation. Capped so a tool
/// failing in a loop cannot flood the context.
///
/// **Deliberately not a goose `Operation`**, unlike the other loop decisions.
/// An operation cannot reach inside a tool result: `ConversationEffect` can append a
/// message or patch a request's *metadata*
/// (`PatchToolRequestMeta`, which goose applies through `SessionManager` --
/// the store buzz does not write to), but nothing edits result *content*. As
/// an operation the reflection would arrive as a separate message after the
/// result instead of inside it, which is the arrangement this deliberately
/// moved away from. Structure follows behaviour, not the other way round.
fn reflect_on_failure(
    result: ToolResult<CallToolResult>,
    reflections: &mut usize,
) -> ToolResult<CallToolResult> {
    let failed = match &result {
        Ok(out) => out.is_error.unwrap_or(false),
        Err(_) => true,
    };
    if !failed || *reflections >= crate::loop_drive::max_reflections() {
        return result;
    }
    *reflections += 1;

    match result {
        Ok(mut out) => {
            out.content
                .push(ContentBlock::text(crate::loop_drive::reflection_text()));
            Ok(out)
        }
        // An `ErrorData` has no content to append to, so the reflection is
        // folded into its message instead of being dropped.
        Err(e) => Err(ErrorData::new(
            e.code,
            format!("{} {}", e.message, crate::loop_drive::reflection_text()),
            e.data,
        )),
    }
}

async fn emit_terminal(
    wire_tx: &WireSender,
    session_id: &str,
    id: &str,
    result: &ToolResult<CallToolResult>,
) {
    let failed = match result {
        Ok(out) => out.is_error.unwrap_or(false),
        Err(_) => true,
    };
    crate::agent::emit_tool_call_update(wire_tx, session_id, id, failed).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(rmcp::model::CallToolRequestParams::new("x")),
            metadata: None,
            tool_meta: None,
        }
    }

    #[test]
    fn cancellation_still_terminates_every_announced_call() {
        // The desktop spins forever on an unresolved tool call, so a cancelled
        // turn must still produce one result per request.
        let requests = vec![request("a"), request("b")];
        let results = cancelled_results(&requests);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a");
        assert_eq!(results[1].0, "b");
    }

    #[test]
    fn reflection_is_appended_to_a_failed_result() {
        let mut reflections = 0;
        let failed = Ok(CallToolResult::error(vec![ContentBlock::text("boom")]));
        let out = reflect_on_failure(failed, &mut reflections).unwrap();
        let text: String = out
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect();
        assert!(text.contains("[Reflect]"));
        assert_eq!(reflections, 1);
    }

    #[test]
    fn success_is_left_alone() {
        let mut reflections = 0;
        let ok = Ok(CallToolResult::success(vec![ContentBlock::text("fine")]));
        let out = reflect_on_failure(ok, &mut reflections).unwrap();
        assert_eq!(out.content.len(), 1);
        assert_eq!(reflections, 0);
    }

    #[test]
    fn reflection_stops_at_the_cap() {
        let mut reflections = crate::loop_drive::max_reflections();
        let failed = Ok(CallToolResult::error(vec![ContentBlock::text("boom")]));
        let out = reflect_on_failure(failed, &mut reflections).unwrap();
        assert_eq!(out.content.len(), 1, "capped: no reflection appended");
    }
}
