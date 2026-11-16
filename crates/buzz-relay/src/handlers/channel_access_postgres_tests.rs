//! Postgres-backed tests for the channel access answer (NOSTR.md, Channel
//! access): every answer goes through the real `POST /query` route with a
//! payload-bound NIP-98 proof, every change through real ingest, and the
//! push through Redis and the real multi-node fan-out.

use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::ws::Message;
use axum::http::{Request, StatusCode};
use base64::Engine;
use buzz_core::TenantContext;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

use crate::handlers::artifact_conversation_postgres_tests::{create_task, open, send, task};
use crate::handlers::group_state_postgres_tests::{community, create_channel, edit_channel};
use crate::handlers::parent_access_postgres_tests::{add, post, set_parent};
use crate::state::AppState;
use buzz_core::kind::{KIND_CHANNEL_ACCESS_CHANGED, KIND_VIEWER_ACCESS_CHANGED};

/// `POST /query` asking what `viewer` may do in `ids`, signed with a payload
/// tag unless `payload` is false.
async fn ask_with(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    viewer: &Keys,
    ids: &[Uuid],
    payload: bool,
) -> (StatusCode, Value) {
    let body = json!([{"access": "channels", "ids": ids}]);
    query(state, tenant, viewer, &body, payload).await
}

/// `POST /query` with `body`, signed by `viewer`, with a payload tag unless
/// `payload` is false.
async fn query(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    viewer: &Keys,
    body: &Value,
    payload: bool,
) -> (StatusCode, Value) {
    let bytes = serde_json::to_vec(&body).unwrap();
    let host = tenant.host();
    let mut tags = vec![
        Tag::parse([
            "u",
            &crate::api::bridge::nip98_expected_url(&state.config.relay_url, tenant, "/query"),
        ])
        .unwrap(),
        Tag::parse(["method", "POST"]).unwrap(),
        Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
    ];
    if payload {
        tags.push(Tag::parse(["payload", &hex::encode(Sha256::digest(&bytes))]).unwrap());
    }
    let proof = EventBuilder::new(Kind::Custom(27235), "")
        .tags(tags)
        .sign_with_keys(viewer)
        .unwrap();
    let auth = format!(
        "Nostr {}",
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&proof).unwrap())
    );
    let response = crate::router::build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query")
                .header("host", host)
                .header("authorization", auth)
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// The answer entries for `ids`, asserting the request succeeded.
async fn ask(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    viewer: &Keys,
    ids: &[Uuid],
) -> Vec<Value> {
    let (status, body) = ask_with(state, tenant, viewer, ids, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array().expect("answer array").clone()
}

/// The single entry for `id`, or `None` when it was left out.
async fn ask_one(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    viewer: &Keys,
    id: Uuid,
) -> Option<Value> {
    let mut answers = ask(state, tenant, viewer, &[id]).await;
    assert!(answers.len() <= 1);
    answers.pop()
}

fn entry(id: Uuid, post: bool, joined: bool, role: &str) -> Value {
    json!({"id": id, "read": true, "post": post, "joined": joined, "role": role, "archived": false})
}

/// `entry` for a channel that is itself archived.
fn archived(mut entry: Value) -> Value {
    entry["archived"] = json!(true);
    entry
}

/// One page of the list form: its entries and its `next`.
async fn page(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    viewer: &Keys,
    body: Value,
) -> (Vec<Value>, Option<Uuid>) {
    let (status, body) = query(state, tenant, viewer, &json!([body]), true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["channels"].as_array().expect("channels").clone();
    let next = match &body["next"] {
        Value::Null => None,
        next => Some(next.as_str().expect("next id").parse().expect("uuid")),
    };
    (entries, next)
}

/// A private project, a private child under it whose owner made it, and a
/// task in the child with its conversation open.
struct Fixture {
    state: Arc<AppState>,
    tenant: TenantContext,
    owner: Keys,
    project: Uuid,
    child: Uuid,
    task: Uuid,
    task_head: nostr::Event,
    conversation: Uuid,
}

impl Fixture {
    async fn new(state: Arc<AppState>, label: &str) -> Self {
        let tenant = community(&state, label).await;
        let owner = Keys::generate();
        let project = create_channel(&state, &tenant, &owner, &[&["visibility", "private"]]).await;
        let child = create_channel(&state, &tenant, &owner, &[&["visibility", "private"]]).await;
        set_parent(&state, &tenant, &owner, child, Some(project))
            .await
            .expect("file the child");
        let (task, task_head) = create_task(&state, &tenant, &owner, child).await;
        let conversation = open(&state, &tenant, &owner, task)
            .await
            .expect("open the conversation");
        Self {
            state,
            tenant,
            owner,
            project,
            child,
            task,
            task_head,
            conversation,
        }
    }
}

/// A viewer learns, per named channel, whether they read, post and joined,
/// and the role they act with: from their own row, from the parents, and
/// from an open channel's `join_role`. Unreadable and missing channels are
/// left out, the order is the request's, archiving closes posting, and a
/// conversation follows its home. A proof without a payload tag is refused,
/// and a client cannot send the push itself.
/// Mutation: report every channel as joined, ignore `closed_by_archive`,
/// ignore an open channel's `join_role`, or never report `archived` → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn a_viewer_learns_what_they_may_do_in_each_channel() {
    let f = Fixture::new(live_state().await, "access").await;
    let (state, tenant) = (&f.state, &f.tenant);
    let member = Keys::generate();
    let guest = Keys::generate();
    let outsider = Keys::generate();
    add(state, tenant, &f.owner, f.project, &member, "member").await;
    add(state, tenant, &f.owner, f.project, &guest, "guest").await;
    let open_channel = create_channel(state, tenant, &f.owner, &[]).await;
    edit_channel(
        state,
        tenant,
        &f.owner,
        open_channel,
        &[&["join_role", "guest"]],
    )
    .await
    .expect("join as guest");
    let missing = Uuid::new_v4();
    let all = [f.child, missing, f.project, f.conversation, open_channel];

    assert_eq!(
        ask(state, tenant, &f.owner, &all).await,
        vec![
            entry(f.child, true, true, "owner"),
            entry(f.project, true, true, "owner"),
            // A role through parents is capped at member.
            entry(f.conversation, true, false, "member"),
            entry(open_channel, true, true, "owner"),
        ],
        "the owner, in request order, without the missing channel"
    );
    assert_eq!(
        ask(state, tenant, &member, &all).await,
        vec![
            entry(f.child, true, false, "member"),
            entry(f.project, true, true, "member"),
            entry(f.conversation, true, false, "member"),
            entry(open_channel, false, false, "guest"),
        ],
        "a project member reads and posts through the parents"
    );
    assert_eq!(
        ask(state, tenant, &guest, &all).await,
        vec![
            entry(f.child, false, false, "guest"),
            entry(f.project, false, true, "guest"),
            entry(f.conversation, false, false, "guest"),
            entry(open_channel, false, false, "guest"),
        ],
        "a project guest reads everywhere and posts nowhere"
    );
    assert_eq!(
        ask(state, tenant, &outsider, &all).await,
        vec![entry(open_channel, false, false, "guest")],
        "an outsider reads only the open channel"
    );
    // The answer agrees with the post gate.
    assert!(post(state, tenant, &member, f.conversation).await.is_ok());
    assert!(post(state, tenant, &guest, f.conversation).await.is_err());
    assert!(post(state, tenant, &member, open_channel).await.is_err());

    edit_channel(state, tenant, &f.owner, f.child, &[&["archived", "true"]])
        .await
        .expect("archive the child");
    assert_eq!(
        ask(state, tenant, &member, &[f.child, f.conversation]).await,
        vec![
            archived(entry(f.child, false, false, "member")),
            entry(f.conversation, false, false, "member"),
        ],
        "an archived home closes itself and its conversation to posts"
    );
    assert!(post(state, tenant, &member, f.conversation).await.is_err());

    let (status, body) = ask_with(state, tenant, &member, &[f.child], false).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let forged = EventBuilder::new(Kind::Custom(KIND_CHANNEL_ACCESS_CHANGED as u16), "")
        .tags([Tag::parse(["h", &f.child.to_string()]).unwrap()])
        .sign_with_keys(&member)
        .unwrap();
    let refused = send(state, tenant, forged.clone())
        .await
        .expect_err("relay-only");
    assert!(refused.contains("relay-only"), "{refused}");
    // Over a WebSocket, where ephemeral kinds skip ingest, too.
    let (conn, mut frames) =
        crate::handlers::group_state_postgres_tests::test_conn(tenant, &member);
    crate::handlers::event::handle_event(forged.clone(), conn, state.clone()).await;
    let Message::Text(text) = frames.try_recv().expect("an OK") else {
        panic!("expected OK")
    };
    let ok: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(ok[0], "OK");
    assert_eq!(ok[1], forged.id.to_hex());
    assert_eq!(ok[2], false);
    assert!(ok[3].as_str().unwrap().contains("relay-only"), "{ok}");
}

/// Without `ids`, a viewer learns about every channel they can reach, in id
/// order: what they joined, listed open channels, and what those hold. A
/// task conversation and a private channel they can't read stay out, and an
/// archived channel is listed as archived.
/// Mutation: drop the parents' children from the list, or answer the raw
/// list without filtering → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn a_viewer_lists_every_channel_they_can_reach() {
    let f = Fixture::new(live_state().await, "reach").await;
    let (state, tenant) = (&f.state, &f.tenant);
    let member = Keys::generate();
    let outsider = Keys::generate();
    add(state, tenant, &f.owner, f.project, &member, "member").await;
    let grandchild = create_channel(state, tenant, &f.owner, &[&["visibility", "private"]]).await;
    set_parent(state, tenant, &f.owner, grandchild, Some(f.child))
        .await
        .expect("file the grandchild");
    let walled = create_channel(state, tenant, &f.owner, &[&["visibility", "private"]]).await;
    set_parent(state, tenant, &f.owner, walled, Some(f.project))
        .await
        .expect("file the walled channel");
    edit_channel(
        state,
        tenant,
        &f.owner,
        walled,
        &[&["parent_access", "none"]],
    )
    .await
    .expect("wall it off");
    let open_channel = create_channel(state, tenant, &f.owner, &[]).await;
    edit_channel(
        state,
        tenant,
        &f.owner,
        open_channel,
        &[&["join_role", "guest"]],
    )
    .await
    .expect("join as guest");
    edit_channel(
        state,
        tenant,
        &f.owner,
        grandchild,
        &[&["archived", "true"]],
    )
    .await
    .expect("archive the grandchild");
    let reach = |viewer: &Keys| {
        let viewer = viewer.clone();
        async move {
            let (entries, next) = page(state, tenant, &viewer, json!({"access": "channels"})).await;
            assert_eq!(next, None, "one page holds them all");
            entries
        }
    };
    let sorted = |mut entries: Vec<Value>| {
        entries.sort_by_key(|e| e["id"].as_str().unwrap().to_string());
        entries
    };

    assert_eq!(
        reach(&member).await,
        sorted(vec![
            entry(f.project, true, true, "member"),
            entry(f.child, true, false, "member"),
            archived(entry(grandchild, false, false, "member")),
            entry(open_channel, false, false, "guest"),
        ]),
        "a project member reaches its tree and the open channel, not the conversation or the walled channel"
    );
    assert_eq!(
        reach(&outsider).await,
        vec![entry(open_channel, false, false, "guest")],
        "an outsider reaches only the open channel"
    );
    let (status, body) = query(
        state,
        tenant,
        &member,
        &json!([{"access": "channels"}]),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

/// Register a connection for `viewer` subscribed to the push in `channels`;
/// its ID and frame receiver.
fn subscribe(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    viewer: &Keys,
    channels: &[Uuid],
) -> (Uuid, tokio::sync::mpsc::Receiver<Message>) {
    let conn_id = Uuid::new_v4();
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    state.conn_manager.register(
        conn_id,
        tx,
        tokio::sync::mpsc::channel(4).0,
        tokio::sync::mpsc::channel(1).0,
        None,
        tokio_util::sync::CancellationToken::new(),
        tenant.community(),
        Arc::new(std::sync::atomic::AtomicU8::new(0)),
        Arc::new(tokio::sync::Mutex::new(Default::default())),
        3,
        crate::state::CommunityConnectionControl::new(tokio_util::sync::CancellationToken::new()),
    );
    state
        .conn_manager
        .set_authenticated_pubkey(conn_id, viewer.public_key().to_bytes().to_vec());
    for &channel in channels {
        let filter = json!({"kinds": [9, KIND_CHANNEL_ACCESS_CHANGED], "#h": [channel]});
        state.sub_registry.register_scoped(
            tenant.community(),
            conn_id,
            format!("chan-{channel}"),
            vec![serde_json::from_value(filter).expect("filter")],
            Some(channel),
        );
    }
    (conn_id, rx)
}

fn subscribed(state: &AppState, tenant: &TenantContext, channel: Uuid, conn_id: Uuid) -> bool {
    state
        .sub_registry
        .channel_subscriber_conns_scoped(tenant.community(), channel)
        .contains(&conn_id)
}

/// The channels named by the relay-signed pushes in `rx` within `wait`.
async fn pushes(
    state: &AppState,
    rx: &mut tokio::sync::mpsc::Receiver<Message>,
    wait: Duration,
) -> Vec<Uuid> {
    let mut channels = Vec::new();
    let deadline = tokio::time::Instant::now() + wait;
    while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        let Message::Text(text) = frame else { continue };
        let frame: Value = serde_json::from_str(&text).expect("frame");
        if frame[0] != "EVENT" {
            continue;
        }
        let event: nostr::Event = serde_json::from_value(frame[2].clone()).expect("event");
        if event.kind != Kind::Custom(KIND_CHANNEL_ACCESS_CHANGED as u16) {
            continue;
        }
        assert_eq!(
            event.pubkey,
            state.relay_keypair.public_key(),
            "relay-signed"
        );
        assert!(event.content.is_empty());
        event.verify().expect("valid signature");
        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.clone().to_vec()).collect();
        assert_eq!(tags.len(), 1, "only the h tag: {tags:?}");
        assert_eq!(tags[0][0], "h");
        channels.push(tags[0][1].parse().expect("channel id"));
    }
    channels
}

