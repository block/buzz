use nostr::{Event, EventId, Keys, PublicKey};
use tauri::{AppHandle, State};

mod forum;

use forum::{forum_message_from_event, forum_reply_from_event};

use crate::{
    app_state::AppState,
    events,
    managed_agents::{find_managed_agent_mut, load_managed_agents, ManagedAgentRecord},
    models::{
        FeedItemInfo, FeedMeta, FeedResponse, FeedSections, ForumMessageInfo, ForumPostsResponse,
        ForumThreadReplyInfo, ForumThreadResponse, SearchResponse, SendChannelMessageResponse,
        ThreadRepliesResponse,
    },
    nostr_convert,
    relay::{query_relay, submit_event, submit_event_with_keys},
};

// ── Reads (pure-nostr) ──────────────────────────────────────────────────────

/// Timeline content kinds — the message/channel-event kinds that make up a
/// channel timeline and a thread's replies. Used to build relay `/query`
/// filters for the keyset readers below. None of these are in
/// `P_GATED_KINDS`, so a filter carrying them clears the bridge p-gate
/// (`p_gated_filters_authorized`) without a `#p` tag — load-bearing for the
/// thread-subtree read, whose relay routing keys off `#e`+`depth_limit` (not
/// kind) but still passes through the p-gate before it runs.
const TIMELINE_KINDS: [u32; 11] = [
    9,
    40002,
    40008,
    40099,
    43001,
    43002,
    43003,
    43004,
    43005,
    43006,
    buzz_core_pkg::kind::KIND_HUDDLE_STARTED,
];

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

fn build_search_messages_filter(q: &str, cap: u32, channel_id: Option<&str>) -> serde_json::Value {
    let mut filter = serde_json::Map::new();
    filter.insert(
        "kinds".to_string(),
        serde_json::json!([9, 40002, 45001, 45003]),
    );
    filter.insert("search".to_string(), serde_json::json!(q.trim()));
    // The desktop topbar is a typeahead surface. This bridge-only extension is
    // consumed before nostr::Filter parsing on the relay, so general WS/NIP-50
    // search remains word/lexeme-based.
    filter.insert("search_mode".to_string(), serde_json::json!("prefix"));
    filter.insert("limit".to_string(), serde_json::json!(cap));
    if let Some(cid) = channel_id {
        filter.insert("#h".to_string(), serde_json::json!([cid]));
    }
    serde_json::Value::Object(filter)
}

