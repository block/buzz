//! Shared catch-up recaps over a loopback NIP-AR relay. The fake parses every
//! filter with the relay's own `buzz_core::artifact::parse_query`, orders
//! revisions by arrival like the relay, and enforces `prev` atomically.
use super::*;
use axum::{extract::State as AxumState, http::StatusCode, routing::post, Json, Router};
use buzz_core_pkg::artifact::{parse_query, route_filter, ArtifactView, FilterRoute};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

const CHANNEL: &str = "9b353519-f4fe-4757-aef4-bec6cc0ae54c";
const OTHER_CHANNEL: &str = "04737c81-e5e8-4412-bb47-f446813cfeba";

fn root() -> String {
    "a".repeat(64)
}
fn source(n: u8) -> String {
    format!("{:064x}", n)
}

#[derive(Default)]
struct Fixture {
    /// Accepted revisions in arrival order.
    revisions: Vec<Event>,
    heads: HashMap<String, String>,
    /// Redacted or expired revisions: withheld from every read.
    hidden: HashSet<String>,
    /// Competing revision accepted just before the next submission.
    race: Option<Event>,
    queries: Vec<Value>,
    submissions: usize,
}

impl Fixture {
    fn accept(&mut self, event: Event) -> Result<(), &'static str> {
        let envelope = artifact::validate(&event).map_err(|_| "invalid")?;
        let id = envelope.id.to_string();
        if self.revisions.iter().any(|e| e.id == event.id) {
            return Ok(());
        }
        let head = self.heads.get(&id);
        match envelope.prev.as_deref().map(hex::encode) {
            None if head.is_some() => return Err("conflict: artifact identity is taken"),
            Some(prev) if head != Some(&prev) => return Err("conflict: artifact head changed"),
            _ => {}
        }
        self.heads.insert(id, event.id.to_hex());
        self.revisions.push(event);
        Ok(())
    }
}

type Shared = Arc<Mutex<Fixture>>;

async fn query_route(
    AxumState(data): AxumState<Shared>,
    Json(filters): Json<Vec<Value>>,
) -> (StatusCode, Json<Value>) {
    let mut data = data.lock().unwrap();
    assert_eq!(filters.len(), 1, "artifact queries take exactly one filter");
    assert_eq!(route_filter(&filters[0]), FilterRoute::Artifact);
    let query = parse_query(&filters[0]).expect("relay accepts the filter");
    data.queries.push(filters[0].clone());
    let heads: HashSet<&String> = data.heads.values().collect();
    let page: Vec<&Event> = data
        .revisions
        .iter()
        .rev()
        .filter(|e| !data.hidden.contains(&e.id.to_hex()))
        .filter(|e| query.view == ArtifactView::History || heads.contains(&e.id.to_hex()))
        .filter(|e| {
            query.tags.iter().all(|(name, values)| {
                e.tags.iter().any(|t| {
                    let t = t.as_slice();
                    t.len() >= 2 && &t[0] == name && values.contains(&t[1])
                })
            })
        })
        .skip(query.offset as usize)
        .take(query.limit as usize)
        .collect();
    (StatusCode::OK, Json(json!(page)))
}

async fn events_route(
    AxumState(data): AxumState<Shared>,
    Json(event): Json<Event>,
) -> (StatusCode, Json<Value>) {
    let mut data = data.lock().unwrap();
    data.submissions += 1;
    if let Some(race) = data.race.take() {
        data.accept(race).unwrap();
    }
    match data.accept(event.clone()) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"event_id": event.id.to_hex(), "accepted": true, "message": ""})),
        ),
        Err(reason) => (StatusCode::CONFLICT, Json(json!({"error": reason}))),
    }
}

