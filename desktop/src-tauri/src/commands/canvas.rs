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
    let events = query_relay(&state, &[get_canvas_filter(&channel_id)]).await?;

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

    // Advisory optimistic-concurrency check (client-side, two-stage). A
    // conflict-checked save asserts the revision the editor loaded. Stage one:
    // read the live head once before publishing and compare locally, returning
    // a frozen pre-write conflict marker if it already moved — this catches the
    // realistic stale-edit case (head moved minutes ago). Stage two, after
    // publishing (below): re-read the head once and confirm our write is (or is
    // built upon by) the visible head, surfacing a distinct post-write
    // supersession marker otherwise. Detection is bounded to a competitor
    // visible at check time; preventing the race entirely — a competitor that
    // lands between our read and write, or after the post-write read — needs
    // relay-side linearization (phase 2).
    //
    // `head` is `None` when the channel has no canvas yet. A matched head's
    // `created_at` is the floor for writer discipline: an accepted save stamps
    // `created_at = max(now, head + 1)` via the SDK's `canvas_write_created_at`
    // — the one home for canvas timestamp discipline — so it sorts strictly
    // ahead of the head it read under `created_at DESC, id ASC`. That helper
    // also refuses a head timestamped far in the future, so a poisoned timeline
    // fails loudly here rather than being silently extended. The no-head /
    // unconditional-append case has no floor and keeps the default `now`.
    let head = current_canvas_head(&state, &channel_id).await?;
    let prior_head_created_at = check_canvas_precondition(expected_revision.as_deref(), head)?;

    let mut builder = events::build_set_canvas(uuid, &content, expected_revision.as_deref())?;
    if let Some(floor) = prior_head_created_at {
        builder = builder.custom_created_at(nostr::Timestamp::from(
            buzz_sdk_pkg::canvas_write_created_at(floor as u64).map_err(|e| e.to_string())?,
        ));
    }
    let result = submit_event(builder, &state).await?;

    // Post-write supersession detection (only for conflict-checked writes). The
    // precondition above closes the stale-edit case; this closes the narrower
    // window where a concurrent write we could not see at precondition time has
    // become visible by now. An unconditional append (`None`) has nothing to
    // assert, so it stays fire-and-forget.
    //
    // The submit above was accepted, so the write is durable. `classify_post_write`
    // maps the ancestry read to a report: a failed read is durable-but-unverified
    // (`verified: false`), not a failed save; a stranger head is a supersession
    // (frozen conflict marker); our head or a descendant is verified success.
    let mut verified = true;
    if expected_revision.is_some() {
        let ancestry = current_canvas_head_ancestry(&state, &channel_id).await;
        verified = classify_post_write(&result.event_id, ancestry)?;
    }

    Ok(serde_json::json!({
        "ok": true,
        "event_id": result.event_id,
        "verified": verified,
    }))
}

/// Classify a conflict-checked write's post-submit outcome from the ancestry
/// read (a recent slice of the canvas revision stream, newest first, each
/// entry `(event_id, the expected-revision it built on)`, or an empty slice for
/// no canvas). The submit was already accepted, so the write is durable — this
/// only decides how to report it:
///
/// - read error → `Ok(false)`: accepted but unverified. A failed verification
///   read must never masquerade as a failed save; the caller reports success
///   with `verified: false`.
/// - our event is the head, or reachable through the head's ancestry chain →
///   `Ok(true)`.
/// - any other head → `Err(CANVAS_SUPERSEDED)`: a concurrent write won the
///   visible head; our revision is preserved in history, not lost.
///
/// This cannot close the residual race where a competitor lands *after* this
/// read — that needs relay linearization (phase 2).
fn classify_post_write(
    our_id: &str,
    ancestry: Result<Vec<(String, Option<String>)>, String>,
) -> Result<bool, String> {
    match ancestry {
        Err(_) => Ok(false),
        Ok(revisions) => {
            if buzz_sdk_pkg::canvas_write_survived(our_id, &revisions) {
                Ok(true)
            } else {
                Err(CANVAS_SUPERSEDED.to_string())
            }
        }
    }
}

/// Frozen conflict markers the desktop TypeScript layer (`canvasConflict.ts`)
/// matches to render the "canvas changed — reload" state. The advisory check
/// in [`set_canvas`] produces these directly; keep them byte-identical to the
/// `CANVAS_CONFLICT_MARKERS` list on the TS side.
const CANVAS_CHANGED: &str = "conflict: canvas changed since it was loaded";
const CANVAS_REVISION_MISSING: &str = "conflict: canvas revision does not exist";
/// Post-write marker: the save published successfully but a concurrent write is
/// now the visible head. The user's revision is **not** lost — it is preserved
/// in history — so the TS surface renders a distinct "reload, then restore it
/// if needed" message rather than the pre-write "reapply your edit" message.
/// Keep byte-identical to `CANVAS_SUPERSEDED_MARKER` on the TS side.
const CANVAS_SUPERSEDED: &str = "conflict: canvas save was superseded by a concurrent write";

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

