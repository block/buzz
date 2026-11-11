//! Opt-in single-publisher streams with replies from channel readers.

use buzz_core::{channel::ChannelType, kind::*, CommunityId};
use buzz_db::channel::ChannelRecord;
use nostr::Event;

use crate::state::AppState;

const DENIED: &str = "restricted: reply to a post by this channel's publisher";

/// Parse a partial metadata update. An empty value explicitly clears policy.
pub(crate) fn publisher_update(event: &Event) -> Result<Option<Option<Vec<u8>>>, String> {
    let tags: Vec<_> = event
        .tags
        .iter()
        .filter(|t| t.kind().to_string() == "message_publisher")
        .collect();
    if tags.is_empty() {
        return Ok(None);
    }
    if tags.len() != 1 || tags[0].as_slice().len() != 2 {
        return Err("invalid: expected one message_publisher tag with one value".into());
    }
    let value = &tags[0].as_slice()[1];
    if value.is_empty() {
        return Ok(Some(None));
    }
    let key = nostr::PublicKey::from_hex(value).map_err(|_| {
        "invalid: message_publisher must be a public key or empty to clear".to_string()
    })?;
    Ok(Some(Some(key.to_bytes().to_vec())))
}

/// Message kinds governed by the publisher policy. Scheduled messages must also
/// pass it when published; ordinary content permissions remain separate.
pub(crate) fn is_message(kind: u32) -> bool {
    matches!(
        kind,
        KIND_STREAM_MESSAGE
            | KIND_STREAM_MESSAGE_V2
            | KIND_STREAM_MESSAGE_SCHEDULED
            | KIND_STREAM_MESSAGE_DIFF
    )
}

/// Check the actual signed ancestry, including legacy events without metadata.
/// The caller must already enforce channel readability, scopes and moderation.
/// Never use a claimed root or a materialized ancestry row as an authority grant.
pub(crate) async fn validate(
    state: &AppState,
    community: CommunityId,
    channel: &ChannelRecord,
    event: &Event,
    actor: &[u8],
) -> Result<(), String> {
    let Some(publisher) = &channel.message_publisher else {
        return Ok(());
    };
    if channel.channel_type != ChannelType::Stream.as_str() {
        return Err(DENIED.into());
    }
    let markers = buzz_core::nip10::parse_thread_markers(&event.tags);
    let is_publisher = actor == publisher;
    if !is_publisher
        && event
            .tags
            .iter()
            .any(|t| t.as_slice().first().is_some_and(|v| v == "broadcast"))
    {
        return Err(DENIED.into());
    }
    let Some((claimed_root, mut parent)) = markers.resolve() else {
        return if is_publisher {
            Ok(())
        } else {
            Err(DENIED.into())
        };
    };
    let mut roots = vec![claimed_root];
    let mut seen = std::collections::HashSet::new();
    for _ in 0..100 {
        let id = hex::decode(&parent).map_err(|_| DENIED.to_string())?;
        if !seen.insert(id.clone()) {
            return Err(DENIED.into());
        }
        let target = state
            .db
            .get_event_by_id_for_event_write(community, &id)
            .await
            .map_err(|e| format!("error: database error: {e}"))?
            .filter(|target| {
                target.channel_id == Some(channel.id)
                    && matches!(
                        target.event.kind.as_u16() as u32,
                        KIND_STREAM_MESSAGE | KIND_STREAM_MESSAGE_V2
                    )
                    && super::req::event_visible_to_reader(&target.event, actor)
            })
            .ok_or_else(|| DENIED.to_string())?;
        let next = buzz_core::nip10::parse_thread_markers(&target.event.tags).resolve();
        if let Some((root, next_parent)) = next {
            roots.push(root);
            parent = next_parent;
        } else {
            let author = super::ingest::effective_message_author(
                &target.event,
                &state.relay_keypair.public_key(),
            );
            return if author == *publisher
                && roots.iter().all(|root| root.eq_ignore_ascii_case(&parent))
            {
                Ok(())
            } else {
                Err(DENIED.into())
            };
        }
    }
    Err("invalid: thread depth limit exceeded".into())
}