struct Relay {
    data: Shared,
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Relay {
    async fn new() -> Self {
        let data = Shared::default();
        let router = Router::new()
            .route("/query", post(query_route))
            .route("/events", post(events_route))
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

    fn bind(&self, state: &AppState, keys: &Keys) -> BoundScope {
        bind_scope(state, &scope(&self.url.replace("http", "ws"), keys)).unwrap()
    }

    fn insert(&self, event: Event) {
        self.data.lock().unwrap().accept(event).unwrap();
    }

    fn head(&self, artifact_id: &str) -> Event {
        let data = self.data.lock().unwrap();
        let id = &data.heads[artifact_id];
        data.revisions
            .iter()
            .find(|e| &e.id.to_hex() == id)
            .unwrap()
            .clone()
    }
}

fn scope(relay_url: &str, keys: &Keys) -> CatchUpScope {
    serde_json::from_value(json!({
        "relayUrl": relay_url,
        "signerPubkey": keys.public_key().to_hex(),
        "channelId": CHANNEL,
        "rootEventId": root(),
    }))
    .unwrap()
}

fn recap() -> Value {
    json!({
        "version": 1,
        "channelId": CHANNEL,
        "rootEventId": root(),
        "since": 1_700_000_000u64,
        "until": 1_700_003_600u64,
        "points": [
            {"id": "p1", "category": "decision", "text": "Ship Friday", "sourceIds": [source(1)]},
            {"id": "p2", "category": "question", "text": "Who owns QA?", "sourceIds": [source(2), source(1)]},
        ],
        "sourceIds": [source(1), source(2)],
        "coverage": {"messageCount": 2, "inputMessageCount": 2, "truncated": false, "warnings": []},
    })
}

/// A revision written by another client, built independently of the
/// production builders.
fn foreign(
    keys: &Keys,
    artifact_id: &str,
    prev: Option<&str>,
    content: Value,
    extra: Vec<Vec<String>>,
) -> Event {
    let op = if prev.is_some() { "update" } else { "create" };
    let mut tags: Vec<Vec<String>> = vec![
        vec!["ar".into(), "1".into()],
        vec!["d".into(), artifact_id.into()],
        vec!["h".into(), CHANNEL.into()],
        vec!["type".into(), THREAD_CATCH_UP_TYPE.into()],
        vec!["title".into(), "Recap".into()],
        vec!["op".into(), op.into()],
        vec!["root".into(), root()],
    ];
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.into()]);
    }
    tags.extend(extra);
    EventBuilder::new(Kind::from_u16(45010), content.to_string())
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .sign_with_keys(keys)
        .unwrap()
}

fn correction(point: &str, text: &str) -> Value {
    json!({"version": 1, "recap": recap(), "correction": {"pointId": point, "text": text}})
}

fn authors(shared: &SharedThreadCatchUp) -> Vec<(String, String, String)> {
    shared
        .corrections
        .iter()
        .map(|c| (c.point_id.clone(), c.text.clone(), c.author_pubkey.clone()))
        .collect()
}

#[test]
fn reads_use_the_relays_real_artifact_filters() {
    let keys = Keys::generate();
    let bound = BoundScope {
        api_base: "http://relay.test".into(),
        keys,
        channel_id: CHANNEL.into(),
        root_event_id: root(),
    };
    let ids = vec![uuid::Uuid::new_v4().to_string()];
    let last = HISTORY_MAX_PAGES - 1;
    for (filter, view) in [
        (current_filter(&bound, None), ArtifactView::Current),
        (
            current_filter(&bound, Some(ids.as_slice())),
            ArtifactView::Current,
        ),
        (history_filter(&bound, &ids, last), ArtifactView::History),
    ] {
        assert_eq!(route_filter(&filter), FilterRoute::Artifact);
        let query = parse_query(&filter).unwrap();
        assert_eq!(query.view, view);
        let names: HashSet<&str> = query.tags.iter().map(|(n, _)| n.as_str()).collect();
        for required in ["h", "type", "root"] {
            assert!(names.contains(required), "{filter}");
        }
        assert_eq!(filter["kinds"], json!([45010]));
    }
    assert_eq!(
        parse_query(&history_filter(&bound, &ids, last))
            .unwrap()
            .offset,
        (last * HISTORY_PAGE) as i64
    );
}