#[tauri::command]
pub async fn search_messages(
    q: String,
    limit: Option<u32>,
    channel_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<SearchResponse, String> {
    let cap = limit.unwrap_or(20).min(100);
    let filter = build_search_messages_filter(&q, cap, channel_id.as_deref());

    let events = query_relay(&state, &[filter]).await?;
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

/// Fetch the full reply subtree under a thread root, server-side.
///
/// Unlike the channel timeline (which the desktop assembles from its local
/// cache by grouping on `e`-root tags), this walks `thread_metadata` on the
/// relay via `get_thread_replies`, so a thread renders complete even when its
/// replies fell outside the channel cold-load window. Results are chronological
/// (oldest first) and are the *replies* under the root (depth >= 1); the root
/// event itself is NOT returned (the relay query keys on `root_event_id`, and a
/// root row has no `root_event_id`). Callers already hold the root — it is the
/// open thread head — so this closes the descendant gap without re-fetching it.
///
/// Paging is forward keyset on `(created_at, event_id)`: pass the `next_cursor`
/// from a previous page back as `cursor` to fetch the next batch. The event-id
/// tiebreak is required because replies routinely share a `created_at` second;
/// a timestamp-only cursor would skip every tied reply past the page limit.
/// `next_cursor` is `Some` only when a full page was returned.
#[tauri::command]
pub async fn get_thread_replies(
    root_event_id: String,
    channel_id: Option<String>,
    limit: Option<u32>,
    depth_limit: Option<u32>,
    cursor: Option<crate::models::ThreadCursor>,
    state: State<'_, AppState>,
) -> Result<ThreadRepliesResponse, String> {
    let cap = limit.unwrap_or(200).min(500);
    let filter = build_thread_replies_filter(
        &root_event_id,
        channel_id.as_deref(),
        depth_limit.unwrap_or(64),
        cap,
        cursor.as_ref(),
    );

    let events = query_relay(&state, &[serde_json::Value::Object(filter)]).await?;

    // A full page implies there may be more; hand back the last event's
    // composite key as the next cursor (the DB returns replies strictly after
    // it, tiebroken by event_id so same-second replies are not skipped).
    let next_cursor = if events.len() as u32 >= cap {
        events.last().map(|ev| crate::models::ThreadCursor {
            created_at: ev.created_at.as_secs() as i64,
            event_id: ev.id.to_hex(),
        })
    } else {
        None
    };

    let event_values: Vec<serde_json::Value> = events
        .iter()
        .filter_map(|ev| serde_json::to_value(ev).ok())
        .collect();

    Ok(ThreadRepliesResponse {
        events: event_values,
        next_cursor,
    })
}

/// Build the relay `/query` filter for the server-side thread-subtree read.
///
/// The relay routes a filter to `get_thread_replies` purely off a single `#e`
/// (root) tag plus `depth_limit` — kind is NOT part of that routing or the
/// underlying DB query (it keys on `root_event_id`). Yet `kinds` is still
/// required here: the bridge runs the p-gate (`p_gated_filters_authorized`) on
/// every filter *before* routing, and a kindless filter "could match" a p-gated
/// kind, so the gate demands a `#p` tag we don't send -> HTTP 403
/// `restricted: p-gated kinds require #p tag`, before the thread query ever
/// runs. Carrying non-p-gated [`TIMELINE_KINDS`] makes the filter provably
/// un-p-gated so it clears the gate. `build_channel_messages_before_filter` is
/// the sibling that already does this, which is why the dense-second channel
/// pager was never gated and this reader was. Extracted so a unit test can pin
/// that `kinds` is present (the e2e mock does not model p-gating, so only a
/// unit test guards this contract).
fn build_thread_replies_filter(
    root_event_id: &str,
    channel_id: Option<&str>,
    depth_limit: u32,
    cap: u32,
    cursor: Option<&crate::models::ThreadCursor>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut filter = serde_json::Map::new();
    filter.insert("#e".to_string(), serde_json::json!([root_event_id]));
    filter.insert("kinds".to_string(), serde_json::json!(TIMELINE_KINDS));
    // depth_limit is what activates the thread-subtree bridge path; the caller
    // defaults it to a deep-but-bounded value so nested replies aren't dropped.
    filter.insert("depth_limit".to_string(), serde_json::json!(depth_limit));
    filter.insert("limit".to_string(), serde_json::json!(cap));
    if let Some(cid) = channel_id {
        filter.insert("#h".to_string(), serde_json::json!([cid]));
    }
    if let Some(c) = cursor {
        filter.insert("thread_cursor".to_string(), serde_json::json!(c.created_at));
        filter.insert(
            "thread_cursor_id".to_string(),
            serde_json::json!(c.event_id),
        );
    }
    filter
}

/// Build the relay `/query` filter for one keyset page of top-level channel
/// history strictly older than `(before, before_id)`. Extracted so a unit test
/// can pin the tiebreak field: it MUST be `before_id` (what the relay's
/// `extract_before_id` reads), else the keyset degrades to a bare `until`.
fn build_channel_messages_before_filter(
    channel_id: &str,
    before: i64,
    before_id: Option<&str>,
    cap: u32,
) -> serde_json::Map<String, serde_json::Value> {
    // Timeline content kinds — mirror the WS history filter so the keyset page
    // and the WS page select the same rows. Top-level filtering is enforced by
    // the relay's thread_metadata join for this channel scope.
    let mut filter = serde_json::Map::new();
    filter.insert("#h".to_string(), serde_json::json!([channel_id]));
    filter.insert("kinds".to_string(), serde_json::json!(TIMELINE_KINDS));
    filter.insert("until".to_string(), serde_json::json!(before));
    filter.insert("limit".to_string(), serde_json::json!(cap));
    // `before_id` is the bridge extension field for the composite tiebreak
    // (relay `extract_before_id`); it requires `until` to be set alongside it.
    if let Some(id) = before_id {
        filter.insert("before_id".to_string(), serde_json::json!(id));
    }
    filter
}

/// Fetch one keyset page of top-level channel history strictly *older* than a
/// cursor, server-side via the bridge composite cursor.
///
/// The desktop timeline normally pages history over WS `REQ` with a bare `until`
/// (`created_at`) cursor. That cursor cannot advance past a single `created_at`
/// second that holds more messages than one page: `until` keeps returning the
/// same newest slice of that second and history behind it is unreachable. This
/// command uses the relay's `(created_at, event_id)` keyset (`until` +
/// `before_id`), which advances within a tied second via `id > before_id` under
/// the relay's `created_at DESC, id ASC` order — the escape hatch for that wall.
///
/// `before` is the cursor's `created_at` (Unix seconds); `before_id` is the hex
/// id of the last (oldest) event already loaded at that second, so the page
/// returned is strictly older. `next_cursor` is the last (oldest) returned
/// event's composite key when a full page came back, else `None`.
#[tauri::command]
pub async fn get_channel_messages_before(
    channel_id: String,
    before: i64,
    before_id: Option<String>,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<crate::models::ChannelMessagesPageResponse, String> {
    let cap = limit.unwrap_or(200).min(500);
    let filter =
        build_channel_messages_before_filter(&channel_id, before, before_id.as_deref(), cap);

    let events = query_relay(&state, &[serde_json::Value::Object(filter)]).await?;

    // Relay order is created_at DESC, id ASC — the last event is the oldest, so
    // it is the cursor for the next (older) page when a full page returned.
    let next_cursor = if events.len() as u32 >= cap {
        events.last().map(|ev| crate::models::ChannelPageCursor {
            created_at: ev.created_at.as_secs() as i64,
            event_id: ev.id.to_hex(),
        })
    } else {
        None
    };

    let event_values: Vec<serde_json::Value> = events
        .iter()
        .filter_map(|ev| serde_json::to_value(ev).ok())
        .collect();

    Ok(crate::models::ChannelMessagesPageResponse {
        events: event_values,
        next_cursor,
    })
}

#[tauri::command]
pub async fn get_event(event_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "ids": [event_id],
            "kinds": [0, 1, 3, 5, 7, 9, 30078, 40002, 40003, 40008, 40099, 40100, 45001, 45003, buzz_core_pkg::kind::KIND_HUDDLE_STARTED],
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
            "kinds": [9, 40002, 45001, 45003, buzz_core_pkg::kind::KIND_HUDDLE_STARTED],
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
    mention_tags: Option<Vec<Vec<String>>>,
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
    let mention_refs_only = mention_tags.unwrap_or_default();
    let kind_num = kind.unwrap_or(buzz_core_pkg::kind::KIND_STREAM_MESSAGE);

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
    agent_pubkey: Option<&str>,
    channel_id: &str,
    marker: &str,
) -> Result<Option<Event>, String> {
    let author = agent_pubkey
        .map(str::trim)
        .filter(|pubkey| !pubkey.is_empty())
        .map(str::to_ascii_lowercase);

    let mut until: Option<u64> = None;

    for _ in 0..10 {
        let mut filter = serde_json::json!({
            "kinds": [buzz_core_pkg::kind::KIND_STREAM_MESSAGE],
            "#h": [channel_id],
            "limit": 500,
        });
        if let Some(author) = author.as_deref() {
            filter["authors"] = serde_json::json!([author]);
        }
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

fn marker_author_for_scope<'a>(
    marker_scope: Option<&str>,
    agent_pubkey: &'a str,
) -> Option<&'a str> {
    match marker_scope {
        Some("channel") => None,
        _ => Some(agent_pubkey),
    }
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
    marker_scope: Option<String>,
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
            marker_author_for_scope(marker_scope.as_deref(), &record.pubkey),
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
    fn marker_author_scope_defaults_to_agent() {
        assert_eq!(
            marker_author_for_scope(None, "agent-pubkey"),
            Some("agent-pubkey")
        );
        assert_eq!(
            marker_author_for_scope(Some("agent"), "agent-pubkey"),
            Some("agent-pubkey")
        );
        assert_eq!(
            marker_author_for_scope(Some("unknown"), "agent-pubkey"),
            Some("agent-pubkey")
        );
    }

    #[test]
    fn marker_author_scope_can_dedupe_across_channel() {
        assert_eq!(
            marker_author_for_scope(Some("channel"), "agent-pubkey"),
            None
        );
    }

    #[test]
    fn search_messages_filter_requests_prefix_mode_for_topbar_typeahead() {
        let filter = build_search_messages_filter("  pro  ", 12, Some("channel-1"));

        assert_eq!(filter["search"], serde_json::json!("pro"));
        assert_eq!(filter["search_mode"], serde_json::json!("prefix"));
        assert_eq!(filter["limit"], serde_json::json!(12));
        assert_eq!(filter["#h"], serde_json::json!(["channel-1"]));
    }

    #[test]
    fn channel_messages_before_filter_sends_before_id_the_relay_reads() {
        // The relay bridge's `extract_before_id` reads the composite tiebreak
        // from `before_id`. If this filter sent the id under any other key (an
        // earlier cut used `n`), the relay would silently drop the tiebreak and
        // the dense-second keyset would degrade to a bare inclusive `until` —
        // re-returning the same page forever. Pin the field name here so the
        // client/relay contract can't drift without a red test (the Playwright
        // mock reimplements the keyset in JS and cannot catch this).
        let filter =
            build_channel_messages_before_filter("channel-1", 1_700_000_000, Some("ab"), 200);

        assert_eq!(filter["until"], serde_json::json!(1_700_000_000));
        assert_eq!(filter["before_id"], serde_json::json!("ab"));
        assert_eq!(filter["limit"], serde_json::json!(200));
        assert_eq!(filter["#h"], serde_json::json!(["channel-1"]));
        assert!(
            !filter.contains_key("n"),
            "tiebreak must be `before_id`, not the `n` alias the relay ignores"
        );
    }

    #[test]
    fn thread_replies_filter_carries_non_p_gated_kinds_to_clear_the_gate() {
        // The relay bridge p-gates EVERY filter before routing
        // (`p_gated_filters_authorized`): a kindless filter "could match" a
        // p-gated kind, so it demands a `#p` tag we don't send -> HTTP 403,
        // before the thread-subtree query runs. The headline Lane-1 fix
        // (`useThreadReplies` closing the descendant gap) then fails on every
        // call against a real relay. So the thread filter MUST carry `kinds`,
        // and every kind MUST be non-p-gated (else the gate still fires). The
        // Playwright mock does not model p-gating, so this unit test is the
        // only guard against the client/relay auth contract drifting.
        let filter = build_thread_replies_filter("root-hex", Some("channel-1"), 64, 200, None);

        let kinds = filter
            .get("kinds")
            .and_then(|v| v.as_array())
            .expect("thread filter must carry `kinds` so the p-gate passes");
        assert!(!kinds.is_empty(), "kinds must be non-empty");
        for kind in kinds {
            let k = kind.as_u64().expect("kind is a number") as u32;
            assert!(
                !buzz_core_pkg::kind::P_GATED_KINDS.contains(&k),
                "kind {k} is p-gated; a p-gated kind in the filter re-triggers the \
                 403 that this fix exists to prevent"
            );
        }
        assert_eq!(filter["#e"], serde_json::json!(["root-hex"]));
        assert_eq!(filter["depth_limit"], serde_json::json!(64));
        assert_eq!(filter["#h"], serde_json::json!(["channel-1"]));
    }

    #[test]
    fn thread_replies_filter_pages_with_composite_cursor() {
        // When a cursor is supplied, both the timestamp and the event-id
        // tiebreak must be emitted (`thread_cursor` + `thread_cursor_id`), else
        // paging degrades to timestamp-only and drops same-second replies.
        let cursor = crate::models::ThreadCursor {
            created_at: 1_700_000_000,
            event_id: "abcd".to_string(),
        };
        let filter = build_thread_replies_filter("root-hex", None, 64, 200, Some(&cursor));
        assert_eq!(filter["thread_cursor"], serde_json::json!(1_700_000_000));
        assert_eq!(filter["thread_cursor_id"], serde_json::json!("abcd"));
        assert!(
            !filter.contains_key("#h"),
            "no channel_id -> no #h scope in the filter"
        );
    }

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