/// Build the filter for [`get_canvas`] (display read).
///
/// Display reads must NOT carry `"consistency"` — they are replica-eligible
/// reads that do not gate writes. Exposed for unit tests so asserting the
/// field is absent is causal: adding `"consistency"` to this function turns
/// the inverse-guard test red.
fn get_canvas_filter(channel_id: &str) -> serde_json::Value {
    serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id],
        "limit": 1,
    })
}

/// Build the base filter for [`get_canvas_history`] (display read).
///
/// Like [`get_canvas_filter`], this must NOT carry `"consistency"` — history
/// pagination is a display read, never write-gating. Exposed for unit tests
/// for the same causal reason. The caller layers optional `until`/`before_id`
/// pagination fields on top.
fn get_canvas_history_base_filter(channel_id: &str, page_size: usize) -> serde_json::Value {
    serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id],
        "limit": page_size,
    })
}

/// Build the filter for [`current_canvas_head`].
///
/// The filter carries `"consistency": "strong"` because this read gates a
/// write (the save's precondition); it must never route to a lagging replica.
/// Exposed for unit tests so asserting the field is present/absent is causal —
/// removing the field from the returned JSON makes the test red, not just a
/// stale copy.
fn canvas_head_filter(channel_id: &str) -> serde_json::Value {
    serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id],
        "limit": 1,
        // Read-your-writes: this head read gates a write (the save's
        // precondition), so it must never route to a lagging replica.
        "consistency": "strong",
    })
}

/// Build the filter for [`current_canvas_head_ancestry`].
///
/// Like [`canvas_head_filter`], carries `"consistency": "strong"` because
/// this post-write verification read must observe the caller's own just-accepted
/// save. Exposed for unit tests for the same mutation-killable reason.
fn canvas_ancestry_filter(channel_id: &str) -> serde_json::Value {
    serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id],
        "limit": buzz_sdk_pkg::CANVAS_ANCESTRY_WALK_MAX,
        // Read-your-writes: this post-write verification read must observe
        // the caller's own just-accepted save, so it pins to the writer.
        "consistency": "strong",
    })
}

/// Read the live canvas head as `(event_id, created_at)`, or `None` when the
/// channel has no canvas yet. The relay orders `created_at DESC, id ASC`, so a
/// `limit: 1` query returns exactly the head every surface agrees on.
async fn current_canvas_head(
    state: &AppState,
    channel_id: &str,
) -> Result<Option<(String, i64)>, String> {
    let events = query_relay(state, &[canvas_head_filter(channel_id)]).await?;
    Ok(events
        .first()
        .map(|event| (event.id.to_hex(), event.created_at.as_secs() as i64)))
}

