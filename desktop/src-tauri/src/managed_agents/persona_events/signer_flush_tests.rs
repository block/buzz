//! Real retention publisher tests; these do not model the transactional writers.
use super::*;
use crate::active_user_signer::tests::ControlledSigner;
use crate::managed_agents::retention::{
    get_retained_event, open_retention_db, retain_event, RetainedEvent,
};
use axum::{http::HeaderMap, routing::post, Router};
use base64::{engine::general_purpose::STANDARD, Engine};
use nostr::{Event, JsonUtil, Keys, Timestamp};
use std::{path::Path, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle};

async fn relay() -> (String, mpsc::Receiver<(Event, Event)>, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(8);
    let router = Router::new().route(
        "/events",
        post(move |headers: HeaderMap, body: String| {
            let tx = tx.clone();
            async move {
                let event = Event::from_json(&body).unwrap();
                let auth = headers["authorization"]
                    .to_str()
                    .unwrap()
                    .strip_prefix("Nostr ")
                    .unwrap();
                let auth = Event::from_json(STANDARD.decode(auth).unwrap()).unwrap();
                tx.send((event.clone(), auth)).await.unwrap();
                axum::Json(serde_json::json!({
                    "event_id": event.id.to_hex(), "accepted": true, "message": ""
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (url, rx, server)
}

fn seed(path: &Path, keys: &Keys, archive: bool, time: u64, tag: &str) -> RetainedEvent {
    let builder = if archive {
        crate::events::build_archive_identity_request(
            &keys.public_key().to_hex(),
            "historical persona alias",
            Some("retired"),
            None,
            None,
        )
        .unwrap()
    } else {
        crate::managed_agents::team_catalog::build_team_catalog_delete(
            "team",
            &keys.public_key().to_hex(),
        )
        .unwrap()
    };
    let event = builder
        .tag(Tag::parse(["test", tag]).unwrap())
        .custom_created_at(Timestamp::from(time))
        .sign_with_keys(keys)
        .unwrap();
    let row = RetainedEvent {
        kind: event.kind.as_u16() as u32,
        pubkey: event.pubkey.to_hex(),
        d_tag: if archive { "agent" } else { "30178:team" }.into(),
        content: event.content.clone(),
        created_at: time as i64,
        raw_event: event.as_json(),
        pending_sync: true,
    };
    retain_event(&open_retention_db(path).unwrap(), &row).unwrap();
    row
}

fn reread(path: &Path, row: &RetainedEvent) -> RetainedEvent {
    get_retained_event(
        &open_retention_db(path).unwrap(),
        row.kind,
        &row.pubkey,
        &row.d_tag,
    )
    .unwrap()
    .unwrap()
}

async fn finish(task: JoinHandle<Result<u32, String>>) -> Result<u32, String> {
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn scoped_archive_flush_keeps_owner_relay_tags_and_retained_retry_identity() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retention.db");
    let controlled = ControlledSigner::new(false);
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    let row = seed(&path, &controlled.keys, true, 1, "original");
    let (url, mut rx, server) = relay().await;
    let state = Arc::new(crate::app_state::build_app_state());
    state
        .replace_local_identity_keys(controlled.keys.clone())
        .unwrap();
    let task = {
        let (path, state, url) = (path.clone(), state.clone(), url.clone());
        tokio::spawn(async move { flush_pending_events_at(&path, &state, &url, &signer).await })
    };
    controlled.wait_entered().await;
    assert!(rx.try_recv().is_err());
    state.replace_local_identity_keys(Keys::generate()).unwrap();
    *state.relay_url_override.lock().unwrap() = Some("http://127.0.0.1:1".into());
    // A writer can take SQLite's write reservation while signing is suspended.
    let conn = open_retention_db(&path).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE; ROLLBACK").unwrap();
    drop(conn);
    controlled.release.notify_one();
    controlled.wait_entered().await; // NIP-98, also the captured owner
    assert!(rx.try_recv().is_err());
    controlled.release.notify_one();
    assert_eq!(finish(task).await.unwrap(), 1);
    let (event, auth) = rx.recv().await.unwrap();
    let original = Event::from_json(&row.raw_event).unwrap();
    assert_eq!(event.pubkey, controlled.keys.public_key());
    assert_eq!(auth.pubkey, event.pubkey);
    assert_ne!(event.pubkey, state.active_signer().unwrap().public_key());
    assert_eq!(event.tags, original.tags); // includes self-target p tag
    assert_eq!(event.content, original.content);
    assert!(event.created_at > original.created_at);
    assert_eq!(
        auth.tags
            .find(nostr::TagKind::custom("u"))
            .unwrap()
            .content(),
        Some(format!("{url}/events").as_str())
    );
    event.verify().unwrap();
    auth.verify().unwrap();
    let retained = reread(&path, &row);
    assert_eq!(retained.raw_event, row.raw_event);
    assert!(!retained.pending_sync);
    server.abort();
}

#[tokio::test]
async fn failed_or_cancelled_resign_keeps_retry_row_and_releases_publisher() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    for fail in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retention.db");
        let controlled = ControlledSigner::new(fail);
        let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
        let row = seed(&path, &controlled.keys, false, 1, "original");
        let (url, mut rx, server) = relay().await;
        let state = Arc::new(crate::app_state::build_app_state());
        let task = {
            let (path, state, url) = (path.clone(), state.clone(), url.clone());
            tokio::spawn(async move { flush_pending_events_at(&path, &state, &url, &signer).await })
        };
        controlled.wait_entered().await;
        assert!(rx.try_recv().is_err());
        if fail {
            controlled.release.notify_one();
            assert!(finish(task)
                .await
                .unwrap_err()
                .contains("deliberate signer failure"));
        } else {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        }
        let retained = reread(&path, &row);
        assert_eq!(retained.raw_event, row.raw_event);
        assert!(retained.pending_sync);
        // An ordinary retry uses the real publisher and must not hang on a leaked lock.
        let signer = ActiveUserSigner::local(controlled.keys.clone());
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(5),
                flush_pending_events_at(&path, &state, &url, &signer)
            )
            .await
            .unwrap()
            .unwrap(),
            1
        );
        let (event, _) = rx.recv().await.unwrap();
        event.verify().unwrap();
        assert!(!reread(&path, &row).pending_sync);
        server.abort();
    }
}

#[tokio::test]
async fn equal_second_tag_replacement_during_signing_is_not_published_or_cleared() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retention.db");
    let controlled = ControlledSigner::new(false);
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    let time = Timestamp::now().as_secs();
    let row = seed(&path, &controlled.keys, false, time, "original");
    let (url, mut rx, server) = relay().await;
    let state = Arc::new(crate::app_state::build_app_state());
    let task = {
        let (path, state, url) = (path.clone(), state.clone(), url.clone());
        tokio::spawn(async move { flush_pending_events_at(&path, &state, &url, &signer).await })
    };
    controlled.wait_entered().await;
    let replacement = seed(&path, &controlled.keys, false, time, "replacement");
    assert_eq!(row.created_at, replacement.created_at);
    assert_eq!(row.content, replacement.content);
    assert_ne!(row.raw_event, replacement.raw_event);
    controlled.release.notify_one();
    assert_eq!(finish(task).await.unwrap(), 0);
    assert!(rx.try_recv().is_err());
    assert_eq!(reread(&path, &row).raw_event, replacement.raw_event);
    assert!(reread(&path, &row).pending_sync);
    let signer = ActiveUserSigner::local(controlled.keys.clone());
    assert_eq!(
        flush_pending_events_at(&path, &state, &url, &signer)
            .await
            .unwrap(),
        1
    );
    let (event, _) = rx.recv().await.unwrap();
    assert_eq!(
        event.tags,
        Event::from_json(&replacement.raw_event).unwrap().tags
    );
    server.abort();
}

#[tokio::test]
async fn future_floor_is_preserved_and_out_of_window_tombstone_never_signs() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    for offset in [100, 86_400] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retention.db");
        let controlled = ControlledSigner::new(false);
        let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
        let time = Timestamp::now().as_secs() + offset;
        let row = seed(&path, &controlled.keys, false, time, "future");
        let (url, mut rx, server) = relay().await;
        let state = crate::app_state::build_app_state();
        let task = {
            let path = path.clone();
            tokio::spawn(async move { flush_pending_events_at(&path, &state, &url, &signer).await })
        };
        if offset > 900 {
            assert_eq!(finish(task).await.unwrap(), 0);
            assert!(rx.try_recv().is_err());
            assert!(reread(&path, &row).pending_sync);
        } else {
            controlled.wait_entered().await;
            controlled.release.notify_one();
            controlled.wait_entered().await;
            controlled.release.notify_one();
            assert_eq!(finish(task).await.unwrap(), 1);
            let (event, _) = rx.recv().await.unwrap();
            assert_eq!(event.created_at.as_secs(), time);
            event.verify().unwrap();
        }
        server.abort();
    }
}

