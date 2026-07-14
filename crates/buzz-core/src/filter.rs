//! NIP-01 filter matching.
//!
//! Multiple filters are OR-ed; fields within one filter are AND-ed.

use nostr::Filter;

use crate::event::StoredEvent;
use crate::kind::{event_kind_u32, DRAFT_MAX_TTL_SECS, KIND_DRAFT};

/// Returns `true` if the event matches any of the provided NIP-01 filters.
pub fn filters_match(filters: &[Filter], event: &StoredEvent) -> bool {
    filters.iter().any(|f| filter_match_one(f, event))
}

/// Result-level read authorization for relay-signed events whose content is
/// private to a single viewer. Currently gates `KIND_DM_VISIBILITY` and
/// `KIND_AGENT_TURN_METRIC`: the reader MUST equal the event's `#p` tag
/// (owner). Returns `true` for every other kind.
///
/// This guards every delivery surface — WS historical pull (`req.rs`), HTTP
/// bridge (`bridge.rs`), and live fan-out (`event.rs`) — so a query that
/// bypasses the filter-level `#p` gate (e.g. a kindless `ids:[…]` lookup of
/// a known event id) still cannot read another user's private event.
pub fn reader_authorized_for_event(event: &nostr::Event, reader_pubkey_hex: &str) -> bool {
    let kind = crate::kind::event_kind_u32(event);
    if kind != crate::kind::KIND_DM_VISIBILITY && kind != crate::kind::KIND_AGENT_TURN_METRIC {
        return true;
    }
    let p = nostr::SingleLetterTag::lowercase(nostr::Alphabet::P);
    event
        .tags
        .filter(nostr::TagKind::SingleLetter(p))
        .any(|t| t.content() == Some(reader_pubkey_hex))
}

/// Returns `true` if the event is an author-only kind and the requester is NOT
/// the author. Used as a per-event filter during historical delivery and fan-out
/// to silently omit unauthorized events from mixed-kind result sets.
pub fn is_author_only_event(event: &nostr::Event, requester_pubkey_bytes: &[u8]) -> bool {
    let kind_u32 = crate::kind::event_kind_u32(event);
    crate::kind::AUTHOR_ONLY_KINDS.contains(&kind_u32)
        && event.pubkey.to_bytes() != requester_pubkey_bytes
}

/// Returns `true` if a draft event should be suppressed at read time due to expiry.
///
/// Only applies to `KIND_DRAFT` events; all other kinds return `false`.
///
/// Effective expiry is `min(client_expiration_tag, created_at + DRAFT_MAX_TTL_SECS)`.
/// When no `expiration` tag is present the server ceiling (`created_at + 30d`) governs.
/// A client tag shorter than the 30-day ceiling is honoured; one longer is clamped to it.
/// Tombstones (empty-content drafts) follow the same rule — expiry is a property of the
/// event envelope, not its payload.
///
/// The `now` parameter is supplied by the caller so unit tests can inject an arbitrary
/// clock. Production call sites pass `nostr::Timestamp::now()`.
pub fn draft_expired(event: &nostr::Event, now: nostr::Timestamp) -> bool {
    if event_kind_u32(event) != KIND_DRAFT {
        return false;
    }
    let server_ceil = event
        .created_at
        .as_secs()
        .saturating_add(DRAFT_MAX_TTL_SECS);
    let effective_expiry = event
        .tags
        .iter()
        .find_map(|t| {
            if t.kind().to_string() == "expiration" {
                t.content().and_then(|v| v.parse::<u64>().ok())
            } else {
                None
            }
        })
        .map(|client_exp| client_exp.min(server_ceil))
        .unwrap_or(server_ceil);
    now.as_secs() >= effective_expiry
}

/// Canonical per-event read-authorization gate: combines `reader_authorized_for_event`
/// (p-gated/result-gated kinds), `is_author_only_event` (author-private kinds), and
/// `draft_expired` (time-based draft suppression) into a single predicate.
///
/// Every delivery surface — WS historical pull, WS fan-out, HTTP bridge (all
/// branches: feed, thread, search, catchall, channel-window), and COUNT fallback
/// — must pass each event through this function before serializing it to the wire.
/// Using one canonical gate instead of composing the predicates at each call site
/// prevents future read surfaces from accidentally omitting half the privacy model.
///
/// Returns `true` if `reader` MAY receive the event.
pub fn reader_can_receive_event(
    event: &nostr::Event,
    reader_pubkey_hex: &str,
    reader_pubkey_bytes: &[u8],
) -> bool {
    reader_authorized_for_event(event, reader_pubkey_hex)
        && !is_author_only_event(event, reader_pubkey_bytes)
        && !draft_expired(event, nostr::Timestamp::now())
}

