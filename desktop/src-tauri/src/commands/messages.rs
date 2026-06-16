use nostr::{Event, EventId, Keys, PublicKey};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    events,
    managed_agents::{find_managed_agent_mut, load_managed_agents, ManagedAgentRecord},
    models::{
        FeedItemInfo, FeedMeta, FeedResponse, FeedSections, ForumMessageInfo, ForumPostsResponse,
        ForumThreadReplyInfo, ForumThreadResponse, SearchResponse, SendChannelMessageResponse,
        ThreadSummary,
    },
    nostr_convert,
    relay::{query_relay, submit_event, submit_event_with_keys},
};

// ── Encrypted channel routing (serverless private channels + DMs) ────────────
//
// In serverless mode a "private" channel or DM is made private by encryption,
// not server access control. When a channel is encrypted, outbound messages
// are NIP-17 gift-wrapped to every member (see crate::encrypted) instead of
// published as plaintext kind:9 events.

/// Whether the given channel must be encrypted: serverless mode AND the channel
/// is a DM or has `private` visibility. Returns the member pubkeys (hex) to
/// encrypt to (always including the sender) when encrypted, else `None`.
async fn encrypted_recipients(
    state: &AppState,
    channel_id: &str,
) -> Result<Option<Vec<String>>, String> {
    if !state.is_serverless() {
        return Ok(None);
    }

    let meta = query_relay(
        state,
        &[serde_json::json!({"kinds":[39000],"#d":[channel_id],"limit":1})],
    )
    .await?;
    let Some(meta) = meta.first() else {
        return Ok(None);
    };
    let info = nostr_convert::channel_info_from_event(meta, None, None)?;
    let encrypted = info.channel_type == "dm" || info.visibility == "private";
    if !encrypted {
        return Ok(None);
    }

    // Members come from the kind:39002 list. 39002 is ADDRESSABLE: different
    // relays may hold different versions (and a multi-relay merge returns them
    // all), so pick the NEWEST by created_at — never an arbitrary `.first()`,
    // which could be a stale/empty copy and silently drop real members.
    // (No `limit:1`: we want every relay's copy so the merge can pick the
    // latest; capping at 1 per relay is fine but selection must be by recency.)
    let member_events = query_relay(
        state,
        &[serde_json::json!({"kinds":[39002],"#d":[channel_id]})],
    )
    .await?;
    let newest = member_events
        .iter()
        .max_by_key(|ev| ev.created_at.as_secs());
    let mut members: Vec<String> = newest
        .map(|ev| {
            ev.tags
                .iter()
                .filter_map(|t| {
                    let s = t.as_slice();
                    if s.len() >= 2 && s[0] == "p" {
                        Some(s[1].to_ascii_lowercase())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let me = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    // Fail LOUDLY rather than silently encrypting to only ourselves. An
    // encrypted channel with no other members means the 39002 lookup came back
    // empty (relay hiccup / propagation lag) — encrypting to just `me` would
    // make the message undeliverable to everyone else (e.g. the agent) with no
    // visible error. Surface it so the caller can retry instead of dropping it.
    let others: Vec<&String> = members.iter().filter(|p| *p != &me).collect();
    if others.is_empty() {
        return Err(format!(
            "could not resolve members for encrypted channel {channel_id} \
             (kind:39002 lookup returned no other participants — relay may be \
             lagging; try again)"
        ));
    }

    if !members.contains(&me) {
        members.push(me);
    }
    members.sort();
    members.dedup();
    Ok(Some(members))
}

/// Build a kind:9 message rumor, gift-wrap it to every member, and publish all
/// wraps. Returns the inner rumor's event id (stable across recipients) so the
/// UI can reference the logical message.
async fn send_encrypted_message(
    state: &AppState,
    channel_id: Uuid,
    content: &str,
    thread_ref: Option<&events::ThreadRef>,
    mention_refs: &[&str],
    media: &[Vec<String>],
    members_hex: &[String],
) -> Result<String, String> {
    let keys = {
        let guard = state.keys.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };

    // The rumor is a normal kind:9 channel message (with the `h` tag and any
    // NIP-10 thread tags), so once unwrapped it renders through the standard
    // message pipeline — including threaded replies.
    let builder = events::build_message(
        channel_id,
        content,
        thread_ref,
        mention_refs,
        media,
        &[],
        &[],
    )?;
    let rumor = builder.build(keys.public_key());
    let rumor_id = rumor.id.map(|id| id.to_hex()).unwrap_or_default();

    let mut recipients = Vec::with_capacity(members_hex.len());
    for hex in members_hex {
        let pk = nostr::PublicKey::from_hex(hex)
            .map_err(|e| format!("invalid member pubkey {hex}: {e}"))?;
        recipients.push(pk);
    }

    let wraps = crate::encrypted::build_gift_wraps(&keys, rumor, &recipients).await?;

    let relay_urls = crate::relay::relay_ws_urls_with_override(state);
    let mut published = 0;
    let mut last_err = None;
    for wrap in &wraps {
        match crate::ws_relay::publish_signed_event_ws(state, wrap, &keys, &relay_urls).await {
            Ok(()) => published += 1,
            Err(e) => last_err = Some(e),
        }
    }
    if published == 0 {
        return Err(last_err.unwrap_or_else(|| "failed to publish any gift wrap".to_string()));
    }
    Ok(rumor_id)
}

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

// Tauri commands take flat args (each maps to a JS object field), so a high
// arg count is idiomatic here rather than a struct.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn send_channel_message(
    channel_id: String,
    content: String,
    parent_event_id: Option<String>,
    media_tags: Option<Vec<Vec<String>>>,
    emoji_tags: Option<Vec<Vec<String>>>,
    mention_tags: Option<Vec<Vec<String>>>,
    mention_pubkeys: Option<Vec<String>>,
    kind: Option<u32>,
    // Thread root for encrypted replies. In an encrypted channel the parent is
    // a gift-wrapped rumor not queryable in plaintext, so the caller resolves
    // the root locally (from decrypted messages) and passes it here. Ignored on
    // the plaintext path (which resolves the root via `resolve_thread_ref`).
    root_event_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<SendChannelMessageResponse, String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let mentions = mention_pubkeys.unwrap_or_default();
    let mention_refs: Vec<&str> = mentions.iter().map(|s| s.as_str()).collect();
    let media = media_tags.unwrap_or_default();
    let emoji = emoji_tags.unwrap_or_default();
    let mention_refs_only = mention_tags.unwrap_or_default();
    let kind_num = kind.unwrap_or(buzz_core_pkg::kind::KIND_STREAM_MESSAGE);

    // Encrypted serverless channels (DM / private): gift-wrap to all members.
    // Only plain messages (kind 9) are encrypted; forum posts/comments fall
    // through to the plaintext path (private forums aren't a serverless model).
    // Replies ARE encrypted too: the NIP-10 thread tags live INSIDE the rumor
    // (the encrypted inner event), so threading is preserved without leaking
    // the reply as plaintext to the relays.
    if kind_num == buzz_core_pkg::kind::KIND_STREAM_MESSAGE {
        let recip = encrypted_recipients(&state, &channel_id).await?;
        eprintln!(
            "sprout-desktop: [serverless] send to {channel_id}: encrypted_recipients = {:?}",
            recip.as_ref().map(|m| m.len())
        );
        if let Some(members) = recip {
            // Build the in-rumor thread ref from caller-supplied ids (no relay
            // lookup — the parent rumor isn't stored plaintext on the relay).
            let thread_ref = match &parent_event_id {
                Some(parent) => {
                    let parent_eid = EventId::from_hex(parent)
                        .map_err(|e| format!("invalid parent event ID: {e}"))?;
                    let root_eid = match &root_event_id {
                        Some(root) if root != parent => EventId::from_hex(root)
                            .map_err(|e| format!("invalid root event ID: {e}"))?,
                        _ => parent_eid,
                    };
                    Some(events::ThreadRef {
                        root_event_id: root_eid,
                        parent_event_id: parent_eid,
                    })
                }
                None => None,
            };
            let depth = match (&parent_event_id, &root_event_id) {
                (None, _) => 0,
                (Some(p), Some(r)) if p == r => 1,
                (Some(_), Some(_)) => 2,
                (Some(_), None) => 1,
            };
            let rumor_id = send_encrypted_message(
                &state,
                channel_uuid,
                content.trim(),
                thread_ref.as_ref(),
                &mention_refs,
                &media,
                &members,
            )
            .await?;
            return Ok(SendChannelMessageResponse {
                event_id: rumor_id,
                root_event_id: thread_ref.as_ref().map(|t| t.root_event_id.to_hex()),
                parent_event_id: parent_event_id.clone(),
                depth,
                created_at: chrono::Utc::now().timestamp(),
            });
        }
    }

    let mut resolved_root: Option<String> = None;

    let builder = match kind_num {
        buzz_core_pkg::kind::KIND_FORUM_POST => events::build_forum_post(
            channel_uuid,
            content.trim(),
            &mention_refs,
            &media,
            &mention_refs_only,
        )?,
        buzz_core_pkg::kind::KIND_FORUM_COMMENT => {
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
                &mention_refs_only,
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
                &mention_refs_only,
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

fn event_has_client_marker(event: &Event, marker: &str) -> bool {
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.len() >= 2 && parts[0] == "client" && parts[1] == marker
    })
}

async fn find_managed_agent_channel_message_by_marker(
    state: &AppState,
    agent_pubkey: &str,
    channel_id: &str,
    marker: &str,
) -> Result<Option<Event>, String> {
    let mut until: Option<u64> = None;

    for _ in 0..10 {
        let mut filter = serde_json::json!({
            "authors": [agent_pubkey],
            "kinds": [buzz_core_pkg::kind::KIND_STREAM_MESSAGE],
            "#h": [channel_id],
            "limit": 500,
        });
        if let Some(until) = until {
            filter["until"] = serde_json::json!(until);
        }

        let events = query_relay(state, &[filter]).await?;
        if let Some(existing) = events
            .iter()
            .find(|event| event_has_client_marker(event, marker))
        {
            return Ok(Some(existing.clone()));
        }

        if events.len() < 500 {
            break;
        }
        until = events
            .iter()
            .map(|event| event.created_at.as_secs())
            .min()
            .map(|timestamp| timestamp.saturating_sub(1));
        if until.is_none() {
            break;
        }
    }

    Ok(None)
}

fn stored_managed_agent_auth_tag(auth_tag: Option<&str>) -> Option<String> {
    auth_tag
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn legacy_managed_agent_auth_tag(
    owner_keys: &Keys,
    agent_pubkey: &PublicKey,
) -> Result<Option<String>, String> {
    if owner_keys.public_key() == *agent_pubkey {
        return Ok(None);
    }

    buzz_sdk_pkg::nip_oa::compute_auth_tag(owner_keys, agent_pubkey, "")
        .map(Some)
        .map_err(|error| format!("failed to compute managed agent auth tag: {error}"))
}

fn managed_agent_submission_auth_tag(
    record: &ManagedAgentRecord,
    state: &AppState,
    agent_pubkey: &PublicKey,
) -> Result<Option<String>, String> {
    if let Some(auth_tag) = stored_managed_agent_auth_tag(record.auth_tag.as_deref()) {
        return Ok(Some(auth_tag));
    }

    let owner_keys = state.keys.lock().map_err(|error| error.to_string())?;
    legacy_managed_agent_auth_tag(&owner_keys, agent_pubkey)
}

#[tauri::command]
pub async fn send_managed_agent_channel_message(
    agent_pubkey: String,
    channel_id: String,
    content: String,
    marker: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SendChannelMessageResponse, String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("message content is required".into());
    }
    let marker = marker
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let requested_pubkey = agent_pubkey.trim().to_ascii_lowercase();

    let record = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(&app)?;
        find_managed_agent_mut(&mut records, &requested_pubkey)?.clone()
    };

    let keys = Keys::parse(record.private_key_nsec.trim())
        .map_err(|error| format!("failed to parse managed agent key: {error}"))?;
    let key_pubkey = keys.public_key().to_hex();
    if key_pubkey != record.pubkey.to_ascii_lowercase() {
        return Err(format!(
            "managed agent key does not match stored pubkey {}",
            record.pubkey
        ));
    }
    let submission_auth_tag =
        managed_agent_submission_auth_tag(&record, &state, &keys.public_key())?;

    if let Some(marker) = marker.as_deref() {
        if let Some(existing) = find_managed_agent_channel_message_by_marker(
            &state,
            &record.pubkey,
            &channel_id,
            marker,
        )
        .await?
        {
            return Ok(SendChannelMessageResponse {
                event_id: existing.id.to_hex(),
                parent_event_id: None,
                root_event_id: None,
                depth: 0,
                created_at: existing.created_at.as_secs() as i64,
            });
        }
    }

    let client_tags = marker
        .as_deref()
        .map(|marker| vec![vec!["client".to_string(), marker.to_string()]])
        .unwrap_or_default();
    let builder = events::build_message_with_client_tags(
        channel_uuid,
        trimmed,
        None,
        &[],
        &[],
        &[],
        &[],
        &client_tags,
    )?;
    let result =
        submit_event_with_keys(builder, &state, &keys, submission_auth_tag.as_deref()).await?;

    Ok(SendChannelMessageResponse {
        event_id: result.event_id,
        parent_event_id: None,
        root_event_id: None,
        depth: 0,
        created_at: chrono::Utc::now().timestamp(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_managed_agent_auth_tag_trims_blank_values() {
        assert_eq!(
            stored_managed_agent_auth_tag(Some("  [\"auth\",\"owner\",\"\",\"sig\"]  ")),
            Some("[\"auth\",\"owner\",\"\",\"sig\"]".to_string())
        );
        assert_eq!(stored_managed_agent_auth_tag(Some("   ")), None);
        assert_eq!(stored_managed_agent_auth_tag(None), None);
    }

    #[test]
    fn legacy_managed_agent_auth_tag_verifies_for_agent_pubkey() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();

        let tag = legacy_managed_agent_auth_tag(&owner_keys, &agent_keys.public_key())
            .expect("legacy auth tag should compute")
            .expect("legacy auth tag should be present");

        let owner = buzz_sdk_pkg::nip_oa::verify_auth_tag(&tag, &agent_keys.public_key())
            .expect("legacy auth tag should verify");
        assert_eq!(owner, owner_keys.public_key());
    }

    #[test]
    fn legacy_managed_agent_auth_tag_skips_self_attestation() {
        let owner_keys = Keys::generate();

        let tag = legacy_managed_agent_auth_tag(&owner_keys, &owner_keys.public_key())
            .expect("self-attestation should be skipped");

        assert_eq!(tag, None);
    }
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
        Some(url) => buzz_sdk_pkg::build_custom_emoji_reaction(target_eid, emoji.trim(), &url)
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
pub async fn delete_message(
    channel_id: String,
    event_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let target_eid = EventId::from_hex(&event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    let builder = events::build_delete_compat(channel_uuid, target_eid)?;
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
