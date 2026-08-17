use tauri::State;

use crate::{
    app_state::AppState,
    events,
    relay::{query_relay, submit_event},
};

/// Publish a Flow Studio canvas graph (kind 46200).
#[tauri::command]
pub async fn publish_flow_graph(
    flow_id: String,
    graph_json: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let builder = events::build_flow_graph_saved(&flow_id, &graph_json)?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({
        "accepted": result.accepted,
        "event_id": result.event_id,
        "message": format!("Flow graph '{flow_id}' published"),
    }))
}

/// Publish an Agent Studio skill import event (kind 47250).
#[tauri::command]
pub async fn publish_skill_import(
    skill_id: String,
    source_repo: Option<String>,
    source_commit: Option<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let builder = events::build_agent_skill_imported(
        &skill_id,
        source_repo.as_deref(),
        source_commit.as_deref(),
    )?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({
        "accepted": result.accepted,
        "event_id": result.event_id,
        "message": format!("Skill '{skill_id}' import published"),
    }))
}

/// Load the latest saved Flow Studio graph for a flow id.
#[tauri::command]
pub async fn get_flow_graph(
    flow_id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [46200],
            "#d": [flow_id.clone()],
            "limit": 1
        })],
    )
    .await?;

    let Some(event) = events.first() else {
        return Ok(serde_json::json!({
            "flow_id": flow_id,
            "graph_json": null,
            "found": false,
        }));
    };

    let payload: serde_json::Value =
        serde_json::from_str(&event.content).map_err(|e| format!("invalid graph payload: {e}"))?;
    Ok(serde_json::json!({
        "flow_id": flow_id,
        "graph_json": payload.get("graph_json").cloned().unwrap_or(serde_json::Value::Null),
        "found": true,
        "event_id": event.id.to_hex(),
    }))
}

/// Publish a knowledge-base document ingest event (kind 46250).
#[tauri::command]
pub async fn publish_kb_document(
    knowledge_base_id: String,
    document_id: String,
    filename: String,
    mime_type: String,
    content: Option<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let builder = events::build_flow_kb_document_ingested(
        &knowledge_base_id,
        &document_id,
        &filename,
        &mime_type,
        content.as_deref(),
    )?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({
        "accepted": result.accepted,
        "event_id": result.event_id,
        "message": format!("Document '{document_id}' ingested"),
    }))
}

/// Publish a Flow Studio table row (kind 46300).
#[tauri::command]
pub async fn publish_table_row(
    table_id: String,
    row_id: String,
    row_json: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let builder = events::build_flow_table_row_created(&table_id, &row_id, &row_json)?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({
        "accepted": result.accepted,
        "event_id": result.event_id,
        "message": format!("Row '{row_id}' saved in table '{table_id}'"),
    }))
}

/// Delete a Flow Studio table row (kind 46302).
#[tauri::command]
pub async fn delete_table_row(
    table_id: String,
    row_id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let builder = events::build_flow_table_row_deleted(&table_id, &row_id)?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({
        "accepted": result.accepted,
        "event_id": result.event_id,
        "message": format!("Row '{row_id}' deleted from table '{table_id}'"),
    }))
}

/// Publish Flow Studio file metadata (kind 46350).
#[tauri::command]
pub async fn publish_flow_file(
    file_id: String,
    filename: String,
    media_url: Option<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let builder = events::build_flow_file_uploaded(&file_id, &filename, media_url.as_deref())?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({
        "accepted": result.accepted,
        "event_id": result.event_id,
        "message": format!("File '{filename}' registered"),
    }))
}