fn filter_match_one(f: &Filter, ev: &StoredEvent) -> bool {
    if let Some(kinds) = &f.kinds {
        if !kinds.contains(&ev.event.kind) {
            return false;
        }
    }

    if let Some(authors) = &f.authors {
        if !authors.contains(&ev.event.pubkey) {
            return false;
        }
    }

    if let Some(since) = f.since {
        if ev.event.created_at < since {
            return false;
        }
    }

    if let Some(until) = f.until {
        if ev.event.created_at > until {
            return false;
        }
    }

    // NIP-01 allows prefix matching on event IDs.
    if let Some(ids) = &f.ids {
        let event_id_hex = ev.event.id.to_hex();
        if !ids.iter().any(|id| event_id_hex.starts_with(&id.to_hex())) {
            return false;
        }
    }

    for (tag_key, tag_values) in f.generic_tags.iter() {
        let tag_key_str = tag_key.to_string();
        let has_match = tag_values.iter().any(|filter_val| {
            ev.event
                .tags
                .iter()
                .filter(|t| t.kind().to_string() == tag_key_str)
                .filter_map(|t| t.content())
                .any(|event_val| event_val == filter_val.as_str())
        });
        // Fallback for #h (channel) filters: some events (reactions kind:7,
        // deletions kind:5) derive their channel from the target event and
        // don't carry an h-tag themselves. Use StoredEvent.channel_id as a
        // fallback ONLY when the event has no h-tags at all — if the event
        // has explicit h-tags, those are authoritative and must match.
        if !has_match && tag_key_str == "h" {
            let event_has_h_tags = ev.event.tags.iter().any(|t| t.kind().to_string() == "h");
            if !event_has_h_tags {
                if let Some(ch_id) = ev.channel_id {
                    let ch_str = ch_id.to_string();
                    if !tag_values.iter().any(|v| v.as_str() == ch_str) {
                        return false;
                    }
                } else {
                    return false;
                }
            } else {
                // Event has h-tags but none matched — strict rejection.
                return false;
            }
        } else if !has_match {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{make_event_with_keys, make_stored_event};
    use chrono::Utc;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn stored_with_tag(tag: Tag) -> StoredEvent {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "test")
            .tags([tag])
            .sign_with_keys(&keys)
            .expect("sign");
        StoredEvent::with_received_at(event, Utc::now(), None, true)
    }

    #[test]
    fn kind_author_since_until_tag_matching() {
        let keys = Keys::generate();
        let ev = StoredEvent::with_received_at(
            make_event_with_keys(&keys, Kind::TextNote),
            Utc::now(),
            None,
            true,
        );
        let pubkey = keys.public_key();
        let now_ts = nostr::Timestamp::now();
        let past = Timestamp::from(now_ts.as_secs() - 3600);
        let future = Timestamp::from(now_ts.as_secs() + 3600);

        assert!(filters_match(&[Filter::new().kind(Kind::TextNote)], &ev));
        assert!(!filters_match(
            &[Filter::new().kind(Kind::ContactList)],
            &ev
        ));

        assert!(filters_match(&[Filter::new().author(pubkey)], &ev));
        assert!(!filters_match(
            &[Filter::new().author(Keys::generate().public_key())],
            &ev
        ));

        assert!(filters_match(
            &[Filter::new().kind(Kind::TextNote).author(pubkey)],
            &ev
        ));
        assert!(!filters_match(
            &[Filter::new().kind(Kind::ContactList).author(pubkey)],
            &ev
        ));

        assert!(filters_match(&[Filter::new().since(past)], &ev));
        assert!(!filters_match(&[Filter::new().since(future)], &ev));
        assert!(filters_match(&[Filter::new().until(future)], &ev));
        assert!(!filters_match(&[Filter::new().until(past)], &ev));
    }

    #[test]
    fn or_semantics() {
        let ev = make_stored_event(Kind::TextNote, None);
        let miss = Filter::new().kind(Kind::ContactList);
        let hit = Filter::new().kind(Kind::TextNote);
        assert!(filters_match(&[miss.clone(), hit], &ev));
        assert!(!filters_match(
            &[miss, Filter::new().kind(Kind::EventDeletion)],
            &ev
        ));
        assert!(!filters_match(&[], &ev));
    }

    #[test]
    fn tag_matching() {
        let target_id = nostr::EventId::all_zeros();
        let ev = stored_with_tag(Tag::event(target_id));
        assert!(filters_match(&[Filter::new().event(target_id)], &ev));
        assert!(!filters_match(
            &[Filter::new().event(nostr::EventId::from_byte_array([1u8; 32]))],
            &ev
        ));
    }

    #[test]
    fn h_tag_fallback_uses_stored_channel_id() {
        // Reactions (kind:7) and deletions (kind:5) don't carry h-tags —
        // they derive their channel from the target event. The filter
        // should fall back to StoredEvent.channel_id for #h matching.
        let channel_id = uuid::Uuid::new_v4();
        let keys = Keys::generate();

        // Event with NO h-tag but with a stored channel_id.
        let reaction = EventBuilder::new(Kind::Reaction, "👍")
            .tags([Tag::event(nostr::EventId::all_zeros())])
            .sign_with_keys(&keys)
            .expect("sign");
        let stored = StoredEvent::with_received_at(reaction, Utc::now(), Some(channel_id), true);

        let h_filter = Filter::new().kind(Kind::Reaction).custom_tags(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
            [channel_id.to_string()],
        );

        // Should match via channel_id fallback.
        assert!(filters_match(std::slice::from_ref(&h_filter), &stored));

        // Wrong channel should NOT match.
        let wrong_channel = Filter::new().kind(Kind::Reaction).custom_tags(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
            [uuid::Uuid::new_v4().to_string()],
        );
        assert!(!filters_match(&[wrong_channel], &stored));

        // No stored channel_id should NOT match.
        let no_channel =
            StoredEvent::with_received_at(stored.event.clone(), Utc::now(), None, true);
        assert!(!filters_match(std::slice::from_ref(&h_filter), &no_channel));

        // Event WITH an explicit h-tag: tag is authoritative, channel_id fallback
        // must NOT override it. Prevents cross-channel leakage.
        let other_channel = uuid::Uuid::new_v4();
        let msg_with_h = EventBuilder::new(Kind::Custom(9), "hello")
            .tags([Tag::parse(["h", &other_channel.to_string()]).unwrap()])
            .sign_with_keys(&keys)
            .expect("sign");
        // channel_id matches the filter, but the h-tag points elsewhere.
        let stored_with_h =
            StoredEvent::with_received_at(msg_with_h, Utc::now(), Some(channel_id), true);
        assert!(
            !filters_match(std::slice::from_ref(&h_filter), &stored_with_h),
            "explicit h-tag must be authoritative — channel_id fallback must not override it"
        );
    }

    #[test]
    fn reader_authorized_for_event_gates_dm_visibility_by_p() {
        let relay = Keys::generate();
        let owner = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let snapshot = EventBuilder::new(Kind::Custom(crate::kind::KIND_DM_VISIBILITY as u16), "")
            .tags([
                Tag::parse(["d", owner]).unwrap(),
                Tag::parse(["p", owner]).unwrap(),
            ])
            .sign_with_keys(&relay)
            .expect("sign");

        assert!(
            reader_authorized_for_event(&snapshot, owner),
            "owner must be authorized to read their own snapshot"
        );
        assert!(
            !reader_authorized_for_event(&snapshot, other),
            "a third party must NOT be authorized to read another viewer's snapshot"
        );

        // Non-DV events are unaffected by this gate.
        let note = EventBuilder::new(Kind::TextNote, "hi")
            .sign_with_keys(&relay)
            .expect("sign");
        assert!(reader_authorized_for_event(&note, other));
    }

    #[test]
    fn reader_authorized_for_event_gates_agent_turn_metric_by_p() {
        let agent_keys = Keys::generate();
        let owner = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let attacker = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

        // Agent turn metric event: pubkey=agent, p tag=owner (NIP-AM envelope shape).
        let metric = EventBuilder::new(
            Kind::Custom(crate::kind::KIND_AGENT_TURN_METRIC as u16),
            "encrypted-payload",
        )
        .tags([
            Tag::parse(["p", owner]).unwrap(),
            Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&agent_keys)
        .expect("sign");

        assert!(
            reader_authorized_for_event(&metric, owner),
            "owner must be authorized to read their own agent turn metric"
        );
        assert!(
            !reader_authorized_for_event(&metric, attacker),
            "non-owner must NOT be authorized to read an agent turn metric via kindless ids"
        );
        // The authoring agent also does not get read-back (NIP-AM: owner-only read).
        assert!(
            !reader_authorized_for_event(&metric, &agent_keys.public_key().to_hex()),
            "the authoring agent must NOT be authorized to read its own metric event (owner-only)"
        );
    }

    // --- draft_expired unit tests ---
    //
    // All tests inject an explicit `now` so they don't depend on the wall clock.
    // Draft events use KIND_DRAFT (31234). Non-draft events are TextNote.

    fn make_draft(keys: &Keys, created_at_secs: u64, expiration_tag: Option<u64>) -> nostr::Event {
        use crate::kind::KIND_DRAFT;
        let mut builder = EventBuilder::new(Kind::Custom(KIND_DRAFT as u16), "ciphertext")
            .custom_created_at(Timestamp::from(created_at_secs));
        if let Some(exp) = expiration_tag {
            builder = builder.tags([Tag::parse(["expiration", &exp.to_string()]).unwrap()]);
        }
        builder.sign_with_keys(keys).expect("sign")
    }

    #[test]
    fn draft_expired_tag_in_past_returns_true() {
        let keys = Keys::generate();
        let now = nostr::Timestamp::now();
        // created 1 day ago, expiration 1 hour ago
        let created = now.as_secs() - 86400;
        let exp = now.as_secs() - 3600;
        let draft = make_draft(&keys, created, Some(exp));
        assert!(draft_expired(&draft, now), "expired tag must be suppressed");
    }

    #[test]
    fn draft_expired_tag_in_future_returns_false() {
        let keys = Keys::generate();
        let now = nostr::Timestamp::now();
        let created = now.as_secs() - 3600;
        let exp = now.as_secs() + 86400; // expires tomorrow
        let draft = make_draft(&keys, created, Some(exp));
        assert!(!draft_expired(&draft, now), "future tag must be served");
    }

    #[test]
    fn draft_expired_no_tag_over_30d_returns_true() {
        use crate::kind::DRAFT_MAX_TTL_SECS;
        let keys = Keys::generate();
        let now = nostr::Timestamp::now();
        // created 31 days ago, no expiration tag
        let created = now.as_secs() - DRAFT_MAX_TTL_SECS - 86400;
        let draft = make_draft(&keys, created, None);
        assert!(
            draft_expired(&draft, now),
            "draft older than 30d with no tag must be suppressed"
        );
    }

    #[test]
    fn draft_expired_no_tag_under_30d_returns_false() {
        let keys = Keys::generate();
        let now = nostr::Timestamp::now();
        // created 1 day ago, no expiration tag
        let created = now.as_secs() - 86400;
        let draft = make_draft(&keys, created, None);
        assert!(
            !draft_expired(&draft, now),
            "draft under 30d with no tag must be served"
        );
    }

    #[test]
    fn draft_expired_tag_longer_than_30d_capped_at_ceiling() {
        use crate::kind::DRAFT_MAX_TTL_SECS;
        let keys = Keys::generate();
        let now = nostr::Timestamp::now();
        // created 31 days ago; client tag says 40 days — should be clamped to 30d ceiling
        let created = now.as_secs() - DRAFT_MAX_TTL_SECS - 86400;
        let long_exp = created + DRAFT_MAX_TTL_SECS + (10 * 86400); // created + 40d
        let draft = make_draft(&keys, created, Some(long_exp));
        assert!(
            draft_expired(&draft, now),
            "tag longer than 30d must be clamped to server ceiling — draft must be suppressed"
        );
    }

    #[test]
    fn draft_expired_non_draft_kind_never_suppressed() {
        let keys = Keys::generate();
        let now = nostr::Timestamp::now();
        // TextNote with an expiration tag that is in the past — must NOT suppress
        let past_exp = now.as_secs() - 3600;
        let event = EventBuilder::new(Kind::TextNote, "hello")
            .tags([Tag::parse(["expiration", &past_exp.to_string()]).unwrap()])
            .sign_with_keys(&keys)
            .expect("sign");
        assert!(
            !draft_expired(&event, now),
            "non-draft kinds must never be suppressed by draft_expired"
        );
    }

    #[test]
    fn draft_expired_tombstone_follows_same_expiry_rule() {
        use crate::kind::DRAFT_MAX_TTL_SECS;
        let keys = Keys::generate();
        let now = nostr::Timestamp::now();
        // Tombstone (empty content) created 31 days ago — same rule as regular draft
        let created = now.as_secs() - DRAFT_MAX_TTL_SECS - 86400;
        let tombstone = EventBuilder::new(Kind::Custom(crate::kind::KIND_DRAFT as u16), "")
            .custom_created_at(Timestamp::from(created))
            .sign_with_keys(&keys)
            .expect("sign");
        assert!(
            draft_expired(&tombstone, now),
            "tombstone draft must be suppressed by the same expiry rule"
        );
    }
}
