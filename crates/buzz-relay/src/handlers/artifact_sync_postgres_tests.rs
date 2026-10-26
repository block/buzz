//! Postgres- and Redis-backed tests for NIP-AR Sync over the real REQ
//! handler, ingest pipeline, fan-out and pub/sub link.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::Message as WsMessage;
use buzz_core::TenantContext;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::group_state_postgres_tests::{create_channel, http_auth, test_conn};
use crate::connection::ConnectionState;
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

struct Fixture {
    state: Arc<AppState>,
    tenant: TenantContext,
}

async fn fixture() -> Fixture {
    let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let host = format!("artifact-sync-{}.local", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    state
        .db
        .maintain_partitions(
            1,
            buzz_db::partition::PartitionMaintenancePolicy {
                create_enabled: true,
                advance_enabled: false,
            },
        )
        .await
        .expect("partitions");
    // The real Redis link: rule 1 waits for it to confirm live interest.
    tokio::spawn(Arc::clone(&state.pubsub).run_subscriber());
    state
        .pubsub
        .link_state()
        .wait_for(|link| link.up)
        .await
        .expect("link up");
    Fixture {
        state,
        tenant: TenantContext::resolved(community, &host),
    }
}

async fn ingest(f: &Fixture, keys: &Keys, event: Event) {
    let kind = event.kind.as_u16();
    let result = ingest_event(&f.state, &f.tenant, event, http_auth(keys))
        .await
        .unwrap_or_else(|error| panic!("ingest kind:{kind}: {error:?}"));
    assert!(result.accepted, "kind:{kind} refused: {}", result.message);
}

/// Accept one `buzz.task` revision and return it.
async fn revision(
    f: &Fixture,
    keys: &Keys,
    home: Uuid,
    d: Uuid,
    op: &str,
    prev: Option<&Event>,
) -> Event {
    typed_revision(f, keys, home, d, "buzz.task", op, prev).await
}

/// Accept one artifact revision of type `ty` and return it.
async fn typed_revision(
    f: &Fixture,
    keys: &Keys,
    home: Uuid,
    d: Uuid,
    ty: &str,
    op: &str,
    prev: Option<&Event>,
) -> Event {
    let mut tags = vec![
        vec!["ar".to_owned(), "1".into()],
        vec!["d".into(), d.to_string()],
        vec!["h".into(), home.to_string()],
        vec!["type".into(), ty.into()],
        vec!["op".into(), op.into()],
    ];
    if op != "delete" {
        tags.push(vec!["title".into(), "Sync task".into()]);
    }
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.id.to_hex()]);
    }
    let event = EventBuilder::new(
        Kind::Custom(45010),
        if op == "delete" { "" } else { "body" },
    )
    .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
    .sign_with_keys(keys)
    .unwrap();
    ingest(f, keys, event.clone()).await;
    event
}

async fn membership(f: &Fixture, owner: &Keys, kind: u16, channel: Uuid, member: &Keys) {
    let event = EventBuilder::new(Kind::Custom(kind), "")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["p", &member.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    ingest(f, owner, event).await;
}

/// A connection for `reader`, registered so fan-out can reach it.
fn connect(f: &Fixture, reader: &Keys) -> (Arc<ConnectionState>, mpsc::Receiver<WsMessage>) {
    let (conn, rx) = test_conn(&f.tenant, reader);
    f.state.conn_manager.register(
        conn.conn_id,
        conn.send_tx.clone(),
        conn.ctrl_tx.clone(),
        mpsc::channel(1).0,
        None,
        conn.cancel.clone(),
        conn.tenant.community(),
        Arc::clone(&conn.backpressure_count),
        Arc::clone(&conn.subscriptions),
        3,
        crate::state::CommunityConnectionControl::new(conn.cancel.clone()),
    );
    (conn, rx)
}

fn start_sync(
    f: &Fixture,
    conn: &Arc<ConnectionState>,
    filter: Value,
) -> tokio::task::JoinHandle<()> {
    let filter = buzz_core::artifact_sync::parse_sync(&filter);
    let permit = f
        .state
        .handler_semaphore
        .clone()
        .try_acquire_owned()
        .expect("permit");
    tokio::spawn(super::artifact_sync::handle_sync(
        "s".into(),
        filter,
        Arc::clone(conn),
        Arc::clone(&f.state),
        permit,
    ))
}

async fn frame(rx: &mut mpsc::Receiver<WsMessage>) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("relay frame in time")
            .expect("connection open");
        if let WsMessage::Text(text) = message {
            return serde_json::from_str(&text).expect("json frame");
        }
    }
}

