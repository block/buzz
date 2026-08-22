//! Resolving the authors of the messages a batch's replies answer.
//!
//! Split out of `unread_catch_up` so the round-trip decision — which replies
//! are ambiguous enough to be worth asking the relay about — sits next to the
//! request that acts on it, and can be tested without a live session.

use std::collections::HashSet;
use std::time::Duration;

use buzz_core_pkg::kind::{
    KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2,
};

use crate::p_tag_role::PTagRole;
use crate::unread_catch_up::{thread_reference, FetchedChannel};
use crate::unread_notify::{has_p_tag_for, role_for};

/// Kinds a reply's parent can be, for looking one up by id.
///
/// Deliberately wider than the catch-up kinds: a reply can answer a diff
/// message (40008), a legacy stream message (40001), or a NIP-01 note bridged in
/// from another client (1). A kind missing here leaves the parent unresolved,
/// which reads as a mention and pierces the mute this lookup exists to protect.
///
/// Keep in sync with `REPLY_PARENT_EVENT_KINDS` in
/// `desktop/src/shared/constants/kinds.ts`.
const REPLY_PARENT_KINDS: &[u32] = &[
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_FORUM_POST,
    KIND_FORUM_COMMENT,
    1,
    40001,
    40008,
];
const PARENT_LOOKUP_CHUNK: usize = 200;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Parent ids worth a round trip, given what the batch already answers.
///
/// A reply is ambiguous only when its `p` tag for the user carries no role
/// marker. Every consumer of the resolved map — `has_authored_mention`,
/// `is_high_priority`, `should_notify` — reads `parent_author` in the unmarked
/// arm alone, so a marked reply's parent would be fetched and never consulted.
/// Skipping it here is where the markers actually save the round trip they were
/// introduced to remove.
///
/// A DM is excluded wholesale: there every message `p`-tags both participants,
/// so the addressing tag is the addressing and no consumer asks for a parent.
fn wanted_parent_ids(
    fetched: &[FetchedChannel],
    self_pubkey: &str,
    known: &std::collections::HashMap<String, String>,
) -> HashSet<String> {
    let mut wanted: HashSet<String> = HashSet::new();
    for item in fetched {
        if item.channel.channel_type == "dm" {
            continue;
        }
        for event in &item.events {
            if event.pubkey.eq_ignore_ascii_case(self_pubkey)
                || !has_p_tag_for(&event.tags, self_pubkey)
            {
                continue;
            }
            if !matches!(
                role_for(&event.tags, self_pubkey),
                PTagRole::Unknown | PTagRole::None
            ) {
                continue;
            }
            if let Some(parent_id) = thread_reference(&event.tags).parent_id {
                if !known.contains_key(&parent_id) && is_event_id_hex(&parent_id) {
                    wanted.insert(parent_id);
                }
            }
        }
    }
    wanted
}

/// Authors of the messages this batch's replies answer, keyed by parent id.
///
/// Only replies whose `p` tag is genuinely ambiguous are worth the round trip.
/// Parents already present in the batch are answered locally.
pub(crate) async fn resolve_parent_authors(
    session: &crate::native_relay_client::SessionLease,
    fetched: &[FetchedChannel],
    self_pubkey: &str,
) -> Result<std::collections::HashMap<String, String>, String> {
    let self_pubkey = self_pubkey.to_lowercase();
    let mut authors: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for item in fetched {
        for event in &item.events {
            authors.insert(event.id.clone(), event.pubkey.clone());
        }
    }
    let wanted = wanted_parent_ids(fetched, &self_pubkey, &authors);
    if wanted.is_empty() {
        return Ok(authors);
    }
    let ids: Vec<String> = wanted.into_iter().collect();
    for chunk in ids.chunks(PARENT_LOOKUP_CHUNK) {
        let filter = serde_json::json!({
            "kinds": REPLY_PARENT_KINDS,
            "ids": chunk,
            "limit": chunk.len(),
        });
        let events = session
            .handle()
            .fetch_events(filter, REQUEST_TIMEOUT)
            .await?;
        for event in events {
            authors.insert(event.id.to_hex(), event.pubkey.to_hex());
        }
    }
    Ok(authors)
}

