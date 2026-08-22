//! Home-feed assembly (`get_feed`).
//!
//! Split out of `messages.rs`: the mention feed is a raw `#p` query, and
//! separating a reply's addressing `p` tag from a real mention needs its own
//! parent lookup, which does not belong in the general message command file.

use tauri::State;

use crate::app_state::AppState;
use crate::models::{FeedItemInfo, FeedMeta, FeedResponse, FeedSections};

use super::messages::forum::{
    apply_link_preview_suppression, fetch_agent_owner_pubkeys, link_preview_suppression_targets,
};
use super::messages::{feed_item_from_event, reply_parent_id};
use crate::relay::query_relay;

/// Kinds the live thread-reply path can take ownership of when this feed marks
/// an item `reply_to_self`.
///
/// Mirrors `CHANNEL_MESSAGE_EVENT_KINDS` (the desktop unread-trigger set) in
/// `desktop/src/shared/constants/kinds.ts`. Deliberately narrower than the
/// mention query above, which also returns kind:1 and git events — the live
/// path never sees those, so handing one over would lose it entirely.
const LIVE_OWNED_REPLY_KINDS: [u16; 4] = [9, 40002, 45001, 45003];

/// How much wider than the caller's cap the `#p` mention query runs.
///
/// Replies-to-you share the `#p` window with real mentions and are filtered out
/// only after the query, so the window has to hold both. 4x covers a thread
/// answering the user three times for every mention it delivers.
const MENTION_OVERFETCH_FACTOR: u64 = 4;

/// Ceiling on the over-fetched mention window.
///
/// Not the relay's page clamp (`DEFAULT_MAX_PAGE_LIMIT`, 1000) — the returned
/// ids flow straight into two unchunked follow-up filters, the `#e` edits query
/// and the reply-parent `ids` query, and `buzz-db`'s batch fetch documents a
/// caller-bounded batch of 500. Keeping the window at 200 leaves both well
/// inside that while still holding four times the default page of mentions.
const MENTION_OVERFETCH_CEILING: u64 = 200;

/// Trims an over-fetched mention list to `cap`, giving real mentions the slots
/// before replies-to-self get any.
///
/// Both kinds of item share one `#p` window, so a plain newest-first truncation
/// lets a chatty thread push every real `@you` out of the response. Order within
/// the result stays newest-first, as callers expect.
fn trim_mentions_preferring_real(mut items: Vec<FeedItemInfo>, cap: usize) -> Vec<FeedItemInfo> {
    if items.len() <= cap {
        return items;
    }
    // Sorted before selecting, not only before returning. Both the truncation
    // and the reply fill-in take from the front of their half, and `partition`
    // preserves input order — so on an input that is not already newest-first
    // this dropped the newer item and kept the older one, then sorted the wrong
    // survivors into the right order. The relay does answer `created_at DESC`
    // today, which is the only reason it looked correct; nothing here should
    // depend on that.
    items.sort_by_key(|item| std::cmp::Reverse(item.created_at));
    let (mut kept, replies): (Vec<FeedItemInfo>, Vec<FeedItemInfo>) =
        items.into_iter().partition(|item| !item.reply_to_self);
    kept.truncate(cap);
    let room = cap.saturating_sub(kept.len());
    kept.extend(replies.into_iter().take(room));
    kept.sort_by_key(|item| std::cmp::Reverse(item.created_at));
    kept
}

use crate::p_tag_role::{p_tag_role_for_event, PTagRole};

/// Whether a string is a well-formed 64-char hex event id, and so safe to put in
/// an `ids` filter.
fn is_event_id_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Mirrors `isBroadcastReply` in `desktop/src/features/messages/lib/threading.ts`.
fn is_broadcast_reply(ev: &nostr::Event) -> bool {
    ev.tags.iter().any(|tag| {
        let s = tag.as_slice();
        s.len() >= 2 && s[0] == "broadcast" && s[1] == "1"
    })
}

