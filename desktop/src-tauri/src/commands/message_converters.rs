//! Event → UI-model converters shared by the message feed commands.

use crate::models::{FeedItemInfo, ForumMessageInfo, ForumThreadReplyInfo, ThreadSummary};

pub(super) fn channel_id_from_tags(ev: &nostr::Event) -> Option<String> {
    ev.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.len() >= 2 && s[0] == "h" {
            Some(s[1].clone())
        } else {
            None
        }
    })
}

pub(super) fn tags_to_vec(ev: &nostr::Event) -> Vec<Vec<String>> {
    ev.tags.iter().map(|t| t.as_slice().to_vec()).collect()
}

pub(super) fn feed_item_from_event(ev: &nostr::Event, category: &str) -> FeedItemInfo {
    let channel_id = channel_id_from_tags(ev);
    FeedItemInfo {
        id: ev.id.to_hex(),
        kind: ev.kind.as_u16() as u32,
        pubkey: ev.pubkey.to_hex(),
        content: ev.content.clone(),
        created_at: ev.created_at.as_secs(),
        channel_id,
        channel_name: String::new(),
        channel_type: None,
        tags: tags_to_vec(ev),
        category: category.to_string(),
    }
}

pub(super) fn forum_message_from_event(ev: &nostr::Event, channel_id: &str) -> ForumMessageInfo {
    ForumMessageInfo {
        event_id: ev.id.to_hex(),
        pubkey: ev.pubkey.to_hex(),
        content: ev.content.clone(),
        kind: ev.kind.as_u16() as u32,
        created_at: ev.created_at.as_secs() as i64,
        channel_id: channel_id.to_string(),
        tags: tags_to_vec(ev),
        thread_summary: Some(ThreadSummary {
            reply_count: 0,
            descendant_count: 0,
            last_reply_at: None,
            participants: Vec::new(),
        }),
        reactions: serde_json::Value::Null,
    }
}

pub(super) fn forum_reply_from_event(
    ev: &nostr::Event,
    channel_id: &str,
    root_event_id: &str,
) -> ForumThreadReplyInfo {
    // Walk e-tags for NIP-10 parent/root markers.
    let (mut parent_id, mut explicit_root) = (None, None);
    for t in ev.tags.iter() {
        let s = t.as_slice();
        if s.len() >= 2 && s[0] == "e" {
            match s.get(3).map(|x| x.as_str()) {
                Some("root") => explicit_root = Some(s[1].clone()),
                Some("reply") => parent_id = Some(s[1].clone()),
                _ => {
                    if parent_id.is_none() {
                        parent_id = Some(s[1].clone());
                    }
                }
            }
        }
    }
    let parent = parent_id
        .clone()
        .unwrap_or_else(|| root_event_id.to_string());
    let root = explicit_root.unwrap_or_else(|| root_event_id.to_string());
    let depth = if parent == root { 1 } else { 2 };

    ForumThreadReplyInfo {
        event_id: ev.id.to_hex(),
        pubkey: ev.pubkey.to_hex(),
        content: ev.content.clone(),
        kind: ev.kind.as_u16() as u32,
        created_at: ev.created_at.as_secs() as i64,
        channel_id: channel_id.to_string(),
        tags: tags_to_vec(ev),
        parent_event_id: Some(parent),
        root_event_id: Some(root),
        depth,
        broadcast: false,
        reactions: serde_json::Value::Null,
    }
}
