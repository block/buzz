//! Exercise the production channel-list fetch over the native HTTP bridge.
use super::*;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Fixture {
    events: Vec<Event>,
    requests: Vec<Vec<Value>>,
    fail_discovery: bool,
    fail_fallback: bool,
    fail_messages: bool,
}

struct Relay {
    data: Arc<Mutex<Fixture>>,
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn query(
    State(data): State<Arc<Mutex<Fixture>>>,
    Json(filters): Json<Vec<Value>>,
) -> (StatusCode, Json<Value>) {
    let mut data = data.lock().unwrap();
    data.requests.push(filters.clone());
    let mut result = Vec::new();
    for filter in &filters {
        let kind = filter["kinds"][0].as_u64().unwrap();
        if (kind == 39002 && filter.get("#p").is_some() && data.fail_discovery)
            || (kind == 39002 && filter.get("#d").is_some() && data.fail_fallback)
            || (kind == 9 && data.fail_messages)
        {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "fixture unavailable"})),
            );
        }
        let mut page: Vec<_> = data
            .events
            .iter()
            .filter(|event| {
                if !filter["kinds"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(event.kind.as_u16()))
                {
                    return false;
                }
                for name in ["p", "d", "h"] {
                    if let Some(values) = filter.get(format!("#{name}")).and_then(Value::as_array) {
                        if !event.tags.iter().any(|tag| {
                            let s = tag.as_slice();
                            s.len() >= 2 && s[0] == name && values.contains(&json!(s[1]))
                        }) {
                            return false;
                        }
                    }
                }
                if let Some(until) = filter["until"].as_u64() {
                    let ts = event.created_at.as_secs();
                    if ts > until
                        || (ts == until
                            && filter["before_id"]
                                .as_str()
                                .is_some_and(|id| event.id.to_hex().as_str() <= id))
                    {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        page.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        page.truncate(filter["limit"].as_u64().unwrap() as usize);
        result.extend(page);
    }
    (StatusCode::OK, Json(json!(result)))
}

impl Relay {
    async fn new(events: Vec<Event>) -> Self {
        let data = Arc::new(Mutex::new(Fixture {
            events,
            ..Default::default()
        }));
        let router = Router::new()
            .route("/query", post(query))
            .with_state(data.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { data, url, task }
    }

    fn state(&self, keys: &Keys) -> AppState {
        let state = crate::app_state::build_app_state();
        *state.keys.lock().unwrap() = keys.clone();
        *state.relay_url_override.lock().unwrap() = Some(self.url.clone());
        state
    }

    fn roster_fallbacks(&self) -> Vec<Value> {
        self.data
            .lock()
            .unwrap()
            .requests
            .iter()
            .flatten()
            .filter(|filter| filter["kinds"] == json!([39002]) && filter.get("#d").is_some())
            .cloned()
            .collect()
    }
}

fn event(keys: &Keys, kind: u16, tags: Vec<Vec<&str>>) -> Event {
    EventBuilder::new(Kind::from_u16(kind), "")
        .allow_self_tagging()
        .tags(tags.into_iter().map(|tag| Tag::parse(tag).unwrap()))
        .custom_created_at(Timestamp::from(1_700_000_000))
        .sign_with_keys(keys)
        .unwrap()
}

fn metadata(keys: &Keys, id: &str) -> Event {
    event(keys, 39000, vec![vec!["d", id], vec!["name", id]])
}

fn roster(keys: &Keys, id: &str, members: &[&str]) -> Event {
    let mut tags = vec![vec!["d", id]];
    tags.extend(members.iter().map(|pk| vec!["p", *pk, "", "member"]));
    event(keys, 39002, tags)
}

async fn fetch(state: &AppState, scope: DirectoryScope) -> Result<Vec<ChannelInfo>, String> {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fetch_channels(state, scope),
    )
    .await
    .expect("bounded channel fetch")
}

#[tokio::test]
async fn member_rosters_are_reused_including_every_discovery_page() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let keys = Keys::generate();
    let me = keys.public_key().to_hex();
    let other = Keys::generate().public_key().to_hex();
    for count in [265, 501] {
        let mut events = Vec::new();
        for index in 0..count {
            let id = format!("channel-{index:04}");
            events.push(metadata(&keys, &id));
            // Repeated p-tag must not inflate the count; non-self members must survive.
            events.push(roster(&keys, &id, &[&me, &other, &other]));
        }
        let relay = Relay::new(events).await;
        let state = relay.state(&keys);
        let started = std::time::Instant::now();
        let channels = fetch(&state, DirectoryScope::MemberOnly).await.unwrap();
        assert_eq!(channels.len(), count);
        assert!(channels.iter().all(|c| c.is_member
            && c.member_count == 2
            && c.member_pubkeys == vec![me.clone(), other.clone()]));
        assert!(
            relay.roster_fallbacks().is_empty(),
            "covered rosters must not be fetched twice"
        );
        let data = relay.data.lock().unwrap();
        let discovery: Vec<_> = data
            .requests
            .iter()
            .flatten()
            .filter(|f| f["kinds"] == json!([39002]))
            .collect();
        assert_eq!(discovery.len(), count / DIRECTORY_PAGE_SIZE + 1);
        if count > DIRECTORY_PAGE_SIZE {
            assert_eq!(discovery[1]["until"], json!(1_700_000_000));
            assert!(discovery[1]["before_id"].is_string());
        }
        // Membership pages + member metadata + hidden DMs + bounded activity batches.
        let expected_reads = count / DIRECTORY_PAGE_SIZE + 1 + 2 + count.div_ceil(128);
        assert_eq!(data.requests.len(), expected_reads);
        eprintln!(
            "roster-reuse fixture channels={count} reads={} elapsed={:?}",
            data.requests.len(),
            started.elapsed()
        );
    }
}

#[tokio::test]
async fn directory_fetches_only_uncovered_rosters_and_keeps_hidden_dm_behavior() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let keys = Keys::generate();
    let me = keys.public_key().to_hex();
    let other = Keys::generate().public_key().to_hex();
    let relay = Relay::new(vec![
        metadata(&keys, "joined"),
        roster(&keys, "joined", &[&me, &other]),
        metadata(&keys, "open"),
        roster(&keys, "open", &[&other]),
        metadata(&keys, "empty"),
        roster(&keys, "empty", &[]),
        event(&keys, 39000, vec![vec!["d", "hidden-dm"], vec!["t", "dm"]]),
        roster(&keys, "hidden-dm", &[&me, &other]),
        event(
            &keys,
            buzz_core_pkg::kind::KIND_DM_VISIBILITY.try_into().unwrap(),
            vec![vec!["p", &me], vec!["h", "hidden-dm"]],
        ),
        event(&keys, 9, vec![vec!["h", "joined"]]),
    ])
    .await;
    let channels = fetch(&relay.state(&keys), DirectoryScope::IncludeOpenDirectory)
        .await
        .unwrap();
    assert_eq!(channels.len(), 3);
    let joined = channels.iter().find(|c| c.id == "joined").unwrap();
    assert!(joined.is_member);
    assert_eq!(joined.member_pubkeys, vec![me, other.clone()]);
    assert!(joined.last_message_at.is_some());
    let open = channels.iter().find(|c| c.id == "open").unwrap();
    assert!(!open.is_member);
    assert_eq!(open.member_count, 1);
    assert_eq!(open.member_pubkeys, vec![other]);
    let empty = channels.iter().find(|c| c.id == "empty").unwrap();
    assert!(!empty.is_member);
    assert_eq!(empty.member_count, 0);
    assert!(empty.member_pubkeys.is_empty());
    let fallbacks = relay.roster_fallbacks();
    assert_eq!(fallbacks.len(), 1);
    let mut ids = fallbacks[0]["#d"].as_array().unwrap().clone();
    ids.sort_by_key(Value::to_string);
    assert_eq!(ids, vec![json!("empty"), json!("open")]);
    assert_eq!(fallbacks[0]["limit"], json!(2));
}

