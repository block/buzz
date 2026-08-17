//! Agent Studio HTTP surface (Buzz Hive).

use std::sync::{Arc, OnceLock};

use axum::{extract::State, http::HeaderMap, response::Json, Json as JsonBody};
use buzz_agent_studio::graph_loader::{graph_from_stored_events, StoredAgentStudioEvent};
use buzz_agent_studio::{
    monitor::{telemetry_json, SessionMonitor, SessionTelemetry},
    skill_import::{import_plan_to_event, plan_skill_import},
};
use buzz_core::kind::{
    KIND_AGENT_CONFIG_CREATED, KIND_AGENT_CONFIG_UPDATED, KIND_AGENT_GRAPH_EDGE,
    KIND_AGENT_SKILL_IMPORTED, KIND_FLOW_BLOCK_EXECUTED,
};
use buzz_db::event::EventQuery;
use buzz_flow::event_payloads::FlowBlockExecuted;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::state::AppState;

fn session_monitor() -> &'static Mutex<SessionMonitor> {
    static MONITOR: OnceLock<Mutex<SessionMonitor>> = OnceLock::new();
    MONITOR.get_or_init(|| Mutex::new(SessionMonitor::new(100)))
}

async fn community_from_host(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<buzz_core::tenant::CommunityId> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    crate::tenant::bind_community(&state.db, raw_host)
        .await
        .ok()
        .map(|tenant| tenant.community())
}

/// `GET /agent-studio/graph` — dependency graph from stored Agent Studio events.
pub async fn graph(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Json<Value> {
    let Some(community_id) = community_from_host(state.as_ref(), &headers).await else {
        return Json(buzz_agent_studio::graph::graph_json(&[], &[], &[], &[]));
    };

    let kinds = vec![
        KIND_AGENT_CONFIG_CREATED as i32,
        KIND_AGENT_CONFIG_UPDATED as i32,
        KIND_AGENT_SKILL_IMPORTED as i32,
        KIND_AGENT_GRAPH_EDGE as i32,
    ];
    let mut query = EventQuery::for_community(community_id);
    query.kinds = Some(kinds);
    query.limit = Some(500);
    query.global_only = true;

    let stored = match state
        .db
        .query_events_routed("agent_studio_graph", &query)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!("agent-studio graph query failed: {error}");
            return Json(buzz_agent_studio::graph::graph_json(&[], &[], &[], &[]));
        }
    };

    let events: Vec<StoredAgentStudioEvent> = stored
        .into_iter()
        .map(|row| StoredAgentStudioEvent {
            kind: buzz_core::kind::event_kind_u32(&row.event),
            content: row.event.content.to_string(),
        })
        .collect();

    Json(graph_from_stored_events(&events))
}

/// `GET /agent-studio/sessions` — recent session telemetry snapshots.
pub async fn sessions(State(_state): State<Arc<AppState>>) -> Json<Value> {
    let guard = session_monitor().lock().await;
    Json(telemetry_json(guard.snapshots()))
}

/// `GET /agent-studio/costs` — unified ACP session + Flow block cost rollup.
pub async fn costs(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Json<Value> {
    let guard = session_monitor().lock().await;
    let sessions = guard.snapshots().to_vec();
    let acp_session_cost_usd = guard.total_cost_usd();
    let acp_tokens: u64 = sessions
        .iter()
        .map(|session| session.input_tokens + session.output_tokens)
        .sum();

    let mut flow_block_cost_usd = 0.0f64;
    if let Some(community_id) = community_from_host(state.as_ref(), &headers).await {
        let mut query = EventQuery::for_community(community_id);
        query.kinds = Some(vec![KIND_FLOW_BLOCK_EXECUTED as i32]);
        query.limit = Some(200);
        query.global_only = true;
        if let Ok(stored) = state
            .db
            .query_events_routed("agent_studio_costs", &query)
            .await
        {
            for row in stored {
                let Ok(payload) =
                    serde_json::from_str::<FlowBlockExecuted>(&row.event.content.to_string())
                else {
                    continue;
                };
                let Ok(output) = serde_json::from_str::<Value>(&payload.output_json) else {
                    continue;
                };
                flow_block_cost_usd += output
                    .get("cost_usd")
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0);
            }
        }
    }

    Json(serde_json::json!({
        "total_cost_usd": acp_session_cost_usd + flow_block_cost_usd,
        "acp_session_cost_usd": acp_session_cost_usd,
        "flow_block_cost_usd": flow_block_cost_usd,
        "total_tokens": acp_tokens,
        "session_count": sessions.len(),
        "sessions": sessions,
    }))
}

