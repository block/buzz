use tauri::State;

use crate::{app_state::AppState, relay::query_relay};

const GET_EVENT_KINDS: [u32; 15] = [
    0,
    1,
    3,
    5,
    7,
    9,
    30078,
    40002,
    40003,
    40008,
    40099,
    40100,
    45001,
    45003,
    buzz_core_pkg::kind::KIND_HUDDLE_STARTED,
];

#[tauri::command]
pub async fn get_event(event_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "ids": [event_id],
            "kinds": GET_EVENT_KINDS,
            "limit": 1
        })],
    )
    .await?;

    let event = events
        .first()
        .ok_or_else(|| "event not found".to_string())?;
    serde_json::to_string(event).map_err(|error| format!("serialize event: {error}"))
}

/// Resolve many exact event IDs with one relay request. Callers still validate
/// event kind, channel scope, and requested ID before using presentation data.
#[tauri::command]
pub async fn get_events(
    event_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    if event_ids.is_empty() {
        return Ok(Vec::new());
    }

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "ids": event_ids,
            "kinds": GET_EVENT_KINDS,
            "limit": event_ids.len()
        })],
    )
    .await?;

    events
        .iter()
        .map(|event| {
            serde_json::to_value(event).map_err(|error| format!("serialize event: {error}"))
        })
        .collect()
}
