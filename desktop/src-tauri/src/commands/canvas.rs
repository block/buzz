use tauri::State;

use crate::{
    app_state::AppState,
    events,
    relay::{query_relay, submit_event},
};

const CANVAS_HISTORY_LIMIT: u64 = 50;

/// Read the most recent canvas event (kind:40100) for a channel.
#[tauri::command]
pub async fn get_canvas(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [40100],
            "#h": [channel_id],
            "limit": 1
        })],
    )
    .await?;

    let Some(event) = events.first() else {
        // Explicit nulls: the TS caller distinguishes "no canvas yet" from
        // "canvas exists" via `updated_at`/`author`, so these keys must be
        // present (absent keys deserialize as `undefined`, not `null`).
        return Ok(serde_json::json!({
            "content": "",
            "event_id": null,
            "updated_at": null,
            "author": null,
        }));
    };

    Ok(serde_json::json!({
        "content": event.content,
        "event_id": event.id.to_hex(),
        "updated_at": event.created_at.as_secs(),
        "author": event.pubkey.to_hex(),
    }))
}

/// Read the most recent signed canvas revisions for a channel.
///
/// Canvas events are append-only kind:40100 events, so the relay already
/// retains the complete revision history. The bounded response keeps the
/// Desktop panel predictable for heavily edited canvases while preserving the
/// newest revision as the first item.
#[tauri::command]
pub async fn get_canvas_history(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let mut events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [40100],
            "#h": [channel_id],
            "limit": CANVAS_HISTORY_LIMIT
        })],
    )
    .await?;

    events.sort_by_key(|event| std::cmp::Reverse(event.created_at));

    Ok(serde_json::json!({
        "revisions": events.into_iter().map(|event| serde_json::json!({
            "event_id": event.id.to_hex(),
            "content": event.content,
            "updated_at": event.created_at.as_secs(),
            "author": event.pubkey.to_hex(),
        })).collect::<Vec<_>>(),
    }))
}

#[tauri::command]
pub async fn set_canvas(
    channel_id: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let builder = events::build_set_canvas(uuid, &content)?;
    let result = submit_event(builder, &state).await?;

    Ok(serde_json::json!({
        "ok": true,
        "event_id": result.event_id,
    }))
}