#[tokio::test]
async fn replacement_during_nip98_signing_is_left_pending_after_old_post_completes() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retention.db");
    let controlled = ControlledSigner::new(false);
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    let time = Timestamp::now().as_secs();
    let row = seed(&path, &controlled.keys, false, time, "original");
    let (url, mut rx, server) = relay().await;
    let state = Arc::new(crate::app_state::build_app_state());
    let task = {
        let (path, state, url) = (path.clone(), state.clone(), url.clone());
        tokio::spawn(async move { flush_pending_events_at(&path, &state, &url, &signer).await })
    };
    controlled.wait_entered().await;
    controlled.release.notify_one();
    controlled.wait_entered().await; // after the exact-row re-read, inside NIP-98 signing
    let replacement = seed(&path, &controlled.keys, false, time, "replacement");
    assert_eq!(row.created_at, replacement.created_at);
    assert_eq!(row.content, replacement.content);
    controlled.release.notify_one();
    assert_eq!(finish(task).await.unwrap(), 1);
    let (old, _) = rx.recv().await.unwrap();
    assert_eq!(old.tags, Event::from_json(&row.raw_event).unwrap().tags);
    assert_eq!(reread(&path, &row).raw_event, replacement.raw_event);
    assert!(reread(&path, &row).pending_sync);
    let signer = ActiveUserSigner::local(controlled.keys.clone());
    assert_eq!(
        flush_pending_events_at(&path, &state, &url, &signer)
            .await
            .unwrap(),
        1
    );
    let (new, _) = rx.recv().await.unwrap();
    assert_eq!(
        new.tags,
        Event::from_json(&replacement.raw_event).unwrap().tags
    );
    assert!(!reread(&path, &row).pending_sync);
    server.abort();
}

