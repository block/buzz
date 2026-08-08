use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    relay::{parse_command_response, query_relay, submit_event},
};

// ── Wire shapes (snake_case, consumed by tauriWorkflows.ts) ──────────────────

/// A workflow definition as the desktop frontend expects it. Mirrors the
/// `RawWorkflow` type in `desktop/src/shared/api/tauriWorkflows.ts`.
///
/// The relay stores a workflow as a single kind:30620 event whose content is
/// the raw YAML. Everything the UI needs is derived from that event:
/// - `id` / `channel_id` from the `d` / `h` tags,
/// - `definition` from parsing the YAML body into a free-form object,
/// - `name` from `definition.name`,
/// - `owner_pubkey` / timestamps from the event itself.
///
/// `status` is always `"active"` here: the relay's disable/archive lifecycle is
/// not reflected back into the kind:30620 event, and the UI derives a
/// "disabled" display state from `definition.enabled` on its own
/// (`getWorkflowDisplayStatus`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowWire {
    pub id: String,
    pub name: String,
    pub owner_pubkey: String,
    pub channel_id: Option<String>,
    pub definition: Value,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Response shape for create/update. Mirrors `RawWorkflowSaveResponse` in the
/// frontend: a full workflow record plus an optional webhook secret (only
/// present for webhook-triggered workflows on creation).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkflowSaveWire {
    #[serde(flatten)]
    pub workflow: WorkflowWire,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
}

fn lifecycle_payload(event: &nostr::Event) -> Option<Value> {
    serde_json::from_str::<Value>(&event.content)
        .ok()
        .filter(Value::is_object)
}

fn lifecycle_run_payload(event: &nostr::Event) -> Option<Value> {
    let payload = lifecycle_payload(event)?;
    let kind = event.kind.as_u16() as u32;
    let run = if (46010..=46012).contains(&kind) {
        payload.get("run").cloned().unwrap_or(payload)
    } else {
        payload
    };
    (run.get("id").and_then(Value::as_str).is_some()
        && run.get("workflow_id").and_then(Value::as_str).is_some())
    .then_some(run)
}

fn lifecycle_precedence(kind: u32) -> u8 {
    match kind {
        46005..=46007 => 4,
        46011..=46012 => 3,
        46010 => 2,
        46001 => 1,
        _ => 0,
    }
}

// ── Reads ────────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_channel_workflows(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<WorkflowWire>, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#h": [channel_id],
        })],
    )
    .await?;

    Ok(events.iter().map(workflow_from_event).collect())
}

/// Fetch workflows across many channels in a single relay round-trip.
///
/// The Workflows overview screen previously issued one `get_channel_workflows`
/// query per member channel (`Promise.all` fanout in `WorkflowsView`), i.e. N
/// relay POSTs. A nostr `#h` filter matches ANY of its listed values, so one
/// query with all channel ids returns the same set. Each `WorkflowWire` carries
/// its own `channel_id` (from the event's `h` tag), so the frontend can still
/// group results by channel. Neither this nor the per-channel command sets a
/// `limit`, so batching does not change result completeness.
#[tauri::command]
pub async fn get_channels_workflows(
    channel_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<WorkflowWire>, String> {
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#h": channel_ids,
        })],
    )
    .await?;

    Ok(events.iter().map(workflow_from_event).collect())
}

#[tauri::command]
pub async fn get_workflow(
    workflow_id: String,
    state: State<'_, AppState>,
) -> Result<WorkflowWire, String> {
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
) -> Result<Vec<Value>, String> {
    let requested_limit = limit.unwrap_or(20).min(100);
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [46001, 46005, 46006, 46007, 46010, 46011, 46012],
            "#d": [workflow_id],
            "limit": 500
        })],
    )
    .await?;

    let mut latest: HashMap<String, (u64, u8, Value)> = HashMap::new();
    for event in events {
        let Some(run) = lifecycle_run_payload(&event) else {
            continue;
        };
        if run.get("workflow_id").and_then(Value::as_str) != Some(workflow_id.as_str()) {
            continue;
        }
        let Some(run_id) = run.get("id").and_then(Value::as_str) else {
            continue;
        };
        let created_at = event.created_at.as_secs();
        let precedence = lifecycle_precedence(event.kind.as_u16() as u32);
        if latest
            .get(run_id)
            .is_none_or(|(seen_at, seen_precedence, _)| {
                (created_at, precedence) >= (*seen_at, *seen_precedence)
            })
        {
            latest.insert(run_id.to_owned(), (created_at, precedence, run));
        }
    }

    let mut runs: Vec<(u64, u8, Value)> = latest.into_values().collect();
    runs.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    runs.truncate(requested_limit as usize);
    Ok(runs.into_iter().map(|(_, _, run)| run).collect())
}

