//! Real command entrypoints with suspended signing and a loopback NIP-98 relay.
use super::*;
use crate::active_user_signer::{tests::ControlledSigner, ActiveUserSigner};
use crate::app_state::{build_app_state, AppState};
use axum::{body::Bytes, http::HeaderMap, routing::post, Json, Router};
use base64::Engine;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tauri::Manager;

struct Request {
    path: &'static str,
    headers: HeaderMap,
    body: Bytes,
}

struct Fixture {
    app: tauri::App<tauri::test::MockRuntime>,
    controlled: Arc<ControlledSigner>,
    requests: Arc<Mutex<Vec<Request>>>,
    base: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    async fn new(fail: bool, prior: Vec<Event>) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let posted = Arc::new(Mutex::new(Vec::<Event>::new()));
        let writes = requests.clone();
        let events = posted.clone();
        let reads = requests.clone();
        let router = Router::new()
            .route("/events", post(move |headers: HeaderMap, body: Bytes| {
                let writes = writes.clone();
                let events = events.clone();
                async move {
                    let event = Event::from_json(&body).unwrap();
                    writes.lock().unwrap().push(Request { path: "/events", headers, body });
                    events.lock().unwrap().push(event.clone());
                    Json(json!({"event_id": event.id.to_hex(), "accepted": true,
                        "message": json!({"channel_id": "00000000-0000-0000-0000-000000000001"}).to_string()}))
                }
            }))
            .route("/query", post(move |headers: HeaderMap, body: Bytes| {
                let reads = reads.clone();
                let posted = posted.clone();
                let prior = prior.clone();
                async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    reads.lock().unwrap().push(Request { path: "/query", headers, body });
                    let kind = filters[0]["kinds"][0].as_u64().unwrap();
                    let result = if kind == 39000 && filters[0]["#d"].is_null() {
                        prior.iter().filter(|e| e.kind.as_u16() == 39000).cloned().collect()
                    } else if kind == 39000 {
                        let id = filters[0]["#d"][0].as_str().unwrap();
                        vec![EventBuilder::new(Kind::Custom(39000), "")
                            .tags([Tag::parse(["d", id]).unwrap(), Tag::parse(["name", "test"]).unwrap()])
                            .sign_with_keys(&Keys::generate()).unwrap()]
                    } else {
                        let posted = posted.lock().unwrap();
                        posted.iter().chain(prior.iter())
                            .filter(|e| e.kind.as_u16() as u64 == kind).take(1).cloned().collect()
                    };
                    Json(json!(result))
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let controlled = ControlledSigner::new(fail);
        let state = build_app_state();
        state
            .replace_local_identity_keys(controlled.keys.clone())
            .unwrap();
        *state.test_signer.lock().unwrap() =
            Some(ActiveUserSigner::new(controlled.clone()).await.unwrap());
        *state.relay_url_override.lock().unwrap() = Some(base.clone());
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        Self {
            app,
            controlled,
            requests,
            base,
            task,
        }
    }

    fn switch_identity_and_relay(&self) {
        let state = self.app.state::<AppState>();
        let replacement = Keys::generate();
        state
            .replace_local_identity_keys(replacement.clone())
            .unwrap();
        *state.test_signer.lock().unwrap() = Some(ActiveUserSigner::local(replacement));
        *state.relay_url_override.lock().unwrap() = Some("http://127.0.0.1:1".into());
        assert!(state.managed_agents_store_lock.try_lock().is_ok());
    }

    async fn release(&self, count: usize) {
        for _ in 0..count {
            self.controlled.wait_entered().await;
            self.controlled.release.notify_one();
        }
    }

    fn verify_requests(&self, count: usize) -> Vec<Event> {
        let requests = self.requests.lock().unwrap();
        assert_eq!(requests.len(), count);
        let mut events = Vec::new();
        for request in requests.iter() {
            let encoded = request.headers["authorization"]
                .to_str()
                .unwrap()
                .strip_prefix("Nostr ")
                .unwrap();
            let auth = Event::from_json(
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .unwrap(),
            )
            .unwrap();
            auth.verify().unwrap();
            assert_eq!(auth.kind.as_u16(), 27235);
            assert_eq!(auth.pubkey, self.controlled.keys.public_key());
            for tag in [
                vec!["u".to_string(), format!("{}{}", self.base, request.path)],
                vec!["method".into(), "POST".into()],
                vec!["payload".into(), hex::encode(Sha256::digest(&request.body))],
            ] {
                assert!(auth.tags.iter().any(|t| t.as_slice() == tag));
            }
            if request.path == "/events" {
                let event = Event::from_json(&request.body).unwrap();
                event.verify().unwrap();
                assert_eq!(event.pubkey, auth.pubkey);
                events.push(event);
            }
        }
        events
    }
}

#[tokio::test]
async fn live_message_command_pins_thread_query_event_auth_and_cursor() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    for kind in [9, 45003] {
        let root = "a".repeat(64);
        let parent = EventBuilder::new(Kind::Custom(9), "parent")
            .tag(Tag::parse(["e", &root, "", "root"]).unwrap())
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let parent_id = parent.id.to_hex();
        let f = Fixture::new(false, vec![parent]).await;
        let app = f.app.handle().clone();
        let base = f.base.clone();
        let owner = f.controlled.keys.public_key().to_hex();
        let channel = uuid::Uuid::new_v4().to_string();
        let send_channel = channel.clone();
        let task = tokio::spawn(async move {
            send_channel_message(
                send_channel,
                " hello ".into(),
                Some(parent_id),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(kind),
                Some(base),
                Some(owner),
                None,
                app.state(),
            )
            .await
        });
        f.controlled.wait_entered().await;
        assert!(f.requests.lock().unwrap().is_empty());
        f.switch_identity_and_relay();
        f.controlled.release.notify_one(); // thread-query AUTH
        f.release(2).await; // message and event AUTH
        let response = task.await.unwrap().unwrap();
        let events = f.verify_requests(2);
        let event = &events[0];
        assert_eq!(event.kind.as_u16() as u32, kind);
        assert_eq!(event.content, "hello");
        assert!(event
            .tags
            .iter()
            .any(|t| t.as_slice() == ["h", channel.as_str()]));
        assert!(event
            .tags
            .iter()
            .any(|t| t.as_slice() == ["e", root.as_str(), "", "root"]));
        assert_eq!(response.created_at, event.created_at.as_secs() as i64);
        assert_eq!(response.event_id, event.id.to_hex());
        assert_eq!(response.root_event_id.as_deref(), Some(root.as_str()));
    }
}

#[tokio::test]
async fn live_message_failure_and_scope_validation_never_egress() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let f = Fixture::new(true, vec![]).await;
    let app = f.app.handle().clone();
    let task = tokio::spawn(async move {
        send_channel_message(
            uuid::Uuid::new_v4().to_string(),
            "hello".into(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            app.state(),
        )
        .await
    });
    f.release(1).await;
    assert!(task
        .await
        .unwrap()
        .err()
        .unwrap()
        .contains("deliberate signer failure"));
    assert!(f.requests.lock().unwrap().is_empty());
    let error = open_dm_with_scope(
        vec!["b".repeat(64)],
        Some("http://other.example"),
        None,
        &f.app.state(),
    )
    .await
    .err()
    .unwrap();
    assert!(error.contains("changed"));
    assert!(f.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn live_dm_and_channel_create_keep_metadata_and_owner_overlay_in_scope() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    for dm in [false, true] {
        let f = Fixture::new(false, vec![]).await;
        let app = f.app.handle().clone();
        let task = tokio::spawn(async move {
            if dm {
                open_dm_with_scope(
                    vec![Keys::generate().public_key().to_hex()],
                    None,
                    None,
                    &app.state(),
                )
                .await
            } else {
                create_channel(
                    "Test".into(),
                    "forum".into(),
                    "private".into(),
                    Some("about".into()),
                    Some(60),
                    app.state(),
                )
                .await
            }
        });
        f.controlled.wait_entered().await;
        f.switch_identity_and_relay();
        f.controlled.release.notify_one();
        f.release(2).await;
        let channel = task.await.unwrap().unwrap();
        let events = f.verify_requests(2);
        let event = &events[0];
        assert_eq!(event.kind.as_u16(), if dm { 41010 } else { 9007 });
        if !dm {
            assert!(event
                .tags
                .iter()
                .any(|t| t.as_slice() == ["channel_type", "forum"]));
            assert!(event
                .tags
                .iter()
                .any(|t| t.as_slice() == ["visibility", "private"]));
            assert!(event.tags.iter().any(|t| t.as_slice() == ["ttl", "60"]));
            assert!(f
                .app
                .state::<AppState>()
                .pending_owned_channel_ids(&event.pubkey.to_hex())
                .contains(&channel.id));
            let new_owner = f
                .app
                .state::<AppState>()
                .identity_public_key()
                .unwrap()
                .to_hex();
            assert!(!f
                .app
                .state::<AppState>()
                .pending_owned_channel_ids(&new_owner)
                .contains(&channel.id));
        }
    }
}

#[tokio::test]
async fn live_member_batch_keeps_signer_and_per_member_errors() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let f = Fixture::new(false, vec![]).await;
    let app = f.app.handle().clone();
    let member = Keys::generate().public_key().to_hex();
    let sent_member = member.clone();
    let task = tokio::spawn(async move {
        add_channel_members(
            uuid::Uuid::new_v4().to_string(),
            vec!["invalid".into(), sent_member],
            Some("admin".into()),
            None,
            None,
            app.state(),
        )
        .await
    });
    f.controlled.wait_entered().await;
    f.switch_identity_and_relay();
    f.controlled.release.notify_one();
    f.release(1).await;
    let result = task.await.unwrap().unwrap();
    assert_eq!(result["added"], json!([member]));
    assert_eq!(result["errors"].as_array().unwrap().len(), 1);
    let events = f.verify_requests(1);
    assert_eq!(events[0].kind.as_u16(), 9000);
    assert!(events[0]
        .tags
        .iter()
        .any(|t| t.as_slice() == ["role", "admin"]));
    assert!(events[0]
        .tags
        .iter()
        .any(|t| t.as_slice() == ["p", member.as_str()]));
}

#[tokio::test]
async fn live_deferred_profile_keeps_monotonic_template_and_captured_scope() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let time = Timestamp::now().as_secs() + 10;
    let prior = EventBuilder::new(
        Kind::Metadata,
        json!({"name":"original", "picture":"old"}).to_string(),
    )
    .custom_created_at(Timestamp::from(time))
    .sign_with_keys(&Keys::generate())
    .unwrap();
    let f = Fixture::new(false, vec![prior]).await;
    let app = f.app.handle().clone();
    let base = f.base.clone();
    let owner = f.controlled.keys.public_key().to_hex();
    let task = tokio::spawn(async move {
        update_profile_at_relay(
            base,
            owner,
            Some("old".into()),
            "new".into(),
            None,
            app.state(),
        )
        .await
    });
    f.controlled.wait_entered().await;
    f.switch_identity_and_relay();
    f.controlled.release.notify_one();
    f.release(3).await;
    task.await.unwrap().unwrap();
    let events = f.verify_requests(3);
    assert_eq!(events[0].kind, Kind::Metadata);
    assert_eq!(events[0].created_at.as_secs(), time + 1);
    let content: Value = serde_json::from_str(&events[0].content).unwrap();
    assert_eq!(content["name"], "original");
    assert_eq!(content["picture"], "new");
}

