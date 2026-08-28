//! Batched native unread catch-up.
//!
//! Native unread catch-up consumes notification membership from the observed-
//! unread SQLite store rather than serializing renderer-owned sets on every
//! request. It uses the authenticated HTTP bridge's composite keyset cursor so
//! dense same-second pages cannot lose events, and separates discovery,
//! top-level, and relevant-thread queries so a stale bare marker never forces
//! the desktop to download an entire busy channel history.

use std::collections::{HashMap, HashSet};

use buzz_core_pkg::kind::{
    KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_HUDDLE_STARTED, KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
};
use futures_util::{stream, StreamExt};
use nostr::{Event, Keys};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::{app_state::AppState, observed_unread::HORIZON_SECONDS};

mod window_page;
use window_page::{parse_top_level_page, PageCursor};

mod page_fetch;
use page_fetch::{apply_page_cursor, fetch_filter_pages, query_page, QueryPacer};

mod relevant_threads;
use relevant_threads::{fetch_broadcast_thread_events, fetch_relevant_thread_events};

const CATCH_UP_LIMIT: usize = 1_000;
const ACTIVITY_LIMIT: usize = 100;
const ROOT_FILTER_CHUNK: usize = 200;
const CHANNEL_FETCH_CONCURRENCY: usize = 8;
const DISCOVERY_OVERLAP_SECONDS: u64 = 7 * 24 * 60 * 60;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnreadCatchUpRequest {
    channels: Vec<CatchUpChannel>,
    self_pubkey: String,
    muted_channel_ids: HashSet<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatchUpChannel {
    id: String,
    #[serde(rename = "type")]
    channel_type: String,
    name: String,
    read_at: Option<u64>,
    timeline_read_at: Option<u64>,
    #[serde(default)]
    discovery_at: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnreadCatchUpResponse {
    channels: Vec<ChannelResult>,
}

#[derive(Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ChannelResult {
    Success {
        channel_id: String,
        observed_events: Vec<ObservedUnreadEvent>,
        max_trigger: u64,
        discovery_through: u64,
        activity_rows: Vec<ActivityRow>,
        discovered: DiscoveredRoots,
    },
    Error {
        channel_id: String,
        error: String,
    },
}

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservedUnreadEvent {
    id: String,
    created_at: u64,
    root_id: Option<String>,
    high_priority: bool,
    counts_toward_badge: bool,
    counts_toward_app_badge: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityRow {
    id: String,
    kind: u16,
    pubkey: String,
    content: String,
    created_at: u64,
    channel_id: String,
    channel_name: String,
    tags: Vec<Vec<String>>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveredRoots {
    participated: Vec<String>,
    authored: Vec<String>,
    mentioned: Vec<String>,
}

struct FetchedChannel {
    channel: CatchUpChannel,
    events: Vec<EventView>,
    discovery_through: u64,
}

#[derive(Clone)]
struct EventView {
    id: String,
    kind: u16,
    pubkey: String,
    content: String,
    created_at: u64,
    tags: Vec<Vec<String>>,
}

impl From<Event> for EventView {
    fn from(event: Event) -> Self {
        Self {
            id: event.id.to_hex(),
            kind: event.kind.as_u16(),
            pubkey: event.pubkey.to_hex(),
            content: event.content,
            created_at: event.created_at.as_secs(),
            tags: event
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect(),
        }
    }
}

fn catch_up_kinds(channel_type: &str) -> &'static [u32] {
    if channel_type == "dm" {
        &[
            KIND_STREAM_MESSAGE,
            KIND_STREAM_MESSAGE_V2,
            KIND_FORUM_POST,
            KIND_FORUM_COMMENT,
            KIND_HUDDLE_STARTED,
        ]
    } else {
        &[
            KIND_STREAM_MESSAGE,
            KIND_STREAM_MESSAGE_V2,
            KIND_FORUM_POST,
            KIND_FORUM_COMMENT,
        ]
    }
}

fn base_filter(channel: &CatchUpChannel, since: u64) -> serde_json::Value {
    serde_json::json!({
        "kinds": catch_up_kinds(&channel.channel_type),
        "#h": [channel.id],
        "since": since,
        "limit": CATCH_UP_LIMIT,
    })
}

fn discovery_filters(
    channel: &CatchUpChannel,
    self_pubkey: &str,
    discovery_through: u64,
) -> (serde_json::Value, serde_json::Value) {
    // Discovery is device-local because membership is device-local. A fresh
    // device must scan old roots once even when synced read markers are newer;
    // subsequent successful catch-up advances this independent watermark.
    let since = channel
        .discovery_at
        .map_or(0, |value| value.saturating_sub(DISCOVERY_OVERLAP_SECONDS));
    let mut authored = base_filter(channel, since);
    authored["until"] = serde_json::json!(discovery_through);
    authored["authors"] = serde_json::json!([self_pubkey]);
    let mut mentioned = base_filter(channel, since);
    mentioned["until"] = serde_json::json!(discovery_through);
    mentioned["#p"] = serde_json::json!([self_pubkey]);
    (authored, mentioned)
}

fn top_level_filter(channel: &CatchUpChannel) -> serde_json::Value {
    let since = top_level_since(channel);
    let mut filter = base_filter(channel, since);
    filter["top_level"] = serde_json::json!(true);
    filter
}

fn top_level_since_at(channel: &CatchUpChannel, now: u64) -> u64 {
    channel.timeline_read_at.or(channel.read_at).map_or_else(
        || now.saturating_sub(HORIZON_SECONDS as u64),
        |value| value.saturating_add(1),
    )
}

fn top_level_since(channel: &CatchUpChannel) -> u64 {
    top_level_since_at(channel, chrono::Utc::now().timestamp().max(0) as u64)
}

fn next_top_level_cursor(next: Option<PageCursor>, since: u64) -> Option<PageCursor> {
    next.filter(|next| next.created_at >= since)
}

async fn fetch_top_level_pages(
    state: &AppState,
    api_base: &str,
    keys: &Keys,
    channel: &CatchUpChannel,
    pacer: &QueryPacer,
) -> Result<Vec<Event>, String> {
    let base = top_level_filter(channel);
    let since = top_level_since(channel);
    let mut events = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    loop {
        let mut filter = base.clone();
        if let Some(current) = &cursor {
            apply_page_cursor(&mut filter, current);
        }
        let page = query_page(state, api_base, keys, filter, pacer).await?;
        let (mut rows, next) = parse_top_level_page(page, &channel.id, cursor.as_ref())?;
        // The relay's special top-level window path does not apply `since`.
        // Enforce the frontier locally and stop once its descending cursor has
        // crossed it, rather than walking the channel's complete history.
        rows.retain(|event| event.created_at.as_secs() >= since);
        events.append(&mut rows);
        let Some(next) = next_top_level_cursor(next, since) else {
            break;
        };
        cursor = Some(next);
    }
    Ok(events)
}

async fn fetch_discovery_events(
    state: &AppState,
    api_base: &str,
    keys: &Keys,
    channel: &CatchUpChannel,
    self_pubkey: &str,
    pacer: &QueryPacer,
) -> Result<(Vec<Event>, u64), String> {
    let discovery_through = (chrono::Utc::now().timestamp().max(0) as u64).saturating_sub(1);
    if channel
        .discovery_at
        .is_some_and(|watermark| watermark >= discovery_through)
    {
        return Ok((Vec::new(), discovery_through));
    }
    let (authored, mentioned) = discovery_filters(channel, self_pubkey, discovery_through);
    let (authored, mentioned) = tokio::try_join!(
        fetch_filter_pages(state, api_base, keys, &authored, pacer),
        fetch_filter_pages(state, api_base, keys, &mentioned, pacer),
    )?;
    let mut seen = HashSet::new();
    Ok((
        authored
            .into_iter()
            .chain(mentioned)
            .filter(|event| seen.insert(event.id))
            .collect(),
        discovery_through,
    ))
}

fn discover_query_membership(
    events: &[Event],
    self_pubkey: &str,
    membership: &mut HashMap<String, HashSet<String>>,
) {
    for event in events {
        let view = EventView::from(event.clone());
        let reference = thread_reference(&view.tags);
        if view.pubkey.eq_ignore_ascii_case(self_pubkey) {
            if let Some(root_id) = reference.root_id {
                membership
                    .entry("participated".into())
                    .or_default()
                    .insert(root_id);
            } else {
                membership
                    .entry("authored".into())
                    .or_default()
                    .insert(view.id);
            }
        } else if has_tag_value(&view.tags, "p", self_pubkey) {
            if let Some(root_id) = reference.root_id {
                membership
                    .entry("mentioned".into())
                    .or_default()
                    .insert(root_id);
            }
        }
    }
}

fn relevant_roots(membership: &HashMap<String, HashSet<String>>) -> Vec<String> {
    let mut roots = HashSet::new();
    for kind in ["participated", "authored", "mentioned", "followed"] {
        if let Some(values) = membership.get(kind) {
            roots.extend(values.iter().cloned());
        }
    }
    let mut roots: Vec<_> = roots.into_iter().collect();
    roots.sort();
    roots
}

#[tauri::command]
pub(crate) async fn unread_catch_up(
    request: UnreadCatchUpRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<UnreadCatchUpResponse, String> {
    let keys = state.signing_keys()?;
    let owner = keys.public_key().to_hex();
    if !owner.eq_ignore_ascii_case(&request.self_pubkey) {
        return Err("unread catch-up identity does not match active scope".to_string());
    }
    let relay_url = crate::relay::relay_ws_url_with_override(&state);
    let api_base = crate::relay::relay_api_base_url_with_override(&state);
    let membership = crate::observed_unread::load_membership(
        &app,
        &crate::observed_unread::ObservedUnreadScope {
            pubkey: owner.clone(),
            relay_url: relay_url.clone(),
        },
    )?;
    let query_pacer = QueryPacer::new();

    // Phase one narrows the deep scan to the current user's own history and
    // direct mentions. Those rows discover the roots phase two is allowed to
    // query; unrelated top-level traffic never crosses the bridge.
    let discovery_results = stream::iter(request.channels.iter().cloned())
        .map(|channel| async {
            let result =
                fetch_discovery_events(&state, &api_base, &keys, &channel, &owner, &query_pacer)
                    .await;
            (channel, result)
        })
        .buffered(CHANNEL_FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    let mut discovery = Vec::with_capacity(discovery_results.len());
    let mut failures = Vec::new();
    for (channel, result) in discovery_results {
        match result {
            Ok((events, discovery_through)) => discovery.push((channel, events, discovery_through)),
            Err(error) => failures.push(ChannelResult::Error {
                channel_id: channel.id,
                error,
            }),
        }
    }
    let mut query_membership = membership.clone();
    for (_, events, _) in &discovery {
        discover_query_membership(events, &owner, &mut query_membership);
    }
    let roots = relevant_roots(&query_membership);
    let discovery_channels = discovery
        .iter()
        .map(|(channel, _, _)| channel.clone())
        .collect::<Vec<_>>();
    let mut relevant = fetch_relevant_thread_events(
        &state,
        &api_base,
        &keys,
        &discovery_channels,
        &roots,
        &query_pacer,
    )
    .await;
    // Broadcast replies are globally notifying even when their thread is not
    // part of the user's retained membership. Recover them independently over
    // the same bounded unread horizon as relevant-thread catch-up.
    let mut broadcast =
        fetch_broadcast_thread_events(&state, &api_base, &keys, &discovery_channels, &query_pacer)
            .await;
    discovery.retain(|(channel, _, _)| {
        let channel_id = channel.id.to_ascii_lowercase();
        let error = relevant
            .errors_by_channel
            .remove(&channel_id)
            .or_else(|| broadcast.errors_by_channel.remove(&channel_id));
        let Some(error) = error else {
            return true;
        };
        failures.push(ChannelResult::Error {
            channel_id: channel.id.clone(),
            error,
        });
        false
    });

    let top_level_results = stream::iter(discovery)
        .map(|(channel, events, discovery_through)| {
            let state = &state;
            let api_base = &api_base;
            let keys = &keys;
            let query_pacer = &query_pacer;
            async move {
                let result =
                    fetch_top_level_pages(state, api_base, keys, &channel, query_pacer).await;
                (channel, events, discovery_through, result)
            }
        })
        .buffered(CHANNEL_FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    let mut fetched = Vec::with_capacity(top_level_results.len());
    for (channel, mut events, discovery_through, result) in top_level_results {
        match result {
            Ok(top_level) => {
                events.extend(top_level);
                events.extend(
                    relevant
                        .by_channel
                        .remove(&channel.id.to_ascii_lowercase())
                        .unwrap_or_default(),
                );
                events.extend(
                    broadcast
                        .by_channel
                        .remove(&channel.id.to_ascii_lowercase())
                        .unwrap_or_default(),
                );
                let mut seen = HashSet::new();
                events.retain(|event| seen.insert(event.id));
                fetched.push(FetchedChannel {
                    channel,
                    events: events.into_iter().map(EventView::from).collect(),
                    discovery_through,
                });
            }
            Err(error) => failures.push(ChannelResult::Error {
                channel_id: channel.id,
                error,
            }),
        }
    }

    let current_keys = state.signing_keys()?;
    if current_keys.public_key().to_hex() != owner
        || crate::relay::relay_ws_url_with_override(&state) != relay_url
    {
        return Err("unread catch-up scope changed while fetching".to_string());
    }

    let mut channels = classify_batch(&request, fetched, &membership);
    channels.extend(failures);
    Ok(UnreadCatchUpResponse { channels })
}

fn classify_batch(
    request: &UnreadCatchUpRequest,
    fetched: Vec<FetchedChannel>,
    membership: &std::collections::HashMap<String, HashSet<String>>,
) -> Vec<ChannelResult> {
    let self_pubkey = request.self_pubkey.to_lowercase();
    let mut participated = membership.get("participated").cloned().unwrap_or_default();
    let mut authored = membership.get("authored").cloned().unwrap_or_default();
    let mut mentioned = membership.get("mentioned").cloned().unwrap_or_default();

    // Pass one is deliberately global, not per-channel: notification validity
    // depends on roots learned from history, while the command observes a batch.
    // Deltas remain attributed to the channel that first discovered each root.
    let mut discoveries = Vec::with_capacity(fetched.len());
    for item in &fetched {
        let mut discovered = DiscoveredRoots::default();
        for event in &item.events {
            if event.pubkey.eq_ignore_ascii_case(&self_pubkey) {
                let reference = thread_reference(&event.tags);
                if let Some(root_id) = reference.root_id {
                    if participated.insert(root_id.clone()) {
                        discovered.participated.push(root_id);
                    }
                } else if authored.insert(event.id.clone()) {
                    discovered.authored.push(event.id.clone());
                }
            } else if has_tag_value(&event.tags, "p", &self_pubkey) {
                if let Some(root_id) = thread_reference(&event.tags).root_id {
                    if mentioned.insert(root_id.clone()) {
                        discovered.mentioned.push(root_id);
                    }
                }
            }
        }
        discoveries.push(discovered);
    }

    let mut outputs = Vec::new();
    let mut all_activity = Vec::new();
    for (item, discovered) in fetched.into_iter().zip(discoveries) {
        let mut observed_events = Vec::new();
        let mut activity_rows = Vec::new();
        let mut max_trigger = 0;
        for event in item.events {
            let reference = thread_reference(&event.tags);
            let broadcast = has_exact_tag(&event.tags, "broadcast", "1");
            let threaded = reference.parent_id.is_some() && !broadcast;
            let read_at = if threaded {
                item.channel.read_at
            } else {
                item.channel.timeline_read_at
            };
            if event.pubkey.eq_ignore_ascii_case(&self_pubkey)
                || read_at.is_some_and(|read_at| event.created_at <= read_at)
                || !should_notify(
                    &event,
                    &self_pubkey,
                    request,
                    membership,
                    &participated,
                    &authored,
                    &mentioned,
                )
            {
                continue;
            }
            let high_priority = item.channel.channel_type == "dm"
                || broadcast
                || has_tag_value(&event.tags, "p", &self_pubkey);
            max_trigger = max_trigger.max(event.created_at);
            observed_events.push(ObservedUnreadEvent {
                id: event.id.clone(),
                created_at: event.created_at,
                root_id: if broadcast {
                    None
                } else {
                    reference.root_id.clone()
                },
                high_priority,
                counts_toward_badge: item.channel.channel_type == "dm" || threaded || high_priority,
                counts_toward_app_badge: item.channel.channel_type == "dm"
                    || (!threaded && high_priority),
            });
            if threaded {
                activity_rows.push(ActivityRow {
                    id: event.id,
                    kind: event.kind,
                    pubkey: event.pubkey,
                    content: event.content,
                    created_at: event.created_at,
                    channel_id: item.channel.id.clone(),
                    channel_name: item.channel.name.clone(),
                    tags: event.tags,
                });
            }
        }
        all_activity.extend(activity_rows.iter().cloned());
        outputs.push((
            item.channel.id,
            observed_events,
            max_trigger,
            item.discovery_through,
            activity_rows,
            discovered,
        ));
    }

    all_activity.sort_by_key(|row| row.created_at);
    let mut seen = HashSet::new();
    all_activity.retain(|row| seen.insert(row.id.clone()));
    if all_activity.len() > ACTIVITY_LIMIT {
        all_activity.drain(..all_activity.len() - ACTIVITY_LIMIT);
    }
    let allowed: HashSet<_> = all_activity.into_iter().map(|row| row.id).collect();

    outputs
        .into_iter()
        .map(
            |(
                channel_id,
                observed_events,
                max_trigger,
                discovery_through,
                mut activity_rows,
                discovered,
            )| {
                activity_rows.retain(|row| allowed.contains(&row.id));
                ChannelResult::Success {
                    channel_id,
                    observed_events,
                    max_trigger,
                    discovery_through,
                    activity_rows,
                    discovered,
                }
            },
        )
        .collect()
}

struct ThreadReference {
    parent_id: Option<String>,
    root_id: Option<String>,
}

fn thread_reference(tags: &[Vec<String>]) -> ThreadReference {
    let event_tags: Vec<_> = tags
        .iter()
        .filter(|tag| tag.first().is_some_and(|v| v == "e") && tag.get(1).is_some())
        .collect();
    let root = event_tags
        .iter()
        .find(|tag| tag.get(3).is_some_and(|v| v == "root"));
    let reply = event_tags
        .iter()
        .rev()
        .find(|tag| tag.get(3).is_some_and(|v| v == "reply"));
    let Some(reply) = reply else {
        return ThreadReference {
            parent_id: None,
            root_id: None,
        };
    };
    let parent_id = reply.get(1).cloned();
    ThreadReference {
        root_id: root
            .and_then(|tag| tag.get(1).cloned())
            .or_else(|| parent_id.clone()),
        parent_id,
    }
}

fn should_notify(
    event: &EventView,
    self_pubkey: &str,
    request: &UnreadCatchUpRequest,
    membership: &std::collections::HashMap<String, HashSet<String>>,
    participated: &HashSet<String>,
    authored: &HashSet<String>,
    mentioned: &HashSet<String>,
) -> bool {
    if has_exact_tag(&event.tags, "broadcast", "1") || has_tag_value(&event.tags, "p", self_pubkey)
    {
        return true;
    }
    let event_channel_id = event
        .tags
        .iter()
        .find(|tag| tag.first().is_some_and(|part| part == "h"))
        .and_then(|tag| tag.get(1));
    if event_channel_id.is_some_and(|id| request.muted_channel_ids.contains(id)) {
        return false;
    }
    let reference = thread_reference(&event.tags);
    if reference.parent_id.is_none() {
        return true;
    }
    let Some(root_id) = reference.root_id else {
        return false;
    };
    if membership
        .get("muted_root")
        .is_some_and(|set| set.contains(&root_id))
    {
        return false;
    }
    participated.contains(&root_id)
        || membership
            .get("followed")
            .is_some_and(|set| set.contains(&root_id))
        || authored.contains(&root_id)
        || mentioned.contains(&root_id)
}

fn has_exact_tag(tags: &[Vec<String>], name: &str, value: &str) -> bool {
    tags.iter().any(|tag| {
        tag.first().is_some_and(|part| part == name) && tag.get(1).is_some_and(|part| part == value)
    })
}

fn has_tag_value(tags: &[Vec<String>], name: &str, value: &str) -> bool {
    tags.iter().any(|tag| {
        tag.first().is_some_and(|part| part == name)
            && tag
                .get(1)
                .is_some_and(|part| part.eq_ignore_ascii_case(value))
    })
}

#[cfg(test)]
mod tests;
