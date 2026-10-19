//! NIP-01 filter matching.
//!
//! Multiple filters are OR-ed; fields within one filter are AND-ed.

use nostr::Filter;

use crate::event::StoredEvent;

/// Returns `true` if the event matches any of the provided NIP-01 filters.
pub fn filters_match(filters: &[Filter], event: &StoredEvent) -> bool {
    filters.iter().any(|f| filter_match_one(f, event))
}

/// Result-level read authorization for relay-signed events whose content is
/// private to a single viewer: the kinds in
/// [`crate::kind::RESULT_GATED_KINDS`]. The reader MUST equal the event's
/// `#p` tag (owner). Returns `true` for every other kind.
///
/// This guards every delivery surface — WS historical pull (`req.rs`), HTTP
/// bridge (`bridge.rs`), and live fan-out (`event.rs`) — so a query that
/// bypasses the filter-level `#p` gate (e.g. a kindless `ids:[…]` lookup of
/// a known event id) still cannot read another user's private event.
pub fn reader_authorized_for_event(event: &nostr::Event, reader_pubkey_hex: &str) -> bool {
    let kind = crate::kind::event_kind_u32(event);
    if !crate::kind::RESULT_GATED_KINDS.contains(&kind) {
        return true;
    }
    let p = nostr::SingleLetterTag::lowercase(nostr::Alphabet::P);
    event
        .tags
        .filter(nostr::TagKind::SingleLetter(p))
        .any(|t| t.content() == Some(reader_pubkey_hex))
}

/// The tag that marks a kind:44100/44101 membership notice about a system
/// channel (NIP-CL): `["t", "system"]`. Only the relay signs these kinds, and
/// it adds the tag only when the channel is a system channel.
pub const SYSTEM_NOTICE_TAG: [&str; 2] = ["t", "system"];

/// The kinds that [`SYSTEM_NOTICE_TAG`] can mark.
pub const SYSTEM_NOTICE_KINDS: [u32; 2] = [
    crate::kind::KIND_MEMBER_ADDED_NOTIFICATION,
    crate::kind::KIND_MEMBER_REMOVED_NOTIFICATION,
];

/// Whether `event` is a membership notice about a system channel.
///
/// Such a notice is hidden by default: it matches only a filter that asks
/// for it ([`filter_asks_for_system_notices`]). Clients that do not know
/// system channels never ask, so they never see one, on any read or live
/// path that matches through [`filters_match`] or applies the same rule in
/// SQL (`EventQuery::include_system_notices`, which defaults to off).
pub fn is_system_notice(event: &nostr::Event) -> bool {
    SYSTEM_NOTICE_KINDS.contains(&crate::kind::event_kind_u32(event))
        && event.tags.iter().any(|tag| {
            let tag = tag.as_slice();
            tag.first().map(String::as_str) == Some(SYSTEM_NOTICE_TAG[0])
                && tag.get(1).map(String::as_str) == Some(SYSTEM_NOTICE_TAG[1])
        })
}

/// Whether `filter` asks for system-channel membership notices: its `#t`
/// values include `"system"`.
pub fn filter_asks_for_system_notices(filter: &Filter) -> bool {
    filter
        .generic_tags
        .get(&nostr::SingleLetterTag::lowercase(nostr::Alphabet::T))
        .is_some_and(|values| values.iter().any(|value| value == SYSTEM_NOTICE_TAG[1]))
}

fn filter_match_one(f: &Filter, ev: &StoredEvent) -> bool {
    if is_system_notice(&ev.event) && !filter_asks_for_system_notices(f) {
        return false;
    }

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
    fn a_system_notice_matches_only_a_filter_that_asks() {
        let relay = Keys::generate();
        let member = Keys::generate().public_key();
        let notice = |kind: u32, system: bool| {
            let mut tags = vec![
                Tag::parse(["p", &member.to_hex()]).unwrap(),
                Tag::parse(["h", "c"]).unwrap(),
            ];
            if system {
                tags.push(Tag::parse(SYSTEM_NOTICE_TAG).unwrap());
            }
            let event = EventBuilder::new(Kind::Custom(kind as u16), "")
                .tags(tags)
                .sign_with_keys(&relay)
                .expect("sign");
            StoredEvent::with_received_at(event, Utc::now(), None, true)
        };
        let filter = |json: serde_json::Value| -> Filter { serde_json::from_value(json).unwrap() };
        let p = member.to_hex();
        let old = [
            filter(serde_json::json!({"kinds": [44100, 44101], "#p": [p]})),
            filter(serde_json::json!({"#p": [p]})),
            filter(serde_json::json!({})),
            filter(serde_json::json!({"#t": ["other"]})),
        ];
        let asks = [
            filter(serde_json::json!({"kinds": [44100, 44101], "#p": [p], "#t": ["system"]})),
            filter(serde_json::json!({"#t": ["other", "system"]})),
        ];
        for kind in SYSTEM_NOTICE_KINDS {
            let system = notice(kind, true);
            assert!(is_system_notice(&system.event));
            for f in &old {
                assert!(!filters_match(std::slice::from_ref(f), &system), "{f:?}");
            }
            for f in &asks {
                assert!(filters_match(std::slice::from_ref(f), &system), "{f:?}");
            }
            // An ordinary notice is unchanged: old filters see it, and the
            // asking filter's `#t` does not match it.
            let ordinary = notice(kind, false);
            assert!(!is_system_notice(&ordinary.event));
            assert!(filters_match(&old[..1], &ordinary));
            assert!(!filters_match(&asks[..1], &ordinary));
        }
        // The tag hides nothing on other kinds.
        let other = notice(44200, true);
        assert!(!is_system_notice(&other.event));
        assert!(filters_match(&old[2..3], &other));
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
    fn h_tag_multi_value_filter_matches_any_channel() {
        let channel_a = uuid::Uuid::new_v4();
        let channel_b = uuid::Uuid::new_v4();
        let stored = stored_with_tag(Tag::parse(["h", &channel_b.to_string()]).unwrap());
        let filter = Filter::new().custom_tags(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
            [channel_a.to_string(), channel_b.to_string()],
        );

        assert!(filters_match(&[filter], &stored));
    }

    #[test]
    fn empty_h_tag_filter_matches_nothing() {
        let channel_id = uuid::Uuid::new_v4();
        let stored = stored_with_tag(Tag::parse(["h", &channel_id.to_string()]).unwrap());
        let filter: Filter = serde_json::from_value(serde_json::json!({ "#h": [] })).unwrap();

        assert!(!filters_match(&[filter], &stored));
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
}
