use tauri::State;

use crate::{
    app_state::AppState,
    events,
    managed_agents::persona_events::monotonic_created_at,
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
    expected_revision: Option<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;

    // Writer discipline (contract v3): sign `created_at = max(now, head + 1)`
    // so an accepted tagged write always sorts strictly ahead of the head it
    // asserts (`created_at DESC, id ASC`). Without this, a same-second or
    // behind-clock writer could satisfy the precondition yet lose the relay's
    // tiebreak, "succeeding" without changing the visible canvas. Only a real
    // head id has a timestamp to clear; `none`/absent asserts no prior head.
    let prior_head_created_at = match expected_revision.as_deref() {
        Some(rev) if rev.len() == 64 && rev.bytes().all(|b| b.is_ascii_hexdigit()) => {
            asserted_head_created_at(&state, &channel_id, rev).await?
        }
        _ => None,
    };

    let builder = events::build_set_canvas(uuid, &content, expected_revision.as_deref())?
        .custom_created_at(monotonic_created_at(prior_head_created_at));
    let result = submit_event(builder, &state).await?;

    Ok(serde_json::json!({
        "ok": true,
        "event_id": result.event_id,
    }))
}

/// `created_at` of the asserted head, or `None` if the relay no longer holds
/// that revision. An id-scoped query is immutable, so the answer cannot shift
/// under a concurrent write; a missing head lets the relay surface the
/// `conflict: canvas revision does not exist` reject on submit rather than
/// masking it with a stale floor here.
async fn asserted_head_created_at(
    state: &AppState,
    channel_id: &str,
    revision: &str,
) -> Result<Option<i64>, String> {
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [40100],
            "#h": [channel_id],
            "ids": [revision],
            "limit": 1
        })],
    )
    .await?;
    Ok(events
        .first()
        .map(|event| event.created_at.as_secs() as i64))
}

/// One page of a channel canvas's revision stream (kind:40100), newest first.
/// Each 40100 write is a regular signed event the relay retains, so the
/// standard query surface holds the complete history. The composite
/// `(until, before_id)` cursor mirrors the relay read order
/// (`created_at DESC, id ASC`) so paging never skips or repeats a revision when
/// several share the same second. `next_cursor` is present only when a full
/// page came back, i.e. older revisions may remain.
#[tauri::command]
pub async fn get_canvas_history(
    channel_id: String,
    limit: Option<usize>,
    until: Option<u64>,
    before_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    if before_id.is_some() && until.is_none() {
        return Err("before_id requires until".to_string());
    }
    // Bound the page size to the relay's read maximum. Beyond 1,000 the relay
    // silently clamps the returned rows, which would make `events.len() ==
    // page_size` false and null the cursor even when older revisions remain,
    // stranding them behind an unreachable page.
    let page_size = resolve_history_page_size(limit)?;

    let mut filter = serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id],
        "limit": page_size,
    });
    if let Some(value) = until {
        filter["until"] = serde_json::json!(value);
    }
    if let Some(ref value) = before_id {
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("before_id must be a 64-character hex event id".to_string());
        }
        filter["before_id"] = serde_json::json!(value);
    }

    let events = query_relay(&state, &[filter]).await?;

    let revisions: Vec<serde_json::Value> = events
        .iter()
        .map(|event| {
            serde_json::json!({
                "event_id": event.id.to_hex(),
                "content": event.content,
                "created_at": event.created_at.as_secs(),
                "author": event.pubkey.to_hex(),
            })
        })
        .collect();

    // A full page means the relay may hold older revisions; hand back the
    // last event as the cursor for the next "Load older" request. A short page
    // is the tail, so there is no next cursor.
    let next_cursor = if events.len() == page_size {
        events.last().map(|last| {
            serde_json::json!({
                "created_at": last.created_at.as_secs(),
                "event_id": last.id.to_hex(),
            })
        })
    } else {
        None
    };

    Ok(serde_json::json!({
        "revisions": revisions,
        "next_cursor": next_cursor,
    }))
}

/// Resolve and validate the history page size against the relay's read
/// maximum. Defaults to 100 when unset; a value outside `1..=1000` is rejected
/// so cursor generation is never based on a size the relay would silently
/// clamp (which strands older revisions behind a falsely-terminated page).
fn resolve_history_page_size(limit: Option<usize>) -> Result<usize, String> {
    let page_size = limit.unwrap_or(100);
    if !(1..=1000).contains(&page_size) {
        return Err("limit must be between 1 and 1000".to_string());
    }
    Ok(page_size)
}

#[cfg(test)]
mod tests {
    use super::resolve_history_page_size;

    #[test]
    fn defaults_to_100_when_unset() {
        assert_eq!(resolve_history_page_size(None).unwrap(), 100);
    }

    #[test]
    fn rejects_zero() {
        assert!(resolve_history_page_size(Some(0)).is_err());
    }

    #[test]
    fn accepts_relay_maximum() {
        assert_eq!(resolve_history_page_size(Some(1000)).unwrap(), 1000);
    }

    #[test]
    fn rejects_above_relay_maximum() {
        assert!(resolve_history_page_size(Some(1001)).is_err());
    }
}
