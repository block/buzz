use nostr::EventId;
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    models::{
        FeedItemInfo, FeedMeta, FeedResponse, FeedSections, ForumMessageInfo, ForumPostsResponse,
        ForumThreadReplyInfo, ForumThreadResponse, SearchResponse, SendChannelMessageResponse,
        ThreadSummary,
    },
    nostr_convert,
    relay::{query_relay, submit_event},
};

// ── Reads (pure-nostr) ──────────────────────────────────────────────────────

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
        "kinds": [9, 40002, 1, 45001, 45003],
        "#p": [my_pubkey],
        "limit": cap,
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

    let mentions: Vec<FeedItemInfo> = mention_events
        .iter()
        .map(|ev| feed_item_from_event(ev, "mentions"))
        .collect();
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

#[tauri::command]
pub async fn search_messages(
    q: String,
    limit: Option<u32>,
    channel_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<SearchResponse, String> {
    let cap = limit.unwrap_or(20).min(100);
    let mut filter = serde_json::Map::new();
    filter.insert(
        "kinds".to_string(),
        serde_json::json!([9, 40002, 45001, 45003]),
    );
    filter.insert("search".to_string(), serde_json::json!(q.trim()));
    filter.insert("limit".to_string(), serde_json::json!(cap));
    if let Some(cid) = channel_id {
        filter.insert("#h".to_string(), serde_json::json!([cid]));
    }

    let events = query_relay(&state, &[serde_json::Value::Object(filter)]).await?;
    Ok(nostr_convert::search_response_from_events(&events))
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
    let messages: Vec<ForumMessageInfo> = events
        .iter()
        .map(|ev| forum_message_from_event(ev, &channel_id))
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

    let mut root: Option<ForumMessageInfo> = None;
    let mut replies: Vec<ForumThreadReplyInfo> = Vec::new();
    for ev in &events {
        if ev.id.to_hex() == event_id {
            root = Some(forum_message_from_event(ev, &channel_id));
        } else {
            replies.push(forum_reply_from_event(ev, &channel_id, &event_id));
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

#[tauri::command]
pub async fn get_event(event_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "ids": [event_id],
            "kinds": [0, 1, 3, 5, 7, 9, 30078, 40002, 40003, 40008, 40099, 40100, 45001, 45003],
            "limit": 1
        })],
    )
    .await?;

    let ev = events
        .first()
        .ok_or_else(|| "event not found".to_string())?;
    serde_json::to_string(ev).map_err(|e| format!("serialize event: {e}"))
}

// ── Writes ──────────────────────────────────────────────────────────────────

