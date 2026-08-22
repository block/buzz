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

use crate::{
    app_state::AppState,
    native_relay_client::NativeRelayClient,
    unread_notify::{has_authored_mention, is_high_priority, should_notify, NotifyGate},
};

const CATCH_UP_LIMIT: usize = 1_000;
const ACTIVITY_LIMIT: usize = 100;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnreadCatchUpRequest {
    channels: Vec<CatchUpChannel>,
    self_pubkey: String,
    pub(crate) muted_channel_ids: HashSet<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatchUpChannel {
    id: String,
    #[serde(rename = "type")]
    pub(crate) channel_type: String,
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

pub(crate) struct FetchedChannel {
    pub(crate) order: usize,
    pub(crate) channel: CatchUpChannel,
    pub(crate) events: Vec<EventView>,
}

#[derive(Clone)]
pub(crate) struct EventView {
    pub(crate) id: String,
    pub(crate) kind: u16,
    pub(crate) pubkey: String,
    pub(crate) content: String,
    pub(crate) created_at: u64,
    pub(crate) tags: Vec<Vec<String>>,
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
    // Resolve the parents of replies that tag us, before classifying. A reply
    // addresses the author it answers with a `p` tag byte-identical to a typed
    // `@mention`, so without the parent's author this batch cannot tell "someone
    // mentioned you" from "someone answered you" — and it persists the verdict.
    //
    // A failed lookup fails the whole batch rather than proceeding on a guess:
    // the renderer releases the claim for an errored channel and retries, which
    // is the recoverable outcome. Guessing marks threads mentioned forever.
    let parent_authors = match crate::unread_parent_authors::resolve_parent_authors(
        &session,
        &fetched,
        &request.self_pubkey,
    )
    .await
    {
        Ok(authors) => authors,
        Err(error) => {
            let mut channels: Vec<ChannelResult> = fetched
                .into_iter()
                .map(|item| ChannelResult::Error {
                    channel_id: item.channel.id,
                    error: format!("reply parent lookup failed: {error}"),
                })
                .collect();
            channels.extend(failures);
            return Ok(UnreadCatchUpResponse { channels });
        }
    };

    let mut channels = classify_batch(&request, fetched, &membership, &parent_authors);
    channels.extend(failures);
    Ok(UnreadCatchUpResponse { channels })
}

fn classify_batch(
    request: &UnreadCatchUpRequest,
    fetched: Vec<FetchedChannel>,
    membership: &std::collections::HashMap<String, HashSet<String>>,
    parent_authors: &std::collections::HashMap<String, String>,
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
            } else if has_authored_mention(
                &event.tags,
                &self_pubkey,
                parent_author_for(&item.channel, &event.tags, parent_authors),
            ) {
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
                    &NotifyGate {
                        muted_channel_ids: &request.muted_channel_ids,
                        membership,
                        participated: &participated,
                        authored: &authored,
                        mentioned: &mentioned,
                    },
                    item.channel.channel_type == "dm",
                    parent_author_for(&item.channel, &event.tags, parent_authors),
                )
            {
                continue;
            }
            let reference = thread_reference(&event.tags);
            let broadcast = has_exact_tag(&event.tags, "broadcast", "1");
            let threaded = reference.parent_id.is_some() && !broadcast;
            let high_priority = item.channel.channel_type == "dm"
                || is_high_priority(
                    &event.tags,
                    &self_pubkey,
                    parent_author_for(&item.channel, &event.tags, parent_authors),
                );
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

pub(crate) struct ThreadReference {
    pub(crate) parent_id: Option<String>,
    pub(crate) root_id: Option<String>,
}

pub(crate) fn thread_reference(tags: &[Vec<String>]) -> ThreadReference {
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

/// The author of the message this event answers, or `None` when there is no
/// parent, the lookup did not resolve it, or the channel is a DM.
///
/// DMs are excluded the same way `needsResolvedParentAuthor` excludes them:
/// every DM message `p`-tags both participants, so no consumer reads the
/// parent's author there.
fn parent_author_for<'a>(
    channel: &CatchUpChannel,
    tags: &[Vec<String>],
    parent_authors: &'a std::collections::HashMap<String, String>,
) -> Option<&'a str> {
    if channel.channel_type == "dm" {
        return None;
    }
    let parent_id = thread_reference(tags).parent_id?;
    parent_authors.get(&parent_id).map(String::as_str)
}

pub(crate) fn has_exact_tag(tags: &[Vec<String>], name: &str, value: &str) -> bool {
    tags.iter().any(|tag| {
        tag.first().is_some_and(|part| part == name) && tag.get(1).is_some_and(|part| part == value)
    })
}

pub(crate) fn has_tag_value(tags: &[Vec<String>], name: &str, value: &str) -> bool {
    tags.iter().any(|tag| {
        tag.first().is_some_and(|part| part == name)
            && tag
                .get(1)
                .is_some_and(|part| part.eq_ignore_ascii_case(value))
    })
}

#[cfg(test)]
pub(crate) mod tests {
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
        let result = classify_batch(&req, fetched, &HashMap::new(), &HashMap::new());
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
        let result = classify_batch(&req, fetched, &membership, &HashMap::new());
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

    // ---- Role markers: what a `p` tag on a reply actually claims ----------
    //
    // Every case below turns on the same ambiguity: a reply `p`-tags the author
    // it answers, and that tag is byte-identical to a typed `@mention`. Reading
    // it as a mention marks the thread mentioned forever (persisted) and makes
    // the channel high-priority, which drops its top-level items out of the dock
    // badge.

    pub(crate) fn stream_channel() -> CatchUpChannel {
        CatchUpChannel {
            id: "ch".into(),
            channel_type: "stream".into(),
            name: "Ch".into(),
            read_at: None,
        }
    }

    pub(crate) fn dm_channel() -> CatchUpChannel {
        CatchUpChannel {
            id: "dm".into(),
            channel_type: "dm".into(),
            name: "DM".into(),
            read_at: None,
        }
    }

    /// Classify one channel's batch and return its discovered mention roots.
    fn mentioned_roots(
        channel: CatchUpChannel,
        events: Vec<EventView>,
        parent_authors: &[(&str, &str)],
    ) -> Vec<String> {
        let authors: HashMap<String, String> = parent_authors
            .iter()
            .map(|(id, author)| ((*id).to_string(), (*author).to_string()))
            .collect();
        let fetched = vec![FetchedChannel {
            order: 0,
            channel,
            events,
        }];
        let result = classify_batch(&request(), fetched, &HashMap::new(), &authors);
        let ChannelResult::Success { discovered, .. } = &result[0] else {
            panic!("expected success")
        };
        discovered.mentioned.clone()
    }

    fn reply_tags<'a>(parent: &'a str, p_tag: &[&'a str]) -> Vec<Vec<String>> {
        let mut tags: Vec<Vec<String>> = vec![
            vec!["e".into(), parent.into(), String::new(), "reply".into()],
            vec!["h".into(), "ch".into()],
        ];
        tags.push(p_tag.iter().map(|part| (*part).to_string()).collect());
        tags
    }

    pub(crate) fn reply(id: &str, author: &str, parent: &str, p_tag: &[&str]) -> EventView {
        EventView {
            id: id.into(),
            kind: 9,
            pubkey: author.into(),
            content: id.into(),
            created_at: 10,
            tags: reply_tags(parent, p_tag),
        }
    }

    #[test]
    fn a_bare_p_tag_on_a_reply_answering_us_is_not_a_mention() {
        // The regression: this reply tags us only because NIP-10 addressing
        // names the author it answers. Recorded as a mention, the thread renders
        // as "Following" while nothing ever notifies for it.
        assert!(mentioned_roots(
            stream_channel(),
            vec![reply("r", "other", "parent", &["p", "self"])],
            &[("parent", "self")],
        )
        .is_empty());
    }

    #[test]
    fn a_bare_p_tag_on_a_reply_answering_someone_else_is_a_mention() {
        assert_eq!(
            mentioned_roots(
                stream_channel(),
                vec![reply("r", "other", "parent", &["p", "self"])],
                &[("parent", "third-party")],
            ),
            vec!["parent".to_string()],
        );
    }

    #[test]
    fn a_mention_marker_beats_the_parent_author() {
        // The case the parent cannot decide: we wrote the message being answered
        // *and* the sender typed our name. Mention is the stronger signal.
        assert_eq!(
            mentioned_roots(
                stream_channel(),
                vec![reply("r", "other", "parent", &["p", "self", "", "mention"])],
                &[("parent", "self")],
            ),
            vec!["parent".to_string()],
        );
    }

    #[test]
    fn an_addressing_marker_needs_no_parent_lookup_to_be_demoted() {
        // The point of the markers: the sender already knew, so no round trip.
        assert!(mentioned_roots(
            stream_channel(),
            vec![reply("r", "other", "parent", &["p", "self", "", "reply"])],
            &[],
        )
        .is_empty());
    }

    #[test]
    fn an_unresolved_parent_still_reads_as_a_mention() {
        // Fails open, matching `hasAuthoredMentionForEvent`: notification
        // delivery would rather over-report than silently drop a real mention.
        assert_eq!(
            mentioned_roots(
                stream_channel(),
                vec![reply("r", "other", "parent", &["p", "self"])],
                &[],
            ),
            vec!["parent".to_string()],
        );
    }

    #[test]
    fn a_dm_reply_answering_us_is_never_demoted() {
        // Every DM message p-tags both participants, so the addressing tag is
        // simply how a DM is addressed. Demoting it would silence answers to us
        // while letting new messages through.
        let mut event = reply("r", "other", "parent", &["p", "self"]);
        event.tags[1] = vec!["h".into(), "dm".into()];
        assert_eq!(
            mentioned_roots(dm_channel(), vec![event], &[("parent", "self")]),
            vec!["parent".to_string()],
        );
    }

    #[test]
    fn a_broadcast_reply_answering_us_is_still_a_mention() {
        // `should_notify` admits a broadcast reply before it ever reads the
        // parent, so demoting it here would leave the surfaces disagreeing.
        let mut event = reply("r", "other", "parent", &["p", "self"]);
        event.tags.push(vec!["broadcast".into(), "1".into()]);
        assert_eq!(
            mentioned_roots(stream_channel(), vec![event], &[("parent", "self")]),
            vec!["parent".to_string()],
        );
    }
}