// ── Writes ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn create_workflow(
    channel_id: String,
    yaml_definition: String,
    state: State<'_, AppState>,
) -> Result<WorkflowSaveWire, String> {
    let workflow_id = uuid::Uuid::new_v4().to_string();
    let builder = events::build_workflow_definition(&workflow_id, &channel_id, &yaml_definition)?;
    let result = submit_event(builder, &state).await?;

    // The relay returns `webhook_secret` in the OK response message for
    // webhook-triggered workflows. Everything else in the save record is built
    // locally from the inputs we already hold — the relay's create response
    // only carries `{ workflow_id, webhook_secret? }`.
    let webhook_secret = parse_command_response::<Value>(&result.message)
        .ok()
        .and_then(|v| {
            v.get("webhook_secret")
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    let now = now_secs();
    let workflow = workflow_record(
        workflow_id,
        Some(channel_id),
        current_pubkey_hex(&state)?,
        &yaml_definition,
        now,
        now,
    );

    Ok(WorkflowSaveWire {
        workflow,
        webhook_secret,
    })
}

#[tauri::command]
pub async fn update_workflow(
    workflow_id: String,
    yaml_definition: String,
    state: State<'_, AppState>,
) -> Result<WorkflowSaveWire, String> {
    // Find the channel id (and creation time) from the existing workflow event
    // so the new event carries the same `h` tag — kind:30620 is replaceable by
    // (pubkey, d-tag).
    let prior = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30620],
            "#d": [workflow_id.clone()],
            "limit": 1
        })],
    )
    .await?;

    let prior_event = prior
        .first()
        .ok_or_else(|| "workflow not found".to_string())?;
    let channel_id = tag_value(prior_event, "h").ok_or_else(|| "workflow not found".to_string())?;
    let created_at = prior_event.created_at.as_secs() as i64;

    let builder = events::build_workflow_definition(&workflow_id, &channel_id, &yaml_definition)?;
    submit_event(builder, &state).await?;

    let updated_at = now_secs();
    let workflow = workflow_record(
        workflow_id,
        Some(channel_id),
        current_pubkey_hex(&state)?,
        &yaml_definition,
        created_at,
        updated_at,
    );

    Ok(WorkflowSaveWire {
        workflow,
        // Updates never rotate the webhook secret.
        webhook_secret: None,
    })
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
    let response = parse_command_response::<Value>(&result.message).unwrap_or(Value::Null);
    Ok(serde_json::json!({
        "event_id": result.event_id,
        "run_id": response.get("run_id").and_then(Value::as_str).unwrap_or(""),
        "workflow_id": workflow_id,
        "status": "pending",
    }))
}

