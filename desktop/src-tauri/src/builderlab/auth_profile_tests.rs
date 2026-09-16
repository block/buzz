//! Profile commands through production remote signing and authenticated loopback HTTP.
use super::*;
use crate::{app_state::AppState, native_identity::SignerMode};
use base64::Engine;
use sha2::{Digest, Sha256};
use tauri::Manager;

struct ProfileRelay {
    base: String,
    events: Arc<Mutex<Vec<nostr::Event>>>,
    requests: Arc<Mutex<Vec<String>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for ProfileRelay {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl ProfileRelay {
    async fn new(keys: &Keys) -> Self {
        let events = Arc::new(Mutex::new(vec![EventBuilder::new(
            nostr::Kind::Metadata,
            serde_json::json!({"display_name":"Before", "name":"preserved", "picture":"old", "about":"bio"}).to_string(),
        ).sign_with_keys(keys).unwrap()]));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let expected_base = base.clone();
        let stored = events.clone();
        let recorded = requests.clone();
        let owner = keys.public_key();
        let router = Router::new().route("/{path}", post(move |Path(path): Path<String>, headers: axum::http::HeaderMap, body: axum::body::Bytes| {
            let base = expected_base.clone();
            let stored = stored.clone();
            let recorded = recorded.clone();
            async move {
                let encoded = headers["authorization"].to_str().unwrap().strip_prefix("Nostr ").unwrap();
                let auth = nostr::Event::from_json(base64::engine::general_purpose::STANDARD.decode(encoded).unwrap()).unwrap();
                auth.verify().unwrap();
                assert_eq!(auth.pubkey, owner);
                assert_eq!(auth.kind, nostr::Kind::HttpAuth);
                for (tag, value) in [("u", format!("{base}/{path}")), ("method", "POST".into()), ("payload", hex::encode(Sha256::digest(&body)))] {
                    assert_eq!(auth.tags.find(nostr::TagKind::custom(tag)).unwrap().content(), Some(value.as_str()));
                }
                recorded.lock().unwrap().push(path.clone());
                if path == "events" {
                    let event = nostr::Event::from_json(&body).unwrap();
                    event.verify().unwrap();
                    assert_eq!(event.pubkey, owner);
                    assert_eq!(event.kind, nostr::Kind::Metadata);
                    let result = serde_json::json!({"event_id":event.id, "accepted":true, "message":""});
                    *stored.lock().unwrap() = vec![event];
                    Json(result)
                } else {
                    assert_eq!(path, "query");
                    let filters: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(filters[0]["kinds"], serde_json::json!([0]));
                    assert_eq!(filters[0]["authors"], serde_json::json!([owner.to_hex()]));
                    Json(serde_json::to_value(stored.lock().unwrap().clone()).unwrap())
                }
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            base,
            events,
            requests,
            server,
        }
    }
}

fn activate(state: &AppState, relay: &str) -> u64 {
    *state.relay_url_override.lock().unwrap() = Some(relay.into());
    let signer = state.active_signer().unwrap();
    state
        .native_auth
        .activate_workspace(&signer, relay)
        .unwrap();
    signer.generation().unwrap()
}

#[tokio::test]
async fn remote_profile_signed_read_write_refetch_and_deferred_avatar() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let api = MockApi::new().await;
    let relay = ProfileRelay::new(&api.state.keys).await;
    let state = crate::app_state::build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let generation = activate(&state, &relay.base);
    assert!(state.local_identity_keys().is_err());
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let result = crate::commands::update_profile(
        Some("After".into()),
        None,
        None,
        None,
        Some(generation),
        app.state(),
    )
    .await
    .unwrap();
    assert_eq!(result.display_name.as_deref(), Some("After"));
    assert_eq!(
        *relay.requests.lock().unwrap(),
        ["query", "events", "query"]
    );
    let content: serde_json::Value =
        serde_json::from_str(&relay.events.lock().unwrap()[0].content).unwrap();
    assert_eq!(content["name"], "preserved");
    assert_eq!(content["about"], "bio");
    crate::commands::update_profile_at_relay(
        relay.base.clone(),
        api.state.keys.public_key().to_hex(),
        Some("old".into()),
        "new".into(),
        Some(generation),
        app.state(),
    )
    .await
    .unwrap();
    let content: serde_json::Value =
        serde_json::from_str(&relay.events.lock().unwrap()[0].content).unwrap();
    assert_eq!(content["picture"], "new");
    assert_eq!(content["display_name"], "After");
    assert_eq!(
        *relay.requests.lock().unwrap(),
        ["query", "events", "query", "query", "events", "query"]
    );
}

#[tokio::test]
async fn remote_profile_callbacks_require_generation_identity_and_workspace_before_io() {
    let api = MockApi::new().await;
    let state = crate::app_state::build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let generation = activate(&state, api.base.as_str());
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let state = app.state::<AppState>();
    // Construct the async command before replacement, but do not poll it yet.
    let queued = crate::commands::update_profile(
        Some("queued".into()),
        None,
        None,
        None,
        Some(generation),
        app.state(),
    );
    api.login(&state.native_auth, "B").await.unwrap();
    let current = activate(&state, api.base.as_str());
    let count = api.state.requests.lock().unwrap().len();
    assert!(queued.await.is_err());
    for expected in [None, Some(generation)] {
        assert!(crate::commands::update_profile(
            Some("stale".into()),
            None,
            None,
            None,
            expected,
            app.state()
        )
        .await
        .is_err());
        assert!(crate::commands::update_profile_at_relay(
            api.base.to_string(),
            api.state.keys.public_key().to_hex(),
            None,
            "new".into(),
            expected,
            app.state()
        )
        .await
        .is_err());
    }
    assert!(crate::commands::update_profile_at_relay(
        "http://127.0.0.1:1".into(),
        api.state.keys.public_key().to_hex(),
        None,
        "new".into(),
        Some(current),
        app.state()
    )
    .await
    .err()
    .unwrap()
    .contains("configured workspace relay"));
    assert!(crate::commands::update_profile_at_relay(
        api.base.to_string(),
        Keys::generate().public_key().to_hex(),
        None,
        "new".into(),
        Some(current),
        app.state()
    )
    .await
    .err()
    .unwrap()
    .contains("identity changed"));
    assert_eq!(api.state.requests.lock().unwrap().len(), count);
}

#[tokio::test]
async fn remote_profile_delayed_signature_is_rejected_after_same_owner_reauth() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let api = MockApi::new().await;
    let relay = ProfileRelay::new(&api.state.keys).await;
    let state = crate::app_state::build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let generation = activate(&state, &relay.base);
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    api.hold("v1/buzz/identity/sign");
    let edit = crate::commands::update_profile(
        Some("stale".into()),
        None,
        None,
        None,
        Some(generation),
        app.state(),
    );
    let replace = async {
        api.arrived().await;
        let state = app.state::<AppState>();
        api.login(&state.native_auth, "B").await.unwrap();
        activate(&state, &relay.base);
        api.release();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(edit, replace)
    })
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(relay.requests.lock().unwrap().is_empty());
}
