use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use base64::Engine;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;

fn proof(key: &Keys, host: &str, path: &str, method: &str, body: Option<&[u8]>) -> String {
    let mut tags = vec![
        Tag::parse(["u", &format!("https://{host}{path}")]).unwrap(),
        Tag::parse(["method", method]).unwrap(),
        Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
    ];
    if let Some(body) = body {
        tags.push(Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).unwrap());
    }
    let event = EventBuilder::new(Kind::Custom(27235), "")
        .tags(tags)
        .sign_with_keys(key)
        .unwrap();
    format!(
        "Nostr {}",
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&event).unwrap())
    )
}

async fn request(
    state: Arc<crate::state::AppState>,
    host: &str,
    path: &str,
    method: &str,
    auth: Option<&str>,
    body: &[u8],
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("host", host);
    if let Some(auth) = auth {
        req = req.header("authorization", auth);
    }
    let response = crate::router::build_router(state)
        .oneshot(req.body(Body::from(body.to_vec())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(
        response.headers().get("cache-control").unwrap(),
        "private, no-store"
    );
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn accessory_router_signed_url_body_replay_and_actor_boundary() {
    let fixture = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .unwrap();
    let mut state = (*fixture).clone();
    let config = Arc::make_mut(&mut state.config);
    config.require_auth_token = true;
    config.require_relay_membership = true;
    config.buzz_v1_enabled = true;
    state.nip98_replay = Arc::new(buzz_pubsub::RedisNip98ReplayGuard::new(
        state.redis_pool.clone(),
    ));
    let state = Arc::new(state);
    let host = format!("bff-{}.local", uuid::Uuid::new_v4());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    let actor = Keys::generate();
    let other = Keys::generate();
    let path = "/buzz/v1/me/sidebar?limit=1";
    let auth = proof(&actor, &host, path, "GET", None);
    assert_eq!(
        request(state.clone(), &host, path, "GET", Some(&auth), b"")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for key in [&actor, &other] {
        state
            .db
            .add_relay_member(community, &key.public_key().to_hex(), "member", None)
            .await
            .unwrap();
    }
    assert_eq!(
        request(state.clone(), &host, path, "GET", None, b"")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let wrong_query = proof(&actor, &host, "/buzz/v1/me/sidebar?limit=2", "GET", None);
    assert_eq!(
        request(state.clone(), &host, path, "GET", Some(&wrong_query), b"")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let wrong_host = proof(&actor, "other.invalid", path, "GET", None);
    assert_eq!(
        request(state.clone(), &host, path, "GET", Some(&wrong_host), b"")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let auth = proof(&actor, &host, path, "GET", None);
    let accepted = request(state.clone(), &host, path, "GET", Some(&auth), b"").await;
    assert_eq!(accepted.0, StatusCode::OK, "{}", accepted.1);
    assert_eq!(
        request(state.clone(), &host, path, "GET", Some(&auth), b"")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let write_path = "/buzz/v1/me/read-state";
    let body = serde_json::to_vec(&json!({"intents":[{"type":"complete_import"}]})).unwrap();
    let missing_hash = proof(&actor, &host, write_path, "POST", None);
    assert_eq!(
        request(
            state.clone(),
            &host,
            write_path,
            "POST",
            Some(&missing_hash),
            &body
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let wrong_hash = proof(&actor, &host, write_path, "POST", Some(b"{}"));
    assert_eq!(
        request(
            state.clone(),
            &host,
            write_path,
            "POST",
            Some(&wrong_hash),
            &body
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let auth = proof(&actor, &host, write_path, "POST", Some(&body));
    let applied = request(state.clone(), &host, write_path, "POST", Some(&auth), &body).await;
    assert_eq!(applied.0, StatusCode::OK, "{}", applied.1);
    assert_eq!(applied.1["outcomes"][0]["status"], "applied");
    for (key, complete) in [(&actor, true), (&other, false)] {
        let auth = proof(key, &host, path, "GET", None);
        let page = request(state.clone(), &host, path, "GET", Some(&auth), b"").await;
        assert_eq!(page.0, StatusCode::OK);
        assert_eq!(!page.1["account"]["imported_at_ms"].is_null(), complete);
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn accessory_context_get_signed_query_and_independent_batch_outcomes() {
    let fixture = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .unwrap();
    let mut state = (*fixture).clone();
    Arc::make_mut(&mut state.config).buzz_v1_enabled = true;
    let state = Arc::new(state);
    let host = format!("bff-context-{}.local", uuid::Uuid::new_v4());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    let actor = Keys::generate();
    let channel = state
        .db
        .create_channel(
            community,
            "context",
            buzz_db::channel::ChannelType::Stream,
            buzz_db::channel::ChannelVisibility::Open,
            None,
            &actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let event = EventBuilder::new(Kind::Custom(9), "message selector")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    state
        .db
        .insert_event(community, &event, Some(channel))
        .await
        .unwrap();
    let targets = json!([{"target":{"channel_id":channel},"message_ids":[event.id.to_hex()]}]);
    let encode = |value: &str| {
        value
            .bytes()
            .map(|b| format!("%{b:02X}"))
            .collect::<String>()
    };
    let path = format!(
        "/buzz/v1/me/read-state?targets={}",
        encode(&targets.to_string())
    );
    let auth = proof(&actor, &host, &path, "GET", None);
    let result = request(state.clone(), &host, &path, "GET", Some(&auth), b"").await;
    assert_eq!(result.0, StatusCode::OK, "{}", result.1);
    assert_eq!(result.1["contexts"][0]["messages"][0]["status"], "unread");
    let auth = proof(&actor, &host, "/buzz/v1/me/read-state", "GET", None);
    assert_eq!(
        request(state.clone(), &host, &path, "GET", Some(&auth), b"")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let bad_path = "/buzz/v1/me/read-state?targets=invalid";
    let auth = proof(&actor, &host, bad_path, "GET", None);
    assert_eq!(
        request(state.clone(), &host, bad_path, "GET", Some(&auth), b"")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let write_path = "/buzz/v1/me/read-state";
    let body = serde_json::to_vec(&json!({"intents":[
        {"type":"unknown"},
        {"type":"mark_through","target":{"channel_id":uuid::Uuid::new_v4()},"message_id":event.id.to_hex()},
        {"type":"mark_through","target":{"channel_id":channel},"message_id":event.id.to_hex()},
        {"type":"legacy_prefix","target":{"channel_id":channel},"through_timestamp":-1}
    ]})).unwrap();
    let auth = proof(&actor, &host, write_path, "POST", Some(&body));
    let result = request(state.clone(), &host, write_path, "POST", Some(&auth), &body).await;
    assert_eq!(result.0, StatusCode::OK, "{}", result.1);
    assert_eq!(
        result.1["outcomes"],
        json!([{"status":"invalid"},{"status":"blocked"},{"status":"applied"},{"status":"invalid"}])
    );
    let auth = proof(&actor, &host, &path, "GET", None);
    let result = request(state.clone(), &host, &path, "GET", Some(&auth), b"").await;
    assert_eq!(result.1["contexts"][0]["messages"][0]["status"], "read");
}