/// Read a recent slice of the canvas revision stream as `(event_id,
/// expected-revision tag)` pairs, newest first, for the post-write supersession
/// check. The head is the first element; the rest let `canvas_write_survived`
/// walk `expected-revision` links back through a descendant chain (A→B→C) so a
/// legitimate later write layered on ours is not misread as a supersession. The
/// second element of each pair is that revision's own `["expected-revision", …]`
/// tag value (the id it built on), or `None` when absent. An empty vec means the
/// channel has no canvas.
async fn current_canvas_head_ancestry(
    state: &AppState,
    channel_id: &str,
) -> Result<Vec<(String, Option<String>)>, String> {
    let events = query_relay(state, &[canvas_ancestry_filter(channel_id)]).await?;
    Ok(events
        .iter()
        .map(|event| {
            let expected_revision = event
                .tags
                .iter()
                .find(|t| {
                    t.as_slice()
                        .first()
                        .is_some_and(|k| k == "expected-revision")
                })
                .and_then(|t| t.as_slice().get(1).cloned());
            (event.id.to_hex(), expected_revision)
        })
        .collect())
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

    let mut filter = get_canvas_history_base_filter(&channel_id, page_size);
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
    use super::{check_canvas_precondition, classify_post_write, resolve_history_page_size};

    const HEAD_ID: &str = "aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44";
    const OUR_ID: &str = "bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55";
    const STRANGER_ID: &str = "cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66";
    const THIRD_ID: &str = "dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11";

    #[test]
    fn post_write_read_error_is_durable_but_unverified() {
        // The submit was accepted; a failed verification read is durable
        // success with verified=false, never a failed save.
        assert_eq!(
            classify_post_write(OUR_ID, Err("relay unreachable".to_string())),
            Ok(false)
        );
    }

    #[test]
    fn post_write_our_head_or_descendant_is_verified() {
        // Our own event is the head → verified.
        assert_eq!(
            classify_post_write(OUR_ID, Ok(vec![(OUR_ID.to_string(), None)])),
            Ok(true)
        );
        // A later write directly built on ours (its expected-revision names us)
        // → verified.
        assert_eq!(
            classify_post_write(
                OUR_ID,
                Ok(vec![(STRANGER_ID.to_string(), Some(OUR_ID.to_string()))])
            ),
            Ok(true)
        );
        // Transitive descendant A(ours) → B(exp=A) → C(exp=B, head): ours is
        // reached by walking the head's ancestry, so it is not a supersession.
        assert_eq!(
            classify_post_write(
                OUR_ID,
                Ok(vec![
                    (THIRD_ID.to_string(), Some(STRANGER_ID.to_string())),
                    (STRANGER_ID.to_string(), Some(OUR_ID.to_string())),
                    (OUR_ID.to_string(), None),
                ])
            ),
            Ok(true)
        );
    }

    #[test]
    fn post_write_stranger_head_is_superseded() {
        // A stranger won the visible head with no ancestry to ours → frozen
        // supersession marker (the read succeeded, so this is detection, not an
        // unverified read).
        assert_eq!(
            classify_post_write(
                OUR_ID,
                Ok(vec![(STRANGER_ID.to_string(), Some(HEAD_ID.to_string()))])
            ),
            Err(super::CANVAS_SUPERSEDED.to_string())
        );
        // No head at all is likewise a supersession, not verified.
        assert_eq!(
            classify_post_write(OUR_ID, Ok(vec![])),
            Err(super::CANVAS_SUPERSEDED.to_string())
        );
    }

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

    // ── Writer-pin contract: filter-construction assertions ──────────────────
    //
    // `canvas_head_filter` and `canvas_ancestry_filter` build the JSON that
    // `current_canvas_head` and `current_canvas_head_ancestry` pass to
    // `query_relay`. `get_canvas_filter` and `get_canvas_history_base_filter`
    // build the corresponding display filters. These tests assert the
    // `"consistency"` field is present/absent in the produced filter, and that
    // each command delegates to its helper (the helpers ARE the production code
    // path — there is no copy).
    //
    // Mutation oracle:
    //   * Removing `"consistency"` from `canvas_head_filter` → the
    //     `head_filter_carries_strong_consistency` test fails.
    //   * Removing `"consistency"` from `canvas_ancestry_filter` → the
    //     `ancestry_filter_carries_strong_consistency` test fails.
    //   * Adding `"consistency"` to `get_canvas_filter` → the
    //     `get_canvas_filter_does_not_carry_consistency` test fails.
    //   * Adding `"consistency"` to `get_canvas_history_base_filter` → the
    //     `get_canvas_history_base_filter_does_not_carry_consistency` test fails.

    #[test]
    fn head_filter_carries_strong_consistency() {
        let f = super::canvas_head_filter("326d56bc-c96c-4af0-86a1-5e804cd1b467");
        assert_eq!(
            f.get("consistency").and_then(|v| v.as_str()),
            Some("strong"),
            "current_canvas_head filter must carry consistency=strong: {f}"
        );
        // Structural sanity: correct kind and limit.
        assert_eq!(
            f["kinds"],
            serde_json::json!([40100]),
            "head filter must query kind 40100"
        );
        assert_eq!(
            f.get("limit").and_then(|v| v.as_u64()),
            Some(1),
            "head filter must have limit=1"
        );
    }

    #[test]
    fn ancestry_filter_carries_strong_consistency() {
        let f = super::canvas_ancestry_filter("326d56bc-c96c-4af0-86a1-5e804cd1b467");
        assert_eq!(
            f.get("consistency").and_then(|v| v.as_str()),
            Some("strong"),
            "current_canvas_head_ancestry filter must carry consistency=strong: {f}"
        );
        // Structural sanity: correct kind and limit > 1.
        assert_eq!(
            f["kinds"],
            serde_json::json!([40100]),
            "ancestry filter must query kind 40100"
        );
        assert!(
            f.get("limit").and_then(|v| v.as_u64()).unwrap_or(0) > 1,
            "ancestry filter limit must be > 1 (walk depth): {f}"
        );
    }

    /// `get_canvas` and `get_canvas_history` are display reads: they must NOT
    /// carry `"consistency"`. These inverse guards call the actual production
    /// filter builders — adding `"consistency"` to either builder turns the
    /// relevant test red immediately (unlike a copied literal, which would
    /// silently stay green while production drifted).
    #[test]
    fn get_canvas_filter_does_not_carry_consistency() {
        let f = super::get_canvas_filter("326d56bc-c96c-4af0-86a1-5e804cd1b467");
        assert!(
            f.get("consistency").is_none(),
            "display (get_canvas) filter must NOT carry consistency: {f}"
        );
        // Structural sanity.
        assert_eq!(f["kinds"], serde_json::json!([40100]));
        assert_eq!(f.get("limit").and_then(|v| v.as_u64()), Some(1));
    }

    #[test]
    fn get_canvas_history_base_filter_does_not_carry_consistency() {
        let f = super::get_canvas_history_base_filter("326d56bc-c96c-4af0-86a1-5e804cd1b467", 50);
        assert!(
            f.get("consistency").is_none(),
            "display (get_canvas_history) filter must NOT carry consistency: {f}"
        );
        // Structural sanity.
        assert_eq!(f["kinds"], serde_json::json!([40100]));
        assert_eq!(f.get("limit").and_then(|v| v.as_u64()), Some(50));
    }
}
