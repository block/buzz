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

#[tauri::command]
pub async fn set_canvas(
    channel_id: String,
    content: String,
    enforce_revision: Option<bool>,
    expected_event_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    if enforce_revision.unwrap_or(false) {
        let events = query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [40100],
                "#h": [channel_id],
                "limit": 1
            })],
        )
        .await?;
        let current_event_id = events.first().map(|event| event.id.to_hex());
        ensure_canvas_revision(expected_event_id.as_deref(), current_event_id.as_deref())?;
    }
    let builder = events::build_set_canvas(uuid, &content)?;
    let result = submit_event(builder, &state).await?;

    Ok(serde_json::json!({
        "ok": true,
        "event_id": result.event_id,
    }))
}

fn ensure_canvas_revision(expected: Option<&str>, current: Option<&str>) -> Result<(), String> {
    if expected == current {
        return Ok(());
    }
    Err("Canvas revision conflict: the board changed before this save.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::ensure_canvas_revision;

    #[test]
    fn canvas_revision_accepts_matching_existing_or_empty_state() {
        assert!(ensure_canvas_revision(Some("abc"), Some("abc")).is_ok());
        assert!(ensure_canvas_revision(None, None).is_ok());
    }

    #[test]
    fn canvas_revision_rejects_stale_or_unexpected_state() {
        assert!(ensure_canvas_revision(Some("old"), Some("new")).is_err());
        assert!(ensure_canvas_revision(None, Some("new")).is_err());
        assert!(ensure_canvas_revision(Some("old"), None).is_err());
    }
}