#[tokio::test]
async fn share_correct_conflict_and_history_journey() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let relay = Relay::new().await;
    let (alice, bob, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
    let alice_state = relay.state(&alice);
    let bob_state = relay.state(&bob);
    let alice_scope = relay.bind(&alice_state, &alice);
    let bob_scope = relay.bind(&bob_state, &bob);

    // A different accepted create with this identity makes recovery take the
    // identity-taken branch even when both requests occur in the same second.
    let existing = foreign(
        &alice,
        &share_artifact_id(&alice.public_key().to_hex(), &recap()),
        None,
        json!({"version":1,"recap":recap()}),
        vec![],
    );
    relay.insert(existing);
    // Share; a retry after a lost acknowledgement returns the same share.
    let shared = publish_share(&alice_state, &alice_scope, recap())
        .await
        .unwrap();
    assert_eq!(shared.author_pubkey, alice.public_key().to_hex());
    assert!(shared.history_complete && shared.corrections.is_empty());
    let retried = publish_share(&alice_state, &alice_scope, recap())
        .await
        .unwrap();
    assert_eq!(retried.artifact_id, shared.artifact_id);
    assert_eq!(retried.event_id, shared.event_id);
    assert_eq!(relay.data.lock().unwrap().revisions.len(), 1);
    let created = relay.head(&shared.artifact_id);
    assert_eq!(
        serde_json::from_str::<Value>(&created.content).unwrap(),
        json!({"version": 1, "recap": recap()})
    );

    // Another member lists and corrects; text is trimmed, author is signed.
    let listed = list_shared(&bob_state, &bob_scope).await.unwrap().recaps;
    assert_eq!(listed.len(), 1);
    assert_eq!(serde_json::to_value(&listed[0].recap).unwrap(), recap());
    let corrected = publish_correction(
        &bob_state,
        &bob_scope,
        &shared.artifact_id,
        &listed[0].event_id,
        "p2",
        "  Dana owns QA  ",
    )
    .await
    .unwrap();
    let bob_hex = bob.public_key().to_hex();
    assert_eq!(
        authors(&corrected),
        vec![(
            "p2".to_string(),
            "Dana owns QA".to_string(),
            bob_hex.clone()
        )]
    );

    // Alice's stale `prev` fails visibly without writing.
    let before = relay.data.lock().unwrap().submissions;
    let stale = publish_correction(
        &alice_state,
        &alice_scope,
        &shared.artifact_id,
        &shared.event_id,
        "p1",
        "Ship Monday",
    )
    .await;
    assert_eq!(stale.unwrap_err(), CATCH_UP_CONFLICT);
    assert_eq!(relay.data.lock().unwrap().submissions, before);

    // A competing revision that lands between read and write wins; ours is
    // rejected by `prev`, not re-targeted over it.
    let competitor = foreign(
        &carol,
        &shared.artifact_id,
        Some(&corrected.event_id),
        correction("p1", "Friday is tentative"),
        vec![],
    );
    relay.data.lock().unwrap().race = Some(competitor.clone());
    let raced = publish_correction(
        &alice_state,
        &alice_scope,
        &shared.artifact_id,
        &corrected.event_id,
        "p1",
        "Ship Monday",
    )
    .await;
    assert_eq!(raced.unwrap_err(), CATCH_UP_CONFLICT);
    assert_eq!(relay.head(&shared.artifact_id).id, competitor.id);

    // A newer client's fields and annotations survive a correction.
    let mut future = correction("p1", "Friday is tentative");
    future["x-future"] = json!({"keep": true});
    let annotated = foreign(
        &carol,
        &shared.artifact_id,
        Some(&competitor.id.to_hex()),
        future.clone(),
        vec![
            vec!["label".into(), "keep".into()],
            vec!["p".into(), alice.public_key().to_hex()],
        ],
    );
    relay.insert(annotated.clone());
    let listed = list_shared(&alice_state, &alice_scope)
        .await
        .unwrap()
        .recaps;
    assert_eq!(listed[0].event_id, annotated.id.to_hex());
    let final_state = publish_correction(
        &alice_state,
        &alice_scope,
        &shared.artifact_id,
        &listed[0].event_id,
        "p1",
        "Ship Monday",
    )
    .await
    .unwrap();
    let written = relay.head(&shared.artifact_id);
    let content: Value = serde_json::from_str(&written.content).unwrap();
    assert_eq!(content["recap"], recap());
    assert_eq!(content["x-future"], json!({"keep": true}));
    assert_eq!(
        content["correction"],
        json!({"pointId": "p1", "text": "Ship Monday"})
    );
    assert!(written
        .tags
        .iter()
        .any(|t| t.as_slice() == ["label".to_string(), "keep".to_string()]));
    assert!(written
        .tags
        .iter()
        .any(|t| t.as_slice() == ["prev".to_string(), annotated.id.to_hex()]));
    assert!(!written.tags.iter().any(|tag| tag.as_slice()[0] == "p"));

    // History attributes every correction to its signer, oldest first.
    let carol_hex = carol.public_key().to_hex();
    let expected: Vec<(String, String, String)> = vec![
        ("p2".into(), "Dana owns QA".into(), bob_hex),
        ("p1".into(), "Friday is tentative".into(), carol_hex),
        (
            "p1".into(),
            "Ship Monday".into(),
            alice.public_key().to_hex(),
        ),
    ];
    assert_eq!(authors(&final_state), expected);
    let reloaded = list_shared(&bob_state, &bob_scope).await.unwrap().recaps;
    assert_eq!(authors(&reloaded[0]), expected);
    assert!(reloaded[0].history_complete);
    assert_eq!(reloaded[0].author_pubkey, alice.public_key().to_hex());

    // Restoring an earlier snapshot carries its correction; it must not
    // attribute that old correction to the person restoring the artifact.
    let deletion = EventBuilder::new(Kind::from_u16(45010), "")
        .tags([
            Tag::parse(["ar", "1"]).unwrap(),
            Tag::parse(["d", &shared.artifact_id]).unwrap(),
            Tag::parse(["h", CHANNEL]).unwrap(),
            Tag::parse(["type", THREAD_CATCH_UP_TYPE]).unwrap(),
            Tag::parse(["op", "delete"]).unwrap(),
            Tag::parse(["root", &root()]).unwrap(),
            Tag::parse(["prev", &written.id.to_hex()]).unwrap(),
        ])
        .sign_with_keys(&bob)
        .unwrap();
    artifact::validate(&deletion).unwrap();
    relay.insert(deletion.clone());
    let mut tags = envelope_tags(
        &shared.artifact_id,
        &bob_scope,
        TITLE,
        "restore",
        Some(&deletion.id.to_hex()),
    );
    tags.push(vec!["label".into(), "restored".into()]);
    let restored = sign_revision(&bob_scope, &future, tags).unwrap();
    relay.insert(restored);
    let after_restore = list_shared(&alice_state, &alice_scope).await.unwrap();
    assert_eq!(authors(&after_restore.recaps[0]), expected);
    assert!(after_restore.recaps[0].history_complete);
}

