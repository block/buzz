use tauri::State;

use crate::{
    app_state::AppState,
    models::{
        ForumMessageInfo, ForumPostsResponse, ForumThreadReplyInfo, ForumThreadResponse,
        ThreadSummary,
    },
    relay::query_relay,
};

pub(super) async fn fetch_agent_owner_pubkeys(
    state: &AppState,
    events: &[nostr::Event],
) -> std::collections::HashMap<String, String> {
    let authors = events
        .iter()
        .map(|event| event.pubkey.to_hex())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if authors.is_empty() {
        return std::collections::HashMap::new();
    }

    super::query_relay(
        state,
        &[serde_json::json!({ "kinds": [0], "authors": authors })],
    )
    .await
    .unwrap_or_default()
    .into_iter()
    .filter_map(|profile| {
        crate::nostr_convert::profile_valid_oa_owner_pubkey(&profile)
            .map(|owner| (profile.pubkey.to_hex(), owner))
    })
    .collect()
}

fn tags_to_vec(event: &nostr::Event) -> Vec<Vec<String>> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect()
}

pub(super) fn forum_message_from_event(event: &nostr::Event, channel_id: &str) -> ForumMessageInfo {
    ForumMessageInfo {
        event_id: event.id.to_hex(),
        pubkey: event.pubkey.to_hex(),
        sig: event.sig.to_string(),
        content: event.content.clone(),
        kind: event.kind.as_u16() as u32,
        created_at: event.created_at.as_secs() as i64,
        channel_id: channel_id.to_string(),
        tags: tags_to_vec(event),
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
    event: &nostr::Event,
    channel_id: &str,
    root_event_id: &str,
) -> ForumThreadReplyInfo {
    let (mut parent_id, mut explicit_root) = (None, None);
    for tag in event.tags.iter() {
        let values = tag.as_slice();
        if values.len() >= 2 && values[0] == "e" {
            match values.get(3).map(String::as_str) {
                Some("root") => explicit_root = Some(values[1].clone()),
                Some("reply") => parent_id = Some(values[1].clone()),
                _ if parent_id.is_none() => parent_id = Some(values[1].clone()),
                _ => {}
            }
        }
    }

    let parent = parent_id
        .clone()
        .unwrap_or_else(|| root_event_id.to_string());
    let root = explicit_root.unwrap_or_else(|| root_event_id.to_string());
    let depth = if parent == root { 1 } else { 2 };

    ForumThreadReplyInfo {
        event_id: event.id.to_hex(),
        pubkey: event.pubkey.to_hex(),
        sig: event.sig.to_string(),
        content: event.content.clone(),
        kind: event.kind.as_u16() as u32,
        created_at: event.created_at.as_secs() as i64,
        channel_id: channel_id.to_string(),
        tags: tags_to_vec(event),
        parent_event_id: Some(parent),
        root_event_id: Some(root),
        depth,
        broadcast: false,
        reactions: serde_json::Value::Null,
    }
}

pub(super) fn link_preview_suppression_targets(
    originals: &[nostr::Event],
    edits: &[nostr::Event],
    owner_pubkeys: &std::collections::HashMap<String, String>,
) -> std::collections::HashSet<String> {
    let originals_by_id = originals
        .iter()
        .map(|event| (event.id.to_hex(), event))
        .collect::<std::collections::HashMap<_, _>>();

    edits
        .iter()
        .filter(|event| {
            event.kind.as_u16() == 40003
                && event
                    .tags
                    .iter()
                    .any(|tag| tag.as_slice() == ["link-preview".to_string(), "none".to_string()])
        })
        .filter_map(|edit| {
            let target_id = edit.tags.iter().find_map(|tag| {
                let values = tag.as_slice();
                (values.first().map(String::as_str) == Some("e"))
                    .then(|| values.get(1).cloned())
                    .flatten()
            })?;
            let target = originals_by_id.get(&target_id)?;
            let author = target.pubkey.to_hex();
            let signer = edit.pubkey.to_hex();
            (signer == author || owner_pubkeys.get(&author) == Some(&signer)).then_some(target_id)
        })
        .collect()
}

pub(super) fn apply_link_preview_suppression(
    tags: &mut Vec<Vec<String>>,
    event_id: &str,
    suppressed: &std::collections::HashSet<String>,
) {
    if suppressed.contains(event_id)
        && !tags
            .iter()
            .any(|tag| tag.as_slice() == ["link-preview".to_string(), "none".to_string()])
    {
        tags.push(vec!["link-preview".to_string(), "none".to_string()]);
    }
}

#[tauri::command]
pub async fn get_forum_posts(
    channel_id: String,
    limit: Option<u32>,
    before: Option<i64>,
    state: State<'_, AppState>,
) -> Result<ForumPostsResponse, String> {
    let cap = limit.unwrap_or(20).min(100);
    let mut filter = serde_json::Map::new();
    filter.insert("kinds".to_string(), serde_json::json!([45001]));
    filter.insert("#h".to_string(), serde_json::json!([channel_id.clone()]));
    filter.insert("limit".to_string(), serde_json::json!(cap));
    if let Some(t) = before {
        filter.insert("until".to_string(), serde_json::json!(t));
    }

    let events = query_relay(&state, &[serde_json::Value::Object(filter)]).await?;
    let ids = events
        .iter()
        .map(|event| event.id.to_hex())
        .collect::<Vec<_>>();
    let edits = if ids.is_empty() {
        Vec::new()
    } else {
        query_relay(
            &state,
            &[serde_json::json!({ "kinds": [40003], "#e": ids })],
        )
        .await
        .unwrap_or_default()
    };
    let owner_pubkeys = fetch_agent_owner_pubkeys(&state, &events).await;
    let suppressed = link_preview_suppression_targets(&events, &edits, &owner_pubkeys);
    let messages: Vec<ForumMessageInfo> = events
        .iter()
        .map(|ev| {
            let mut message = forum_message_from_event(ev, &channel_id);
            apply_link_preview_suppression(&mut message.tags, &message.event_id, &suppressed);
            message
        })
        .collect();

    let next_cursor = messages.last().map(|m| m.created_at);
    Ok(ForumPostsResponse {
        messages,
        next_cursor,
    })
}

#[tauri::command]
pub async fn get_forum_thread(
    channel_id: String,
    event_id: String,
    limit: Option<u32>,
    cursor: Option<String>,
    state: State<'_, AppState>,
) -> Result<ForumThreadResponse, String> {
    let _ = (limit, cursor);
    // Two filters: the root event itself, plus any reply (kinds 9/45003)
    // that references it via #e.
    let events = query_relay(
        &state,
        &[
            serde_json::json!({ "ids": [event_id.clone()], "kinds": [9, 40002, 45001, 45003] }),
            serde_json::json!({
                "kinds": [9, 45003],
                "#e": [event_id.clone()],
                "#h": [channel_id.clone()],
            }),
        ],
    )
    .await?;
    let ids = events
        .iter()
        .map(|event| event.id.to_hex())
        .collect::<Vec<_>>();
    let edits = if ids.is_empty() {
        Vec::new()
    } else {
        query_relay(
            &state,
            &[serde_json::json!({ "kinds": [40003], "#e": ids })],
        )
        .await
        .unwrap_or_default()
    };
    let owner_pubkeys = fetch_agent_owner_pubkeys(&state, &events).await;
    let suppressed = link_preview_suppression_targets(&events, &edits, &owner_pubkeys);

    let mut root: Option<ForumMessageInfo> = None;
    let mut replies: Vec<ForumThreadReplyInfo> = Vec::new();
    for ev in &events {
        if ev.id.to_hex() == event_id {
            let mut message = forum_message_from_event(ev, &channel_id);
            apply_link_preview_suppression(&mut message.tags, &message.event_id, &suppressed);
            root = Some(message);
        } else if ev.kind.as_u16() as u32 != 40003 {
            let mut reply = forum_reply_from_event(ev, &channel_id, &event_id);
            apply_link_preview_suppression(&mut reply.tags, &reply.event_id, &suppressed);
            replies.push(reply);
        }
    }
    let total_replies = replies.len() as u32;

    let root = root.ok_or_else(|| "forum thread root event not found".to_string())?;
    Ok(ForumThreadResponse {
        root,
        replies,
        total_replies,
        next_cursor: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    fn signed_event(keys: &Keys, kind: u16, tags: Vec<Vec<String>>) -> nostr::Event {
        let tags = tags
            .into_iter()
            .map(nostr::Tag::parse)
            .collect::<Result<Vec<_>, _>>()
            .expect("valid tags");
        EventBuilder::new(Kind::Custom(kind), "body")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("event signs")
    }

    #[test]
    fn suppression_targets_accepts_author_and_verified_owner_only() {
        let author = Keys::generate();
        let owner = Keys::generate();
        let attacker = Keys::generate();
        let original = signed_event(&author, 9, Vec::new());
        let marker = vec!["link-preview".to_string(), "none".to_string()];
        let target = vec!["e".to_string(), original.id.to_hex()];
        let author_edit = signed_event(&author, 40003, vec![target.clone(), marker.clone()]);
        let owner_edit = signed_event(&owner, 40003, vec![target.clone(), marker.clone()]);
        let spoofed_edit = signed_event(&attacker, 40003, vec![target, marker]);
        let owners = std::collections::HashMap::from([(
            author.public_key().to_hex(),
            owner.public_key().to_hex(),
        )]);

        for edit in [&author_edit, &owner_edit] {
            assert!(link_preview_suppression_targets(
                std::slice::from_ref(&original),
                std::slice::from_ref(edit),
                &owners,
            )
            .contains(&original.id.to_hex()));
        }
        assert!(link_preview_suppression_targets(
            std::slice::from_ref(&original),
            std::slice::from_ref(&spoofed_edit),
            &owners,
        )
        .is_empty());
    }
}