/// A state with live Redis at `REDIS_URL` and multi-node fan-out: `POST
/// /query` admission and the push both go through Redis.
pub(crate) async fn live_state() -> Arc<AppState> {
    let redis_url = std::env::var("REDIS_URL").expect("REDIS_URL names a live Redis");
    let state = crate::state::tests::test_state_with_database_and_redis_url(
        &crate::test_support::database_url(),
        &redis_url,
    )
    .await;
    tokio::spawn(state.pubsub.clone().run_subscriber());
    crate::handlers::multinode_fanout::spawn(state.clone(), state.pubsub.subscribe_local());
    state
}

/// Listen for Redis topics of `channels` on `state`, as a subscribing
/// client's REQ does.
async fn retain(state: &AppState, tenant: &TenantContext, channels: &[Uuid]) {
    for &channel in channels {
        state
            .pubsub
            .retain_topic(tenant, buzz_pubsub::EventTopic::Channel(channel))
            .await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// A project guest has a task's conversation open. Made a member of the
/// project, they are told to ask again in the project, the child and the
/// conversation, the answer now lets them post, and they stay subscribed
/// throughout. Removed from the project, the other subscribers are told, and
/// the removed person's subscriptions close and the answer leaves them out.
/// Mutation: drop the push from the kind:9000 path, or the descendants from
/// `announce_changed` → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn an_upgraded_guest_is_told_to_ask_again_and_stays_subscribed() {
    let state = live_state().await;
    let f = Fixture::new(state, "access-push").await;
    let (state, tenant) = (&f.state, &f.tenant);
    let guest = Keys::generate();
    add(state, tenant, &f.owner, f.project, &guest, "guest").await;
    let channels = [f.project, f.child, f.conversation];
    retain(state, tenant, &channels).await;
    let (guest_conn, mut guest_rx) = subscribe(state, tenant, &guest, &channels);
    let (_owner_conn, mut owner_rx) = subscribe(state, tenant, &f.owner, &[f.conversation]);
    assert_eq!(
        ask_one(state, tenant, &guest, f.conversation).await,
        Some(entry(f.conversation, false, false, "guest"))
    );

    add(state, tenant, &f.owner, f.project, &guest, "member").await;
    let mut told = pushes(state, &mut guest_rx, Duration::from_secs(3)).await;
    told.sort();
    let mut expected = channels.to_vec();
    expected.sort();
    assert_eq!(told, expected, "told once in each channel it reads");
    assert_eq!(
        ask_one(state, tenant, &guest, f.conversation).await,
        Some(entry(f.conversation, true, false, "member")),
        "the answer now lets them post"
    );
    for channel in channels {
        assert!(
            subscribed(state, tenant, channel, guest_conn),
            "still subscribed"
        );
    }
    assert_eq!(
        pushes(state, &mut owner_rx, Duration::from_millis(500)).await,
        vec![f.conversation],
        "everyone reading the conversation is told"
    );

    let remove = EventBuilder::new(Kind::Custom(9001), "")
        .tags([
            Tag::parse(["h", &f.project.to_string()]).unwrap(),
            Tag::parse(["p", &guest.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&f.owner)
        .unwrap();
    let result = crate::handlers::ingest::ingest_event(
        state,
        tenant,
        remove,
        crate::handlers::group_state_postgres_tests::http_auth(&f.owner),
    )
    .await
    .expect("remove");
    assert!(result.accepted, "{:?}", result.message);
    assert_eq!(
        pushes(state, &mut owner_rx, Duration::from_secs(3)).await,
        vec![f.conversation],
        "the remaining readers are told"
    );
    for channel in channels {
        assert!(
            !subscribed(state, tenant, channel, guest_conn),
            "revoked: {channel}"
        );
    }
    assert!(ask(state, tenant, &guest, &channels).await.is_empty());
}

/// Archiving a home and moving a task tell the conversation's subscribers
/// to ask again; a change that leaves access alone does not.
/// Mutation: drop the push from the kind:9002 path or from
/// `artifact_conversation_access_changed` → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn archiving_and_moving_tell_the_conversation_to_ask_again() {
    let state = live_state().await;
    let f = Fixture::new(state, "access-archive").await;
    let (state, tenant) = (&f.state, &f.tenant);
    retain(state, tenant, &[f.conversation]).await;
    let (_conn, mut rx) = subscribe(state, tenant, &f.owner, &[f.conversation]);

    edit_channel(state, tenant, &f.owner, f.child, &[&["archived", "true"]])
        .await
        .expect("archive");
    assert_eq!(
        pushes(state, &mut rx, Duration::from_secs(3)).await,
        vec![f.conversation]
    );
    edit_channel(state, tenant, &f.owner, f.child, &[&["archived", "false"]])
        .await
        .expect("unarchive");
    assert_eq!(
        pushes(state, &mut rx, Duration::from_secs(3)).await,
        vec![f.conversation]
    );
    edit_channel(state, tenant, &f.owner, f.child, &[&["about", "renamed"]])
        .await
        .expect("describe");
    assert!(
        pushes(state, &mut rx, Duration::from_millis(500))
            .await
            .is_empty(),
        "a change that leaves access alone is not announced"
    );

    let other = create_channel(state, tenant, &f.owner, &[&["visibility", "private"]]).await;
    let moved = task(&f.owner, other, f.task, "move", Some(&f.task_head));
    send(state, tenant, moved).await.expect("move the task");
    assert_eq!(
        pushes(state, &mut rx, Duration::from_secs(3)).await,
        vec![f.conversation]
    );
}

/// Joining and leaving tell the channel's subscribers, and those of the
/// private channels under it, to ask again.
/// Mutation: drop the push from the kind:9021 or kind:9022 path → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn joining_and_leaving_are_announced() {
    let state = live_state().await;
    let f = Fixture::new(state, "access-join").await;
    let (state, tenant) = (&f.state, &f.tenant);
    let person = Keys::generate();
    add(state, tenant, &f.owner, f.project, &person, "member").await;
    let open_channel = create_channel(state, tenant, &f.owner, &[]).await;
    retain(state, tenant, &[f.conversation, open_channel]).await;
    let (_conn, mut rx) = subscribe(state, tenant, &f.owner, &[f.conversation, open_channel]);

    let join = EventBuilder::new(Kind::Custom(9021), "")
        .tags([Tag::parse(["h", &open_channel.to_string()]).unwrap()])
        .sign_with_keys(&person)
        .unwrap();
    send(state, tenant, join).await.expect("join");
    assert_eq!(
        pushes(state, &mut rx, Duration::from_secs(3)).await,
        vec![open_channel]
    );

    let leave = EventBuilder::new(Kind::Custom(9022), "")
        .tags([Tag::parse(["h", &f.project.to_string()]).unwrap()])
        .sign_with_keys(&person)
        .unwrap();
    send(state, tenant, leave).await.expect("leave");
    assert_eq!(
        pushes(state, &mut rx, Duration::from_secs(3)).await,
        vec![f.conversation]
    );
}

/// Walking the list form page by page, a viewer who can reach more than 500
/// channels gets each exactly once and in id order, at the default page
/// size and at a small one, and the last page says there are no more.
/// Mutation: start a page at `after` inclusive, or end the walk at a fixed
/// count → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn a_viewer_walks_every_reachable_channel_page_by_page() {
    let state = live_state().await;
    let tenant = community(&state, "access-pages").await;
    let owner = Keys::generate();
    let viewer = Keys::generate();
    let mut made = Vec::new();
    for _ in 0..523 {
        made.push(create_channel(&state, &tenant, &owner, &[]).await);
    }
    made.sort();

    for limit in [None, Some(37)] {
        let mut seen = Vec::new();
        let mut after: Option<Uuid> = None;
        let mut pages = 0;
        loop {
            let mut body = json!({"access": "channels"});
            if let Some(after) = after {
                body["after"] = json!(after);
            }
            if let Some(limit) = limit {
                body["limit"] = json!(limit);
            }
            let (entries, next) = page(&state, &tenant, &viewer, body).await;
            assert!(entries.len() <= limit.unwrap_or(100));
            seen.extend(
                entries
                    .iter()
                    .map(|e| e["id"].as_str().unwrap().parse::<Uuid>().unwrap()),
            );
            pages += 1;
            match next {
                Some(next) => after = Some(next),
                None => break,
            }
        }
        assert_eq!(seen, made, "each channel once, in id order ({limit:?})");
        assert_eq!(pages, 523usize.div_ceil(limit.unwrap_or(100)), "{limit:?}");
    }
}

/// A full page under nested projects answers well inside the deadline:
/// 100 private grandchildren, each read through two levels of parents.
/// Mutation: make each answer wait 100ms → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn a_full_page_under_nested_projects_answers_quickly() {
    let state = live_state().await;
    let tenant = community(&state, "access-cost").await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let private: &[&[&str]] = &[&["visibility", "private"]];
    let project = create_channel(&state, &tenant, &owner, private).await;
    add(&state, &tenant, &owner, project, &member, "member").await;
    let mut grandchildren = Vec::new();
    for _ in 0..10 {
        let child = create_channel(&state, &tenant, &owner, private).await;
        set_parent(&state, &tenant, &owner, child, Some(project))
            .await
            .expect("file the child");
        for _ in 0..10 {
            let grandchild = create_channel(&state, &tenant, &owner, private).await;
            set_parent(&state, &tenant, &owner, grandchild, Some(child))
                .await
                .expect("file the grandchild");
            grandchildren.push(grandchild);
        }
    }

    let started = std::time::Instant::now();
    let answers = ask(&state, &tenant, &member, &grandchildren).await;
    let named = started.elapsed();
    assert_eq!(answers.len(), 100);
    assert!(answers
        .iter()
        .all(|a| a["post"] == true && a["role"] == "member"));

    let started = std::time::Instant::now();
    let (entries, next) = page(&state, &tenant, &member, json!({"access": "channels"})).await;
    let listed = started.elapsed();
    assert_eq!(entries.len(), 100, "a full page of the 111 reachable");
    assert!(next.is_some());

    let budget = Duration::from_secs(2);
    assert!(named < budget, "named form took {named:?}");
    assert!(listed < budget, "list form took {listed:?}");
}