#[tokio::test]
async fn pending_owner_fallback_failure_preserves_covered_rosters() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let keys = Keys::generate();
    let me = keys.public_key().to_hex();
    let other = Keys::generate().public_key().to_hex();
    let relay = Relay::new(vec![
        metadata(&keys, "joined"),
        roster(&keys, "joined", &[&me, &other]),
        metadata(&keys, "pending"),
        roster(&keys, "pending", &[&other]),
        metadata(&keys, "unpropagated"),
        metadata(&keys, "someone-elses-pending"),
    ])
    .await;
    let state = relay.state(&keys);
    state.mark_pending_owned_channel(&me, "joined");
    state.mark_pending_owned_channel(&me, "pending");
    state.mark_pending_owned_channel(&me, "unpropagated");
    state.mark_pending_owned_channel(&other, "someone-elses-pending");
    for fail in [false, true] {
        relay.data.lock().unwrap().fail_fallback = fail;
        let channels = fetch(&state, DirectoryScope::MemberOnly).await.unwrap();
        assert_eq!(channels.len(), 3);
        let unpropagated = channels.iter().find(|c| c.id == "unpropagated").unwrap();
        assert!(unpropagated.is_member);
        assert_eq!(unpropagated.member_count, 0);
        assert!(unpropagated.member_pubkeys.is_empty());
        let pending = channels.iter().find(|c| c.id == "pending").unwrap();
        assert!(pending.is_member);
        assert_eq!(pending.member_count, if fail { 0 } else { 1 });
        assert_eq!(
            channels
                .iter()
                .find(|c| c.id == "joined")
                .unwrap()
                .member_count,
            2
        );
        assert!(!state.is_pending_owned_channel(&me, "joined"));
        assert!(state.is_pending_owned_channel(&me, "pending"));
    }
    for fallback in relay.roster_fallbacks() {
        let mut ids = fallback["#d"].as_array().unwrap().clone();
        ids.sort_by_key(Value::to_string);
        assert_eq!(ids, vec![json!("pending"), json!("unpropagated")]);
        assert_eq!(fallback["limit"], json!(2));
    }
}