#[tokio::test]
async fn share_retry_does_not_adopt_another_or_unverifiable_artifact() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    for invalid in ["another-author", "different-recap", "missing-origin"] {
        let relay = Relay::new().await;
        let alice = Keys::generate();
        let other = Keys::generate();
        let state = relay.state(&alice);
        let bound = relay.bind(&state, &alice);
        let id = share_artifact_id(&alice.public_key().to_hex(), &recap());
        let mut content = json!({"version":1,"recap":recap()});
        if invalid == "different-recap" {
            content["recap"]["points"][0]["text"] = json!("A different recap");
        }
        let keys = if invalid == "another-author" {
            &other
        } else {
            &alice
        };
        if invalid == "missing-origin" {
            let origin = foreign(keys, &id, None, content.clone(), vec![]);
            relay.insert(origin.clone());
            relay.insert(foreign(
                keys,
                &id,
                Some(&origin.id.to_hex()),
                content,
                vec![],
            ));
            // Simulate retention after accepting a valid chain, rather than
            // asking the relay double to accept an orphan update.
            relay
                .data
                .lock()
                .unwrap()
                .revisions
                .retain(|event| event.id != origin.id);
        } else {
            relay.insert(foreign(keys, &id, None, content, vec![]));
        }
        assert!(
            publish_share(&state, &bound, recap()).await.is_err(),
            "{invalid}"
        );
    }
}

