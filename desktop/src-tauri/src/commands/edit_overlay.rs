//! Latest-edit overlay for read paths that project stored events directly.
//!
//! A `kind:40003` edit carries the full replacement content for the event its
//! `e` tag points at. Any reader that renders stored content without applying
//! these shows the original forever — which is merely stale for a normal
//! message, but wrong for a surface card, whose entire update model is
//! full-spec replacement.
//!
//! `created_at` is second-precision, so two edits can tie. Ties break on the
//! lexicographically SMALLEST event id — not an arbitrary choice: the relay
//! orders `created_at DESC, id ASC`, so the winner under this rule is always
//! the first row a query returns. That makes a `limit: 1` per-target lookup
//! provably sufficient, and it is the same rule the channel timeline uses
//! (`formatTimelineMessages`), so every reader converges on one state.
//!
//! Edits must be fetched **by target id**, never by scanning a channel window:
//! an edit tags the event it replaces (`e` = that event's id), so a reply's
//! edit does not carry the thread root's id, and a busy channel can push the
//! relevant edit outside any window. Hence the two-phase read — fetch the
//! content page, then [`fetch_latest_edits`] for exactly the ids it returned.

use std::collections::HashMap;

use buzz_core_pkg::kind::KIND_STREAM_MESSAGE_EDIT;
use nostr::Event;

use crate::app_state::AppState;
use crate::models::FeedItemInfo;
use crate::relay::query_relay;

/// Filters sent per query. One filter per target keeps a heavily-edited event
/// from crowding out other targets — a single OR-ed `#e` filter shares one
/// row budget (the relay caps a filter at 1000 rows), so one noisy card could
/// hide every other card's edit.
const EDIT_FILTERS_PER_QUERY: usize = 25;

/// Rows per target. One is enough: the relay orders `created_at DESC, id ASC`
/// and the tie-break picks the smallest id, so the first row IS the winner.
const EDIT_ROWS_PER_TARGET: usize = 1;

/// Fetch the latest edit content for exactly `target_ids`.
///
/// Returns `target event id -> replacement content`. Failures are non-fatal:
/// a read that cannot reach the relay renders original content rather than
/// failing outright, which is the same degradation the callers already have.
pub(crate) async fn fetch_latest_edits(
    state: &AppState,
    target_ids: &[String],
    channel_id: Option<&str>,
) -> HashMap<String, String> {
    let mut overlay: HashMap<String, String> = HashMap::new();
    for chunk in target_ids.chunks(EDIT_FILTERS_PER_QUERY) {
        let filters: Vec<serde_json::Value> = chunk
            .iter()
            .map(|id| {
                let mut filter = serde_json::json!({
                    "kinds": [KIND_STREAM_MESSAGE_EDIT],
                    "#e": [id],
                    "limit": EDIT_ROWS_PER_TARGET,
                });
                if let Some(channel_id) = channel_id {
                    filter["#h"] = serde_json::json!([channel_id]);
                }
                filter
            })
            .collect();
        match query_relay(state, &filters).await {
            Ok(events) => overlay.extend(latest_edit_by_target(&events)),
            Err(error) => {
                tracing::warn!(
                    "edit overlay fetch failed ({} targets): {error}",
                    chunk.len()
                );
            }
        }
    }
    overlay
}

/// Overlay current content onto feed items, looked up by their exact ids.
///
/// Feed rows render the event's content — a surface card's `fallbackText` —
/// so without this an updated card keeps its original summary in the Inbox
/// while the detail view shows the new one.
pub(crate) async fn apply_to_feed_items(
    state: &AppState,
    events: &[Event],
    items: &mut [FeedItemInfo],
) {
    let ids: Vec<String> = events.iter().map(|ev| ev.id.to_hex()).collect();
    let overlay = fetch_latest_edits(state, &ids, None).await;
    if overlay.is_empty() {
        return;
    }
    for item in items.iter_mut() {
        if let Some(content) = overlay.get(&item.id) {
            item.content = content.clone();
        }
    }
}

/// Map of `target event id -> replacement content`, keeping the latest edit
/// per target under `(created_at, id)` ordering.
pub(crate) fn latest_edit_by_target(events: &[Event]) -> HashMap<String, String> {
    // (created_at, edit id, content) per target, so ties resolve deterministically.
    let mut best: HashMap<String, (u64, String, String)> = HashMap::new();

    for event in events {
        if u32::from(event.kind.as_u16()) != KIND_STREAM_MESSAGE_EDIT {
            continue;
        }
        let Some(target) = event.tags.iter().find_map(|tag| {
            let parts = tag.as_slice();
            (parts.len() >= 2 && parts[0].as_str() == "e").then(|| parts[1].to_string())
        }) else {
            continue;
        };

        let created_at = event.created_at.as_secs();
        let id = event.id.to_hex();
        let wins = best.get(&target).is_none_or(|(at, existing_id, _)| {
            created_at > *at || (created_at == *at && id < *existing_id)
        });
        if wins {
            best.insert(target, (created_at, id, event.content.clone()));
        }
    }

    best.into_iter()
        .map(|(target, (_, _, content))| (target, content))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn edit(keys: &Keys, target: &str, content: &str, created_at: u64) -> Event {
        EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE_EDIT as u16), content)
            .tags([Tag::parse(["e", target]).expect("tag")])
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(keys)
            .expect("sign")
    }

    #[test]
    fn latest_edit_wins_by_timestamp() {
        let keys = Keys::generate();
        let target = "a".repeat(64);
        let events = vec![
            edit(&keys, &target, "first", 1_700_000_000),
            edit(&keys, &target, "second", 1_700_000_010),
        ];
        let overlay = latest_edit_by_target(&events);
        assert_eq!(overlay.get(&target).map(String::as_str), Some("second"));
    }

    #[test]
    fn same_second_edits_break_the_tie_on_event_id() {
        let keys = Keys::generate();
        let target = "b".repeat(64);
        let a = edit(&keys, &target, "one", 1_700_000_000);
        let b = edit(&keys, &target, "two", 1_700_000_000);
        // Smallest id wins — see the module docs for why that is the rule.
        let expected = if a.id.to_hex() < b.id.to_hex() {
            "one"
        } else {
            "two"
        };

        // Order of arrival must not change the outcome.
        for events in [vec![a.clone(), b.clone()], vec![b, a]] {
            let overlay = latest_edit_by_target(&events);
            assert_eq!(overlay.get(&target).map(String::as_str), Some(expected));
        }
    }

    #[test]
    fn overlay_is_addressed_per_target_not_per_thread() {
        // A reply's edit tags the REPLY, never the thread root. A reader that
        // looked edits up by root id (or by scanning a channel window) would
        // leave an edited reply — a surface card posted as a reply — stale.
        let keys = Keys::generate();
        let root = "1".repeat(64);
        let reply = "2".repeat(64);
        let events = vec![
            edit(&keys, &root, "root updated", 1_700_000_000),
            edit(&keys, &reply, "reply updated", 1_700_000_000),
        ];
        let overlay = latest_edit_by_target(&events);
        assert_eq!(overlay.get(&root).map(String::as_str), Some("root updated"));
        assert_eq!(
            overlay.get(&reply).map(String::as_str),
            Some("reply updated"),
            "an edit addressed to a reply must resolve to that reply"
        );
    }

    #[test]
    fn non_edit_events_and_untargeted_edits_are_ignored() {
        let keys = Keys::generate();
        let message = EventBuilder::new(Kind::Custom(9), "hello")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        let untargeted = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE_EDIT as u16), "orphan")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        assert!(latest_edit_by_target(&[message, untargeted]).is_empty());
    }
}
