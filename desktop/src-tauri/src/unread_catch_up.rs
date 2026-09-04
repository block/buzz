//! Batched native unread catch-up.
//!
//! Native unread catch-up consumes notification membership from the observed-
//! unread SQLite store rather than serializing renderer-owned sets on every
//! request. Rust performs every channel REQ over the shared authenticated
//! session, then classifies the complete successful batch in two passes so a
//! root learned anywhere in pass one is visible everywhere in pass two.

use std::{collections::HashSet, time::Duration};

use buzz_core_pkg::kind::{
    KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_HUDDLE_STARTED, KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
};
use nostr::Event;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio::{sync::Semaphore, task::JoinSet};

use crate::{app_state::AppState, native_relay_client::NativeRelayClient};

const CATCH_UP_LIMIT: usize = 1_000;
const ACTIVITY_LIMIT: usize = 100;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

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
    order: usize,
    channel: CatchUpChannel,
    events: Vec<EventView>,
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

#[tauri::command]
pub(crate) async fn unread_catch_up(
    request: UnreadCatchUpRequest,
    state: State<'_, AppState>,
    relay_client: State<'_, NativeRelayClient>,
    app: AppHandle,
) -> Result<UnreadCatchUpResponse, String> {
    let keys = state.signing_keys()?;
    let owner = keys.public_key().to_hex();
    if !owner.eq_ignore_ascii_case(&request.self_pubkey) {
        return Err("unread catch-up identity does not match active scope".to_string());
    }
    let relay_url = crate::relay::relay_ws_url_with_override(&state);
    // The lease must outlive every task below: when the leased session is
    // private (a scope switch landed mid-command), dropping the lease shuts
    // that session down, and a `handle()` clone still held by a running fetch
    // would then be reading a cancelled socket. The `join_next` drain ends
    // before this binding does, so that holds today — keep it that way, and in
    // particular do not move the lease into a task or narrow its scope.
    let session = relay_client.session(relay_url.clone(), keys).await;

    let concurrency = std::sync::Arc::new(Semaphore::new(8));
    let mut pending = JoinSet::new();
    // One command replaces N renderer invokes while the shared session still
    // multiplexes bounded finite REQs on one authenticated socket.
    for (order, channel) in request.channels.iter().cloned().enumerate() {
        let permit = concurrency
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| error.to_string())?;
        let session = session.handle();
        pending.spawn(async move {
            let _permit = permit;
            let kinds: &[u32] = if channel.channel_type == "dm" {
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
            };
            let filter = serde_json::json!({
                "kinds": kinds,
                "#h": [channel.id],
                "since": channel.read_at.map_or(0, |value| value.saturating_add(1)),
                "limit": CATCH_UP_LIMIT,
            });
            let result = session.fetch_events(filter, REQUEST_TIMEOUT).await;
            (order, channel, result)
        });
    }

    let mut fetched = Vec::new();
    let mut failures = Vec::new();
    while let Some(joined) = pending.join_next().await {
        let (order, channel, result) =
            joined.map_err(|error| format!("unread catch-up task failed: {error}"))?;
        match result {
            Ok(events) => fetched.push(FetchedChannel {
                order,
                channel,
                events: events
                    .into_iter()
                    .take(CATCH_UP_LIMIT)
                    .map(EventView::from)
                    .collect(),
            }),
            Err(error) => failures.push(ChannelResult::Error {
                channel_id: channel.id,
                error,
            }),
        }
    }

    fetched.sort_by_key(|item| item.order);

    let current_keys = state.signing_keys()?;
    if current_keys.public_key().to_hex() != owner
        || crate::relay::relay_ws_url_with_override(&state) != relay_url
    {
        return Err("unread catch-up scope changed while fetching".to_string());
    }

    let membership = crate::observed_unread::load_membership(
        &app,
        &crate::observed_unread::ObservedUnreadScope {
            pubkey: owner,
            relay_url,
        },
    )?;
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
            if event.pubkey.eq_ignore_ascii_case(&self_pubkey)
                || item
                    .channel
                    .read_at
                    .is_some_and(|read_at| event.created_at <= read_at)
                || !should_notify(
                    &event,
                    &self_pubkey,
                    request,
                    membership,
                    &participated,
                    &authored,
                )
            {
                continue;
            }
            let reference = thread_reference(&event.tags);
            let broadcast = has_exact_tag(&event.tags, "broadcast", "1");
            let threaded = reference.parent_id.is_some() && !broadcast;
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
            |(channel_id, observed_events, max_trigger, mut activity_rows, discovered)| {
                activity_rows.retain(|row| allowed.contains(&row.id));
                ChannelResult::Success {
                    channel_id,
                    observed_events,
                    max_trigger,
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
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn event(id: &str, pubkey: &str, created_at: u64, tags: &[&[&str]]) -> EventView {
        EventView {
            id: id.into(),
            kind: 9,
            pubkey: pubkey.into(),
            content: id.into(),
            created_at,
            tags: tags
                .iter()
                .map(|tag| tag.iter().map(|part| (*part).to_string()).collect())
                .collect(),
        }
    }

    fn request() -> UnreadCatchUpRequest {
        UnreadCatchUpRequest {
            channels: vec![],
            self_pubkey: "self".into(),
            muted_channel_ids: HashSet::new(),
        }
    }

    #[test]
    fn pass_one_history_changes_later_classification() {
        let req = request();
        let channel = CatchUpChannel {
            id: "ch".into(),
            channel_type: "stream".into(),
            name: "Ch".into(),
            read_at: Some(9),
        };
        let fetched = vec![FetchedChannel {
            order: 0,
            channel,
            events: vec![
                event(
                    "self-reply",
                    "self",
                    10,
                    &[&["e", "root", "", "reply"], &["h", "ch"]],
                ),
                event(
                    "external-reply",
                    "other",
                    11,
                    &[&["e", "root", "", "reply"], &["h", "ch"]],
                ),
            ],
        }];
        let result = classify_batch(&req, fetched, &HashMap::new());
        let ChannelResult::Success {
            observed_events,
            discovered,
            ..
        } = &result[0]
        else {
            panic!("expected success")
        };
        assert_eq!(
            observed_events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            ["external-reply"]
        );
        assert_eq!(discovered.participated, ["root"]);
    }

    #[test]
    fn same_second_marker_and_mutes_match_renderer_rules() {
        let req = request();
        let mut membership = HashMap::new();
        membership.insert("muted_root".into(), HashSet::from(["muted".into()]));
        let channel = CatchUpChannel {
            id: "ch".into(),
            channel_type: "stream".into(),
            name: "Ch".into(),
            read_at: Some(10),
        };
        let fetched = vec![FetchedChannel {
            order: 0,
            channel,
            events: vec![
                event("boundary", "other", 10, &[&["h", "ch"]]),
                event(
                    "muted",
                    "other",
                    11,
                    &[&["e", "muted", "", "reply"], &["h", "ch"]],
                ),
                event(
                    "broadcast",
                    "other",
                    12,
                    &[&["broadcast", "1"], &["h", "ch"]],
                ),
            ],
        }];
        let result = classify_batch(&req, fetched, &membership);
        let ChannelResult::Success {
            observed_events,
            max_trigger,
            ..
        } = &result[0]
        else {
            panic!("expected success")
        };
        assert_eq!(
            observed_events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            ["broadcast"]
        );
        assert_eq!(*max_trigger, 12);
    }

    /// Pins the SERIALIZED wire contract against `tauriUnreadCatchUp.ts`.
    ///
    /// Asserts on serde's OUTPUT, not on `ChannelResult`: the renderer never
    /// sees the Rust type, it sees bytes, through an `invokeTauri<T>` cast
    /// that validates nothing. Every other test here inspects the enum before
    /// serialization and the e2e bridge hand-writes the intended shape, so
    /// without this nothing compares what Rust emits to what TypeScript
    /// declares.
    ///
    /// Whole-value rather than a key list, deliberately: a key-set assertion
    /// passes a mutant that drops the variant rename and emits `"Success"`,
    /// which the renderer's `status === "error"` branch silently misreads.
    /// Failure here means the merge loop throws on the first success row and
    /// catch-up yields nothing, silently.
    #[test]
    fn serialized_response_matches_the_typescript_contract() {
        let channels = vec![
            ChannelResult::Success {
                channel_id: "ch".into(),
                observed_events: vec![ObservedUnreadEvent {
                    id: "evt".into(),
                    created_at: 11,
                    root_id: Some("root".into()),
                    high_priority: true,
                    counts_toward_badge: true,
                    counts_toward_app_badge: false,
                }],
                max_trigger: 11,
                activity_rows: vec![ActivityRow {
                    id: "evt".into(),
                    kind: 9,
                    pubkey: "other".into(),
                    content: "hi".into(),
                    created_at: 11,
                    channel_id: "ch".into(),
                    channel_name: "Ch".into(),
                    tags: vec![vec!["h".into(), "ch".into()]],
                }],
                discovered: DiscoveredRoots {
                    participated: vec!["root".into()],
                    authored: Vec::new(),
                    mentioned: Vec::new(),
                },
            },
            ChannelResult::Error {
                channel_id: "ch-2".into(),
                error: "relay request timed out".into(),
            },
        ];

        let actual = serde_json::to_value(UnreadCatchUpResponse { channels }).unwrap();
        let expected = serde_json::json!({
            "channels": [
                {
                    "status": "success",
                    "channelId": "ch",
                    "observedEvents": [{
                        "id": "evt",
                        "createdAt": 11,
                        "rootId": "root",
                        "highPriority": true,
                        "countsTowardBadge": true,
                        "countsTowardAppBadge": false,
                    }],
                    "maxTrigger": 11,
                    "activityRows": [{
                        "id": "evt",
                        "kind": 9,
                        "pubkey": "other",
                        "content": "hi",
                        "createdAt": 11,
                        "channelId": "ch",
                        "channelName": "Ch",
                        "tags": [["h", "ch"]],
                    }],
                    "discovered": {
                        "participated": ["root"],
                        "authored": [],
                        "mentioned": [],
                    },
                },
                {
                    "status": "error",
                    "channelId": "ch-2",
                    "error": "relay request timed out",
                },
            ]
        });

        assert_eq!(actual, expected);
    }
}
