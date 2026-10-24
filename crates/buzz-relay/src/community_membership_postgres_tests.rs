//! Real-router acceptance for membership-only cross-community isolation.
//! Only object storage is faked; signed proofs, tenancy, owners, member commands,
//! Postgres, Redis replay, media processing, and serving use production paths.
use crate::state::AppState;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use buzz_auth::NipFiMode;
use nostr::{EventBuilder, Keys, Kind, Tag};
use std::sync::Arc;
use tower::ServiceExt;

async fn media_off_test_state() -> Option<Arc<AppState>> {
    let mut config = crate::config::Config::for_test();
    config.database_url = crate::test_support::database_url();
    config.redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    config.relay_url = "wss://nip-fi-media-off.local".to_string();
    config.require_auth_token = false;
    config.require_relay_membership = false;
    config.nip_fi.mode = NipFiMode::Off;

    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .ok()?;
    let db = buzz_db::Db::from_pool(pool.clone());
    let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .ok()?;
    let pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
            .await
            .ok()?,
    );
    let audit = buzz_audit::AuditService::new(pool.clone());
    let auth = buzz_auth::AuthService::new(config.auth.clone());
    let search = buzz_search::SearchService::new(pool.clone());
    let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
        db.clone(),
        buzz_workflow::WorkflowConfig::default(),
    ));
    let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;
    let (mut state, _) = crate::state::AppState::new(
        config,
        db,
        redis_pool,
        audit,
        pubsub,
        auth,
        search,
        workflow_engine,
        Keys::generate(),
        media_storage,
    );
    state.nip98_replay = Arc::new(buzz_pubsub::RedisNip98ReplayGuard::new(
        state.redis_pool.clone(),
    ));
    Some(Arc::new(state))
}

async fn media_oneshot(
    state: Arc<AppState>,
    method: &str,
    uri: &str,
    host: &str,
    headers: axum::http::HeaderMap,
    body: &[u8],
) -> (StatusCode, axum::http::HeaderMap, bytes::Bytes) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", host);
    for (name, value) in &headers {
        builder = builder.header(name, value);
    }
    let resp = crate::router::build_router(state)
        .oneshot(
            builder
                .body(Body::from(body.to_vec()))
                .expect("build request"),
        )
        .await
        .expect("router oneshot");
    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let resp_body = to_bytes(resp.into_body(), 8192).await.unwrap_or_default();
    (status, resp_headers, resp_body)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}
fn blossom_proof(keys: &Keys, host: &str, hash: &str, verb: &str) -> String {
    use base64::Engine;
    let expiration = (nostr::Timestamp::now().as_secs() + 55).to_string();
    let event = EventBuilder::new(Kind::Custom(24242), "fixture proof")
        .tags([
            Tag::parse(["t", verb]).unwrap(),
            Tag::parse(["expiration", &expiration]).unwrap(),
            Tag::parse(["server", host]).unwrap(),
            Tag::parse(["x", hash]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    format!(
        "Nostr {}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&event).unwrap())
    )
}
fn blossom_get_auth_value(keys: &Keys, host: &str, hash: &str) -> String {
    blossom_proof(keys, host, hash, "get")
}
fn blossom_upload_auth_value(keys: &Keys, host: &str, hash: &str) -> String {
    blossom_proof(keys, host, hash, "upload")
}
const PNG_BODY: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f,
    0x00, 0x05, 0xfe, 0x02, 0xfe, 0x0d, 0xef, 0x46, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

async fn isolation_signed_post(
    state: Arc<AppState>,
    host: &str,
    path: &str,
    key: &Keys,
    body: &[u8],
) -> (StatusCode, axum::http::HeaderMap, bytes::Bytes) {
    use base64::Engine;
    let proof = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", &format!("https://{host}{path}")]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &sha256_hex(body)]).unwrap(),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(key)
        .unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        "authorization",
        format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&proof).unwrap())
        )
        .parse()
        .unwrap(),
    );
    headers.insert("content-type", "application/json".parse().unwrap());
    media_oneshot(state, "POST", path, host, headers, body).await
}

async fn isolation_add_member(state: Arc<AppState>, host: &str, owner: &Keys, member: &Keys) {
    let event = EventBuilder::new(Kind::Custom(9030), "")
        .tags([Tag::public_key(member.public_key())])
        .sign_with_keys(owner)
        .unwrap();
    let response = isolation_signed_post(
        state,
        host,
        "/events",
        owner,
        &serde_json::to_vec(&event).unwrap(),
    )
    .await;
    assert_eq!(
        response.0,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&response.2)
    );
    let body: serde_json::Value = serde_json::from_slice(&response.2).unwrap();
    assert_eq!(body["accepted"], true, "{body}");
}

