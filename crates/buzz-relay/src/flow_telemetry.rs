//! Publish Flow Studio block execution telemetry (kind 46201) after workflow runs.

use std::sync::Arc;

use buzz_core::kind::KIND_FLOW_BLOCK_EXECUTED;
use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_flow::event_payloads::FlowBlockExecuted;
use buzz_workflow::executor::ExecutionResult;
use buzz_workflow::schema::WorkflowDef;
use buzz_workflow::{PartialProgress, WorkflowError};
use nostr::{EventBuilder, Kind, Tag};
use serde_json::Value;

use crate::handlers::event::dispatch_persistent_event;
use crate::state::AppState;

fn block_type_for_step(def: &WorkflowDef, step_id: &str) -> String {
    def.steps
        .iter()
        .find(|step| step.id == step_id)
        .and_then(|step| step.block_type.clone())
        .unwrap_or_else(|| "unknown".into())
}

fn output_with_cost(output: &Value, cost_usd: f64) -> String {
    let mut merged = match output {
        Value::Object(map) => map.clone(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("result".into(), other.clone());
            map
        }
    };
    merged.insert("cost_usd".into(), Value::from(cost_usd));
    Value::Object(merged).to_string()
}

fn estimate_cost_usd(block_type: &str) -> f64 {
    match block_type {
        "agent" => 0.001,
        "http" => 0.0001,
        "code" => 0.0005,
        "human_approval" | "condition" => 0.0,
        _ => 0.0,
    }
}

/// Merge an executor result trace with any pre-approval trace entries.
pub fn trace_from_execution(
    result: &Result<ExecutionResult, (WorkflowError, PartialProgress)>,
    existing_trace: Option<Vec<Value>>,
) -> Vec<Value> {
    let mut trace = existing_trace.unwrap_or_default();
    match result {
        Ok(exec) => trace.extend(exec.trace.clone()),
        Err((_, progress)) => trace.extend(progress.trace.clone()),
    }
    trace
}

/// Emit kind 46201 events for completed steps when the workflow carries Flow Studio metadata.
pub async fn publish_flow_block_telemetry(
    state: &Arc<AppState>,
    community_id: CommunityId,
    def: &WorkflowDef,
    trace: &[Value],
) {
    let Some(flow_id) = def.flow_id.as_deref().filter(|id| !id.is_empty()) else {
        return;
    };

    let host = match state.db.lookup_community_host(community_id).await {
        Ok(Some(host)) => host,
        Ok(None) => {
            tracing::warn!(
                community_id = %community_id,
                "flow telemetry skipped: community host not mapped"
            );
            return;
        }
        Err(error) => {
            tracing::warn!("flow telemetry host lookup failed: {error}");
            return;
        }
    };
    let tenant = TenantContext::resolved(community_id, host);
    let relay_pubkey_hex = state.relay_keypair.public_key().to_hex();

    for entry in trace {
        let Some(status) = entry.get("status").and_then(Value::as_str) else {
            continue;
        };
        if status != "completed" {
            continue;
        }
        let Some(step_id) = entry.get("step_id").and_then(Value::as_str) else {
            continue;
        };
        let output = entry.get("output").cloned().unwrap_or(Value::Null);
        let block_type = block_type_for_step(def, step_id);
        let cost_usd = output
            .get("cost_usd")
            .and_then(Value::as_f64)
            .unwrap_or_else(|| estimate_cost_usd(&block_type));
        let payload = FlowBlockExecuted {
            flow_id: flow_id.to_string(),
            block_id: step_id.to_string(),
            block_type: block_type.clone(),
            output_json: output_with_cost(&output, cost_usd),
        };
        let content = match serde_json::to_string(&payload) {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!("flow telemetry serialize failed: {error}");
                continue;
            }
        };

        let tags = match Tag::parse(["d", flow_id]) {
            Ok(tag) => vec![tag],
            Err(error) => {
                tracing::warn!("flow telemetry d-tag failed: {error}");
                continue;
            }
        };

        let event = match EventBuilder::new(Kind::from(KIND_FLOW_BLOCK_EXECUTED as u16), &content)
            .tags(tags)
            .sign_with_keys(&state.relay_keypair)
        {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!("flow telemetry signing failed: {error}");
                continue;
            }
        };

        let insert_result = state.db.insert_event(community_id, &event, None).await;
        match insert_result {
            Ok((stored_event, was_inserted)) if was_inserted => {
                let _ = dispatch_persistent_event(
                    &tenant,
                    state,
                    &stored_event,
                    KIND_FLOW_BLOCK_EXECUTED,
                    &relay_pubkey_hex,
                    None,
                )
                .await;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(flow_id, step_id, "flow telemetry insert failed: {error}");
            }
        }
    }
}
