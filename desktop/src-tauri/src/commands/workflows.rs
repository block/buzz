use serde_json::Value;
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    relay::{parse_command_response, query_relay, submit_event},
};

// ── Reads ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_channel_workflows(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#h": [channel_id],
        })],
    )
    .await?;

    let workflows: Vec<Value> = events.iter().map(workflow_from_event).collect();
    Ok(serde_json::json!({ "workflows": workflows }))
}

#[tauri::command]
pub async fn get_workflow(
    workflow_id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#d": [workflow_id],
            "limit": 1
        })],
    )
    .await?;

    events
        .first()
        .map(workflow_from_event)
        .ok_or_else(|| "workflow not found".to_string())
}

#[tauri::command]
pub async fn get_workflow_runs(
    workflow_id: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let cap = limit.unwrap_or(50).min(200);
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [46001, 46002, 46003, 46004, 46005, 46006, 46007, 46010, 46011, 46012],
            "#d": [workflow_id],
            "limit": cap,
        })],
    )
    .await?;

    let runs: Vec<Value> = events
        .iter()
        .map(|ev| {
            serde_json::json!({
                "event_id": ev.id.to_hex(),
                "kind": ev.kind.as_u16(),
                "pubkey": ev.pubkey.to_hex(),
                "created_at": ev.created_at.as_secs(),
                "content": ev.content,
                "tags": ev.tags.iter().map(|t| t.as_slice().to_vec()).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(serde_json::json!({ "runs": runs }))
}

// ── Writes ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn create_workflow(
    channel_id: String,
    yaml_definition: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let workflow_id = uuid::Uuid::new_v4().to_string();
    let builder = events::build_workflow_definition(&workflow_id, &channel_id, &yaml_definition)?;
    let result = submit_event(builder, &state).await?;

    // The relay returns webhook_secret in the OK response message for new workflows.
    let mut response = serde_json::json!({
        "workflow_id": workflow_id,
        "event_id": result.event_id,
    });
    if let Ok(cmd_resp) = parse_command_response::<Value>(&result.message) {
        if let Some(secret) = cmd_resp.get("webhook_secret") {
            response["webhook_secret"] = secret.clone();
        }
    }
    Ok(response)
}

#[tauri::command]
pub async fn update_workflow(
    workflow_id: String,
    yaml_definition: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    // Find the channel id from the existing workflow event so the new event
    // carries the same `h` tag — kind:30620 is replaceable by (pubkey, d-tag).
    let prior = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#d": [workflow_id.clone()],
            "limit": 1
        })],
    )
    .await?;

    let channel_id = prior
        .first()
        .and_then(|ev| {
            ev.tags.iter().find_map(|t| {
                let s = t.as_slice();
                if s.len() >= 2 && s[0] == "h" {
                    Some(s[1].clone())
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| "workflow not found".to_string())?;

    let builder = events::build_workflow_definition(&workflow_id, &channel_id, &yaml_definition)?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({
        "workflow_id": workflow_id,
        "event_id": result.event_id,
    }))
}

#[tauri::command]
pub async fn delete_workflow(
    workflow_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let builder = events::build_workflow_delete(&workflow_id, &current_pubkey_hex(&state)?)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn trigger_workflow(
    workflow_id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let builder = events::build_workflow_trigger(&workflow_id)?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({ "event_id": result.event_id }))
}

// ── Approvals ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_run_approvals(
    workflow_id: String,
    run_id: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let _ = run_id;
    // Approval-request events for a workflow are kinds 46010/46011/46012.
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [46010, 46011, 46012],
            "#d": [workflow_id],
        })],
    )
    .await?;
    let approvals: Vec<Value> = events
        .iter()
        .map(|ev| {
            serde_json::json!({
                "event_id": ev.id.to_hex(),
                "kind": ev.kind.as_u16(),
                "pubkey": ev.pubkey.to_hex(),
                "created_at": ev.created_at.as_secs(),
                "content": ev.content,
                "tags": ev.tags.iter().map(|t| t.as_slice().to_vec()).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(serde_json::json!({ "approvals": approvals }))
}

#[tauri::command]
pub async fn grant_approval(
    token: String,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let builder = events::build_approval_grant(&token, note.as_deref())?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({ "event_id": result.event_id }))
}

#[tauri::command]
pub async fn deny_approval(
    token: String,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let builder = events::build_approval_deny(&token, note.as_deref())?;
    let result = submit_event(builder, &state).await?;
    Ok(serde_json::json!({ "event_id": result.event_id }))
}

fn current_pubkey_hex(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    Ok(keys.public_key().to_hex())
}

fn workflow_from_event(ev: &nostr::Event) -> Value {
    let workflow_id = ev
        .tags
        .iter()
        .find_map(|t| {
            let s = t.as_slice();
            if s.len() >= 2 && s[0] == "d" {
                Some(s[1].clone())
            } else {
                None
            }
        })
        .unwrap_or_default();
    let channel_id = ev
        .tags
        .iter()
        .find_map(|t| {
            let s = t.as_slice();
            if s.len() >= 2 && s[0] == "h" {
                Some(s[1].clone())
            } else {
                None
            }
        })
        .unwrap_or_default();
    serde_json::json!({
        "workflow_id": workflow_id,
        "channel_id": channel_id,
        "yaml_definition": ev.content,
        "event_id": ev.id.to_hex(),
        "pubkey": ev.pubkey.to_hex(),
        "created_at": ev.created_at.as_secs(),
    })
}
