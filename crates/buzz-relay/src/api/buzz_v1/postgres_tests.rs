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

/// Start a reader's account before any fixture arrival. Nothing counts as
/// unread until an account starts.
async fn start_before_everything(
    state: &crate::state::AppState,
    community: buzz_core::CommunityId,
    key: &Keys,
) {
    sqlx::query(
        "INSERT INTO personal_read_accounts (community_id,actor,started_at)
         VALUES ($1,$2,'1900-01-01T00:00:00Z')",
    )
    .bind(community.as_uuid())
    .bind(key.public_key().to_bytes().as_slice())
    .execute(state.db.pool())
    .await
    .unwrap();
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
    // Targeted refresh is exclusive with paging and bounded to unique IDs.
    let id = uuid::Uuid::new_v4();
    let many = (0..21)
        .map(|_| uuid::Uuid::new_v4().to_string())
        .collect::<Vec<_>>()
        .join(",");
    for (query, status) in [
        (format!("channel_ids={id}"), StatusCode::OK),
        (format!("channel_ids={id},{id}"), StatusCode::BAD_REQUEST),
        (format!("channel_ids={id}&limit=1"), StatusCode::BAD_REQUEST),
        (
            format!("channel_ids={id}&cursor={id}"),
            StatusCode::BAD_REQUEST,
        ),
        ("channel_ids=".into(), StatusCode::BAD_REQUEST),
        ("channel_ids=not-a-uuid".into(), StatusCode::BAD_REQUEST),
        (format!("channel_ids={many}"), StatusCode::BAD_REQUEST),
    ] {
        let path = format!("/buzz/v1/me/sidebar?{query}");
        let auth = proof(&actor, &host, &path, "GET", None);
        let (got, body) = request(state.clone(), &host, &path, "GET", Some(&auth), b"").await;
        assert_eq!(got, status, "{query}: {body}");
        if status == StatusCode::OK {
            assert_eq!(body["channels"], json!([]), "unjoined ID is simply absent");
            assert!(body["next_cursor"].is_null());
        }
    }
    let channel = state
        .db
        .create_channel(
            community,
            "boundary",
            buzz_db::channel::ChannelType::Stream,
            buzz_db::channel::ChannelVisibility::Open,
            None,
            &actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let event = EventBuilder::new(Kind::Custom(9), "read by one signer only")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    state
        .db
        .insert_event(community, &event, Some(channel))
        .await
        .unwrap();
    let write_path = "/buzz/v1/me/read-state";
    let body = serde_json::to_vec(&json!({"intents":[{"type":"mark_channel_read",
        "channel_id":channel,"message_id":event.id.to_hex()}]}))
    .unwrap();
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
    // The frontier belongs to the signer alone.
    let targets = json!([{"target":{"channel_id":channel},"message_ids":[event.id.to_hex()]}]);
    let targets: String = targets
        .to_string()
        .bytes()
        .map(|b| format!("%{b:02X}"))
        .collect();
    let path = format!("/buzz/v1/me/read-state?targets={targets}");
    start_before_everything(&state, community, &other).await;
    for (key, status) in [(&actor, "read"), (&other, "unread")] {
        let auth = proof(key, &host, &path, "GET", None);
        let page = request(state.clone(), &host, &path, "GET", Some(&auth), b"").await;
        assert_eq!(page.0, StatusCode::OK, "{}", page.1);
        assert_eq!(page.1["contexts"][0]["messages"][0]["status"], status);
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
    start_before_everything(&state, community, &actor).await;
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
        {"type":"mark_through","target":{"channel_id":channel},"message_id":"not an event id"}
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

#[tokio::test]
#[ignore = "requires Postgres"]
async fn accessory_write_revocation_is_terminal_before_persistence() {
    let fixture = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .unwrap();
    let mut state = (*fixture).clone();
    let config = Arc::make_mut(&mut state.config);
    config.buzz_v1_enabled = true;
    config.require_relay_membership = true;
    let state = Arc::new(state);
    let host = format!("bff-revocation-{}.local", uuid::Uuid::new_v4());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    let actor = Keys::generate();
    state
        .db
        .add_relay_member(community, &actor.public_key().to_hex(), "member", None)
        .await
        .unwrap();
    let channel = state
        .db
        .create_channel(
            community,
            "revocation",
            buzz_db::channel::ChannelType::Stream,
            buzz_db::channel::ChannelVisibility::Open,
            None,
            &actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let event = EventBuilder::new(Kind::Custom(9), "markable before revoke")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    state
        .db
        .insert_event(community, &event, Some(channel))
        .await
        .unwrap();
    let intent = buzz_db::personal_read::ReadIntent::MarkChannelRead {
        channel_id: channel,
        message_id: event.id.to_hex(),
    };
    let path = "/buzz/v1/me/read-state";
    let body = &serde_json::to_vec(&json!({"intents":[intent]})).unwrap()[..];
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("host", host.parse().unwrap());
    headers.insert(
        "authorization",
        proof(&actor, &host, path, "POST", Some(body))
            .parse()
            .unwrap(),
    );
    let principal =
        super::auth::authorize(&state, &headers, &path.parse().unwrap(), "POST", Some(body))
            .await
            .ok()
            .expect("admitted before revoke");
    state
        .db
        .remove_relay_member(community, &actor.public_key().to_hex())
        .await
        .unwrap();
    // Exercise the same per-item function the batch handler uses after admission.
    let result = super::handlers::write_intent(&state, &headers, &principal, &intent).await;
    assert_eq!(result, json!({"status":"blocked"}));
    let persisted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM personal_read_accounts WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(persisted, 0, "denied intent must not persist");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn accessory_discovery_is_host_bound_and_opt_in() {
    let fixture = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .unwrap();
    let host = format!("bff-discovery-{}.local", uuid::Uuid::new_v4());
    fixture.db.ensure_configured_community(&host).await.unwrap();
    for enabled in [false, true] {
        let mut state = (*fixture).clone();
        let config = Arc::make_mut(&mut state.config);
        config.buzz_v1_enabled = enabled;
        config.buzz_v1_retention_seconds = 1234;
        let state = Arc::new(state);
        for known_host in [true, false] {
            let request_host = if known_host {
                host.as_str()
            } else {
                "unknown.invalid"
            };
            for path in ["/", "/info"] {
                let response = crate::router::build_router(state.clone())
                    .oneshot(
                        Request::builder()
                            .uri(path)
                            .header("host", request_host)
                            .header("accept", "application/nostr+json")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
                let doc: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(
                    doc.get("buzz_v1").is_some(),
                    enabled && known_host,
                    "enabled={enabled} known_host={known_host} path={path}"
                );
                if enabled && known_host {
                    let d = &doc["buzz_v1"];
                    assert_eq!(d["version"], 1);
                    assert_eq!(d["base_path"], "/buzz/v1");
                    assert_eq!(d["retention_seconds"], 1234);
                    assert_eq!(d["max_channels"], buzz_db::personal_read::MAX_CHANNELS);
                    assert_eq!(d["max_intents"], buzz_db::personal_read::MAX_INTENTS);
                    assert_eq!(d["max_contexts"], buzz_db::personal_read::MAX_CONTEXTS);
                    assert_eq!(
                        d["max_context_messages"],
                        buzz_db::personal_read::MAX_CONTEXT_MESSAGES
                    );
                    assert_eq!(d["max_thread_summaries"], 5);
                    assert_eq!(d["eligible_kinds"], json!([9, 40002, 45001, 45003]));
                }
            }
        }
        // Disabled means unmounted: indistinguishable from a path that never existed.
        let mut statuses = Vec::new();
        for path in ["/buzz/v1/me/sidebar", "/buzz/v1-never-existed"] {
            let request = Request::builder().uri(path).header("host", host.as_str());
            let response = crate::router::build_router(state.clone())
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            statuses.push(response.status());
        }
        assert_eq!(statuses[0] == statuses[1], !enabled, "{statuses:?}");
    }
}

// Pins Off parity under NIP-FI Shadow. These error bodies carry a per-request
// id, so they cannot ride the router's byte-for-byte Shadow rows: Shadow keeps
// Off's status and code and leaves exactly one verdict.
// Mutation: binding the tenant without `bind_tenant` leaves the unseeded row
// with no record; answering a failed proof with the NIP-FI response whenever
// the mode is not Off changes the seeded row's code.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn accessory_shadow_matches_off() {
    use crate::nip_fi_core::tests::ScriptedVerifier;
    let fixture = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .unwrap();
    for (seeded, status, code) in [
        (false, StatusCode::NOT_FOUND, "not_found"),
        (true, StatusCode::UNAUTHORIZED, "unauthorized"),
    ] {
        let host = format!("bff-shadow-{}.local", uuid::Uuid::new_v4());
        if seeded {
            fixture.db.ensure_configured_community(&host).await.unwrap();
        }
        for (mode, records) in [
            (buzz_auth::NipFiMode::Off, 0),
            (buzz_auth::NipFiMode::Shadow, 1),
        ] {
            let mut state = (*fixture).clone();
            let config = Arc::make_mut(&mut state.config);
            config.buzz_v1_enabled = true;
            config.nip_fi.mode = mode;
            config.nip_fi.communities = crate::nip_fi_config::NipFiCommunities::for_test(
                &format!("https://{host}"),
                &["https://issuer.test"],
            );
            state.nip_fi_verifier = Some(Arc::new(ScriptedVerifier::new(Ok(Some(
                Keys::generate().public_key(),
            )))));
            let recorder = metrics_util::debugging::DebuggingRecorder::new();
            let snapshotter = recorder.snapshotter();
            let _guard = metrics::set_default_local_recorder(&recorder);
            // An assertion the guard accepts, and no NIP-98 proof.
            let req = Request::get("/buzz/v1/me/sidebar")
                .header("host", &host)
                .header(buzz_auth::CLIENT_ATTACHED_HEADER, "Bearer a.b.c")
                .body(Body::empty())
                .unwrap();
            let response = crate::router::build_router(Arc::new(state))
                .oneshot(req)
                .await
                .unwrap();
            let got = response.status();
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap_or_default();
            let recorded: u64 = snapshotter
                .snapshot()
                .into_vec()
                .iter()
                .filter(|(key, ..)| key.key().name() == "buzz_nip_fi_shadow_total")
                .map(|(.., value)| match value {
                    metrics_util::debugging::DebugValue::Counter(n) => *n,
                    _ => 0,
                })
                .sum();
            assert_eq!(
                (got, &body["error"]["code"], recorded),
                (status, &json!(code), records),
                "seeded={seeded} {mode:?}"
            );
        }
    }
}

// Exercise the real signed HTTP ingest path, including deletion side effects,
// rather than directly tombstoning an event in the database.
async fn signed_sidebar_deletion(deletion_kind: u16) {
    let fixture = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .unwrap();
    let mut state = (*fixture).clone();
    let config = Arc::make_mut(&mut state.config);
    config.require_auth_token = true;
    config.require_relay_membership = true;
    config.buzz_v1_enabled = true;
    let state = Arc::new(state);
    let host = format!("bff-deletion-{}.local", uuid::Uuid::new_v4());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    let admin = Keys::generate();
    let author = Keys::generate();
    let reader = Keys::generate();
    for key in [&admin, &author, &reader] {
        state
            .db
            .add_relay_member(community, &key.public_key().to_hex(), "member", None)
            .await
            .unwrap();
    }
    let channel = state
        .db
        .create_channel(
            community,
            "deletion",
            buzz_db::channel::ChannelType::Stream,
            buzz_db::channel::ChannelVisibility::Open,
            None,
            &admin.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    for key in [&author, &reader] {
        state
            .db
            .add_member(
                community,
                channel,
                &key.public_key().to_bytes(),
                buzz_db::channel::MemberRole::Member,
                None,
            )
            .await
            .unwrap();
    }
    let message = EventBuilder::new(Kind::Custom(9), "unread message to delete")
        .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
        .sign_with_keys(&author)
        .unwrap();
    let mut tags = vec![Tag::parse(["e", &message.id.to_hex()]).unwrap()];
    let signer = if deletion_kind == 5 {
        &author
    } else {
        tags.push(Tag::parse(["h", &channel.to_string()]).unwrap());
        &admin
    };
    let deletion = EventBuilder::new(Kind::Custom(deletion_kind), "")
        .tags(tags)
        .sign_with_keys(signer)
        .unwrap();
    let path = format!("/buzz/v1/me/sidebar?channel_ids={channel}");
    start_before_everything(&state, community, &reader).await;
    for (event, key, count) in [(&message, &author, 1), (&deletion, signer, 0)] {
        let body = serde_json::to_vec(event).unwrap();
        let auth = proof(key, &host, "/events", "POST", Some(&body));
        // /events has a different response contract from the accessory helper.
        let response = crate::router::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/events")
                    .header("host", &host)
                    .header("authorization", auth)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let result: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["accepted"], true, "{result}");
        assert_eq!(result["event_id"], event.id.to_hex(), "{result}");

        // This distinct member never writes a frontier. Reads alone must not
        // erase unread state; only the accepted signed deletion changes it.
        let auth = proof(&reader, &host, &path, "GET", None);
        let (status, sidebar) = request(state.clone(), &host, &path, "GET", Some(&auth), b"").await;
        assert_eq!(status, StatusCode::OK, "{sidebar}");
        let channels = sidebar["channels"].as_array().unwrap();
        assert_eq!(channels.len(), 1, "{sidebar}");
        assert_eq!(channels[0]["channel_id"], channel.to_string());
        assert_eq!(
            channels[0]["unread"],
            json!({"status":"exact", "value":count}),
            "kind {deletion_kind}, after kind {}: {sidebar}",
            event.kind.as_u16()
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn accessory_signed_kind5_deletion_clears_another_readers_sidebar_count() {
    signed_sidebar_deletion(5).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn accessory_signed_kind9005_deletion_clears_another_readers_sidebar_count() {
    signed_sidebar_deletion(9005).await;
}