#[tokio::test]
async fn history_completeness_is_truthful() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let relay = Relay::new().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let state = relay.state(&bob);
    let bound = relay.bind(&state, &bob);
    let id = uuid::Uuid::new_v4().to_string();
    let create = foreign(
        &alice,
        &id,
        None,
        json!({"version": 1, "recap": recap()}),
        vec![],
    );
    let mut chain = vec![create.clone()];
    relay.insert(create);
    // Longer than one history page, so completeness needs pagination.
    for n in 0..(HISTORY_PAGE + 20) {
        let prev = chain.last().unwrap().id.to_hex();
        let next = foreign(
            &bob,
            &id,
            Some(&prev),
            correction("p1", &n.to_string()),
            vec![],
        );
        relay.insert(next.clone());
        chain.push(next);
    }
    let shared = list_shared(&state, &bound).await.unwrap().recaps;
    assert!(shared[0].history_complete);
    assert_eq!(shared[0].author_pubkey, alice.public_key().to_hex());
    assert_eq!(shared[0].corrections.len(), HISTORY_PAGE + 20);
    let offsets: Vec<Value> = relay
        .data
        .lock()
        .unwrap()
        .queries
        .iter()
        .filter(|q| q["artifact"] == "history")
        .map(|q| q["offset"].clone())
        .collect();
    assert_eq!(offsets, vec![json!(0), json!(HISTORY_PAGE)]);

    // A withheld middle revision leaves a gap: only the attributable tail is
    // reported, the sharer falls back to the earliest retrieved revision, and
    // the gap is surfaced rather than presented as complete history.
    relay
        .data
        .lock()
        .unwrap()
        .hidden
        .insert(chain[10].id.to_hex());
    let shared = list_shared(&state, &bound).await.unwrap().recaps;
    assert!(!shared[0].history_complete);
    assert_eq!(shared[0].corrections.len(), HISTORY_PAGE + 20 - 10 - 1);
    assert_eq!(shared[0].author_pubkey, bob.public_key().to_hex());
    assert_eq!(shared[0].event_id, chain.last().unwrap().id.to_hex());

    // Reads stop at the page cap and report the unreached origin.
    relay.data.lock().unwrap().hidden.clear();
    while chain.len() <= HISTORY_PAGE * HISTORY_MAX_PAGES {
        let prev = chain.last().unwrap().id.to_hex();
        let next = foreign(
            &bob,
            &id,
            Some(&prev),
            correction("p1", &format!("more-{}", chain.len())),
            vec![],
        );
        relay.insert(next.clone());
        chain.push(next);
    }
    relay.data.lock().unwrap().queries.clear();
    let shared = list_shared(&state, &bound).await.unwrap().recaps;
    assert!(!shared[0].history_complete);
    assert_eq!(
        shared[0].corrections.len(),
        HISTORY_PAGE * HISTORY_MAX_PAGES - 1
    );
    let history_reads = relay
        .data
        .lock()
        .unwrap()
        .queries
        .iter()
        .filter(|q| q["artifact"] == "history")
        .count();
    assert_eq!(history_reads, HISTORY_MAX_PAGES);
}

