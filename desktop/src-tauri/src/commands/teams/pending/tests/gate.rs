// Wes/Carl P1: tombstones must publish through the real relay ingest gate.
//
// The relay rejects any event more than ±900s from server time
// (`crates/buzz-relay/src/handlers/ingest.rs` MAX_TIMESTAMP_DRIFT_SECS). A
// future-dated head forces a future-dated tombstone, so a byte-frozen replay
// can age out of the acceptance window and strand the head live forever. These
// tests drive the real enqueue helpers for BOTH coordinates (30176 team,
// 30178 catalog) through a stub relay that enforces that exact gate, including
// the delayed/offline-retry case where the tombstone was signed strictly past
// a future head. Gated off Windows like `persona_events::flush_barrier`:
// `build_app_state()` pulls native DLLs unavailable on the Windows runner.
#![cfg(not(target_os = "windows"))]

use super::*;
use crate::app_state::build_app_state;
use crate::managed_agents::persona_events::flush_pending_events;
use crate::managed_agents::team_catalog::build_team_catalog_delete;
use crate::managed_agents::team_events::{build_team_delete, build_team_event};
use buzz_core_pkg::kind::KIND_TEAM_CATALOG;
use std::sync::{Arc, Mutex};

const RELAY_ACCEPT_WINDOW_SECS: i64 = 900;

/// A single `POST /events` the stub saw: its `kind`, `created_at`, and whether
/// the ±900s gate accepted it. Recording every attempt — not just accepts —
/// lets a test assert the beyond-window branch emits ZERO posts, which is the
/// only assertion that distinguishes the domination-aware flush from the
/// byte-frozen replay it replaces (that replay DOES post, and is merely
/// rejected).
#[derive(Clone, Copy)]
struct PostAttempt {
    kind: u64,
    created_at: i64,
    accepted: bool,
}

/// Every `POST /events` the gate stub received, in order.
type PostLog = Arc<Mutex<Vec<PostAttempt>>>;

/// Stub relay enforcing the real ingest timestamp gate: `POST /events`
/// rejects any event whose `created_at` is more than ±900s from server
/// time (HTTP 200 + `accepted:false`, which the submit path treats as a
/// failure). It records EVERY post with its accept/reject status so tests can
/// assert both "no rejectable event was ever sent" and domination of the head.
/// Returns the HTTP base URL and the shared post log.
async fn spawn_gate_relay() -> (String, PostLog) {
    use axum::{extract::State, routing::post, Json, Router};

    let posts: PostLog = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/events",
            post(|State(log): State<PostLog>, body: String| async move {
                let event: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let kind = event.get("kind").and_then(serde_json::Value::as_u64);
                let created_at = event.get("created_at").and_then(serde_json::Value::as_i64);
                let now = chrono::Utc::now().timestamp();
                let accepted =
                    created_at.is_some_and(|ts| (ts - now).abs() <= RELAY_ACCEPT_WINDOW_SECS);
                log.lock().unwrap().push(PostAttempt {
                    kind: kind.unwrap_or(0),
                    created_at: created_at.unwrap_or_default(),
                    accepted,
                });
                if !accepted {
                    return Json(serde_json::json!({
                        "event_id": "",
                        "accepted": false,
                        "message": "event timestamp too far from server time"
                    }));
                }
                Json(serde_json::json!({
                    "event_id": event.get("id").and_then(serde_json::Value::as_str).unwrap_or(""),
                    "accepted": true,
                    "message": ""
                }))
            }),
        )
        .with_state(posts.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gate relay");
    let addr = listener.local_addr().expect("gate relay addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    (format!("http://{addr}"), posts)
}

/// Seed a kind:5 tombstone already signed at `floor` seconds since epoch and
/// then aged into the past — the delayed/offline retry state. When the
/// tombstone was signed, `floor` was strictly past a then-future head
/// (`monotonic_created_at(Some(head)) = head + 1`); the client was offline, the
/// wall clock advanced beyond `floor`, and now `floor` sits more than 900s in
/// the PAST. A byte-frozen replay at `floor` is rejected by the gate; only a
/// re-date to `now` can publish. This reproduces the aged queue row directly
/// rather than sleeping, so the delayed retry is deterministic. `target_kind`
/// selects the retracted coordinate (30176 team or 30178 catalog).
fn seed_stale_tombstone(db_path: &Path, keys: &nostr::Keys, target_kind: u32, floor: i64) {
    let owner = keys.public_key().to_hex();
    let builder = if target_kind == KIND_TEAM_CATALOG {
        build_team_catalog_delete("team-abc", &owner)
    } else {
        build_team_delete("team-abc", &owner)
    }
    .unwrap();
    let event = builder
        .custom_created_at(nostr::Timestamp::from(floor as u64))
        .sign_with_keys(keys)
        .unwrap();
    let conn = open_retention_db(db_path).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_DELETE,
            pubkey: owner,
            d_tag: tombstone_retention_d_tag(target_kind, "team-abc"),
            content: event.content.to_string(),
            created_at: floor,
            raw_event: event.as_json(),
            pending_sync: true,
        },
    )
    .unwrap();
}