#[tokio::test]
async fn live_starter_membership_joins_share_the_captured_signer() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let prior = ["general", "welcome-everyone"]
        .into_iter()
        .map(|name| {
            EventBuilder::new(Kind::Custom(39000), "")
                .tags([
                    Tag::parse(["d", &uuid::Uuid::new_v4().to_string()]).unwrap(),
                    Tag::parse(["name", name]).unwrap(),
                    Tag::parse(["t", "stream"]).unwrap(),
                    Tag::parse(["visibility", "open"]).unwrap(),
                ])
                .sign_with_keys(&Keys::generate())
                .unwrap()
        })
        .collect();
    let f = Fixture::new(false, prior).await;
    let app = f.app.handle().clone();
    let mut task = tokio::spawn(async move { ensure_starter_channels(app.state()).await });
    f.controlled.wait_entered().await;
    // Discovery must use the same captured signer as the joins, not bypass it
    // with local keys and then select a new scope after its network awaits.
    assert!(f.requests.lock().unwrap().is_empty());
    f.switch_identity_and_relay();
    let channels = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut releases = tokio::time::interval(std::time::Duration::from_millis(1));
        loop {
            tokio::select! {
                result = &mut task => break result.unwrap().unwrap(),
                _ = releases.tick() => f.controlled.release.notify_waiters(),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(channels.len(), 2);
    assert!(channels.iter().all(|channel| channel.is_member));
    let events = f.verify_requests(7); // five directory queries, two joins
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.kind.as_u16() == 9021));
    for (channel, event) in channels.iter().zip(events.iter()) {
        assert!(event
            .tags
            .iter()
            .any(|t| t.as_slice() == ["h", channel.id.as_str()]));
    }
}

#[tokio::test]
async fn profile_edit_retains_owner_and_relay_across_read_write_reread() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let f = Fixture::new(false, vec![]).await;
    let app = f.app.handle().clone();
    let task = tokio::spawn(async move {
        update_profile(
            Some("Captured profile".into()),
            None,
            Some("About".into()),
            None,
            None,
            app.state::<AppState>(),
        )
        .await
    });
    f.controlled.wait_entered().await;
    f.switch_identity_and_relay();
    f.controlled.release.notify_one();
    // Event signature, its HTTP auth, and canonical reread HTTP auth.
    f.release(3).await;
    let profile = task.await.unwrap().unwrap();
    let events = f.verify_requests(3);
    assert_eq!(events.len(), 1);
    let content: Value = serde_json::from_str(&events[0].content).unwrap();
    assert_eq!(content["display_name"], "Captured profile");
    assert_eq!(profile.pubkey, f.controlled.keys.public_key().to_hex());
    for request in f
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.path == "/query")
    {
        let filters: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            filters[0]["authors"][0],
            f.controlled.keys.public_key().to_hex()
        );
    }
}