#[tokio::test]
async fn each_fetch_observes_roster_changes_and_the_current_identity_and_relay() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let keys = Keys::generate();
    let me = keys.public_key().to_hex();
    let next_keys = Keys::generate();
    let next = next_keys.public_key().to_hex();
    let relay = Relay::new(vec![
        metadata(&keys, "same-id"),
        roster(&keys, "same-id", &[&me]),
    ])
    .await;
    let state = relay.state(&keys);
    assert_eq!(
        fetch(&state, DirectoryScope::MemberOnly).await.unwrap()[0].member_pubkeys,
        vec![me.clone()]
    );
    relay.data.lock().unwrap().events[1] = roster(&keys, "same-id", &[&me, &next]);
    assert_eq!(
        fetch(&state, DirectoryScope::MemberOnly).await.unwrap()[0].member_count,
        2
    );
    relay.data.lock().unwrap().events[1] = roster(&keys, "same-id", &[&next]);
    assert!(fetch(&state, DirectoryScope::MemberOnly)
        .await
        .unwrap()
        .is_empty());
    *state.keys.lock().unwrap() = next_keys.clone();
    assert_eq!(
        fetch(&state, DirectoryScope::MemberOnly).await.unwrap()[0].member_pubkeys,
        vec![next.clone()]
    );
    let second = Relay::new(vec![
        metadata(&keys, "same-id"),
        roster(&keys, "same-id", &[&next, &me]),
    ])
    .await;
    *state.relay_url_override.lock().unwrap() = Some(second.url.clone());
    assert_eq!(
        fetch(&state, DirectoryScope::MemberOnly).await.unwrap()[0].member_pubkeys,
        vec![next, me]
    );
    assert!(relay.roster_fallbacks().is_empty());
    assert!(second.roster_fallbacks().is_empty());
}

#[tokio::test]
async fn empty_membership_does_not_issue_metadata_roster_or_activity_queries() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let keys = Keys::generate();
    let relay = Relay::new(vec![]).await;
    assert!(fetch(&relay.state(&keys), DirectoryScope::MemberOnly)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(relay.data.lock().unwrap().requests.len(), 2);
    assert!(relay.roster_fallbacks().is_empty());
}

#[tokio::test]
async fn discovery_and_activity_failures_still_abort_the_refresh() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let keys = Keys::generate();
    let me = keys.public_key().to_hex();
    let relay = Relay::new(vec![
        metadata(&keys, "joined"),
        roster(&keys, "joined", &[&me]),
    ])
    .await;
    let state = relay.state(&keys);
    relay.data.lock().unwrap().fail_discovery = true;
    assert!(fetch(&state, DirectoryScope::MemberOnly).await.is_err());
    relay.data.lock().unwrap().fail_discovery = false;
    relay.data.lock().unwrap().fail_messages = true;
    assert!(fetch(&state, DirectoryScope::MemberOnly).await.is_err());
}
