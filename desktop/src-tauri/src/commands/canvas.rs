use std::collections::{HashMap, HashSet};

use tauri::State;

use crate::{
    app_state::AppState,
    events,
    relay::{query_relay, submit_event},
};

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

/// Read the most recent canvas event for each requested channel.
#[tauri::command]
pub async fn get_canvases(
    channel_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let channel_ids: HashSet<_> = channel_ids.into_iter().collect();
    let filters: Vec<_> = channel_ids
        .iter()
        .map(|channel_id| {
            serde_json::json!({
                "kinds": [40100],
                "#h": [channel_id],
                "limit": 1
            })
        })
        .collect();
    let mut canvases = HashMap::new();

    for batch in filters.chunks(64) {
        for event in query_relay(&state, batch).await? {
            let Some(channel_id) = event.tags.iter().find_map(|tag| {
                let values = tag.as_slice();
                (values.first().map(String::as_str) == Some("h"))
                    .then(|| values.get(1).cloned())
                    .flatten()
            }) else {
                continue;
            };
            if !channel_ids.contains(&channel_id) {
                continue;
            }
            canvases.insert(
                channel_id,
                serde_json::json!({
                    "content": event.content,
                    "updated_at": event.created_at.as_secs(),
                    "author": event.pubkey.to_hex(),
                }),
            );
        }
    }

    serde_json::to_value(canvases).map_err(|error| error.to_string())
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
