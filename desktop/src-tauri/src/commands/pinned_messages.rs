use tauri::State;

use crate::{
    app_state::AppState,
    events,
    relay::{query_relay, submit_event},
};

/// Server-side sanity ceiling — the client never sends more than this, but
/// this command doesn't trust that blindly.
const MAX_PINNED_MESSAGES: usize = 3;

/// Read the most recent pinned-messages event (kind:40004) for a channel/DM
/// and return the pinned event-id list. Mirrors `get_canvas`'s
/// "latest wins via `#h` + `limit:1`" read pattern. Returns an empty vec if
/// no pin event has been published yet for this channel.
#[tauri::command]
pub async fn get_pinned_messages(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [40004],
            "#h": [channel_id],
            "limit": 1
        })],
    )
    .await?;

    let Some(event) = events.first() else {
        return Ok(Vec::new());
    };

    let parsed: serde_json::Value = serde_json::from_str(&event.content)
        .map_err(|e| format!("failed to parse pinned-messages content: {e}"))?;

    let pinned = parsed
        .get("pinned")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    Ok(pinned)
}

/// Publish a fresh kind:40004 event with the full new pinned-message list.
/// This is a full replace — the frontend computes the new complete list
/// client-side (after adding/removing one entry) and calls this with it.
#[tauri::command]
pub async fn set_pinned_messages(
    channel_id: String,
    pinned_event_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if pinned_event_ids.len() > MAX_PINNED_MESSAGES {
        return Err(format!(
            "cannot pin more than {MAX_PINNED_MESSAGES} messages (got {})",
            pinned_event_ids.len()
        ));
    }

    let uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let builder = events::build_set_pinned_messages(uuid, &pinned_event_ids)?;
    submit_event(builder, &state).await?;

    Ok(())
}
