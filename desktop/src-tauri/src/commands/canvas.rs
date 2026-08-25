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

    // Advisory optimistic-concurrency check (client-side). A conflict-checked
    // save asserts the revision the editor loaded; we read the live head once
    // and compare locally, returning one of the frozen conflict markers so
    // `canvasConflict.ts` renders the reload state. This catches the realistic
    // stale-edit case (head moved minutes ago). It cannot close the
    // millisecond race between this read and the write — that would need relay
    // enforcement — but the day-to-day protection and conflict UX are intact.
    //
    // `head` is `None` when the channel has no canvas yet. The head's
    // `created_at` doubles as the floor for writer discipline below: an
    // accepted save signs `created_at = max(now, head + 1)` so it sorts
    // strictly ahead of the head it read under `created_at DESC, id ASC`.
    let head = current_canvas_head(&state, &channel_id).await?;
    let prior_head_created_at = check_canvas_precondition(expected_revision.as_deref(), head)?;

    let builder = events::build_set_canvas(uuid, &content, expected_revision.as_deref())?
        .custom_created_at(monotonic_created_at(prior_head_created_at));
    let result = submit_event(builder, &state).await?;

    Ok(serde_json::json!({
        "ok": true,
        "event_id": result.event_id,
    }))
}

/// Frozen conflict markers the desktop TypeScript layer (`canvasConflict.ts`)
/// matches to render the "canvas changed — reload" state. The advisory check
/// in [`set_canvas`] produces these directly; keep them byte-identical to the
/// `CANVAS_CONFLICT_MARKERS` list on the TS side.
const CANVAS_CHANGED: &str = "conflict: canvas changed since it was loaded";
const CANVAS_REVISION_MISSING: &str = "conflict: canvas revision does not exist";

/// Pure advisory precondition: compare the revision the editor asserts against
/// the live `head` (`(event_id, created_at)` or `None` when no canvas exists),
/// returning the head `created_at` floor for writer discipline on success or a
/// frozen conflict marker on mismatch.
///
/// - `None` asserts nothing (unconditional append) — no floor.
/// - `Some("none")` asserts no canvas yet — a present head is a conflict.
/// - `Some(id)` asserts that head — a missing head is `revision does not
///   exist`, a different head is `changed since it was loaded`, a match returns
///   its `created_at` as the floor.
fn check_canvas_precondition(
    expected_revision: Option<&str>,
    head: Option<(String, i64)>,
) -> Result<Option<i64>, String> {
    match expected_revision {
        None => Ok(None),
        Some("none") => {
            if head.is_some() {
                Err(CANVAS_CHANGED.to_string())
            } else {
                Ok(None)
            }
        }
        Some(revision) => match head {
            None => Err(CANVAS_REVISION_MISSING.to_string()),
            Some((head_id, _)) if !head_id.eq_ignore_ascii_case(revision) => {
                Err(CANVAS_CHANGED.to_string())
            }
            Some((_, created_at)) => Ok(Some(created_at)),
        },
    }
}

/// Read the live canvas head as `(event_id, created_at)`, or `None` when the
/// channel has no canvas yet. The relay orders `created_at DESC, id ASC`, so a
/// `limit: 1` query returns exactly the head every surface agrees on.
async fn current_canvas_head(
    state: &AppState,
    channel_id: &str,
) -> Result<Option<(String, i64)>, String> {
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [40100],
            "#h": [channel_id],
            "limit": 1
        })],
    )
    .await?;
    Ok(events
        .first()
        .map(|event| (event.id.to_hex(), event.created_at.as_secs() as i64)))
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
    use super::{check_canvas_precondition, resolve_history_page_size};

    const HEAD_ID: &str = "aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44";

    #[test]
    fn precondition_none_assertion_is_unconditional_append() {
        // No asserted revision: append regardless of head, no floor.
        assert_eq!(check_canvas_precondition(None, None), Ok(None));
        assert_eq!(
            check_canvas_precondition(None, Some((HEAD_ID.to_string(), 100))),
            Ok(None)
        );
    }

    #[test]
    fn precondition_expect_none_conflicts_when_a_head_exists() {
        // First-creation race: expected no canvas but one now exists.
        assert_eq!(check_canvas_precondition(Some("none"), None), Ok(None));
        assert_eq!(
            check_canvas_precondition(Some("none"), Some((HEAD_ID.to_string(), 100))),
            Err(super::CANVAS_CHANGED.to_string())
        );
    }

    #[test]
    fn precondition_expect_head_returns_floor_or_conflict() {
        // Matching head returns its created_at as the writer-discipline floor.
        assert_eq!(
            check_canvas_precondition(Some(HEAD_ID), Some((HEAD_ID.to_string(), 100))),
            Ok(Some(100))
        );
        // Case-insensitive id match still resolves.
        assert_eq!(
            check_canvas_precondition(
                Some(&HEAD_ID.to_uppercase()),
                Some((HEAD_ID.to_string(), 100))
            ),
            Ok(Some(100))
        );
        // Head moved to a different revision since load.
        assert_eq!(
            check_canvas_precondition(Some(HEAD_ID), Some(("ff".repeat(32), 100))),
            Err(super::CANVAS_CHANGED.to_string())
        );
        // Asserted a head but the canvas no longer has one.
        assert_eq!(
            check_canvas_precondition(Some(HEAD_ID), None),
            Err(super::CANVAS_REVISION_MISSING.to_string())
        );
    }

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