/// Whether a string is a well-formed 64-char hex event id, and so safe to put in
/// an `ids` filter. A relay rejects the whole REQ over one malformed id.
fn is_event_id_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::unread_catch_up::tests::{dm_channel, reply, stream_channel};
    use crate::unread_catch_up::{CatchUpChannel, EventView};

    const PARENT: &str = "aa11bb22cc33dd44ee55ff6600778899aabbccddeeff00112233445566778899";
    const OTHER_PARENT: &str = "bb11cc22dd33ee44ff5500667788990011223344556677889900aabbccddeeff";

    /// One channel's batch, seeded with whatever the batch already answers.
    fn wanted(
        channel: CatchUpChannel,
        events: Vec<EventView>,
        known: &[(&str, &str)],
    ) -> Vec<String> {
        let known: HashMap<String, String> = known
            .iter()
            .map(|(id, author)| ((*id).to_string(), (*author).to_string()))
            .collect();
        let fetched = vec![FetchedChannel {
            order: 0,
            channel,
            events,
        }];
        let mut ids: Vec<String> = wanted_parent_ids(&fetched, "self", &known)
            .into_iter()
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn an_unmarked_p_tag_needs_its_parent_fetched() {
        // The only ambiguous shape: a pre-marker sender's reply. Nothing in the
        // event says whether we were typed or merely answered.
        let events = vec![reply("r1", "other", PARENT, &["p", "self"])];
        assert_eq!(wanted(stream_channel(), events, &[]), vec![PARENT]);
    }

    #[test]
    fn a_marked_reply_costs_no_round_trip() {
        // `role_for` already answers both of these, so fetching the parent
        // would resolve an author no consumer goes on to read.
        let addressing = vec![reply("r1", "other", PARENT, &["p", "self", "", "reply"])];
        assert!(wanted(stream_channel(), addressing, &[]).is_empty());
        let mention = vec![reply(
            "r2",
            "other",
            OTHER_PARENT,
            &["p", "self", "", "mention"],
        )];
        assert!(wanted(stream_channel(), mention, &[]).is_empty());
    }

    #[test]
    fn one_unmarked_reply_in_a_marked_batch_is_still_fetched() {
        // The skip is per event, not per batch: a mixed window must not let a
        // marked reply suppress the lookup an unmarked one needs.
        let events = vec![
            reply("r1", "other", PARENT, &["p", "self", "", "reply"]),
            reply("r2", "other", OTHER_PARENT, &["p", "self"]),
        ];
        assert_eq!(wanted(stream_channel(), events, &[]), vec![OTHER_PARENT]);
    }

    #[test]
    fn a_dm_never_asks_for_a_parent() {
        let events = vec![reply("r1", "other", PARENT, &["p", "self"])];
        assert!(wanted(dm_channel(), events, &[]).is_empty());
    }

    #[test]
    fn a_parent_already_in_the_batch_is_answered_locally() {
        let events = vec![reply("r1", "other", PARENT, &["p", "self"])];
        assert!(wanted(stream_channel(), events, &[(PARENT, "other")]).is_empty());
    }

    #[test]
    fn our_own_reply_needs_no_parent() {
        let events = vec![reply("r1", "self", PARENT, &["p", "other"])];
        assert!(wanted(stream_channel(), events, &[]).is_empty());
    }

    #[test]
    fn a_malformed_parent_id_is_never_put_in_a_filter() {
        // The relay accepts a junk `e` tag; feeding it back as an `ids` filter
        // earns a bare NOTICE and a hung request.
        let events = vec![reply("r1", "other", "not-hex", &["p", "self"])];
        assert!(wanted(stream_channel(), events, &[]).is_empty());
    }
}