/// Fetch a parent event and extract the thread root from its NIP-10 e-tags.
async fn resolve_thread_ref(
    parent_event_id: &str,
    state: &AppState,
) -> Result<events::ThreadRef, String> {
    let parent_eid =
        EventId::from_hex(parent_event_id).map_err(|e| format!("invalid parent event ID: {e}"))?;

    let evs = query_relay(
        state,
        &[serde_json::json!({
            "ids": [parent_event_id],
            "kinds": [9, 40002, 45001, 45003],
            "limit": 1
        })],
    )
    .await?;

    let parent = evs
        .first()
        .ok_or_else(|| "parent event not found".to_string())?;

    // Walk tags looking for NIP-10 root/reply markers.
    let (mut root, mut reply) = (None, None);
    for tag in parent.tags.iter() {
        let s = tag.as_slice();
        if s.len() >= 4 && s[0] == "e" {
            match s[3].as_str() {
                "root" => root = Some(s[1].clone()),
                "reply" => reply = Some(s[1].clone()),
                _ => {}
            }
        }
    }
    let root_hex = root.or(reply);

    let root_eid = match root_hex {
        Some(hex) if hex != parent_event_id => {
            EventId::from_hex(&hex).map_err(|e| format!("invalid root event ID: {e}"))?
        }
        _ => parent_eid,
    };

    Ok(events::ThreadRef {
        root_event_id: root_eid,
        parent_event_id: parent_eid,
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_channel_message(
    channel_id: String,
    content: String,
    parent_event_id: Option<String>,
    media_tags: Option<Vec<Vec<String>>>,
    emoji_tags: Option<Vec<Vec<String>>>,
    mention_pubkeys: Option<Vec<String>>,
    kind: Option<u32>,
    state: State<'_, AppState>,
) -> Result<SendChannelMessageResponse, String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let mentions = mention_pubkeys.unwrap_or_default();
    let mention_refs: Vec<&str> = mentions.iter().map(|s| s.as_str()).collect();
    let media = media_tags.unwrap_or_default();
    let emoji = emoji_tags.unwrap_or_default();
    let kind_num = kind.unwrap_or(sprout_core::kind::KIND_STREAM_MESSAGE);

    let mut resolved_root: Option<String> = None;

    let builder = match kind_num {
        sprout_core::kind::KIND_FORUM_POST => {
            events::build_forum_post(channel_uuid, content.trim(), &mention_refs, &media)?
        }
        sprout_core::kind::KIND_FORUM_COMMENT => {
            let parent_id = parent_event_id
                .as_deref()
                .ok_or("forum comment requires parent_event_id")?;
            let thread_ref = resolve_thread_ref(parent_id, &state).await?;
            resolved_root = Some(thread_ref.root_event_id.to_hex());
            events::build_forum_comment(
                channel_uuid,
                content.trim(),
                &thread_ref,
                &mention_refs,
                &media,
            )?
        }
        _ => {
            let thread_ref = match parent_event_id.as_deref() {
                Some(pid) => {
                    let tr = resolve_thread_ref(pid, &state).await?;
                    resolved_root = Some(tr.root_event_id.to_hex());
                    Some(tr)
                }
                None => None,
            };
            events::build_message(
                channel_uuid,
                content.trim(),
                thread_ref.as_ref(),
                &mention_refs,
                &media,
                &emoji,
            )?
        }
    };

    let result = submit_event(builder, &state).await?;

    let depth = match (&parent_event_id, &resolved_root) {
        (None, _) => 0,
        (Some(pid), Some(root)) if pid == root => 1,
        (Some(_), Some(_)) => 2,
        (Some(_), None) => 1,
    };

    Ok(SendChannelMessageResponse {
        event_id: result.event_id,
        root_event_id: resolved_root,
        parent_event_id,
        depth,
        created_at: chrono::Utc::now().timestamp(),
    })
}

#[tauri::command]
pub async fn add_reaction(
    event_id: String,
    emoji: String,
    emoji_url: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let target_eid = EventId::from_hex(&event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    let builder = match emoji_url {
        // Custom-emoji reaction (NIP-30): kind:7 with `:shortcode:` content and
        // an `["emoji", shortcode, url]` tag. Delegates to the SDK builder so
        // shortcode normalization + validation match the relay exactly.
        Some(url) => sprout_sdk::build_custom_emoji_reaction(target_eid, emoji.trim(), &url)
            .map_err(|e| format!("invalid custom emoji reaction: {e}"))?,
        None => events::build_reaction(target_eid, emoji.trim())?,
    };
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn remove_reaction(
    event_id: String,
    emoji: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Find our own kind:7 reaction event referencing the target.
    let my_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };
    let target = event_id.trim();
    let trimmed_emoji = emoji.trim();

    let reactions = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [7],
            "#e": [target],
            "authors": [my_pubkey],
        })],
    )
    .await?;

    let reaction_event = reactions
        .iter()
        .find(|ev| ev.content.trim() == trimmed_emoji)
        .ok_or("could not find your reaction event for this emoji")?;

    let builder = events::build_remove_reaction(reaction_event.id)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn edit_message(
    channel_id: String,
    event_id: String,
    content: String,
    media_tags: Vec<Vec<String>>,
    emoji_tags: Option<Vec<Vec<String>>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let target_eid = EventId::from_hex(&event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    let trimmed = content.trim();
    // Empty text is allowed when the edit still carries imeta attachments
    // (a media-only edit). Reject only when both are empty.
    if trimmed.is_empty() && media_tags.is_empty() {
        return Err("edit must have content or attachments".into());
    }
    let emoji = emoji_tags.unwrap_or_default();
    let builder =
        events::build_message_edit(channel_uuid, target_eid, trimmed, &media_tags, &emoji)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn delete_message(event_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let target_eid = EventId::from_hex(&event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    let builder = events::build_delete_compat(target_eid)?;
    submit_event(builder, &state).await?;
    Ok(())
}

// ── Local helpers ───────────────────────────────────────────────────────────

fn channel_id_from_tags(ev: &nostr::Event) -> Option<String> {
    ev.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.len() >= 2 && s[0] == "h" {
            Some(s[1].clone())
        } else {
            None
        }
    })
}

fn tags_to_vec(ev: &nostr::Event) -> Vec<Vec<String>> {
    ev.tags.iter().map(|t| t.as_slice().to_vec()).collect()
}

fn feed_item_from_event(ev: &nostr::Event, category: &str) -> FeedItemInfo {
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

fn forum_message_from_event(ev: &nostr::Event, channel_id: &str) -> ForumMessageInfo {
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

fn forum_reply_from_event(
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