#[tokio::test]
async fn unsupported_recaps_are_not_attributed_or_edited() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let relay = Relay::new().await;
    let (alice, mallory) = (Keys::generate(), Keys::generate());
    let state = relay.state(&alice);
    let bound = relay.bind(&state, &alice);

    // Payload-claimed attribution is ignored in favor of the signer.
    let id = uuid::Uuid::new_v4().to_string();
    let create = foreign(
        &alice,
        &id,
        None,
        json!({"version": 1, "recap": recap()}),
        vec![],
    );
    relay.insert(create.clone());
    let mut claimed = correction("p1", "Not mine");
    claimed["correction"]["authorPubkey"] = json!(alice.public_key().to_hex());
    let claim = foreign(&mallory, &id, Some(&create.id.to_hex()), claimed, vec![]);
    relay.insert(claim.clone());
    let shared = list_shared(&state, &bound).await.unwrap().recaps;
    assert_eq!(
        shared[0].corrections[0].author_pubkey,
        mallory.public_key().to_hex()
    );

    // A revision that rewrites the recap disqualifies the artifact.
    let mut rewritten = correction("p1", "x");
    rewritten["recap"]["points"][0]["text"] = json!("Ship never");
    let rewrite = foreign(
        &mallory,
        &id,
        Some(&claim.id.to_hex()),
        json!({"version": 1, "recap": rewritten["recap"]}),
        vec![],
    );
    relay.insert(rewrite.clone());
    // A rewrite introduced earlier in the chain also disqualifies it.
    let after = foreign(
        &mallory,
        &id,
        Some(&rewrite.id.to_hex()),
        correction("p1", "y"),
        vec![],
    );
    relay.insert(after.clone());
    let list = list_shared(&state, &bound).await.unwrap();
    assert!(list.recaps.is_empty());
    assert_eq!((list.unreadable_count, list.complete), (1, true));
    let before = relay.data.lock().unwrap().submissions;
    let declined = publish_correction(&state, &bound, &id, &after.id.to_hex(), "p1", "z").await;
    assert!(declined.is_err());
    assert_eq!(relay.data.lock().unwrap().submissions, before);

    // Unknown points, malformed payloads, and other threads are rejected.
    let other_id = uuid::Uuid::new_v4().to_string();
    let other = foreign(
        &alice,
        &other_id,
        None,
        json!({"version": 1, "recap": recap()}),
        vec![],
    );
    relay.insert(other.clone());
    let unknown =
        publish_correction(&state, &bound, &other_id, &other.id.to_hex(), "p9", "z").await;
    assert!(unknown.unwrap_err().contains("not part of this recap"));
    let mut tampered = other.clone();
    tampered.content =
        json!({"version": 1, "recap": recap(), "correction": {"pointId": "p1", "text": "forged"}})
            .to_string();
    assert!(
        Head::parse(tampered, &bound).is_err(),
        "signature binds content"
    );
    // The envelope must match the thread even when the payload claims it:
    // a payload naming another thread cannot move a recap there.
    for (field, value) in [
        ("rootEventId", "b".repeat(64)),
        ("channelId", OTHER_CHANNEL.into()),
    ] {
        let mut claimed = recap();
        claimed[field] = json!(value);
        let event = foreign(
            &alice,
            &other_id,
            None,
            json!({"version": 1, "recap": claimed}),
            vec![],
        );
        let elsewhere = BoundScope {
            api_base: bound.api_base.clone(),
            keys: bound.keys.clone(),
            channel_id: claimed["channelId"].as_str().unwrap().into(),
            root_event_id: claimed["rootEventId"].as_str().unwrap().into(),
        };
        assert!(check_recap(&claimed, &elsewhere).is_ok(), "{field}");
        assert!(Head::parse(event, &elsewhere).is_err(), "{field}");
    }
}

