use super::*;
use crate::{
    app_state::build_app_state,
    commands::teams::pending::prepare_team_publication_at,
    managed_agents::{
        retention::{get_retained_event, open_retention_db, RetentionScope},
        AgentDefinition,
    },
};
use buzz_core_pkg::kind::KIND_TEAM_CATALOG;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn member(id: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: "One".to_string(),
        description: None,
        avatar_url: None,
        system_prompt: "Do the work.".to_string(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

fn team() -> TeamRecord {
    TeamRecord {
        id: "team-abc".to_string(),
        name: "Catalog Team".to_string(),
        description: None,
        instructions: None,
        persona_ids: vec!["m1".to_string()],
        is_builtin: false,
        shared: false,
        catalog_source: None,
        source_dir: Some(PathBuf::from("/local/only/path")),
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

async fn spawn_relay(accepted: bool) -> String {
    use axum::{routing::post, Router};

    let app = Router::new().route(
        "/events",
        post(move |body: String| async move {
            let event: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            serde_json::json!({
                "event_id": event.get("id").and_then(serde_json::Value::as_str).unwrap_or(""),
                "accepted": accepted,
                "message": if accepted { "" } else { "policy rejection" }
            })
            .to_string()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    format!("http://{addr}")
}

fn prepared(
    db_path: &std::path::Path,
    relay_url: String,
    keys: nostr::Keys,
    shared: bool,
) -> PreparedTeamPublication {
    let (_event, retained, team) =
        prepare_team_publication_at(db_path, &keys, &team(), &[member("m1")], Some(shared))
            .unwrap();
    PreparedTeamPublication {
        scope: RetentionScope {
            db_path: db_path.to_path_buf(),
            relay_url,
            owner_keys: keys,
        },
        retained,
        team,
    }
}

fn retained_head(
    db_path: &std::path::Path,
    owner: &str,
) -> crate::managed_agents::retention::RetainedEvent {
    get_retained_event(
        &open_retention_db(db_path).unwrap(),
        KIND_TEAM_CATALOG,
        owner,
        "team-abc",
    )
    .unwrap()
    .expect("the head is retained before the relay is ever contacted")
}

#[tokio::test]
async fn test_accepted_share_reports_published_and_clears_the_pending_flag() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let prepared = prepared(&db_path, spawn_relay(true).await, keys, true);
    let state = build_app_state();

    let result = publish_prepared_team(&state, prepared).await.unwrap();

    assert_eq!(
        result.publication_status,
        TeamSharePublicationStatus::Published
    );
    assert!(result.team.shared);
    assert!(
        !retained_head(&db_path, &owner).pending_sync,
        "a confirmed publish must not be republished by the flush loop"
    );
}

#[tokio::test]
async fn test_relay_rejection_stays_durably_queued() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let prepared = prepared(&db_path, spawn_relay(false).await, keys, true);
    let state = build_app_state();

    let result = publish_prepared_team(&state, prepared).await.unwrap();

    assert_eq!(
        result.publication_status,
        TeamSharePublicationStatus::Queued
    );
    // The head publishes through the flush loop, which swallows the relay's
    // per-event rejection to its own log, so the queued outcome no longer
    // carries the relay message — only the durable pending row proves it will
    // retry.
    assert!(
        retained_head(&db_path, &owner).pending_sync,
        "a rejected share stays pending for the flush loop to retry"
    );
}

#[tokio::test]
async fn test_unavailable_relay_stays_durably_queued() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let prepared = prepared(&db_path, relay_url, keys, true);
    let state = build_app_state();

    let result = publish_prepared_team(&state, prepared).await.unwrap();

    assert_eq!(
        result.publication_status,
        TeamSharePublicationStatus::Queued
    );
    assert!(
        retained_head(&db_path, &owner).pending_sync,
        "an offline share must survive for the flush loop rather than failing the command"
    );
}

#[tokio::test]
async fn test_unshare_leaves_an_untagged_head_retained_after_publication() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let relay_url = spawn_relay(true).await;
    let state = build_app_state();
    publish_prepared_team(
        &state,
        prepared(&db_path, relay_url.clone(), keys.clone(), true),
    )
    .await
    .unwrap();

    let result = publish_prepared_team(&state, prepared(&db_path, relay_url, keys, false))
        .await
        .unwrap();

    assert_eq!(
        result.publication_status,
        TeamSharePublicationStatus::Published
    );
    assert!(!result.team.shared);
    let row = retained_head(&db_path, &owner);
    assert!(
        !buzz_core_pkg::kind::event_is_shared(
            &<nostr::Event as nostr::JsonUtil>::from_json(&row.raw_event).unwrap()
        ),
        "unshare retracts by replacement, so the coordinate stays readable by its author"
    );
}

/// A recording relay: accepts every `POST /events` and logs each event's
/// `kind`, so a test can assert exactly which coordinates reached the relay
/// and in what order.
async fn spawn_recording_relay() -> (String, Arc<Mutex<Vec<u64>>>) {
    use axum::{extract::State, routing::post, Json, Router};

    let kinds: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/events",
            post(|State(log): State<Arc<Mutex<Vec<u64>>>>, body: String| async move {
                let event: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                if let Some(kind) = event.get("kind").and_then(serde_json::Value::as_u64) {
                    log.lock().unwrap().push(kind);
                }
                Json(serde_json::json!({
                    "event_id": event.get("id").and_then(serde_json::Value::as_str).unwrap_or(""),
                    "accepted": true,
                    "message": ""
                }))
            }),
        )
        .with_state(kinds.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    (format!("http://{addr}"), kinds)
}

/// P1 (Carl/Wes): a share delayed past a concurrent team deletion must NOT
/// resurrect the deleted catalog entry.
///
/// Carl's contract interleave: prepare the share (retains a pending 30178
/// head) → a concurrent `delete_team` purges that head and enqueues a newer
/// 30178 tombstone (`tombstone_team_catalog_at`, one atomic transaction) →
/// FLUSH the tombstone to the relay → THEN release the delayed share. Because
/// the share now routes through the flush loop rather than submitting the
/// prepared event directly, and the flush re-reads each row before publishing,
/// the purged head can never reach the relay after its tombstone. The assertion
/// that distinguishes the fix from the bug: after the tombstone has landed, the
/// delayed share publishes NO 30178 head, and no pending 30178 row survives to
/// publish it later. Under the reverted direct-submit path the share would
/// re-post the 30178 head here and resurrect the deleted team.
#[tokio::test]
async fn delayed_share_after_delete_never_republishes_the_catalog_head() {
    use crate::commands::teams::pending::tombstone_team_catalog_at;
    use crate::managed_agents::persona_events::flush_pending_events_at;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let (relay_url, relayed_kinds) = spawn_recording_relay().await;

    // 1. Prepare the share: a pending 30178 head is retained but not yet sent.
    let prepared = prepared(&db_path, relay_url.clone(), keys.clone(), true);
    assert!(
        retained_head(&db_path, &owner).pending_sync,
        "the share is retained pending before any publish"
    );

    // 2. Concurrent delete: purge the retained head and enqueue a newer
    //    30178 tombstone, atomically — exactly what `delete_team` does.
    tombstone_team_catalog_at(&db_path, &keys, "team-abc").unwrap();
    assert!(
        get_retained_event(
            &open_retention_db(&db_path).unwrap(),
            KIND_TEAM_CATALOG,
            &owner,
            "team-abc"
        )
        .unwrap()
        .is_none(),
        "the delete purged the retained 30178 head"
    );

    // 3. Flush the tombstone to the relay (Carl's contract: the tombstone
    //    lands BEFORE the delayed publish is released).
    let state = build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(relay_url);
    flush_pending_events_at(
        &db_path,
        &state,
        &prepared.scope.relay_url,
        &prepared.scope.owner_keys,
    )
    .await
    .unwrap();
    assert!(
        relayed_kinds.lock().unwrap().contains(&5),
        "the deletion tombstone reached the relay before the delayed publish"
    );

    // 4. The delayed share finally publishes — through the flush loop.
    let result = publish_prepared_team(&state, prepared).await.unwrap();

    // The purged head was never resurrected: after the tombstone landed, the
    // delayed share published NO 30178 head, and no pending 30178 row survived.
    // The direct-submit path this replaces would re-post the head here.
    let kinds = relayed_kinds.lock().unwrap();
    assert!(
        !kinds.contains(&(KIND_TEAM_CATALOG as u64)),
        "the deleted team's 30178 head must NEVER be published after its tombstone; relay saw {kinds:?}"
    );
    assert_eq!(
        result.publication_status,
        TeamSharePublicationStatus::Queued,
        "with its head purged, the share has nothing live to publish"
    );
    let conn = open_retention_db(&db_path).unwrap();
    assert!(
        get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-abc")
            .unwrap()
            .is_none(),
        "no local 30178 head survives to resurrect the deleted team"
    );
}

/// A recording relay that GATES the first 30178 head POST: it signals the test
/// the moment that POST arrives, then blocks the response until the test
/// releases it. This holds a flush *inside* the await gap between its row
/// re-read and its relay POST — the exact window a second concurrent flush
/// could otherwise use to publish a deletion tombstone first. Kind-5 tombstone
/// POSTs are recorded and answered immediately. The recorded kind order is the
/// relay's landing order (each kind is pushed only once its response is sent).
async fn spawn_gated_recording_relay() -> (String, Arc<Mutex<Vec<u64>>>, GatedRelay) {
    use axum::{extract::State, routing::post, Json, Router};

    let kinds: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let (reached_tx, reached_rx) = tokio::sync::oneshot::channel::<()>();
    let gate = GatedRelayInner {
        kinds: kinds.clone(),
        reached_head_post: Arc::new(Mutex::new(Some(reached_tx))),
        release_head_post: Arc::new(tokio::sync::Notify::new()),
    };
    let release_head_post = gate.release_head_post.clone();

    let app = Router::new()
        .route(
            "/events",
            post(|State(gate): State<GatedRelayInner>, body: String| async move {
                let event: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let kind = event.get("kind").and_then(serde_json::Value::as_u64);
                if kind == Some(KIND_TEAM_CATALOG as u64) {
                    // Flush H has reached its head POST (past the re-read,
                    // holding the publisher lock). Signal the test, then block
                    // until it releases us — pinning H inside the await gap.
                    if let Some(tx) = gate.reached_head_post.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                    gate.release_head_post.notified().await;
                }
                if let Some(kind) = kind {
                    gate.kinds.lock().unwrap().push(kind);
                }
                Json(serde_json::json!({
                    "event_id": event.get("id").and_then(serde_json::Value::as_str).unwrap_or(""),
                    "accepted": true,
                    "message": ""
                }))
            }),
        )
        .with_state(gate);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    (
        format!("http://{addr}"),
        kinds,
        GatedRelay {
            reached_head_post: reached_rx,
            release_head_post,
        },
    )
}

#[derive(Clone)]
struct GatedRelayInner {
    kinds: Arc<Mutex<Vec<u64>>>,
    reached_head_post: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    release_head_post: Arc<tokio::sync::Notify>,
}

/// Test-side handles to the gated relay: `reached_head_post` fires when the
/// head POST arrives; `release_head_post` unblocks its response.
struct GatedRelay {
    reached_head_post: tokio::sync::oneshot::Receiver<()>,
    release_head_post: Arc<tokio::sync::Notify>,
}

/// P1-A (Thufir pass 1): the single-publisher invariant must be enforced by a
/// lock, not merely by the re-read. Two concurrent flushes race across the
/// re-read→POST await gap: flush H selects the live 30178 head and enters its
/// POST; a concurrent delete then purges that head and enqueues a kind-5
/// tombstone; flush D publishes the tombstone. Without serialization, D's
/// tombstone lands while H is still mid-POST, and H's delayed head lands
/// *after* it — the forbidden relay order `[5, 30178]` that resurrects the
/// deleted team.
///
/// The per-scope publisher lock (keyed by the retention db_path, held across
/// each flush's entire invocation) forbids that interleaving: H holds the lock
/// through its POST, so D cannot publish
/// the tombstone until H has finished. The only orderings left are
/// head-before-tombstone (`[30178, 5]`, the head dominated by the later
/// tombstone) or purged-row-skip (H re-reads after the delete and publishes
/// nothing). This test pins H inside its POST via the gated relay, commits the
/// delete, starts D, lets D attempt its POST, then releases H — and asserts the
/// relay order is `[30178, 5]`, never `[5, 30178]`. Removing the lock makes D
/// win the gap and turns this RED on `[5, 30178]`.
#[tokio::test]
async fn concurrent_flushes_never_land_the_head_after_its_tombstone() {
    use crate::commands::teams::pending::tombstone_team_catalog_at;
    use crate::managed_agents::persona_events::flush_pending_events_at;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let (relay_url, relayed_kinds, gate) = spawn_gated_recording_relay().await;

    // A pending 30178 head is retained but not yet published.
    let _prepared = prepared(&db_path, relay_url.clone(), keys.clone(), true);

    let state = Arc::new(build_app_state());
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(relay_url.clone());

    // Flush H: publishes the pending head. Its POST blocks in the gated relay,
    // holding the publisher lock across the await gap.
    let h = {
        let (state, db_path, relay_url, keys) = (
            state.clone(),
            db_path.clone(),
            relay_url.clone(),
            keys.clone(),
        );
        tokio::spawn(
            async move { flush_pending_events_at(&db_path, &state, &relay_url, &keys).await },
        )
    };

    // Wait until H is inside its head POST — past the re-read, lock held.
    gate.reached_head_post.await.unwrap();

    // Concurrent delete commits: purge the head, enqueue the kind-5 tombstone.
    tombstone_team_catalog_at(&db_path, &keys, "team-abc").unwrap();

    // Flush D: would publish the tombstone. Under the lock it blocks on H.
    let d = {
        let (state, db_path, relay_url, keys) = (
            state.clone(),
            db_path.clone(),
            relay_url.clone(),
            keys.clone(),
        );
        tokio::spawn(
            async move { flush_pending_events_at(&db_path, &state, &relay_url, &keys).await },
        )
    };

    // Give D time to reach its tombstone POST. Serialized, it is parked on the
    // lock; unserialized, it POSTs kind 5 now — while H is still blocked.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Release H's head POST. Serialized: H lands 30178, drops the lock, then D
    // lands 5. Unserialized: D already landed 5, so H's 30178 lands after it.
    gate.release_head_post.notify_one();

    h.await.unwrap().unwrap();
    d.await.unwrap().unwrap();

    let kinds = relayed_kinds.lock().unwrap().clone();
    assert_ne!(
        kinds,
        vec![5, KIND_TEAM_CATALOG as u64],
        "the purged head must NEVER land after its tombstone; relay saw {kinds:?}"
    );
    assert_eq!(
        kinds,
        vec![KIND_TEAM_CATALOG as u64, 5],
        "serialized flushes publish the head before its dominating tombstone; relay saw {kinds:?}"
    );
}

/// State for the stalling relay: records landed kinds and fires `reached` once
/// the head POST arrives.
#[derive(Clone)]
struct StallingRelayState {
    kinds: Arc<Mutex<Vec<u64>>>,
    reached: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

/// A recording relay that STALLS its head POST forever: it signals the test the
/// moment the 30178 head POST arrives, then never sends a response. This pins
/// the flush holding that scope's publisher lock inside its bounded relay await.
async fn spawn_stalling_head_relay() -> (
    String,
    Arc<Mutex<Vec<u64>>>,
    tokio::sync::oneshot::Receiver<()>,
) {
    use axum::{extract::State, routing::post, Json, Router};

    let kinds: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let (reached_tx, reached_rx) = tokio::sync::oneshot::channel::<()>();
    let state = StallingRelayState {
        kinds: kinds.clone(),
        reached: Arc::new(Mutex::new(Some(reached_tx))),
    };

    let app = Router::new()
        .route(
            "/events",
            post(
                |State(state): State<StallingRelayState>, body: String| async move {
                    let event: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let kind = event.get("kind").and_then(serde_json::Value::as_u64);
                    if kind == Some(KIND_TEAM_CATALOG as u64) {
                        if let Some(tx) = state.reached.lock().unwrap().take() {
                            let _ = tx.send(());
                        }
                        // Hold the response open forever: the client's POST
                        // never completes, so the flush must rely on its own
                        // bounded timeout to release the publisher lock.
                        std::future::pending::<()>().await;
                    }
                    if let Some(kind) = kind {
                        state.kinds.lock().unwrap().push(kind);
                    }
                    Json(serde_json::json!({
                        "event_id": event.get("id").and_then(serde_json::Value::as_str).unwrap_or(""),
                        "accepted": true,
                        "message": ""
                    }))
                },
            ),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    (format!("http://{addr}"), kinds, reached_rx)
}

/// P1-A follow-up (Thufir pass 2): the publisher lock is keyed per retention
/// scope, so a stalled relay in one community can NOT block publication in
/// another. Scope A's flush is pinned mid-POST on a relay that never responds
/// (holding scope A's lock); scope B's flush, on its own accepting relay, must
/// still publish without waiting on A. A process-global lock would deadlock B
/// behind A here. Re-globalizing the key turns this RED (B never publishes
/// within the harness bound).
#[tokio::test]
async fn a_stalled_scope_does_not_block_publication_in_another_scope() {
    use crate::managed_agents::persona_events::flush_pending_events_at;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let db_path_a = dir.path().join("scope_a.db");
    let db_path_b = dir.path().join("scope_b.db");
    let keys_a = nostr::Keys::generate();
    let keys_b = nostr::Keys::generate();
    let owner_b = keys_b.public_key().to_hex();

    let (relay_a, _kinds_a, reached_a) = spawn_stalling_head_relay().await;
    let (relay_b, kinds_b) = spawn_recording_relay().await;

    // A pending 30178 head in each scope, retained but not yet published.
    let _prep_a = prepared(&db_path_a, relay_a.clone(), keys_a.clone(), true);
    let _prep_b = prepared(&db_path_b, relay_b.clone(), keys_b.clone(), true);

    let state = Arc::new(build_app_state());

    // Flush A pins scope A's publisher lock: its head POST stalls forever.
    let _a = {
        let (state, db_path_a, relay_a, keys_a) = (
            state.clone(),
            db_path_a.clone(),
            relay_a.clone(),
            keys_a.clone(),
        );
        tokio::spawn(async move {
            let _ = flush_pending_events_at(&db_path_a, &state, &relay_a, &keys_a).await;
        })
    };
    reached_a.await.unwrap();

    // Scope B must publish while A is still stalled. Bound the wait so a
    // regression (global lock) fails RED instead of hanging the suite.
    let flushed_b = tokio::time::timeout(
        Duration::from_secs(10),
        flush_pending_events_at(&db_path_b, &state, &relay_b, &keys_b),
    )
    .await
    .expect("scope B must not be blocked by scope A's stalled relay")
    .expect("scope B flush");

    assert_eq!(flushed_b, 1, "scope B publishes its own pending head");
    assert!(
        kinds_b
            .lock()
            .unwrap()
            .contains(&(KIND_TEAM_CATALOG as u64)),
        "scope B's head reached its own relay while scope A stalled"
    );
    assert!(
        !retained_head(&db_path_b, &owner_b).pending_sync,
        "scope B's head is marked synced"
    );
}

/// P1-A follow-up (Thufir pass 2): a non-responding relay must not pin the
/// publisher lock forever — the per-row relay await is bounded, so the flush
/// returns (leaving the row pending) and drops its guard for the next sweep.
/// Time is paused: the production bound fires in virtual time, so each flush
/// completes well inside the harness bound. The first flush stalls on a relay
/// that never responds and must still return; the row stays pending. A *second*
/// flush on the SAME scope and SAME stalled relay must also return within the
/// harness bound — which is only possible if the first flush already dropped
/// its publisher guard (a leaked lock has no timer, so the second flush's mutex
/// await would never wake and tokio would advance to the harness bound and fire
/// it instead). Removing the production timeout makes the first flush hang on
/// the socket, the harness bound fires, and this turns RED.
#[tokio::test(start_paused = true)]
async fn a_stalled_relay_releases_the_publisher_lock_within_the_bound() {
    use crate::managed_agents::persona_events::flush_pending_events_at;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();

    let (stall_relay, _kinds, _reached) = spawn_stalling_head_relay().await;
    let _prep = prepared(&db_path, stall_relay.clone(), keys.clone(), true);
    let state = Arc::new(build_app_state());

    // First flush hits the stalled relay. It must return within its own bound
    // rather than hanging; the harness bound (much larger) only fires if the
    // production timeout is gone.
    let first = tokio::time::timeout(
        Duration::from_secs(600),
        flush_pending_events_at(&db_path, &state, &stall_relay, &keys),
    )
    .await;
    assert!(
        first.is_ok(),
        "the flush must return within its own timeout, not hang on a stalled relay"
    );
    assert!(
        retained_head(&db_path, &owner).pending_sync,
        "a timed-out publish leaves the row pending for the next sweep"
    );

    // A second flush on the SAME scope must also return within the harness
    // bound. It can only acquire the per-scope publisher lock if the first
    // flush dropped its guard on return; a leaked lock would park this flush on
    // a timer-less mutex await, so tokio would advance to the harness bound and
    // fire it instead of completing.
    let second = tokio::time::timeout(
        Duration::from_secs(600),
        flush_pending_events_at(&db_path, &state, &stall_relay, &keys),
    )
    .await;
    assert!(
        second.is_ok(),
        "the publisher lock was released, so a later flush on the same scope proceeds"
    );
    assert!(
        retained_head(&db_path, &owner).pending_sync,
        "the row is still pending after the second timed-out attempt"
    );
}