/// Listen on `viewer`'s connection for the relay's per-viewer push, as a
/// client's `{"kinds":[20005],"#p":[<self>]}` REQ does. With `p` false the
/// filter names no `#p`, as a REQ the relay would refuse; fan-out must still
/// deliver nothing that names someone else.
pub(crate) fn subscribe_viewer(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    viewer: &Keys,
    p: bool,
) -> tokio::sync::mpsc::Receiver<Message> {
    let (conn_id, rx) = subscribe(state, tenant, viewer, &[]);
    let filter = if p {
        json!({"kinds": [KIND_VIEWER_ACCESS_CHANGED], "#p": [viewer.public_key().to_hex()]})
    } else {
        json!({"kinds": [KIND_VIEWER_ACCESS_CHANGED]})
    };
    state.sub_registry.register_scoped(
        tenant.community(),
        conn_id,
        "viewer".to_string(),
        vec![serde_json::from_value(filter).expect("filter")],
        None,
    );
    rx
}

/// The pubkeys named by relay-signed per-viewer pushes in `rx` within `wait`.
pub(crate) async fn viewer_pushes(
    state: &AppState,
    rx: &mut tokio::sync::mpsc::Receiver<Message>,
    wait: Duration,
) -> Vec<String> {
    let mut named = Vec::new();
    let deadline = tokio::time::Instant::now() + wait;
    while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        let Message::Text(text) = frame else { continue };
        let frame: Value = serde_json::from_str(&text).expect("frame");
        if frame[0] != "EVENT" {
            continue;
        }
        let event: nostr::Event = serde_json::from_value(frame[2].clone()).expect("event");
        if event.kind != Kind::Custom(KIND_VIEWER_ACCESS_CHANGED as u16) {
            continue;
        }
        assert_eq!(event.pubkey, state.relay_keypair.public_key());
        assert!(event.content.is_empty());
        event.verify().expect("valid signature");
        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.clone().to_vec()).collect();
        assert_eq!(tags.len(), 1, "only the p tag: {tags:?}");
        assert_eq!(tags[0][0], "p");
        named.push(tags[0][1].clone());
    }
    named
}