/// Seed a retained 30176 team head dated `created_at` seconds since epoch.
fn seed_team_head(db_path: &Path, keys: &nostr::Keys, created_at: i64) {
    let event = build_team_event(&team())
        .unwrap()
        .custom_created_at(nostr::Timestamp::from(created_at as u64))
        .sign_with_keys(keys)
        .unwrap();
    let conn = open_retention_db(db_path).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_TEAM,
            pubkey: keys.public_key().to_hex(),
            d_tag: "team-abc".to_string(),
            content: event.content.to_string(),
            created_at,
            raw_event: event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
}

fn app_state_for(keys: nostr::Keys, relay_http: &str) -> crate::app_state::AppState {
    let state = build_app_state();
    *state.keys.lock().unwrap() = keys;
    *state.relay_url_override.lock().unwrap() = Some(relay_http.to_string());
    state
}

/// A tombstone signed strictly past a head that is already inside the
/// relay window publishes verbatim at that floor and dominates the head —
/// the delayed retry that lands once the wall clock is within 900s of the
/// signed timestamp.
#[tokio::test]
async fn catalog_tombstone_within_window_publishes_and_dominates() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let head = nostr::Timestamp::now().as_secs() as i64 + 600;
    seed_catalog_head(&db_path, &keys, head);
    tombstone_team_catalog_at(&db_path, &keys, "team-abc").unwrap();
    let floor = enqueued_tombstone(&db_path).created_at;
    assert!(
        floor > head,
        "tombstone must dominate the head before flush"
    );

    let (relay_http, posts) = spawn_gate_relay().await;
    let state = app_state_for(keys, &relay_http);
    let flushed = flush_pending_events(&db_path, &state).await.unwrap();

    assert_eq!(flushed, 1, "the in-window tombstone must publish");
    let posts = posts.lock().unwrap();
    assert_eq!(posts.len(), 1, "gate saw exactly the tombstone");
    assert!(posts[0].accepted, "the in-window tombstone was accepted");
    assert_eq!(posts[0].kind, KIND_DELETE as u64);
    assert!(
        posts[0].created_at > head,
        "accepted tombstone {} must dominate head {head}",
        posts[0].created_at
    );
    let conn = open_retention_db(&db_path).unwrap();
    assert!(
        !get_pending_sync(&conn).unwrap().iter().any(|r| r.kind == 5),
        "the published tombstone must be marked synced"
    );
}

/// A tombstone signed further ahead than the relay window is NOT sent — it
/// stays pending and converges as the wall clock advances toward its floor,
/// instead of being published and rejected forever. The gate never sees a
/// rejectable event.
#[tokio::test]
async fn catalog_tombstone_beyond_window_stays_pending_never_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let head = nostr::Timestamp::now().as_secs() as i64 + 5_000;
    seed_catalog_head(&db_path, &keys, head);
    tombstone_team_catalog_at(&db_path, &keys, "team-abc").unwrap();

    let (relay_http, posts) = spawn_gate_relay().await;
    let state = app_state_for(keys, &relay_http);
    let flushed = flush_pending_events(&db_path, &state).await.unwrap();

    assert_eq!(flushed, 0, "a beyond-window tombstone must not publish");
    assert!(
        posts.lock().unwrap().is_empty(),
        "the gate must never receive an out-of-window event — zero POSTs, not just zero accepts"
    );
    let conn = open_retention_db(&db_path).unwrap();
    assert!(
        get_pending_sync(&conn)
            .unwrap()
            .iter()
            .any(|r| r.kind == 5 && r.pending_sync),
        "the tombstone stays pending to converge on a later sweep"
    );
}

/// Delayed/offline retry (catalog 30178): a tombstone signed strictly past a
/// then-future head has sat in the queue while the client was offline until its
/// signed floor aged more than 900s into the PAST. A byte-frozen replay at the
/// stale floor is rejected forever; the flush must re-date to `now`, which the
/// gate accepts and which still dominates the head (whose `created_at` is below
/// the stale floor, hence also below `now`).
#[tokio::test]
async fn catalog_tombstone_stale_retry_redates_to_now_and_dominates() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    // Signed when the head was ~1h in the future; the client stayed offline
    // long enough that the floor is now ~1h in the past — well beyond ±900s.
    let stale_floor = nostr::Timestamp::now().as_secs() as i64 - 3_600;
    seed_stale_tombstone(&db_path, &keys, KIND_TEAM_CATALOG, stale_floor);

    let (relay_http, posts) = spawn_gate_relay().await;
    let before = nostr::Timestamp::now().as_secs() as i64;
    let state = app_state_for(keys, &relay_http);
    let flushed = flush_pending_events(&db_path, &state).await.unwrap();
    let after = nostr::Timestamp::now().as_secs() as i64;

    assert_eq!(flushed, 1, "the stale tombstone must re-date and publish");
    let posts = posts.lock().unwrap();
    assert_eq!(posts.len(), 1, "exactly one POST — the re-dated tombstone");
    assert!(
        posts[0].accepted,
        "the re-dated tombstone must clear the gate; a stale replay would be rejected"
    );
    assert_eq!(posts[0].kind, KIND_DELETE as u64);
    let ts = posts[0].created_at;
    assert!(
        ts >= before && ts <= after,
        "tombstone re-dated to wall clock, not left at the stale floor {stale_floor}; got {ts}"
    );
    assert!(
        ts > stale_floor,
        "the re-dated tombstone dominates the head, which was below the stale floor {stale_floor}"
    );
    let conn = open_retention_db(&db_path).unwrap();
    assert!(
        !get_pending_sync(&conn).unwrap().iter().any(|r| r.kind == 5),
        "the published tombstone must be marked synced"
    );
}