#[tokio::test]
async fn scope_switch_fails_closed_before_any_write() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let relay = Relay::new().await;
    let alice = Keys::generate();
    let state = relay.state(&alice);
    let blank = scope("", &alice);
    assert!(bind_scope(&state, &blank).is_err());
    assert!(bind_scope(&state, &scope("ws://127.0.0.1:1", &alice)).is_err());
    assert!(bind_scope(&state, &scope(&relay.url, &Keys::generate())).is_err());

    let switches: [fn(&AppState); 2] = [
        |s| *s.relay_url_override.lock().unwrap() = Some("http://127.0.0.1:1".into()),
        |s| *s.keys.lock().unwrap() = Keys::generate(),
    ];
    for switch in switches {
        let bound = relay.bind(&state, &alice);
        switch(&state);
        assert_eq!(
            publish_share(&state, &bound, recap()).await.unwrap_err(),
            SCOPE_CHANGED
        );
        assert_eq!(relay.data.lock().unwrap().submissions, 0);
        *state.keys.lock().unwrap() = alice.clone();
        *state.relay_url_override.lock().unwrap() = Some(relay.url.clone());
    }

    // Legacy snapshots remain readable, but the production share command
    // must refuse new publication without frozen source revisions.
    use tauri::Manager;
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let result = share_thread_catch_up(
        ShareThreadCatchUpInput {
            scope: scope(&relay.url, &alice),
            recap: serde_json::from_value(recap()).unwrap(),
        },
        app.state(),
    )
    .await;
    assert!(result.unwrap_err().starts_with("source_changed:"));
    assert_eq!(relay.data.lock().unwrap().submissions, 0);
}

#[test]
fn recap_shape_table() {
    let bound = BoundScope {
        api_base: String::new(),
        keys: Keys::generate(),
        channel_id: CHANNEL.into(),
        root_event_id: root(),
    };
    assert_eq!(check_recap(&recap(), &bound), Ok(()));
    type ValueMutation = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, ValueMutation)> = vec![
        ("version", Box::new(|r| r["version"] = json!(2))),
        (
            "channel",
            Box::new(|r| r["channelId"] = json!(OTHER_CHANNEL)),
        ),
        (
            "root",
            Box::new(|r| r["rootEventId"] = json!("b".repeat(64))),
        ),
        ("range", Box::new(|r| r["since"] = json!(1_800_000_000u64))),
        (
            "source case",
            Box::new(|r| r["sourceIds"][0] = json!("A".repeat(64))),
        ),
        (
            "duplicate source",
            Box::new(|r| r["sourceIds"][1] = json!(source(1))),
        ),
        (
            "duplicate point",
            Box::new(|r| r["points"][1]["id"] = json!("p1")),
        ),
        (
            "empty point id",
            Box::new(|r| r["points"][0]["id"] = json!("")),
        ),
        (
            "category",
            Box::new(|r| r["points"][0]["category"] = json!("assigned")),
        ),
        (
            "blank text",
            Box::new(|r| r["points"][0]["text"] = json!("  ")),
        ),
        (
            "uncited",
            Box::new(|r| r["points"][0]["sourceIds"] = json!([])),
        ),
        (
            "unknown source",
            Box::new(|r| r["points"][0]["sourceIds"] = json!([source(9)])),
        ),
        (
            "point size",
            Box::new(|r| r["points"][0]["text"] = json!("x".repeat(MAX_TEXT_BYTES + 1))),
        ),
        (
            "total size",
            Box::new(|r| {
                let points: Vec<Value> = (0..=MAX_POINT_TEXT_BYTES / MAX_TEXT_BYTES)
                    .map(|n| json!({"id": n.to_string(), "category": "decision", "text": "x".repeat(MAX_TEXT_BYTES), "sourceIds": [source(1)]}))
                    .collect();
                r["points"] = json!(points);
            }),
        ),
        (
            "too many points",
            Box::new(|r| {
                let points: Vec<Value> = (0..=MAX_POINTS)
                .map(|n| json!({"id": n.to_string(), "category": "decision", "text": "t", "sourceIds": [source(1)]}))
                .collect();
                r["points"] = json!(points);
            }),
        ),
        (
            "coverage",
            Box::new(|r| r["coverage"]["truncated"] = json!("no")),
        ),
        (
            "warnings",
            Box::new(|r| r["coverage"]["warnings"] = json!([1])),
        ),
    ];
    for (name, mutate) in cases {
        let mut value = recap();
        mutate(&mut value);
        assert!(check_recap(&value, &bound).is_err(), "{name}");
    }
}

