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

/// Fold the latest kind:40003 edit content into an original event.
///
/// A 40003 edit is valid when its signer is the original's author or the
/// author's verified agent owner. Among valid edits targeting the same
/// original, the one with the highest `created_at` wins. The edit's content
/// replaces the original's content; the edit's tags (excluding the routing
/// `e` tag and any `link-preview` marker, which are already handled
/// separately) are not folded — only the text changes.
///
/// `owner_pubkeys` maps **agent pubkey → owner pubkey** (built by
/// `fetch_agent_owner_pubkeys` from NIP-OA `auth` tags on kind:0 profiles).
/// The lookup is one-directional: an owner can edit their agent's posts, but
/// an agent cannot edit their owner's posts. When the original author is a
/// human (no NIP-OA profile), the map has no entry, so only the author
/// themself is authorized.
///
/// Returns the edited content when a valid edit exists, or `None` when the
/// original should be used as-is.
pub(super) fn fold_edit_content(
    original: &nostr::Event,
    edits: &[nostr::Event],
    owner_pubkeys: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let original_author = original.pubkey.to_hex();
    let original_id = original.id.to_hex();

    let mut best: Option<&nostr::Event> = None;
    for edit in edits {
        if edit.kind.as_u16() != 40003 {
            continue;
        }
        // Find the target event id from the e tag.
        let target_id = edit.tags.iter().find_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("e"))
                .then(|| values.get(1).map(String::as_str))
                .flatten()
        });
        if target_id != Some(original_id.as_str()) {
            continue;
        }
        // Authorization: signer must be the original author or the
        // author's verified agent owner.
        let signer = edit.pubkey.to_hex();
        let authorized = signer == original_author
            || owner_pubkeys.get(&original_author) == Some(&signer);
        if !authorized {
            continue;
        }
        // Latest edit wins.
        if best.is_none_or(|b| edit.created_at > b.created_at) {
            best = Some(edit);
        }
    }
    best.map(|e| e.content.clone())
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
            if let Some(edited_content) = fold_edit_content(ev, &edits, &owner_pubkeys) {
                message.content = edited_content;
            }
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
            if let Some(edited_content) = fold_edit_content(ev, &edits, &owner_pubkeys) {
                message.content = edited_content;
            }
            apply_link_preview_suppression(&mut message.tags, &message.event_id, &suppressed);
            root = Some(message);
        } else if ev.kind.as_u16() as u32 != 40003 {
            let mut reply = forum_reply_from_event(ev, &channel_id, &event_id);
            if let Some(edited_content) = fold_edit_content(ev, &edits, &owner_pubkeys) {
                reply.content = edited_content;
            }
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

    fn edit_event(
        keys: &Keys,
        content: &str,
        target_id: &str,
        created_at: nostr::Timestamp,
    ) -> nostr::Event {
        let tags = vec![nostr::Tag::parse(["e", target_id]).unwrap()];
        EventBuilder::new(Kind::Custom(40003), content)
            .tags(tags)
            .custom_created_at(created_at)
            .sign_with_keys(keys)
            .expect("edit signs")
    }

    #[test]
    fn fold_edit_content_applies_author_edit() {
        let author = Keys::generate();
        let original = signed_event(&author, 45001, Vec::new());
        let edit = edit_event(&author, "edited body", &original.id.to_hex(), nostr::Timestamp::now());
        let owners = std::collections::HashMap::new();
        let result = fold_edit_content(&original, std::slice::from_ref(&edit), &owners);
        assert_eq!(result.as_deref(), Some("edited body"));
    }

    #[test]
    fn fold_edit_content_applies_owner_edit() {
        let author = Keys::generate();
        let owner = Keys::generate();
        let original = signed_event(&author, 45001, Vec::new());
        let edit = edit_event(&owner, "owner-edited", &original.id.to_hex(), nostr::Timestamp::now());
        let owners = std::collections::HashMap::from([(
            author.public_key().to_hex(),
            owner.public_key().to_hex(),
        )]);
        let result = fold_edit_content(&original, std::slice::from_ref(&edit), &owners);
        assert_eq!(result.as_deref(), Some("owner-edited"));
    }

    #[test]
    fn fold_edit_content_rejects_unauthorized_signer() {
        let author = Keys::generate();
        let attacker = Keys::generate();
        let original = signed_event(&author, 45001, Vec::new());
        let edit = edit_event(&attacker, "hacked", &original.id.to_hex(), nostr::Timestamp::now());
        let owners = std::collections::HashMap::new();
        let result = fold_edit_content(&original, std::slice::from_ref(&edit), &owners);
        assert!(result.is_none(), "unauthorized edit must not apply");
    }

    #[test]
    fn fold_edit_content_latest_edit_wins() {
        let author = Keys::generate();
        let original = signed_event(&author, 45001, Vec::new());
        let older = edit_event(&author, "first", &original.id.to_hex(), nostr::Timestamp::from_secs(100));
        let newer = edit_event(&author, "second", &original.id.to_hex(), nostr::Timestamp::from_secs(200));
        let owners = std::collections::HashMap::new();
        let edits = [older, newer];
        let result = fold_edit_content(&original, &edits, &owners);
        assert_eq!(result.as_deref(), Some("second"));
    }

    #[test]
    fn fold_edit_content_ignores_edit_targeting_different_event() {
        let author = Keys::generate();
        let original = signed_event(&author, 45001, Vec::new());
        // Distinct created_at so `other` has a different event id from `original`.
        // Without this, both events share the same author, kind, content, and
        // timestamp, producing the same event id — the edit targeting `other`
        // would then match `original`, defeating the test's purpose.
        let other = EventBuilder::new(Kind::Custom(45001), "body")
            .custom_created_at(nostr::Timestamp::from_secs(1))
            .sign_with_keys(&author)
            .expect("event signs");
        let edit = edit_event(&author, "wrong target", &other.id.to_hex(), nostr::Timestamp::now());
        let owners = std::collections::HashMap::new();
        let result = fold_edit_content(&original, std::slice::from_ref(&edit), &owners);
        assert!(result.is_none(), "edit targeting a different event must not apply");
    }

    #[test]
    fn fold_edit_content_rejects_agent_editing_owner_post() {
        // The owner_pubkeys map is agent → owner, not bidirectional.
        // A human (owner) post must NOT be editable by their agent,
        // because the map has no entry keyed by the human's pubkey.
        let owner = Keys::generate();
        let agent = Keys::generate();
        let original = signed_event(&owner, 45001, Vec::new());
        let edit = edit_event(&agent, "agent-edited", &original.id.to_hex(), nostr::Timestamp::now());
        let owners = std::collections::HashMap::from([(
            agent.public_key().to_hex(),
            owner.public_key().to_hex(),
        )]);
        let result = fold_edit_content(&original, std::slice::from_ref(&edit), &owners);
        assert!(result.is_none(), "agent must not edit owner's post");
    }

    #[test]
    fn fold_edit_content_no_edits_returns_none() {
        let author = Keys::generate();
        let original = signed_event(&author, 45001, Vec::new());
        let owners = std::collections::HashMap::new();
        let result = fold_edit_content(&original, &[], &owners);
        assert!(result.is_none());
    }
}
