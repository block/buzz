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

/// Accept one artifact revision and return it.
async fn revision(
    f: &Fixture,
    keys: &Keys,
    home: Uuid,
    d: Uuid,
    op: &str,
    prev: Option<&Event>,
) -> Event {
    let mut tags = vec![
        vec!["ar".to_owned(), "1".into()],
        vec!["d".into(), d.to_string()],
        vec!["h".into(), home.to_string()],
        vec!["type".into(), "buzz.task".into()],
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
    json!({"artifact": "sync", "#h": channels, "#type": ["buzz.task"]})
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