// ── Approvals ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_run_approvals(
    workflow_id: String,
    run_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Value>, String> {
    let mut events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [46010, 46011, 46012],
            "#d": [workflow_id],
            "limit": 500
        })],
    )
    .await?;
    events.sort_by_key(|event| event.created_at.as_secs());

    let mut latest: HashMap<String, (u64, u8, Value)> = HashMap::new();
    for event in events {
        let Some(mut approval) = lifecycle_payload(&event) else {
            continue;
        };
        if approval.get("workflow_id").and_then(Value::as_str) != Some(workflow_id.as_str())
            || approval.get("run_id").and_then(Value::as_str) != Some(run_id.as_str())
        {
            continue;
        }
        let Some(token) = approval
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let created_at = event.created_at.as_secs();
        let precedence = match approval.get("status").and_then(Value::as_str) {
            Some("granted" | "denied") => 2,
            Some("pending") => 1,
            _ => 0,
        };
        if approval.get("message").is_none() {
            if let Some((_, _, previous)) = latest.get(&token) {
                if let Some(message) = previous.get("message") {
                    approval["message"] = message.clone();
                }
            }
        }
        let should_replace = latest.get(&token).is_none_or(
            |(seen_at, seen_precedence, _)| {
                (created_at, precedence) >= (*seen_at, *seen_precedence)
            },
        );
        if should_replace {
            latest.insert(token, (created_at, precedence, approval));
        }
    }

    Ok(latest
        .into_values()
        .map(|(_, _, approval)| approval)
        .collect())
}

#[tauri::command]
pub async fn grant_approval(
    token: String,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let builder = events::build_approval_grant(&token, note.as_deref())?;
    let result = submit_event(builder, &state).await?;
    let response = parse_command_response::<Value>(&result.message).unwrap_or(Value::Null);
    Ok(serde_json::json!({
        "event_id": result.event_id,
        "token": token,
        "status": response.get("status").and_then(Value::as_str).unwrap_or("granted"),
        "run_id": response.get("run_id").and_then(Value::as_str).unwrap_or(""),
        "workflow_id": "",
    }))
}

#[tauri::command]
pub async fn deny_approval(
    token: String,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let builder = events::build_approval_deny(&token, note.as_deref())?;
    let result = submit_event(builder, &state).await?;
    let response = parse_command_response::<Value>(&result.message).unwrap_or(Value::Null);
    Ok(serde_json::json!({
        "event_id": result.event_id,
        "token": token,
        "status": response.get("status").and_then(Value::as_str).unwrap_or("denied"),
        "run_id": response.get("run_id").and_then(Value::as_str).unwrap_or(""),
        "workflow_id": "",
    }))
}

// ── Helpers (pure, unit-tested in workflows_tests.rs) ─────────────────────────

fn current_pubkey_hex(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    Ok(keys.public_key().to_hex())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// First value of the tag whose name matches `name` (e.g. `d`, `h`).
fn tag_value(ev: &nostr::Event, name: &str) -> Option<String> {
    ev.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.len() >= 2 && s[0] == name).then(|| s[1].clone())
    })
}

/// Parse a workflow's YAML body into a free-form JSON object. The frontend
/// consumes `definition` as `Record<string, unknown>`, so we preserve the full
/// document. On parse failure (or a non-object document) we fall back to an
/// empty object rather than failing the whole list query — a single malformed
/// workflow must not break the page.
fn parse_definition(yaml: &str) -> Value {
    match serde_yaml::from_str::<Value>(yaml) {
        Ok(v @ Value::Object(_)) => v,
        _ => Value::Object(serde_json::Map::new()),
    }
}

/// Build a [`WorkflowWire`] record from its parts. Shared by the read path
/// (from a relay event) and the write path (from local inputs).
fn workflow_record(
    id: String,
    channel_id: Option<String>,
    owner_pubkey: String,
    yaml_definition: &str,
    created_at: i64,
    updated_at: i64,
) -> WorkflowWire {
    let definition = parse_definition(yaml_definition);
    let name = definition
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.clone());

    WorkflowWire {
        id,
        name,
        owner_pubkey,
        channel_id,
        definition,
        status: "active".to_string(),
        created_at,
        updated_at,
    }
}

/// Convert a kind:30620 workflow definition event into a [`WorkflowWire`].
fn workflow_from_event(ev: &nostr::Event) -> WorkflowWire {
    let id = tag_value(ev, "d").unwrap_or_default();
    let channel_id = tag_value(ev, "h");
    let ts = ev.created_at.as_secs() as i64;
    workflow_record(id, channel_id, ev.pubkey.to_hex(), &ev.content, ts, ts)
}

#[cfg(test)]
#[path = "workflows_tests.rs"]
mod tests;
