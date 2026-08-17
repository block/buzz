//! Flow Studio HTTP surface (Buzz Hive).

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::Json,
    Json as JsonBody,
};
use buzz_flow::{
    blocks::blocks_json,
    events::FlowGraphSaved,
    tools::tools_json,
    workflow_bridge::{block_to_step, CanvasBlock},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::AppState;

async fn community_from_host(
    state: &AppState,
    headers: &axum::http::HeaderMap,
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

/// `GET /flow-studio/blocks` — draggable block palette catalog.
pub async fn blocks(State(_state): State<Arc<AppState>>) -> Json<Value> {
    Json(blocks_json())
}

/// `GET /flow-studio/tools` — tool registry for agent blocks.
pub async fn tools(State(_state): State<Arc<AppState>>) -> Json<Value> {
    Json(tools_json())
}

/// Canvas graph input for YAML conversion.
#[derive(Debug, Deserialize)]
pub struct CanvasToYamlRequest {
    /// Canvas blocks in execution order.
    pub blocks: Vec<CanvasBlock>,
    /// Flow Studio canvas id (`d` tag) embedded in exported workflow metadata.
    #[serde(default)]
    pub flow_id: Option<String>,
}

/// YAML conversion response.
#[derive(Debug, Serialize)]
pub struct CanvasToYamlResponse {
    /// Serialized workflow YAML on success.
    pub yaml: String,
    /// Error message when conversion fails.
    pub error: Option<String>,
}

/// Save graph request body.
#[derive(Debug, Deserialize)]
pub struct SaveGraphRequest {
    /// Flow identifier (`d` tag).
    pub flow_id: String,
    /// Serialized canvas graph JSON.
    pub graph_json: String,
}

/// Save graph response — event payload for client publish (kind 46200).
#[derive(Debug, Serialize)]
pub struct SaveGraphResponse {
    /// Whether the payload was accepted for publish.
    pub accepted: bool,
    /// Event content to sign and submit as kind 46200.
    pub event_payload: Value,
    /// Human-readable status message.
    pub message: String,
}

/// Query params for loading a saved canvas graph.
#[derive(Debug, Deserialize)]
pub struct GetGraphQuery {
    /// Flow identifier (`d` tag).
    pub flow_id: String,
}

/// Response for a saved canvas graph lookup.
#[derive(Debug, Serialize)]
pub struct GetGraphResponse {
    /// Flow identifier echoed from the query.
    pub flow_id: String,
    /// Serialized canvas graph when found.
    pub graph_json: Option<String>,
    /// Whether a saved graph exists for this flow id.
    pub found: bool,
}

/// Knowledge search query params.
#[derive(Debug, Deserialize)]
pub struct KnowledgeSearchQuery {
    /// Knowledge base to search within.
    pub knowledge_base_id: String,
    /// Query string (keyword or semantic depending on `mode`).
    pub q: String,
    /// Maximum hits to return.
    #[serde(default = "default_search_limit")]
    pub limit: i64,
    /// `keyword` (default) or `semantic` (pgvector cosine distance).
    #[serde(default = "default_search_mode")]
    pub mode: String,
}

fn default_search_mode() -> String {
    "semantic".into()
}

fn default_search_limit() -> i64 {
    10
}

/// `GET /flow-studio/graph` — latest saved canvas for a flow id (kind 46200).
pub async fn get_graph(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(query): Query<GetGraphQuery>,
) -> Json<GetGraphResponse> {
    let Some(community_id) = community_from_host(state.as_ref(), &headers).await else {
        return Json(GetGraphResponse {
            flow_id: query.flow_id,
            graph_json: None,
            found: false,
        });
    };

    let content = match state
        .db
        .get_latest_flow_graph(community_id, &query.flow_id)
        .await
    {
        Ok(content) => content,
        Err(error) => {
            tracing::warn!("flow-studio graph lookup failed: {error}");
            None
        }
    };

    let graph_json = content
        .as_deref()
        .and_then(|raw| serde_json::from_str::<FlowGraphSaved>(raw).ok())
        .map(|payload| payload.graph_json);

    Json(GetGraphResponse {
        flow_id: query.flow_id,
        graph_json: graph_json.clone(),
        found: graph_json.is_some(),
    })
}

/// `GET /flow-studio/knowledge/search` — keyword search over indexed chunks.
pub async fn knowledge_search(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(query): Query<KnowledgeSearchQuery>,
) -> Json<Value> {
    let Some(community_id) = community_from_host(state.as_ref(), &headers).await else {
        return Json(serde_json::json!({ "hits": [] }));
    };

    let hits = if query.mode == "keyword" {
        state
            .db
            .search_flow_knowledge(
                community_id,
                &query.knowledge_base_id,
                &query.q,
                query.limit,
            )
            .await
            .unwrap_or_default()
    } else {
        let embedding = buzz_flow::knowledge::embed::text_to_embedding(&query.q);
        state
            .db
            .search_flow_knowledge_semantic(
                community_id,
                &query.knowledge_base_id,
                &embedding,
                query.limit,
            )
            .await
            .unwrap_or_default()
    };

    Json(serde_json::json!({
        "mode": query.mode,
        "hits": hits.into_iter().map(|hit| serde_json::json!({
            "document_id": hit.document_id,
            "chunk_index": hit.chunk_index,
            "content": hit.content,
        })).collect::<Vec<_>>()
    }))
}

/// Query params for listing table rows.
#[derive(Debug, Deserialize)]
pub struct ListTableRowsQuery {
    /// Maximum rows to return.
    #[serde(default = "default_list_limit")]
    pub limit: i64,
}

fn default_list_limit() -> i64 {
    50
}

/// `GET /flow-studio/tables/{table_id}/rows` — list projected table rows.
pub async fn list_table_rows(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(table_id): axum::extract::Path<String>,
    Query(query): Query<ListTableRowsQuery>,
) -> Json<Value> {
    let Some(community_id) = community_from_host(state.as_ref(), &headers).await else {
        return Json(serde_json::json!({ "rows": [] }));
    };

    let rows = state
        .db
        .list_flow_table_rows(community_id, &table_id, query.limit)
        .await
        .unwrap_or_default();

    Json(serde_json::json!({
        "table_id": table_id,
        "rows": rows.into_iter().map(|row| serde_json::json!({
            "row_id": row.row_id,
            "row_json": row.row_json,
        })).collect::<Vec<_>>()
    }))
}

/// `GET /flow-studio/files` — list projected file metadata.
pub async fn list_files(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(query): Query<ListTableRowsQuery>,
) -> Json<Value> {
    let Some(community_id) = community_from_host(state.as_ref(), &headers).await else {
        return Json(serde_json::json!({ "files": [] }));
    };

    let files = state
        .db
        .list_flow_files(community_id, query.limit)
        .await
        .unwrap_or_default();

    Json(serde_json::json!({
        "files": files.into_iter().map(|file| serde_json::json!({
            "file_id": file.file_id,
            "filename": file.filename,
            "media_url": file.media_url,
            "version": file.version,
        })).collect::<Vec<_>>()
    }))
}

/// `POST /flow-studio/graph/save` — build FlowGraphSaved event payload.
pub async fn save_graph(
    State(_state): State<Arc<AppState>>,
    JsonBody(body): JsonBody<SaveGraphRequest>,
) -> Json<SaveGraphResponse> {
    let payload = buzz_flow::events::FlowGraphSaved {
        flow_id: body.flow_id.clone(),
        graph_json: body.graph_json,
    };
    Json(SaveGraphResponse {
        accepted: true,
        event_payload: serde_json::to_value(&payload).unwrap_or(Value::Null),
        message: format!(
            "Publish kind 46200 with d-tag '{}' to persist this canvas",
            body.flow_id
        ),
    })
}

/// `POST /flow-studio/yaml/from-canvas` — convert canvas blocks to workflow YAML steps.
pub async fn yaml_from_canvas(
    State(_state): State<Arc<AppState>>,
    JsonBody(body): JsonBody<CanvasToYamlRequest>,
) -> Json<CanvasToYamlResponse> {
    let mut steps = Vec::new();
    for block in &body.blocks {
        match block_to_step(block) {
            Ok(step) => steps.push(step),
            Err(e) => {
                return Json(CanvasToYamlResponse {
                    yaml: String::new(),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    let def = buzz_workflow::schema::WorkflowDef {
        name: "flow-studio-export".into(),
        description: None,
        trigger: buzz_workflow::schema::TriggerDef::Webhook,
        steps,
        enabled: true,
        flow_id: body.flow_id,
    };

    match serde_yaml::to_string(&def) {
        Ok(yaml) => Json(CanvasToYamlResponse { yaml, error: None }),
        Err(e) => Json(CanvasToYamlResponse {
            yaml: String::new(),
            error: Some(e.to_string()),
        }),
    }
}