/// Collect the initial set up to `EOSE`; panics on any other terminal frame.
async fn initial_set(rx: &mut mpsc::Receiver<WsMessage>) -> Vec<String> {
    let mut ids = vec![];
    loop {
        let message = frame(rx).await;
        match message[0].as_str() {
            Some("EVENT") => ids.push(message[2]["id"].as_str().unwrap().to_owned()),
            Some("EOSE") => return ids,
            _ => panic!("unexpected frame before EOSE: {message}"),
        }
    }
}

async fn no_frame(rx: &mut mpsc::Receiver<WsMessage>) {
    if let Ok(Some(message)) = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
        panic!("unexpected frame: {message:?}");
    }
}

const PRIVATE: &[&[&str]] = &[&["visibility", "private"]];

fn sync_filter(channels: &[Uuid]) -> Value {
    json!({"artifact": "current", "#h": channels, "#type": ["buzz.task"]})
}

/// Accept one `buzz.task` revision with a `status` tag and return it.
async fn status_revision(
    f: &Fixture,
    keys: &Keys,
    home: Uuid,
    d: Uuid,
    status: &str,
    prev: Option<&Event>,
) -> Event {
    let op = if prev.is_some() { "update" } else { "create" };
    let mut tags = vec![
        vec!["ar".to_owned(), "1".into()],
        vec!["d".into(), d.to_string()],
        vec!["h".into(), home.to_string()],
        vec!["type".into(), "buzz.task".into()],
        vec!["op".into(), op.into()],
        vec!["title".into(), "Sync task".into()],
        vec!["status".into(), status.into()],
    ];
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.id.to_hex()]);
    }
    let event = EventBuilder::new(Kind::Custom(45010), "body")
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .sign_with_keys(keys)
        .unwrap();
    ingest(f, keys, event.clone()).await;
    event
}

async fn live_event(rx: &mut mpsc::Receiver<WsMessage>) -> String {
    let message = frame(rx).await;
    assert_eq!(message[0].as_str(), Some("EVENT"), "{message}");
    message[2]["id"].as_str().unwrap().to_owned()
}

/// Rule 3 with a mutable predicate: a live head goes out only for an
/// artifact in scope before the change (in the client's copy) or after it.
/// A head that never matched never crosses the socket; one that stops
/// matching is sent once, so the client can drop it. Mutation: send every
/// same-type head (the old `never_in_scope` check) → the `done` task is
/// sent → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn live_heads_follow_every_predicate() {
    let f = fixture().await;
    let owner = Keys::generate();
    let home = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let open = status_revision(&f, &owner, home, Uuid::new_v4(), "open", None).await;

    let (conn, mut rx) = connect(&f, &owner);
    let filter =
        json!({"artifact": "current", "#h": [home], "#type": ["buzz.task"], "#status": ["open"]});
    let task = start_sync(&f, &conn, filter);
    assert_eq!(initial_set(&mut rx).await, vec![open.id.to_hex()]);

    // Never in scope: not sent.
    let other = Uuid::new_v4();
    let done = status_revision(&f, &owner, home, other, "done", None).await;
    no_frame(&mut rx).await;
    // Leaves the set: sent once.
    let d = open
        .tags
        .iter()
        .find_map(|t| match t.as_slice() {
            [name, value, ..] if name == "d" => value.parse().ok(),
            _ => None,
        })
        .unwrap();
    let closed = status_revision(&f, &owner, home, d, "done", Some(&open)).await;
    assert_eq!(live_event(&mut rx).await, closed.id.to_hex());
    let again = status_revision(&f, &owner, home, d, "done", Some(&closed)).await;
    no_frame(&mut rx).await;
    // Enters the set: sent.
    let reopened = status_revision(&f, &owner, home, other, "open", Some(&done)).await;
    assert_eq!(live_event(&mut rx).await, reopened.id.to_hex());
    let back = status_revision(&f, &owner, home, d, "open", Some(&again)).await;
    assert_eq!(live_event(&mut rx).await, back.id.to_hex());

    super::close::handle_close("s".into(), Arc::clone(&conn), Arc::clone(&f.state)).await;
    assert_eq!(frame(&mut rx).await, json!(["CLOSED", "s", ""]));
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("sync task ends on CLOSE")
        .unwrap();
}