#[test]
fn correction_text_table() {
    for (text, ok) in [
        ("Dana owns QA", true),
        ("line one\nline two\twith tab", true),
        ("", false),
        ("   ", false),
        (" padded", false),
        ("bell\u{7}", false),
        ("nul\u{0}", false),
    ] {
        assert_eq!(check_correction_text(text).is_ok(), ok, "{text:?}");
    }
    assert!(check_correction_text(&"x".repeat(MAX_TEXT_BYTES)).is_ok());
    assert!(check_correction_text(&"x".repeat(MAX_TEXT_BYTES + 1)).is_err());
}

#[test]
fn share_ids_are_stable_per_signer_and_recap() {
    let (a, b) = (
        Keys::generate().public_key().to_hex(),
        Keys::generate().public_key().to_hex(),
    );
    let mut changed = recap();
    changed["until"] = json!(1_700_003_601u64);
    let id = share_artifact_id(&a, &recap());
    assert!(artifact::canonical_uuid(&id).is_ok());
    assert_eq!(id, share_artifact_id(&a, &recap()));
    assert_ne!(id, share_artifact_id(&b, &recap()));
    assert_ne!(id, share_artifact_id(&a, &changed));
}

#[tokio::test]
async fn list_reports_omitted_and_capped_recaps() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let relay = Relay::new().await;
    let alice = Keys::generate();
    let state = relay.state(&alice);
    let bound = relay.bind(&state, &alice);
    let share = || {
        let id = uuid::Uuid::new_v4().to_string();
        foreign(
            &alice,
            &id,
            None,
            json!({"version": 1, "recap": recap()}),
            vec![],
        )
    };

    // An unreadable recap is counted, never silently dropped.
    let malformed = foreign(
        &alice,
        &uuid::Uuid::new_v4().to_string(),
        None,
        json!("not json"),
        vec![],
    );
    relay.insert(malformed);
    relay.insert(share());
    let list = list_shared(&state, &bound).await.unwrap();
    assert_eq!(
        (list.recaps.len(), list.unreadable_count, list.complete),
        (1, 1, true)
    );

    // Up to the cap the list is complete; one more marks it incomplete and
    // only the most recently changed window is returned.
    for _ in 0..(LIST_LIMIT - 2) {
        relay.insert(share());
    }
    let list = list_shared(&state, &bound).await.unwrap();
    assert_eq!(
        (list.recaps.len(), list.unreadable_count, list.complete),
        (LIST_LIMIT - 1, 1, true)
    );
    relay.insert(share());
    let list = list_shared(&state, &bound).await.unwrap();
    assert_eq!(
        (list.recaps.len(), list.unreadable_count, list.complete),
        (LIST_LIMIT, 0, false)
    );
    let limits: Vec<Value> = relay
        .data
        .lock()
        .unwrap()
        .queries
        .iter()
        .filter(|q| q["artifact"] == "current")
        .map(|q| q["limit"].clone())
        .collect();
    assert!(limits.iter().all(|limit| *limit == json!(LIST_LIMIT + 1)));
}
