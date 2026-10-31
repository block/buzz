//! Validation for kind:40004 pinned-message events.
//!
//! A pin is a stored channel event `["h", <channel>]` + `["e", <message>]`.
//! Who may write it is the generic channel write gate (members, bots, admins
//! and owners; never guests). This module only checks the shape and target.

use buzz_core::kind::{
    event_kind_u32, KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_DIFF, KIND_STREAM_MESSAGE_PINNED, KIND_STREAM_MESSAGE_V2,
};
use buzz_core::CommunityId;
use buzz_db::event::EventQuery;
use nostr::Event;

use super::ingest::{effective_message_author, extract_channel_id, target_readable_in_channel};
use crate::state::AppState;

const PINNABLE_KINDS: [u32; 5] = [
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_STREAM_MESSAGE_DIFF,
    KIND_FORUM_POST,
    KIND_FORUM_COMMENT,
];

/// The pinned message id: the single `e` tag, as 64 lowercase hex characters.
fn pin_target_hex(event: &Event) -> Result<String, String> {
    let mut targets = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "e");
    let (Some(tag), None) = (targets.next(), targets.next()) else {
        return Err("pin must reference exactly one message via an e tag".to_string());
    };
    tag.content()
        .filter(|v| v.len() == 64 && v.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .map(str::to_string)
        .ok_or_else(|| "pin e tag must be a 64-character lowercase hex event ID".to_string())
}

/// Validate a kind:40004 pin: one live message of the pin's own channel that
/// the author can read, not already pinned.
pub(crate) async fn validate_pin_event(
    community_id: CommunityId,
    event: &Event,
    state: &AppState,
) -> Result<(), String> {
    let target_hex = pin_target_hex(event)?;
    let channel_id =
        extract_channel_id(event).ok_or_else(|| "missing h tag for pin".to_string())?;
    let reader = effective_message_author(event, &state.relay_keypair.public_key());
    let target_bytes = hex::decode(&target_hex).map_err(|_| "invalid target event ID")?;

    let target = state
        .db
        .get_event_by_id_for_event_write(community_id, &target_bytes)
        .await
        .map_err(|e| format!("db error: {e}"))?
        .filter(|target| target_readable_in_channel(target, channel_id, &reader))
        .ok_or_else(|| "pin target message not found".to_string())?;
    if !PINNABLE_KINDS.contains(&event_kind_u32(&target.event)) {
        return Err("pin target must be a channel message".to_string());
    }

    let existing = state
        .db
        .query_events(&EventQuery {
            channel_id: Some(channel_id),
            kinds: Some(vec![KIND_STREAM_MESSAGE_PINNED as i32]),
            e_tags: Some(vec![target_hex]),
            limit: Some(1),
            ..EventQuery::for_community(community_id)
        })
        .await
        .map_err(|e| format!("db error: {e}"))?;
    if !existing.is_empty() {
        return Err("message is already pinned".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const MESSAGE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn pin(tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::Custom(40004), "")
            .tags(tags.iter().map(|t| Tag::parse(t.iter().copied()).unwrap()))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn target_is_the_single_lowercase_hex_e_tag() {
        let event = pin(&[
            &["h", "6f1c7c1e-1d5b-4c55-9d43-0c1f0b6a2f10"],
            &["e", MESSAGE],
        ]);
        assert_eq!(pin_target_hex(&event).unwrap(), MESSAGE);
    }

    #[test]
    fn rejects_a_pin_that_does_not_name_exactly_one_valid_message() {
        let upper = MESSAGE.to_uppercase();
        let cases: [&[&[&str]]; 5] = [
            &[&["h", "x"]],
            &[&["e", MESSAGE], &["e", MESSAGE]],
            &[&["e", "not-hex"]],
            &[&["e", &upper]],
            &[&["e", "abc"]],
        ];
        for tags in cases {
            assert!(pin_target_hex(&pin(tags)).is_err(), "{tags:?}");
        }
    }

    #[test]
    fn only_channel_messages_are_pinnable() {
        for kind in [9, 40002, 40008, 45001, 45003] {
            assert!(PINNABLE_KINDS.contains(&kind), "{kind}");
        }
        for kind in [5, 7, 40003, 40004, 40099, 9005] {
            assert!(!PINNABLE_KINDS.contains(&kind), "{kind}");
        }
    }
}