/// Rules 2–4 over the real pipeline: the initial set in `d` order, `EOSE`,
/// then the current head of each later change (including a deletion), and
/// no live entry ever arrives as the published event before `EOSE`.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn sync_sends_one_cut_then_current_heads() {
    let f = fixture().await;
    let owner = Keys::generate();
    let home = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let mut ds = [Uuid::new_v4(), Uuid::new_v4()];
    ds.sort();
    let first = revision(&f, &owner, home, ds[0], "create", None).await;
    let second = revision(&f, &owner, home, ds[1], "create", None).await;

    let (conn, mut rx) = connect(&f, &owner);
    let task = start_sync(&f, &conn, sync_filter(&[home]));
    assert_eq!(
        initial_set(&mut rx).await,
        vec![first.id.to_hex(), second.id.to_hex()]
    );

    let updated = revision(&f, &owner, home, ds[0], "update", Some(&first)).await;
    let message = frame(&mut rx).await;
    assert_eq!(
        (message[0].as_str(), message[2]["id"].as_str()),
        (Some("EVENT"), Some(updated.id.to_hex().as_str()))
    );
    let deleted = revision(&f, &owner, home, ds[1], "delete", Some(&second)).await;
    let message = frame(&mut rx).await;
    assert_eq!(
        (message[0].as_str(), message[2]["id"].as_str()),
        (Some("EVENT"), Some(deleted.id.to_hex().as_str()))
    );
    // An artifact of another type in the same channel is never in scope.
    typed_revision(
        &f,
        &owner,
        home,
        Uuid::new_v4(),
        "buzz.note",
        "create",
        None,
    )
    .await;
    no_frame(&mut rx).await;
    // An ordinary chat message in the channel is not a change.
    let chat = EventBuilder::new(Kind::Custom(9), "hi")
        .tags([Tag::parse(["h", &home.to_string()]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();
    ingest(&f, &owner, chat).await;
    no_frame(&mut rx).await;

    super::close::handle_close("s".into(), Arc::clone(&conn), Arc::clone(&f.state)).await;
    assert_eq!(frame(&mut rx).await, json!(["CLOSED", "s", ""]));
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("sync task ends on CLOSE")
        .unwrap();
    assert!(!f.state.artifact_syncs.contains(conn.conn_id, "s"));
}

/// Rule 9: losing access to one named channel ends the whole sync with a
/// resync, and leaves no fan-out entry for its other channel.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn membership_removal_resyncs_the_whole_sync() {
    let f = fixture().await;
    let owner = Keys::generate();
    let reader = Keys::generate();
    let shared = create_channel(&f.state, &f.tenant, &owner, PRIVATE).await;
    membership(&f, &owner, 9000, shared, &reader).await;
    let own = create_channel(&f.state, &f.tenant, &reader, &[]).await;

    let (conn, mut rx) = connect(&f, &reader);
    let task = start_sync(&f, &conn, sync_filter(&[shared, own]));
    assert!(initial_set(&mut rx).await.is_empty());

    membership(&f, &owner, 9001, shared, &reader).await;
    let message = frame(&mut rx).await;
    assert_eq!(message[0], "CLOSED", "{message}");
    assert!(
        message[2].as_str().unwrap().starts_with("resync: "),
        "{message}"
    );
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("sync task ends")
        .unwrap();
    no_frame(&mut rx).await;
    assert!(!f.state.artifact_syncs.contains(conn.conn_id, "s"));
    assert!(f
        .state
        .sub_registry
        .channel_subscriber_conns_scoped(f.tenant.community(), own)
        .is_empty());
}

/// Rule 3 under a channel-scoped token: the token bounds what the reader
/// can read, so a move to a channel outside it sends the source removal,
/// never the destination head, even though the pubkey can read that
/// channel. Mutation: drop the token bound on the live head read → the
/// moved head is sent → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn scoped_token_gets_only_the_removal_for_a_move_outside_it() {
    let f = fixture().await;
    let owner = Keys::generate();
    let source = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let destination = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let d = Uuid::new_v4();
    let created = revision(&f, &owner, source, d, "create", None).await;

    let (conn, mut rx) = connect(&f, &owner);
    match &mut *conn.auth_state.lock().unwrap() {
        crate::connection::AuthState::Authenticated(ctx) => {
            ctx.channel_ids = Some(vec![source]);
        }
        _ => unreachable!("test connections are authenticated"),
    }
    let task = start_sync(&f, &conn, sync_filter(&[source]));
    assert_eq!(initial_set(&mut rx).await, vec![created.id.to_hex()]);

    let moved = revision(&f, &owner, destination, d, "move", Some(&created)).await;
    let message = frame(&mut rx).await;
    assert_eq!(
        (message[0].as_str(), message[2]["kind"].as_u64()),
        (Some("EVENT"), Some(45011)),
        "{message}"
    );
    assert_ne!(message[2]["id"].as_str(), Some(moved.id.to_hex().as_str()));
    no_frame(&mut rx).await;

    super::close::handle_close("s".into(), Arc::clone(&conn), Arc::clone(&f.state)).await;
    assert_eq!(frame(&mut rx).await, json!(["CLOSED", "s", ""]));
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("sync task ends on CLOSE")
        .unwrap();
}