/// Apply a moderation command signed by `actor` against `target`.
async fn moderate(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    actor: &Keys,
    kind: u32,
    target: &Keys,
    until: Option<i64>,
) {
    let mut tags = vec![Tag::public_key(target.public_key())];
    if let Some(until) = until {
        tags.push(Tag::parse(["expiration", &until.to_string()]).unwrap());
    }
    let event = EventBuilder::new(Kind::Custom(kind as u16), "")
        .tags(tags)
        .sign_with_keys(actor)
        .unwrap();
    crate::handlers::moderation_commands::handle_moderation_command(tenant, state, &event)
        .await
        .unwrap_or_else(|e| panic!("kind {kind} must apply: {e:?}"));
}

/// A timed-out person, and an agent they own, are told they may not post
/// anywhere and until when, matching the post gate. The timeout and its
/// lifting are each pushed to that person and their agent, and to no one
/// else. A timeout that runs out lifts on time with no push: the answer's
/// `timed_out_until` is when to ask again. A banned person may not post and
/// has no end to wait for.
/// Mutation: leave the restriction out of `post`, drop `timed_out_until`,
/// report it for a ban,
/// drop the push from 9042 or 9043, skip the owner's agents, or deliver the
/// push without the p gate → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn a_timeout_closes_posting_and_is_pushed_to_that_person_only() {
    let f = Fixture::new(live_state().await, "access-timeout").await;
    let (state, tenant) = (&f.state, &f.tenant);
    let (member, agent, bystander) = (Keys::generate(), Keys::generate(), Keys::generate());
    for (keys, role) in [
        (&f.owner, "owner"),
        (&member, "member"),
        (&bystander, "member"),
    ] {
        state
            .db
            .add_relay_member(
                tenant.community(),
                &keys.public_key().to_hex(),
                role,
                None,
                &Keys::generate(),
            )
            .await
            .map(|write| write.outcome)
            .expect("relay member");
    }
    for keys in [&member, &agent] {
        state
            .db
            .ensure_user(tenant.community(), keys.public_key().as_bytes())
            .await
            .expect("user row");
    }
    assert!(state
        .db
        .set_agent_owner(
            tenant.community(),
            agent.public_key().as_bytes(),
            member.public_key().as_bytes(),
        )
        .await
        .expect("own the agent"));
    for keys in [&member, &agent] {
        add(state, tenant, &f.owner, f.project, keys, "member").await;
    }
    state
        .pubsub
        .retain_topic(tenant, buzz_pubsub::EventTopic::Global)
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut member_rx = subscribe_viewer(state, tenant, &member, true);
    let mut agent_rx = subscribe_viewer(state, tenant, &agent, true);
    let mut snoop_rx = subscribe_viewer(state, tenant, &bystander, false);
    let channels = [f.project, f.child, f.conversation];
    let open_to_post = |id| entry(id, true, id == f.project, "member");
    for keys in [&member, &agent] {
        assert_eq!(
            ask(state, tenant, keys, &channels).await,
            channels.map(open_to_post).to_vec()
        );
    }

    let until = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
    moderate(
        state,
        tenant,
        &f.owner,
        buzz_core::kind::KIND_MODERATION_TIMEOUT,
        &member,
        Some(until),
    )
    .await;
    let member_hex = member.public_key().to_hex();
    let agent_hex = agent.public_key().to_hex();
    assert_eq!(
        viewer_pushes(state, &mut member_rx, Duration::from_secs(3)).await,
        vec![member_hex.clone()]
    );
    assert_eq!(
        viewer_pushes(state, &mut agent_rx, Duration::from_secs(1)).await,
        vec![agent_hex.clone()]
    );
    assert!(
        viewer_pushes(state, &mut snoop_rx, Duration::from_millis(500))
            .await
            .is_empty(),
        "no one else learns of the timeout"
    );
    for keys in [&member, &agent] {
        let closed: Vec<Value> = channels
            .map(|id| {
                let mut e = entry(id, false, id == f.project, "member");
                e["timed_out_until"] = json!(until);
                e
            })
            .to_vec();
        assert_eq!(ask(state, tenant, keys, &channels).await, closed);
        assert!(post(state, tenant, keys, f.child).await.is_err());
    }

    moderate(
        state,
        tenant,
        &f.owner,
        buzz_core::kind::KIND_MODERATION_UNTIMEOUT,
        &member,
        None,
    )
    .await;
    assert_eq!(
        viewer_pushes(state, &mut member_rx, Duration::from_secs(3)).await,
        vec![member_hex.clone()]
    );
    assert_eq!(
        viewer_pushes(state, &mut agent_rx, Duration::from_secs(1)).await,
        vec![agent_hex]
    );
    assert!(
        viewer_pushes(state, &mut snoop_rx, Duration::from_millis(500))
            .await
            .is_empty()
    );
    for keys in [&member, &agent] {
        assert_eq!(
            ask(state, tenant, keys, &channels).await,
            channels.map(open_to_post).to_vec()
        );
        assert!(post(state, tenant, keys, f.child).await.is_ok());
    }

    // A short timeout lifts on its own, and the answer follows the clock.
    let soon = chrono::Utc::now().timestamp() + 3;
    moderate(
        state,
        tenant,
        &f.owner,
        buzz_core::kind::KIND_MODERATION_TIMEOUT,
        &member,
        Some(soon),
    )
    .await;
    let entry = ask_one(state, tenant, &member, f.child).await.unwrap();
    assert_eq!(
        (entry["post"].clone(), entry["timed_out_until"].clone()),
        (json!(false), json!(soon))
    );
    tokio::time::sleep(Duration::from_millis(
        u64::try_from(soon * 1000 - chrono::Utc::now().timestamp_millis()).unwrap_or(0) + 500,
    ))
    .await;
    assert_eq!(
        ask_one(state, tenant, &member, f.child).await,
        Some(open_to_post(f.child))
    );
    assert!(post(state, tenant, &member, f.child).await.is_ok());
    assert_eq!(
        viewer_pushes(state, &mut member_rx, Duration::from_millis(200)).await,
        vec![member_hex.clone()],
        "the short timeout is pushed when set, and not when it runs out"
    );

    // A banned person is told they may not post, with no end to wait for,
    // even under a timeout that has one.
    moderate(
        state,
        tenant,
        &f.owner,
        buzz_core::kind::KIND_MODERATION_TIMEOUT,
        &member,
        Some(until),
    )
    .await;
    moderate(
        state,
        tenant,
        &f.owner,
        buzz_core::kind::KIND_MODERATION_BAN,
        &member,
        None,
    )
    .await;
    assert!(viewer_pushes(state, &mut member_rx, Duration::from_secs(3))
        .await
        .contains(&member.public_key().to_hex()));
    let banned = ask_one(state, tenant, &member, f.child).await.unwrap();
    assert_eq!(banned["post"], json!(false), "{banned}");
    assert!(banned.get("timed_out_until").is_none(), "{banned}");
    assert!(post(state, tenant, &member, f.child).await.is_err());
}