/// Request body for skill import planning.
#[derive(Debug, Deserialize)]
pub struct ImportSkillRequest {
    /// GitHub repo URL or `owner/repo`.
    pub repo: String,
    /// Target skill slug.
    pub skill_id: String,
    /// Optional path within the repo.
    pub path: Option<String>,
}

/// Response for skill import planning (event payload to publish client-side).
#[derive(Debug, Serialize)]
pub struct ImportSkillResponse {
    /// Whether the import plan was accepted.
    pub accepted: bool,
    /// Serialized Nostr event content for kind 47250.
    pub event_payload: Value,
    /// Human-readable status message.
    pub message: String,
}

/// `POST /agent-studio/skills/import` — plan a skill import (returns event payload).
pub async fn import_skill(
    State(_state): State<Arc<AppState>>,
    JsonBody(body): JsonBody<ImportSkillRequest>,
) -> Json<ImportSkillResponse> {
    match plan_skill_import(&body.repo, &body.skill_id, body.path.as_deref()) {
        Ok(plan) => {
            let payload = import_plan_to_event(&plan, None);
            Json(ImportSkillResponse {
                accepted: true,
                event_payload: serde_json::to_value(&payload).unwrap_or(Value::Null),
                message: format!(
                    "Import planned for skill '{}' from {}",
                    plan.skill_id, plan.repo.slug
                ),
            })
        }
        Err(e) => Json(ImportSkillResponse {
            accepted: false,
            event_payload: Value::Null,
            message: e.to_string(),
        }),
    }
}

/// Record telemetry from ACP harness (internal hook — MVP).
pub async fn record_session_telemetry(telemetry: SessionTelemetry) {
    let mut guard = session_monitor().lock().await;
    guard.record(telemetry);
}

/// Request body for direct session telemetry injection (testing / HTTP bridge).
#[derive(Debug, Deserialize)]
pub struct PostTelemetryRequest {
    /// ACP session identifier.
    pub session_id: String,
    /// Optional managed-agent identifier.
    pub agent_id: Option<String>,
    /// Input tokens consumed this turn.
    pub input_tokens: u64,
    /// Output tokens produced this turn.
    pub output_tokens: u64,
    /// Estimated USD cost for this turn.
    pub cost_usd: f64,
    /// Tool invocations this turn.
    #[serde(default)]
    pub tool_calls: u32,
}

/// `POST /agent-studio/telemetry` — record a session telemetry snapshot.
pub async fn post_telemetry(
    State(_state): State<Arc<AppState>>,
    JsonBody(body): JsonBody<PostTelemetryRequest>,
) -> Json<Value> {
    let recorded_at = chrono::Utc::now().timestamp();
    record_session_telemetry(SessionTelemetry {
        session_id: body.session_id,
        agent_id: body.agent_id,
        input_tokens: body.input_tokens,
        output_tokens: body.output_tokens,
        cost_usd: body.cost_usd,
        tool_calls: body.tool_calls,
        recorded_at,
    })
    .await;
    Json(serde_json::json!({ "accepted": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sessions_endpoint_returns_json() {
        record_session_telemetry(SessionTelemetry {
            session_id: "t1".into(),
            agent_id: None,
            input_tokens: 1,
            output_tokens: 1,
            cost_usd: 0.0,
            tool_calls: 0,
            recorded_at: 0,
        })
        .await;
        let guard = session_monitor().lock().await;
        let json = telemetry_json(guard.snapshots());
        assert!(json.get("sessions").and_then(|v| v.as_array()).is_some());
    }
}