/// Rule 2: an unreadable or unknown channel gets `CLOSED restricted` and
/// no `EOSE`, with the same reason for both.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn unreadable_channel_is_restricted_without_eose() {
    let f = fixture().await;
    let owner = Keys::generate();
    let private = create_channel(&f.state, &f.tenant, &owner, PRIVATE).await;
    let stranger = Keys::generate();
    for channel in [private, Uuid::new_v4()] {
        let (conn, mut rx) = connect(&f, &stranger);
        start_sync(&f, &conn, sync_filter(&[channel]))
            .await
            .unwrap();
        assert_eq!(
            frame(&mut rx).await,
            json!(["CLOSED", "s", "restricted: channel not readable"])
        );
        no_frame(&mut rx).await;
        assert!(!f.state.artifact_syncs.contains(conn.conn_id, "s"));
    }
}

/// Rule 5: a node-wide loss (fan-out lag or a link epoch change) ends a live
/// sync with a resync.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn node_loss_resyncs_a_live_sync() {
    let f = fixture().await;
    let owner = Keys::generate();
    let home = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let (conn, mut rx) = connect(&f, &owner);
    let task = start_sync(&f, &conn, sync_filter(&[home]));
    assert!(initial_set(&mut rx).await.is_empty());
    f.state
        .artifact_syncs
        .resync_all("resync: live delivery lagged");
    assert_eq!(
        frame(&mut rx).await,
        json!(["CLOSED", "s", "resync: live delivery lagged"])
    );
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("sync task ends")
        .unwrap();
    assert!(!f.state.artifact_syncs.contains(conn.conn_id, "s"));
}

/// A CLOSE while a live head read is in flight retires the sync: the head
/// it read is never sent under the closed ID. Mutation: drop the owner
/// check in `try_send_owned` → the EVENT follows `CLOSED` → RED.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn close_during_a_live_head_read_sends_nothing_more() {
    let f = fixture().await;
    let owner = Keys::generate();
    let home = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let d = Uuid::new_v4();
    let first = revision(&f, &owner, home, d, "create", None).await;

    let (conn, mut rx) = connect(&f, &owner);
    let filter = buzz_core::artifact_sync::parse_sync(&sync_filter(&[home]));
    let permit = f
        .state
        .handler_semaphore
        .clone()
        .try_acquire_owned()
        .expect("permit");
    let pause = Arc::new(super::close::test_seam::Pause::default());
    let task = tokio::spawn(super::artifact_sync::test_seam::LIVE_SEND_PAUSE.scope(
        Arc::clone(&pause),
        super::artifact_sync::handle_sync(
            "s".into(),
            filter,
            Arc::clone(&conn),
            Arc::clone(&f.state),
            permit,
        ),
    ));
    assert_eq!(initial_set(&mut rx).await, vec![first.id.to_hex()]);

    revision(&f, &owner, home, d, "update", Some(&first)).await;
    tokio::time::timeout(Duration::from_secs(10), pause.reached.notified())
        .await
        .expect("live head read reached");
    super::close::handle_close("s".into(), Arc::clone(&conn), Arc::clone(&f.state)).await;
    assert_eq!(frame(&mut rx).await, json!(["CLOSED", "s", ""]));
    pause.resume.notify_one();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("sync task ends")
        .unwrap();
    no_frame(&mut rx).await;
}