/// The sibling 30176 team tombstone flows through the identical gate — the
/// flush fix is coordinate-agnostic, so fixing only the catalog helper would
/// have left team deletion broken (Carl's explicit note).
#[tokio::test]
async fn team_tombstone_within_window_publishes_and_dominates() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let head = nostr::Timestamp::now().as_secs() as i64 + 600;
    seed_team_head(&db_path, &keys, head);
    super::super::super::tombstone_team_at(&db_path, &keys, "team-abc").unwrap();
    let floor = enqueued_tombstone(&db_path).created_at;
    assert!(
        floor > head,
        "team tombstone must dominate the head before flush"
    );

    let (relay_http, posts) = spawn_gate_relay().await;
    let state = app_state_for(keys, &relay_http);
    let flushed = flush_pending_events(&db_path, &state).await.unwrap();

    assert_eq!(flushed, 1, "the in-window team tombstone must publish");
    let posts = posts.lock().unwrap();
    assert_eq!(posts.len(), 1);
    assert!(
        posts[0].accepted,
        "the in-window team tombstone was accepted"
    );
    assert_eq!(posts[0].kind, KIND_DELETE as u64);
    assert!(
        posts[0].created_at > head,
        "accepted team tombstone {} must dominate head {head}",
        posts[0].created_at
    );
}

/// A beyond-window 30176 tombstone likewise stays pending rather than
/// publishing an event the relay would reject.
#[tokio::test]
async fn team_tombstone_beyond_window_stays_pending_never_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let head = nostr::Timestamp::now().as_secs() as i64 + 5_000;
    seed_team_head(&db_path, &keys, head);
    super::super::super::tombstone_team_at(&db_path, &keys, "team-abc").unwrap();

    let (relay_http, posts) = spawn_gate_relay().await;
    let state = app_state_for(keys, &relay_http);
    let flushed = flush_pending_events(&db_path, &state).await.unwrap();

    assert_eq!(
        flushed, 0,
        "a beyond-window team tombstone must not publish"
    );
    assert!(
        posts.lock().unwrap().is_empty(),
        "zero POSTs — the gate never sees an out-of-window team tombstone"
    );
    let conn = open_retention_db(&db_path).unwrap();
    assert!(
        get_pending_sync(&conn)
            .unwrap()
            .iter()
            .any(|r| r.kind == 5 && r.pending_sync),
        "the team tombstone stays pending to converge later"
    );
}

/// Delayed/offline retry (team 30176): the sibling coordinate must re-date a
/// stale-floored tombstone identically — Carl's contract requires the
/// delayed-retry case for BOTH coordinates, and the flush fix is
/// coordinate-agnostic.
#[tokio::test]
async fn team_tombstone_stale_retry_redates_to_now_and_dominates() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let stale_floor = nostr::Timestamp::now().as_secs() as i64 - 3_600;
    seed_stale_tombstone(&db_path, &keys, KIND_TEAM, stale_floor);

    let (relay_http, posts) = spawn_gate_relay().await;
    let before = nostr::Timestamp::now().as_secs() as i64;
    let state = app_state_for(keys, &relay_http);
    let flushed = flush_pending_events(&db_path, &state).await.unwrap();
    let after = nostr::Timestamp::now().as_secs() as i64;

    assert_eq!(
        flushed, 1,
        "the stale team tombstone must re-date and publish"
    );
    let posts = posts.lock().unwrap();
    assert_eq!(
        posts.len(),
        1,
        "exactly one POST — the re-dated team tombstone"
    );
    assert!(
        posts[0].accepted,
        "the re-dated team tombstone must clear the gate; a stale replay would be rejected"
    );
    assert_eq!(posts[0].kind, KIND_DELETE as u64);
    let ts = posts[0].created_at;
    assert!(
        ts >= before && ts <= after,
        "team tombstone re-dated to wall clock, not left at the stale floor {stale_floor}; got {ts}"
    );
    assert!(
        ts > stale_floor,
        "the re-dated team tombstone dominates its head, below the stale floor {stale_floor}"
    );
    let conn = open_retention_db(&db_path).unwrap();
    assert!(
        !get_pending_sync(&conn).unwrap().iter().any(|r| r.kind == 5),
        "the published team tombstone must be marked synced"
    );
}
