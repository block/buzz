use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use base64::Engine;
use nostr::{EventBuilder, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;

fn filter(viewer: &Keys) -> Value {
    json!([{"kinds":[39008],"#p":[viewer.public_key().to_hex()],
        "#t":["dm","forum","stream","system","workflow"],"membership_snapshot":1}])
}

fn proof(key: &Keys, host: &str, body: &Value, payload: bool) -> (String, nostr::Event) {
    proof_at(key, host, "/query", body, payload)
}

fn proof_at(
    key: &Keys,
    host: &str,
    path: &str,
    body: &Value,
    payload: bool,
) -> (String, nostr::Event) {
    let mut tags = vec![
        Tag::parse(["u", &format!("https://{host}{path}")]).unwrap(),
        Tag::parse(["method", "POST"]).unwrap(),
        Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
    ];
    if payload {
        tags.push(
            Tag::parse([
                "payload",
                &hex::encode(Sha256::digest(serde_json::to_vec(body).unwrap())),
            ])
            .unwrap(),
        );
    }
    let event = EventBuilder::new(Kind::Custom(27235), "")
        .tags(tags)
        .sign_with_keys(key)
        .unwrap();
    (
        format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&event).unwrap())
        ),
        event,
    )
}

async fn post(
    state: Arc<AppState>,
    host: &str,
    body: Value,
    auth: Option<String>,
    owner_tag: Option<String>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/query")
        .header("host", host);
    if let Some(auth) = auth {
        request = request.header("authorization", auth);
    }
    if let Some(tag) = owner_tag {
        request = request.header("x-auth-tag", tag);
    }
    let response = crate::router::build_router(state)
        .oneshot(
            request
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 3 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn setup() -> (Arc<AppState>, String, buzz_core::CommunityId, Keys) {
    let fixture = super::super::postgres_tests::bridge_handler_test_state()
        .await
        .expect("isolated Postgres/Redis fixture");
    let host = format!("membership-{}.local", uuid::Uuid::new_v4().simple());
    let community = fixture
        .db
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    let mut state = (*fixture).clone();
    Arc::make_mut(&mut state.config).membership_snapshot_enabled = true;
    state.nip98_replay = Arc::new(buzz_pubsub::RedisNip98ReplayGuard::new(
        state.redis_pool.clone(),
    ));
    (Arc::new(state), host, community, Keys::generate())
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn membership_snapshot_router_signature_scope_strict_auth_replay_and_owner_substitution() {
    let (state, host, community, viewer) = setup().await;
    let channel = state
        .db
        .create_channel(
            community,
            "private",
            buzz_db::channel::ChannelType::Stream,
            buzz_db::channel::ChannelVisibility::Private,
            None,
            &viewer.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap();
    let body = filter(&viewer);
    assert_eq!(
        post(state.clone(), &host, body.clone(), None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    for (signed_host, payload, submitted) in [
        ("wrong.local", true, body.clone()),
        (host.as_str(), false, body.clone()),
        (host.as_str(), true, json!([])),
    ] {
        let (auth, _) = proof(&viewer, signed_host, &body, payload);
        assert_eq!(
            post(state.clone(), &host, submitted, Some(auth), None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (auth, event) = proof(&viewer, &host, &body, true);
    let (status, response) =
        post(state.clone(), &host, body.clone(), Some(auth.clone()), None).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["membership_snapshot"], 1);
    let signed: nostr::Event = serde_json::from_value(response["event"].clone()).unwrap();
    signed.verify().unwrap();
    assert_eq!(signed.pubkey, state.relay_keypair.public_key());
    assert_eq!(signed.kind, Kind::Custom(39008));
    assert!(signed
        .content
        .starts_with("{\"version\":1,\"community_id\":"));
    let content: Value = serde_json::from_str(&signed.content).unwrap();
    assert_eq!(content["request_id"], event.id.to_hex());
    assert!(signed
        .tags
        .iter()
        .any(|t| t.as_slice() == ["d", event.id.to_hex().as_str()]));

    assert_eq!(content["viewer"], viewer.public_key().to_hex());
    assert_eq!(content["community_id"], community.as_uuid().to_string());
    assert_eq!(content["host"], host);
    assert_eq!(content["types"], body[0]["#t"]);
    assert_eq!(content["complete"], true);
    assert_eq!(
        content["channels"],
        json!([{"id":channel.id,"role":"owner","channel_type":"stream","visibility":"private","archived":false}])
    );
    assert!(serde_json::to_vec(&response).unwrap().len() < 2000);
    assert_eq!(
        post(state.clone(), &host, body.clone(), Some(auth), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (auth, _) = proof(&viewer, &host, &body, true);
    assert_eq!(
        post(state.clone(), &host, body.clone(), Some(auth), None)
            .await
            .0,
        StatusCode::OK
    );
    // Real NIP-OA owner admission never changes the projection's viewer.
    let agent = Keys::generate();
    let mut enforced = (*state).clone();
    Arc::make_mut(&mut enforced.config).require_relay_membership = true;
    Arc::make_mut(&mut enforced.config).allow_nip_oa_auth = true;
    let _ = enforced
        .db
        .add_relay_member(
            community,
            &viewer.public_key().to_hex(),
            "owner",
            None,
            &enforced.relay_keypair,
        )
        .await
        .unwrap();
    let tag = buzz_sdk::nip_oa::compute_auth_tag(&viewer, &agent.public_key(), "").unwrap();
    let enforced = Arc::new(enforced);
    let (auth, _) = proof(&agent, &host, &body, true);
    assert_eq!(
        post(enforced.clone(), &host, body, Some(auth), Some(tag.clone()))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let own = filter(&agent);
    let (auth, _) = proof(&agent, &host, &own, true);
    let (status, own_response) = post(enforced, &host, own, Some(auth), Some(tag)).await;
    assert_eq!(status, StatusCode::OK, "{own_response}");
    let own_content: Value =
        serde_json::from_str(own_response["event"]["content"].as_str().unwrap()).unwrap();
    assert_eq!(own_content["channels"], json!([]));
    assert_eq!(own_content["viewer"], agent.public_key().to_hex());
    let other_host = format!("other-{}.local", uuid::Uuid::new_v4().simple());
    state
        .db
        .ensure_configured_community(&other_host)
        .await
        .unwrap();
    let own = filter(&viewer);
    let (auth, _) = proof(&viewer, &other_host, &own, true);
    let (status, other) = post(state.clone(), &other_host, own, Some(auth), None).await;
    assert_eq!(status, StatusCode::OK, "{other}");
    let other: Value = serde_json::from_str(other["event"]["content"].as_str().unwrap()).unwrap();
    assert_ne!(other["community_id"], content["community_id"]);
    assert_eq!(other["channels"], json!([]));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn membership_snapshot_router_disabled_malformed_and_legacy_roster_compatibility() {
    let (state, host, community, viewer) = setup().await;
    let capability = crate::nip11::nip11_document(&state, &host)
        .await
        .unwrap()
        .membership_snapshot
        .unwrap();
    assert_eq!(capability["version"], 1);
    assert_eq!(capability["community_id"], community.as_uuid().to_string());
    assert_eq!(
        capability["max_channels"],
        buzz_db::membership_snapshot::MAX_CHANNELS
    );
    assert_eq!(capability["max_response_bytes"], MAX_RESPONSE_BYTES);
    let body = filter(&viewer);
    let mut disabled = (*state).clone();
    Arc::make_mut(&mut disabled.config).membership_snapshot_enabled = false;
    assert!(crate::nip11::nip11_document(&disabled, &host)
        .await
        .unwrap()
        .membership_snapshot
        .is_none());
    let (auth, _) = proof(&viewer, &host, &body, true);
    let (status, response) = post(Arc::new(disabled), &host, body.clone(), Some(auth), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(response, json!({"error":"membership_snapshot unsupported"}));
    let mut malformed = vec![];
    for (key, value) in [
        ("membership_snapshot", json!(2)),
        ("#p", json!([Keys::generate().public_key().to_hex()])),
        ("#t", json!(["system", "stream"])),
        ("#t", json!(["unknown"])),
        ("#t", json!([])),
        ("#t", json!(["dm", "dm"])),
        ("limit", json!(1)),
        ("artifact", json!("incremental")),
        ("read_state_snapshot", json!(1)),
    ] {
        let mut changed = body.clone();
        changed[0][key] = value;
        malformed.push(changed);
    }
    malformed.push(json!([body[0], body[0]]));
    for bad in malformed {
        let (auth, _) = proof(&viewer, &host, &bad, true);
        let (status, error) = post(state.clone(), &host, bad, Some(auth), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
        assert!(error.get("event").is_none());
    }
    let channel = state
        .db
        .create_channel(
            community,
            "legacy",
            buzz_db::channel::ChannelType::Stream,
            buzz_db::channel::ChannelVisibility::Private,
            None,
            &viewer.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap();
    // Ordinary legacy reads and full signatures are unchanged on the new relay.
    let roster = EventBuilder::new(Kind::Custom(39002), "")
        .tags([
            Tag::identifier(channel.id.to_string()),
            Tag::parse(["t", "stream"]).unwrap(),
            Tag::parse(["p", &viewer.public_key().to_hex(), "", "owner"]).unwrap(),
        ])
        .sign_with_keys(&state.relay_keypair)
        .unwrap();
    state
        .db
        .replace_addressable_event(community, &roster, Some(channel.id))
        .await
        .unwrap();
    let legacy = json!([{"kinds":[39002],"#p":[viewer.public_key().to_hex()]}]);
    let (auth, _) = proof(&viewer, &host, &legacy, true);
    let (status, events) = post(state.clone(), &host, legacy, Some(auth), None).await;
    assert_eq!(status, StatusCode::OK, "{events}");
    assert_eq!(events, json!([roster]));
    // The relay-only gate is exercised through the real ingest boundary.
    for kind in [39008, 39009] {
        let forged = EventBuilder::new(Kind::Custom(kind), "{}")
            .tags([Tag::identifier(viewer.public_key().to_hex())])
            .sign_with_keys(&viewer)
            .unwrap();
        let forged = serde_json::to_value(forged).unwrap();
        let (auth, _) = proof_at(&viewer, &host, "/events", &forged, true);
        let response = crate::router::build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/events")
                    .header("host", &host)
                    .header("authorization", auth)
                    .body(Body::from(serde_json::to_vec(&forged).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(!response.status().is_success());
        // Ordinary reads never synthesize a compact snapshot (old-relay wire shape).
        let ordinary = json!([{"kinds":[kind],"#p":[viewer.public_key().to_hex()]}]);
        let (auth, _) = proof(&viewer, &host, &ordinary, true);
        let (status, events) = post(state.clone(), &host, ordinary, Some(auth), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(events, json!([]));
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn membership_snapshot_router_scale_overflow_and_backend_failure_never_complete_partial() {
    for public in [false, true] {
        let (state, host, community, viewer) = setup().await;
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let ids: Vec<uuid::Uuid> = (0..4097).map(|_| uuid::Uuid::new_v4()).collect();
        // 468 memberships first: the query and signing path must handle an ordinary
        // large account without pagination or full roster demand.
        sqlx::query(
            "INSERT INTO channels (community_id,id,name,created_by)
        SELECT $1,id,id::text,$3 FROM unnest($2::uuid[]) id",
        )
        .bind(community.as_uuid())
        .bind(&ids)
        .bind(viewer.public_key().to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channel_members (community_id,channel_id,pubkey)
        SELECT $1,id,$3 FROM unnest($2::uuid[]) id",
        )
        .bind(community.as_uuid())
        .bind(&ids[..468])
        .bind(viewer.public_key().to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let mut enabled = (*state).clone();
        Arc::make_mut(&mut enabled.config).membership_public_enabled = public;
        let state = Arc::new(enabled);
        let body = if public {
            json!([{"kinds":[39009],"#p":[viewer.public_key().to_hex()],"#t":["stream"],"membership_public":1}])
        } else {
            filter(&viewer)
        };
        let started = std::time::Instant::now();
        let (auth, _) = proof(&viewer, &host, &body, true);
        let (status, response) = post(state.clone(), &host, body.clone(), Some(auth), None).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        let signed: nostr::Event = serde_json::from_value(response["event"].clone()).unwrap();
        signed.verify().unwrap();
        let content: Value = serde_json::from_str(&signed.content).unwrap();
        assert_eq!(content["channels"].as_array().unwrap().len(), 468);
        let bytes = serde_json::to_vec(&response).unwrap().len();
        println!(
            "468-channel signed HTTP envelope bytes={bytes}, router_ms={}",
            started.elapsed().as_secs_f64() * 1000.0
        );
        assert!(bytes < 100_000);
        sqlx::query(
            "INSERT INTO channel_members (community_id,channel_id,pubkey)
        SELECT $1,id,$3 FROM unnest($2::uuid[]) id",
        )
        .bind(community.as_uuid())
        .bind(&ids[468..])
        .bind(viewer.public_key().to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let (auth, _) = proof(&viewer, &host, &body, true);
        let (status, error) = post(state.clone(), &host, body.clone(), Some(auth), None).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{error}");
        assert!(error.get("event").is_none());
        sqlx::query("ALTER TABLE channel_members RENAME TO membership_snapshot_unavailable")
            .execute(&pool)
            .await
            .unwrap();
        let (auth, _) = proof(&viewer, &host, &body, true);
        let (status, error) = post(state, &host, body, Some(auth), None).await;
        sqlx::query("ALTER TABLE membership_snapshot_unavailable RENAME TO channel_members")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{error}");
        assert!(error.get("event").is_none());
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn public_memberships_router_bindings_scope_flags_and_strict_filters() {
    let (state, host, community, requester) = setup().await;
    let subject = Keys::generate();
    let channel = state
        .db
        .create_channel(
            community,
            "public",
            buzz_db::channel::ChannelType::Stream,
            buzz_db::channel::ChannelVisibility::Open,
            None,
            &subject.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap();
    let body = json!([{"kinds":[39009],"#p":[subject.public_key().to_hex()],"#t":["forum","stream","workflow"],"membership_public":1}]);
    assert!(crate::nip11::nip11_document(&state, &host)
        .await
        .unwrap()
        .membership_public
        .is_none());
    let (auth, _) = proof(&requester, &host, &body, true);
    let (status, error) = post(state.clone(), &host, body.clone(), Some(auth), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error, json!({"error":"membership_public unsupported"}));
    let mut enabled = (*state).clone();
    Arc::make_mut(&mut enabled.config).membership_public_enabled = true;
    Arc::make_mut(&mut enabled.config).membership_snapshot_enabled = false;
    let state = Arc::new(enabled);
    let info = crate::nip11::nip11_document(&state, &host).await.unwrap();
    assert!(info.membership_snapshot.is_none());
    assert_eq!(info.membership_public.unwrap()["kind"], 39009);
    assert!(crate::nip11::nip11_document(&state, "unknown.local")
        .await
        .is_err());
    assert_eq!(
        post(state.clone(), &host, body.clone(), None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    for (signed_host, payload, submitted) in [
        ("wrong.local", true, body.clone()),
        (host.as_str(), false, body.clone()),
        (host.as_str(), true, json!([])),
    ] {
        let (auth, _) = proof(&requester, signed_host, &body, payload);
        assert_eq!(
            post(state.clone(), &host, submitted, Some(auth), None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (auth, event) = proof(&requester, &host, &body, true);
    let (status, response) =
        post(state.clone(), &host, body.clone(), Some(auth.clone()), None).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["membership_public"], 1);
    assert!(response.get("membership_snapshot").is_none());
    let signed: nostr::Event = serde_json::from_value(response["event"].clone()).unwrap();
    signed.verify().unwrap();
    assert_eq!(signed.kind, Kind::Custom(39009));
    assert_eq!(signed.pubkey, state.relay_keypair.public_key());
    assert_eq!(
        signed
            .tags
            .iter()
            .map(|t| t.as_slice().to_vec())
            .collect::<Vec<_>>(),
        vec![
            vec!["d".to_string(), event.id.to_hex()],
            vec!["p".to_string(), subject.public_key().to_hex()]
        ]
    );
    let content: Value = serde_json::from_str(&signed.content).unwrap();
    assert_eq!(content["scope"], "public");
    assert_eq!(content["requester"], requester.public_key().to_hex());
    assert_eq!(content["subject"], subject.public_key().to_hex());
    assert_eq!(content["request_id"], event.id.to_hex());
    assert_eq!(content["community_id"], community.as_uuid().to_string());
    assert_eq!(content["host"], host);
    assert_eq!(content["types"], body[0]["#t"]);
    assert_eq!(content["complete"], true);
    assert_eq!(
        content["channels"],
        json!([{"id":channel.id,"role":"owner","channel_type":"stream","visibility":"open","archived":false}])
    );
    assert_eq!(
        post(state.clone(), &host, body.clone(), Some(auth), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for (key, value) in [
        ("kinds", json!([39008])),
        ("membership_public", json!(2)),
        ("membership_snapshot", json!(1)),
        ("#t", json!(["dm", "stream"])),
        ("#t", json!(["system"])),
        ("#t", json!(["stream", "forum"])),
        ("#t", json!(["stream", "stream"])),
        ("#t", json!([])),
        (
            "#p",
            json!([
                requester.public_key().to_hex(),
                subject.public_key().to_hex()
            ]),
        ),
        ("#p", json!([subject.public_key().to_hex().to_uppercase()])),
        ("limit", json!(1)),
    ] {
        let mut bad = body.clone();
        bad[0][key] = value;
        let (auth, _) = proof(&requester, &host, &bad, true);
        assert_eq!(
            post(state.clone(), &host, bad.clone(), Some(auth), None)
                .await
                .0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    let ordinary = json!([{"kinds":[39009],"#p":[subject.public_key().to_hex()]}]);
    let (auth, _) = proof(&requester, &host, &ordinary, true);
    assert_eq!(
        post(state, &host, ordinary, Some(auth), None).await.1,
        json!([])
    );
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn public_memberships_share_requester_quota_across_subjects_and_fail_closed() {
    let (base, host, _, requester) = setup().await;
    let mut quota = (*base).clone();
    Arc::make_mut(&mut quota.config).membership_public_enabled = true;
    let mut config = quota.config.auth.clone();
    config.rate_limits.human_api_calls_per_min = 1;
    quota.auth = Arc::new(buzz_auth::AuthService::new(config));
    let state = Arc::new(quota);
    let body = |subject: &Keys| json!([{"kinds":[39009],"#p":[subject.public_key().to_hex()],"#t":["stream"],"membership_public":1}]);
    for (subject, expected) in [
        (Keys::generate(), StatusCode::OK),
        (Keys::generate(), StatusCode::TOO_MANY_REQUESTS),
    ] {
        let body = body(&subject);
        let (auth, _) = proof(&requester, &host, &body, true);
        let (status, response) = post(state.clone(), &host, body, Some(auth), None).await;
        assert_eq!(status, expected, "{response}");
        if status == StatusCode::TOO_MANY_REQUESTS {
            assert!(response["error"]
                .as_str()
                .unwrap()
                .starts_with("rate-limited: quota exceeded; retry in "));
            assert!(response.get("event").is_none());
        }
    }
    let other = Keys::generate();
    let body = body(&requester);
    let (auth, _) = proof(&other, &host, &body, true);
    assert_eq!(
        post(state.clone(), &host, body.clone(), Some(auth), None)
            .await
            .0,
        StatusCode::OK
    );
    let mut unavailable = (*state).clone();
    let dead = buzz_pubsub::redis_connection("redis://127.0.0.1:1").unwrap();
    unavailable.admission_rate_limiter =
        Arc::new(buzz_pubsub::rate_limiter::RedisRateLimiter::new(dead));
    let (auth, _) = proof(&Keys::generate(), &host, &body, true);
    let (status, error) = post(Arc::new(unavailable), &host, body, Some(auth), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(error.get("event").is_none());
}