async fn isolation_publish_profile(
    state: Arc<AppState>,
    host: &str,
    bot: &Keys,
    event: &nostr::Event,
) {
    let response = isolation_signed_post(
        state,
        host,
        "/events",
        bot,
        &serde_json::to_vec(event).unwrap(),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK);
    let result: serde_json::Value = serde_json::from_slice(&response.2).unwrap();
    assert_eq!(result["accepted"], true, "{result}");
    assert_eq!(result["event_id"], event.id.to_hex());
}

async fn isolation_profile_readback(
    state: Arc<AppState>,
    host: &str,
    reader: &Keys,
    event: &nostr::Event,
    present: bool,
) {
    let query =
        serde_json::to_vec(&serde_json::json!([{"kinds":[0], "authors":[event.pubkey.to_hex()]}]))
            .unwrap();
    let response = isolation_signed_post(state, host, "/query", reader, &query).await;
    assert_eq!(response.0, StatusCode::OK);
    let result: Vec<nostr::Event> = serde_json::from_slice(&response.2).unwrap();
    if present {
        assert_eq!(result, vec![event.clone()]);
    } else {
        assert!(result.is_empty(), "foreign community profile leaked");
    }
}

#[tokio::test]
#[ignore = "requires an explicit disposable loopback Postgres and Redis"]
async fn membership_only_two_community_media_and_service_bot_roundtrip() {
    use axum::{
        extract::{Path, State},
        http::{HeaderMap, Method},
        response::Response,
        routing::any,
        Router,
    };
    use std::collections::HashMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    // Never fall back to the Desktop database for this acceptance fixture.
    let db_url =
        std::env::var("BUZZ_TEST_DATABASE_URL").expect("explicit disposable test DB required");
    let parsed = url::Url::parse(&db_url).unwrap();
    assert!(matches!(
        parsed.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    ));
    assert_ne!(parsed.path(), "/buzz", "do not use the Desktop database");
    let redis_url = std::env::var("REDIS_URL").expect("explicit disposable Redis required");
    assert!(matches!(
        url::Url::parse(&redis_url).unwrap().host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    ));

    #[derive(Clone, Default)]
    struct Objects {
        bytes: Arc<Mutex<HashMap<String, bytes::Bytes>>>,
        reads: Arc<AtomicUsize>,
        writes: Arc<AtomicUsize>,
    }
    async fn object_request(
        State(objects): State<Objects>,
        Path(key): Path<String>,
        method: Method,
        headers: HeaderMap,
        body: bytes::Bytes,
    ) -> Response {
        if method == Method::PUT {
            objects.writes.fetch_add(1, Ordering::SeqCst);
            objects.bytes.lock().unwrap().insert(key, body);
            return Response::builder().status(200).body(Body::empty()).unwrap();
        }
        objects.reads.fetch_add(1, Ordering::SeqCst);
        let Some(bytes) = objects.bytes.lock().unwrap().get(&key).cloned() else {
            return Response::builder().status(404).body(Body::empty()).unwrap();
        };
        let total = bytes.len();
        let mut status = 200;
        let mut selected = bytes;
        if let Some(range) = headers.get("range") {
            let (start, end) = range
                .to_str()
                .unwrap()
                .strip_prefix("bytes=")
                .unwrap()
                .split_once('-')
                .unwrap();
            let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
            selected = selected.slice(start..=end);
            status = 206;
        }
        let mut response = Response::builder()
            .status(status)
            .header("content-length", selected.len());
        if status == 206 {
            response = response.header(
                "content-range",
                format!("bytes 0-{}/{total}", selected.len() - 1),
            );
        }
        response
            .body(if method == Method::HEAD {
                Body::empty()
            } else {
                Body::from(selected)
            })
            .unwrap()
    }
    let objects = Objects::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/{*key}", any(object_request))
        .with_state(objects.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let operator = Keys::generate();
    let owner_a = Keys::generate();
    let owner_b = Keys::generate();
    let only_a = Keys::generate();
    let only_b = Keys::generate();
    let dual = Keys::generate();
    let bot = Keys::generate();
    let hosts = [
        format!("a-{}.local", uuid::Uuid::new_v4().simple()),
        format!("b-{}.local", uuid::Uuid::new_v4().simple()),
    ];
    let mut state = media_off_test_state()
        .await
        .expect("disposable dependencies reachable");
    let mutable = Arc::get_mut(&mut state).unwrap();
    let config = Arc::make_mut(&mut mutable.config);
    config.require_auth_token = true;
    config.require_relay_membership = true;
    config.pubkey_allowlist_enabled = false;
    // Prove the configured deployment owner is not a tenant membership bypass.
    config.relay_owner_pubkey = Some(bot.public_key().to_hex());
    config.relay_operator_pubkeys = vec![operator.public_key().to_hex()];
    config.relay_operator_api_origin = Some("https://operator.local".into());
    config.media.s3_endpoint = format!("http://{address}");
    config.media.s3_access_key = "synthetic".into();
    config.media.s3_secret_key = "synthetic".into();
    config.media.s3_bucket = "fixture".into();
    mutable.media_storage = Arc::new(buzz_media::MediaStorage::new(&config.media).unwrap());
    mutable.nip98_replay = Arc::new(buzz_pubsub::RedisNip98ReplayGuard::new(
        mutable.redis_pool.clone(),
    ));

    for (host, owner) in hosts.iter().zip([&owner_a, &owner_b]) {
        let request = serde_json::to_vec(&serde_json::json!({"host":host,
            "initial_owner_pubkey":owner.public_key().to_hex(), "create_only":true}))
        .unwrap();
        let response = isolation_signed_post(
            state.clone(),
            "operator.local",
            "/operator/communities",
            &operator,
            &request,
        )
        .await;
        assert_eq!(
            response.0,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&response.2)
        );
        let community = state
            .db
            .lookup_community_by_host(host)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            state
                .db
                .get_relay_member(community.id, &owner.public_key().to_hex())
                .await
                .unwrap()
                .unwrap()
                .role,
            "owner"
        );
    }
    let profile = EventBuilder::new(Kind::Metadata, r#"{"name":"fixture bot"}"#)
        .sign_with_keys(&bot)
        .unwrap();
    let profile_body = serde_json::to_vec(&profile).unwrap();
    for host in &hosts {
        assert_eq!(
            isolation_signed_post(state.clone(), host, "/events", &bot, &profile_body)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    isolation_add_member(state.clone(), &hosts[0], &owner_a, &bot).await;
    isolation_publish_profile(state.clone(), &hosts[0], &bot, &profile).await;
    isolation_profile_readback(state.clone(), &hosts[0], &bot, &profile, true).await;
    isolation_profile_readback(state.clone(), &hosts[1], &owner_b, &profile, false).await;
    assert_eq!(
        isolation_signed_post(state.clone(), &hosts[1], "/events", &bot, &profile_body)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    isolation_add_member(state.clone(), &hosts[1], &owner_b, &bot).await;
    // The exact same signed event is valid independently in two admitted communities.
    isolation_publish_profile(state.clone(), &hosts[1], &bot, &profile).await;
    for host in &hosts {
        isolation_profile_readback(state.clone(), host, &bot, &profile, true).await;
    }
    for (host, owner, exclusive) in [
        (&hosts[0], &owner_a, &only_a),
        (&hosts[1], &owner_b, &only_b),
    ] {
        isolation_add_member(state.clone(), host, owner, exclusive).await;
        isolation_add_member(state.clone(), host, owner, &dual).await;
        let community = state
            .db
            .lookup_community_by_host(host)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            state
                .db
                .get_relay_member(community.id, &bot.public_key().to_hex())
                .await
                .unwrap()
                .unwrap()
                .role,
            "member"
        );
    }

    let upload = |host: String, key: Keys| {
        let state = state.clone();
        async move {
            let mut headers = HeaderMap::new();
            headers.insert(
                "authorization",
                blossom_upload_auth_value(&key, &host, &sha256_hex(PNG_BODY))
                    .parse()
                    .unwrap(),
            );
            headers.insert("content-type", "image/png".parse().unwrap());
            headers.insert("x-sha-256", sha256_hex(PNG_BODY).parse().unwrap());
            media_oneshot(state, "PUT", "/upload", &host, headers, PNG_BODY).await
        }
    };
    let writes_before = objects.writes.load(Ordering::SeqCst);
    let denied_upload = upload(hosts[1].clone(), only_a.clone()).await;
    assert_eq!(denied_upload.0, StatusCode::FORBIDDEN);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&denied_upload.2).unwrap(),
        serde_json::json!({"error":"relay membership required"})
    );
    assert_eq!(
        objects.writes.load(Ordering::SeqCst),
        writes_before,
        "foreign upload reached storage"
    );
    let uploaded = upload(hosts[0].clone(), only_a.clone()).await;
    assert_eq!(
        uploaded.0,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&uploaded.2)
    );
    let descriptor: serde_json::Value = serde_json::from_slice(&uploaded.2).unwrap();
    assert_eq!(
        url::Url::parse(descriptor["url"].as_str().unwrap())
            .unwrap()
            .host_str(),
        Some(hosts[0].as_str())
    );
    let sha = descriptor["sha256"].as_str().unwrap();
    let path = url::Url::parse(descriptor["url"].as_str().unwrap())
        .unwrap()
        .path()
        .to_owned();
    let thumb = format!("/media/{sha}.thumb.jpg");
    let mut expected_full = bytes::Bytes::new();
    for (host, key, allowed) in [
        (&hosts[0], &only_a, true),
        (&hosts[0], &only_b, false),
        (&hosts[0], &dual, true),
        (&hosts[0], &bot, true),
        (&hosts[1], &only_a, false),
    ] {
        for (method, uri, range) in [
            ("GET", path.as_str(), false),
            ("HEAD", path.as_str(), false),
            ("GET", path.as_str(), true),
            ("GET", thumb.as_str(), false),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                "authorization",
                blossom_get_auth_value(key, host, sha).parse().unwrap(),
            );
            if range {
                headers.insert("range", "bytes=0-3".parse().unwrap());
            }
            let before = objects.reads.load(Ordering::SeqCst);
            let response = media_oneshot(state.clone(), method, uri, host, headers, b"").await;
            assert_eq!(
                response.0,
                if !allowed {
                    StatusCode::FORBIDDEN
                } else if range {
                    StatusCode::PARTIAL_CONTENT
                } else {
                    StatusCode::OK
                },
                "{method} {host} {uri}: {}",
                String::from_utf8_lossy(&response.2)
            );
            if !allowed {
                assert_eq!(
                    objects.reads.load(Ordering::SeqCst),
                    before,
                    "non-member must not reach storage"
                );
            } else if method == "GET" {
                if range {
                    assert_eq!(response.2.as_ref(), &expected_full[..4]);
                } else if uri == thumb {
                    assert!(response.2.starts_with(&[0xff, 0xd8]), "JPEG thumbnail");
                } else {
                    assert_eq!(sha256_hex(&response.2), sha, "exact uploaded content");
                    expected_full = response.2;
                }
            }
        }
    }
    // B members, including the same bot/dual identity, cannot see A-only metadata.
    for key in [&only_b, &dual, &bot] {
        for (method, uri, range) in [
            ("GET", path.as_str(), false),
            ("HEAD", path.as_str(), false),
            ("GET", path.as_str(), true),
            ("GET", thumb.as_str(), false),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                "authorization",
                blossom_get_auth_value(key, &hosts[1], sha).parse().unwrap(),
            );
            if range {
                headers.insert("range", "bytes=0-3".parse().unwrap());
            }
            assert_eq!(
                media_oneshot(state.clone(), method, uri, &hosts[1], headers, b"")
                    .await
                    .0,
                StatusCode::NOT_FOUND
            );
        }
    }
    // Independently publishing the same content in B is legitimate; it creates
    // B's sidecar without borrowing A's membership or conversation authority.
    let uploaded_b = upload(hosts[1].clone(), only_b.clone()).await;
    assert_eq!(
        uploaded_b.0,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&uploaded_b.2)
    );
    let b: serde_json::Value = serde_json::from_slice(&uploaded_b.2).unwrap();
    assert_eq!(b["sha256"], descriptor["sha256"]);
    assert_eq!(
        url::Url::parse(b["url"].as_str().unwrap())
            .unwrap()
            .host_str(),
        Some(hosts[1].as_str())
    );
    for key in [&only_b, &dual, &bot] {
        for (method, uri, range) in [
            ("GET", path.as_str(), false),
            ("HEAD", path.as_str(), false),
            ("GET", path.as_str(), true),
            ("GET", thumb.as_str(), false),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                "authorization",
                blossom_get_auth_value(key, &hosts[1], sha).parse().unwrap(),
            );
            if range {
                headers.insert("range", "bytes=0-3".parse().unwrap());
            }
            let response = media_oneshot(state.clone(), method, uri, &hosts[1], headers, b"").await;
            assert_eq!(
                response.0,
                if range {
                    StatusCode::PARTIAL_CONTENT
                } else {
                    StatusCode::OK
                }
            );
            if range {
                assert_eq!(response.2.as_ref(), &expected_full[..4]);
            } else if method == "HEAD" {
                assert!(response.2.is_empty());
                assert_eq!(
                    response.1["content-length"],
                    expected_full.len().to_string()
                );
            } else if uri == thumb {
                assert!(response.2.starts_with(&[0xff, 0xd8]));
            } else {
                assert_eq!(response.2, expected_full);
            }
        }
    }
    // A legitimate dual member still needs a fresh host-bound proof in B.
    for (signed_host, target) in [(&hosts[0], &hosts[1]), (&hosts[1], &hosts[0])] {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            blossom_get_auth_value(&dual, signed_host, sha)
                .parse()
                .unwrap(),
        );
        let before = objects.reads.load(Ordering::SeqCst);
        let response = media_oneshot(state.clone(), "GET", &path, target, headers, b"").await;
        assert!(matches!(
            response.0,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
        assert_eq!(
            objects.reads.load(Ordering::SeqCst),
            before,
            "wrong-host proof reached storage"
        );
    }
    server.abort();
}