#[tauri::command]
pub async fn get_feed(
    since: Option<i64>,
    limit: Option<u32>,
    types: Option<String>,
    state: State<'_, AppState>,
) -> Result<FeedResponse, String> {
    let cap = limit.unwrap_or(50).min(100);

    // Parse types filter — if absent, run all sub-queries.
    // Comma-separated: e.g. "mentions,needs_action".
    let want_mentions = types
        .as_deref()
        .map(|t| t.split(',').any(|s| s.trim() == "mentions"))
        .unwrap_or(true);
    let want_needs_action = types
        .as_deref()
        .map(|t| t.split(',').any(|s| s.trim() == "needs_action"))
        .unwrap_or(true);

    let my_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    // Mentions: messages that reference me via #p.
    let mut mention_filter = serde_json::json!({
        "kinds": [
            9,
            40002,
            1,
            45001,
            45003,
            buzz_core_pkg::kind::KIND_GIT_PULL_REQUEST,
            buzz_core_pkg::kind::KIND_GIT_PR_UPDATE,
            buzz_core_pkg::kind::KIND_GIT_ISSUE,
            buzz_core_pkg::kind::KIND_GIT_STATUS_OPEN,
            buzz_core_pkg::kind::KIND_GIT_STATUS_MERGED,
            buzz_core_pkg::kind::KIND_GIT_STATUS_CLOSED,
            buzz_core_pkg::kind::KIND_GIT_STATUS_DRAFT,
        ],
        "#p": [my_pubkey],
        // Over-fetch. A reply carries a `p` tag naming the author it answers, so
        // this `#p` query now returns replies-to-you as well as real mentions,
        // and the replies are only discarded *after* the query by `reply_to_self`
        // filtering. At `limit: cap` a thread with `cap` recent replies to your
        // own messages evicts every real mention from the window, so an `@you`
        // from earlier the same day never reaches the Inbox, the badge, or the
        // mention toast — and the live path cannot compensate, because it only
        // sees events published while the app was connected.
        //
        "limit": (cap as u64 * MENTION_OVERFETCH_FACTOR).min(MENTION_OVERFETCH_CEILING),
    });
    if let Some(s) = since {
        mention_filter["since"] = serde_json::json!(s);
    }
    // Needs-action: workflow approval-request events sent to me.
    let mut approval_filter = serde_json::json!({
        "kinds": [46010, 46011, 46012],
        "#p": [my_pubkey],
        "limit": 20,
    });
    if let Some(s) = since {
        approval_filter["since"] = serde_json::json!(s);
    }

    let mention_events = if want_mentions {
        query_relay(&state, &[mention_filter])
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let approval_events = if want_needs_action {
        query_relay(&state, &[approval_filter])
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let mention_ids = mention_events
        .iter()
        .map(|event| event.id.to_hex())
        .collect::<Vec<_>>();
    let mention_edits = if mention_ids.is_empty() {
        Vec::new()
    } else {
        query_relay(
            &state,
            &[serde_json::json!({ "kinds": [40003], "#e": mention_ids })],
        )
        .await
        .unwrap_or_default()
    };
    // A reply p-tags the author it answers, so the `#p` mention query above
    // also returns every answer to one of the user's own messages. Resolve the
    // parents so the client can tell "you were mentioned" from "someone
    // replied to you" — they differ on whether a channel mute applies.
    // Parents already in this batch are answered from it rather than skipped:
    // a message can both p-tag the user and be authored by them, and treating
    // that as "not self-authored" would leave the reply in this feed while the
    // live path also claims it — two toasts for one event.
    let mut self_authored_parent_ids: std::collections::HashSet<String> = mention_events
        .iter()
        .filter(|ev| ev.pubkey.to_hex() == my_pubkey)
        .map(|ev| ev.id.to_hex())
        .collect();
    let reply_parent_ids: Vec<String> = mention_events
        .iter()
        .filter(|ev| !is_broadcast_reply(ev))
        // The round trip the markers exist to remove. A sender that told us
        // which role its `p` tag plays has already answered the only question
        // this query asks.
        .filter(|ev| p_tag_role_for_event(ev, &my_pubkey) == PTagRole::Unknown)
        .filter_map(reply_parent_id)
        // `reply_parent_id` lowercases the tag value but does not validate it,
        // and the relay *accepts* an event whose `e` value is not hex at all — its
        // thread-meta resolver silently ignores such a tag rather than rejecting
        // the event. Passing one into an `ids` filter makes the relay reject the
        // whole filter as malformed, so a single junk event in the mention window
        // would take out every later query built from it.
        .filter(|id| is_event_id_hex(id))
        .filter(|id| !mention_ids.contains(id))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let queried_parent_ids: std::collections::HashSet<String> = if reply_parent_ids.is_empty() {
        std::collections::HashSet::new()
    } else {
        query_relay(
            &state,
            &[serde_json::json!({
                "ids": reply_parent_ids,
                // Keep in sync with REPLY_PARENT_EVENT_KINDS in
                // desktop/src/shared/constants/kinds.ts. The live desktop path
                // answers the same question from its own lookup, so a kind
                // missing here but present there makes both paths claim the
                // event and notify twice.
                // 40001 is a legacy pre-migration stream message. It is still a
                // repliable parent, and omitting it made every reply to one
                // resolve as "parent absent" — which reads as a real mention and
                // pierces the mute the lookup exists to protect.
                "kinds": [9, 40001, 40002, 40008, 1, 45001, 45003],
                "authors": [my_pubkey],
                "limit": reply_parent_ids.len(),
            })],
        )
        .await
        // Deliberately not `unwrap_or_default()` like the queries above. Their
        // failure only shortens the feed, which is harmless. This one's failure
        // *flips a classification*: an empty result is indistinguishable from
        // "none of these parents are mine", so every reply in the batch would be
        // relabelled a real mention. The frontend fails open on the documented
        // assumption that this feed already dropped what it resolved to us, so
        // that relabelling would double-notify in an unmuted channel and pierce
        // the mute in a muted one.
        //
        // Failing the poll is the right response, and is safe *because* the ids
        // above are validated: a malformed id used to make the relay reject the
        // filter outright, which — with no `since` on the mention query — failed
        // every later poll forever. What is left is transient, and React Query
        // keeps the previous `data` on error, so the `feed` reference does not
        // change, the notification effect does not re-run, no id is consumed from
        // the seen set, and the next good poll delivers normally.
        //
        // Do not "degrade" by guessing here. Marking the batch `reply_to_self`
        // hands it to the live path, but `collectHomeAlertItems` still adds every
        // declined item to the persisted seen set — so the guess consumes the
        // notification slot and the next poll drops the item as already-seen. A
        // genuine typed `@mention` inside a thread is silently lost for good,
        // across restarts, because its parent belongs to a third party and so
        // looks unresolved. Events in channels absent from the local list are not
        // deferrable at all: the live path returns early for those.
        .map_err(|e| {
            format!("could not resolve reply parents, so this feed poll cannot tell a mention from a reply: {e}")
        })?
        .iter()
        .map(|event| event.id.to_hex())
        .collect()
    };
    self_authored_parent_ids.extend(queried_parent_ids);
    let mention_owner_pubkeys = fetch_agent_owner_pubkeys(&state, &mention_events).await;
    let suppressed_mentions =
        link_preview_suppression_targets(&mention_events, &mention_edits, &mention_owner_pubkeys);
    let mentions: Vec<FeedItemInfo> = mention_events
        .iter()
        .map(|ev| {
            // Canonical singular category, matching `FeedItemCategory` on the TS
            // side. The plural spelling here silently disabled every
            // `category === "mention"` check downstream.
            let mut item = feed_item_from_event(ev, "mention");
            // `reply_to_self` hands the event to the live thread-reply path,
            // which never sees a broadcast reply — `isThreadReply` excludes
            // them by design. Marking one here would drop it from this feed
            // with nothing on the other side to pick it up.
            //
            // Restricted to the kinds the live path can actually own. The
            // mention query is wider than its unread-trigger set, so marking a
            // bridged kind:1 note or a git event would drop it here with
            // nothing on the other side to pick it up.
            item.reply_to_self = LIVE_OWNED_REPLY_KINDS.contains(&ev.kind.as_u16())
                && !is_broadcast_reply(ev)
                && match p_tag_role_for_event(ev, &my_pubkey) {
                    // The sender said so, and it knew without asking anyone.
                    PTagRole::Addressing => true,
                    // Typed in the body. This is the case the parent lookup
                    // below cannot decide, because the recipient is both the
                    // author being answered and someone the author named.
                    PTagRole::Mention => false,
                    PTagRole::Unknown | PTagRole::None => reply_parent_id(ev)
                        .is_some_and(|parent| self_authored_parent_ids.contains(&parent)),
                };
            apply_link_preview_suppression(&mut item.tags, &item.id, &suppressed_mentions);
            item
        })
        .collect();
    // Back down to what the caller asked for. The query above deliberately
    // over-fetches, so trimming has to prefer real mentions — taking the newest
    // `cap` items would reintroduce the eviction the over-fetch exists to
    // prevent.
    let mentions = trim_mentions_preferring_real(mentions, cap as usize);
    let needs_action: Vec<FeedItemInfo> = approval_events
        .iter()
        .map(|ev| feed_item_from_event(ev, "needs_action"))
        .collect();

    let total = (mentions.len() + needs_action.len()) as u64;
    Ok(FeedResponse {
        feed: FeedSections {
            mentions,
            needs_action,
            activity: Vec::new(),
            agent_activity: Vec::new(),
        },
        meta: FeedMeta {
            since: since.unwrap_or(0),
            total,
            generated_at: chrono::Utc::now().timestamp(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, created_at: u64, reply_to_self: bool) -> FeedItemInfo {
        FeedItemInfo {
            id: id.to_string(),
            kind: 9,
            pubkey: String::new(),
            content: String::new(),
            created_at,
            channel_id: None,
            channel_name: String::new(),
            channel_type: None,
            tags: Vec::new(),
            category: "mention".to_string(),
            reply_to_self,
        }
    }

    #[test]
    fn trim_keeps_a_real_mention_a_burst_of_replies_would_evict() {
        // The regression: replies-to-you share the `#p` window with real
        // mentions, so newest-first truncation lets a chatty thread push every
        // `@you` out of the response entirely.
        let mut items: Vec<FeedItemInfo> = (0..5)
            .map(|i| item(&format!("reply-{i}"), 200 + i, true))
            .collect();
        items.push(item("real-mention", 100, false));

        let trimmed = trim_mentions_preferring_real(items, 3);

        // The real mention survives, and the two replies that keep it company
        // are the newest ones — not whichever two happened to come first.
        assert_eq!(
            trimmed.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["reply-4", "reply-3", "real-mention"],
        );
    }

    #[test]
    fn trim_returns_newest_first() {
        let items = vec![
            item("old-reply", 100, true),
            item("new-mention", 300, false),
            item("mid-reply", 200, true),
        ];

        let trimmed = trim_mentions_preferring_real(items, 2);

        assert_eq!(
            trimmed.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["new-mention", "mid-reply"],
        );
    }

    #[test]
    fn trim_is_a_no_op_under_the_cap() {
        let items = vec![item("a", 100, true), item("b", 200, false)];

        let trimmed = trim_mentions_preferring_real(items, 10);

        assert_eq!(trimmed.len(), 2);
        // Untouched, so the original order survives.
        assert_eq!(trimmed[0].id, "a");
    }

    #[test]
    fn trim_fills_remaining_slots_with_replies() {
        let items = vec![
            item("mention", 100, false),
            item("reply-new", 300, true),
            item("reply-old", 200, true),
        ];

        let trimmed = trim_mentions_preferring_real(items, 2);

        assert_eq!(
            trimmed.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["reply-new", "mention"],
        );
    }
}