#[tokio::test]
async fn retention_scope_publisher_uses_captured_capability_not_rebuilt_local_keys() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let controlled = ControlledSigner::new(true);
    let state = std::sync::Arc::new(crate::app_state::build_app_state());
    state
        .replace_local_identity_keys(controlled.keys.clone())
        .unwrap();
    *state.test_signer.lock().unwrap() =
        Some(ActiveUserSigner::new(controlled.clone()).await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scope.db");
    let row = seed(&path, &controlled.keys, false, 123, "scope");
    let (url, mut rx, server) = relay().await;
    let signer = state.active_signer().unwrap();
    let scope = crate::managed_agents::retention::RetentionScope {
        db_path: path.clone(),
        relay_url: url,
        signer,
    };
    let task_state = state.clone();
    let task = tokio::spawn(async move {
        flush_pending_events_at(
            &scope.db_path,
            &task_state,
            &scope.relay_url,
            &scope.owner_signer(),
        )
        .await
    });
    controlled.wait_entered().await;
    state.replace_local_identity_keys(Keys::generate()).unwrap();
    *state.test_signer.lock().unwrap() = None;
    assert!(state.managed_agents_store_lock.try_lock().is_ok());
    let conn = open_retention_db(&path).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE; ROLLBACK").unwrap();
    controlled.release.notify_one();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .contains("deliberate signer failure"));
    assert!(reread(&path, &row).pending_sync);
    assert!(rx.try_recv().is_err());
    server.abort();
}
