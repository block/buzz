//! Real media handlers and PostgreSQL, with an in-process S3 HTTP fixture.
use super::media::{get_blob, head_blob, upload_blob};
use crate::state::AppState;
use axum::{
    body::{to_bytes, Body, Bytes},
    http::{Method, Request},
    routing::any,
    Router,
};
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use base64::Engine;
use buzz_db::channel::{ChannelType, ChannelVisibility, MemberRole};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Clone, Default)]
struct Bucket {
    objects: Arc<Mutex<HashMap<String, Bytes>>>,
    reads: Arc<AtomicUsize>,
}
async fn object(
    State(bucket): State<Bucket>,
    Path(key): Path<String>,
    request: Request<Body>,
) -> Response {
    if request.method() == Method::PUT {
        let data = to_bytes(request.into_body(), 2_000_000).await.unwrap();
        bucket.objects.lock().await.insert(key, data);
        return StatusCode::OK.into_response();
    }
    bucket.reads.fetch_add(1, Ordering::SeqCst);
    let Some(mut data) = bucket.objects.lock().await.get(&key).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut status = StatusCode::OK;
    if let Some(range) = request.headers().get(header::RANGE) {
        let (start, end) = range
            .to_str()
            .unwrap()
            .strip_prefix("bytes=")
            .unwrap()
            .split_once('-')
            .unwrap();
        let start: usize = start.parse().unwrap();
        let end: usize = end.parse().unwrap_or(data.len() - 1).min(data.len() - 1);
        data = data.slice(start..=end);
        status = StatusCode::PARTIAL_CONTENT;
    }
    Response::builder()
        .status(status)
        .header(header::CONTENT_LENGTH, data.len())
        .body(if request.method() == Method::HEAD {
            Body::empty()
        } else {
            Body::from(data)
        })
        .unwrap()
}
fn proof(keys: &Keys, host: &str, hash: &str, verb: &str) -> String {
    let event = EventBuilder::new(Kind::Custom(24242), "media fixture")
        .tags([
            Tag::parse(["t", verb]).unwrap(),
            Tag::parse(["x", hash]).unwrap(),
            Tag::parse(["server", host]).unwrap(),
            Tag::parse(["expiration", &(Timestamp::now().as_secs() + 55).to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    format!(
        "Nostr {}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(event.as_json())
    )
}
fn get(
    keys: &Keys,
    host: &str,
    hash: &str,
    path: &str,
    method: &str,
    range: Option<&str>,
) -> Request<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(format!("/media/{path}"))
        .header(header::HOST, host)
        .header(header::AUTHORIZATION, proof(keys, host, hash, "get"));
    if let Some(range) = range {
        req = req.header(header::RANGE, range);
    }
    req.body(Body::empty()).unwrap()
}
async fn fixture(endpoint: &str, mode: super::media_access::MediaReadMode) -> Arc<AppState> {
    let mut config = crate::config::Config::for_test();
    config.database_url = crate::test_support::database_url();
    config.require_relay_membership = false;
    config.media_read_mode = mode;
    config.media.s3_endpoint = endpoint.to_owned();
    config.media.s3_access_key = "fixture".into();
    config.media.s3_secret_key = "fixture".into();
    config.media_uploads_per_minute = 100;
    config.redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
    let pool = sqlx::PgPool::connect(&config.database_url).await.unwrap();
    let db = buzz_db::Db::from_pool(pool.clone());
    let redis = buzz_pubsub::redis_connection(&config.redis_url).unwrap();
    let pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new(&config.redis_url, redis.clone())
            .await
            .unwrap(),
    );
    let audit = buzz_audit::AuditService::new(pool.clone());
    let auth = buzz_auth::AuthService::new(config.auth.clone());
    let search = buzz_search::SearchService::new(pool);
    let workflow = Arc::new(buzz_workflow::WorkflowEngine::new(
        db.clone(),
        buzz_workflow::WorkflowConfig::default(),
    ));
    let media = buzz_media::MediaStorage::new(&config.media).unwrap();
    let (state, _) = AppState::new(
        config,
        db,
        redis,
        audit,
        pubsub,
        auth,
        search,
        workflow,
        Keys::generate(),
        media,
    );
    Arc::new(state)
}
fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/upload", axum::routing::put(upload_blob))
        .route(
            "/media/{sha256_ext}",
            axum::routing::get(get_blob).head(head_blob),
        )
        .with_state(state)
}
async fn post(
    state: &Arc<AppState>,
    tenant: &buzz_core::TenantContext,
    keys: &Keys,
    event: nostr::Event,
) -> Result<crate::handlers::ingest::IngestResult, crate::handlers::ingest::IngestError> {
    crate::handlers::ingest::ingest_event(
        state,
        tenant,
        event,
        crate::handlers::ingest::IngestAuth::Nip42 {
            pubkey: keys.public_key(),
            scopes: buzz_auth::Scope::all_known(),
            channel_ids: None,
            conn_id: Uuid::new_v4(),
        },
    )
    .await
}

mod cli_edit_postgres_tests;
mod emoji_reference_postgres_tests;
mod reference_budget_postgres_tests;
mod reference_formats_postgres_tests;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn private_media_revocation_http_journey() {
    let bucket = Bucket::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let service = Router::new()
        .route("/{*key}", any(object))
        .with_state(bucket.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, service).await.unwrap();
    });
    let state = fixture(&endpoint, super::media_access::MediaReadMode::Enforce).await;
    let host = format!("private-media-{}.example", Uuid::new_v4());
    state.db.ensure_configured_community(&host).await.unwrap();
    let tenant = crate::tenant::bind_community(&state.db, &host)
        .await
        .unwrap();
    let owner = Keys::generate();
    let member = Keys::generate();
    let survivor = Keys::generate();
    for keys in [&owner, &member, &survivor] {
        state
            .db
            .ensure_user(tenant.community(), keys.public_key().as_bytes())
            .await
            .unwrap();
    }
    let channel = state
        .db
        .create_channel(
            tenant.community(),
            "private-media",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    for keys in [&member, &survivor] {
        state
            .db
            .add_member(
                tenant.community(),
                channel,
                keys.public_key().as_bytes(),
                MemberRole::Member,
                Some(owner.public_key().as_bytes()),
            )
            .await
            .unwrap();
    }
    let bytes = b"channel-secret attachment bytes";
    let hash = hex::encode(Sha256::digest(bytes));
    let upload = Request::builder()
        .method("PUT")
        .uri("/upload")
        .header(header::HOST, &host)
        .header(header::AUTHORIZATION, proof(&owner, &host, &hash, "upload"))
        .header("X-SHA-256", &hash)
        .body(Body::from(bytes.as_slice()))
        .unwrap();
    let response = router(state.clone()).oneshot(upload).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 100_000).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "upload: {}",
        String::from_utf8_lossy(&body)
    );
    let descriptor: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let url = descriptor["url"].as_str().unwrap();
    let ext = url.rsplit('.').next().unwrap();
    assert_eq!(
        router(state.clone())
            .oneshot(get(&owner, &host, &hash, &hash, "GET", None))
            .await
            .unwrap()
            .status(),
        StatusCode::OK,
        "uploader preview"
    );
    assert_eq!(
        router(state.clone())
            .oneshot(get(&member, &host, &hash, &hash, "GET", None))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND,
        "unposted is uploader-only"
    );
    let message = EventBuilder::new(Kind::Custom(9), "private file")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse([
                "imeta",
                &format!("url {url}"),
                &format!("x {hash}"),
                "m application/octet-stream",
                &format!("size {}", bytes.len()),
            ])
            .unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let result = post(&state, &tenant, &owner, message.clone()).await;
    assert!(result.is_ok(), "post failed: {:?}", result.err());
    let response = router(state.clone())
        .oneshot(get(&member, &host, &hash, &hash, "GET", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    for range in ["bytes=99999999-", "bytes=invalid"] {
        let response = router(state.clone())
            .oneshot(get(&survivor, &host, &hash, &hash, "GET", Some(range)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
    }
    // Model an attachment posted before recording existed. A real reupload
    // must retain its legacy publication boundary even after source purging.
    sqlx::query("DELETE FROM media_publications WHERE community_id=$1 AND sha256=$2")
        .bind(tenant.community().as_uuid())
        .bind(&hash)
        .execute(state.db.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM media_uploaders WHERE community_id=$1 AND sha256=$2")
        .bind(tenant.community().as_uuid())
        .bind(&hash)
        .execute(state.db.pool())
        .await
        .unwrap();
    let reupload = Request::builder()
        .method("PUT")
        .uri("/upload")
        .header(header::HOST, &host)
        .header(header::AUTHORIZATION, proof(&owner, &host, &hash, "upload"))
        .header("X-SHA-256", &hash)
        .body(Body::from(bytes.as_slice()))
        .unwrap();
    assert_eq!(
        router(state.clone())
            .oneshot(reupload)
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    sqlx::query("DELETE FROM events WHERE community_id=$1 AND id=$2")
        .bind(tenant.community().as_uuid())
        .bind(message.id.as_bytes().as_slice())
        .execute(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        router(state.clone())
            .oneshot(get(&owner, &host, &hash, &hash, "GET", None))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND,
        "legacy reupload cannot resurrect preview after purging"
    );
    state
        .db
        .insert_event(tenant.community(), &message, Some(channel))
        .await
        .unwrap();
    // A normal channel view can request attachments and avatars in parallel.
    let requests = (0..32)
        .map(|_| router(state.clone()).oneshot(get(&member, &host, &hash, &hash, "GET", None)));
    for response in futures_util::future::join_all(requests).await {
        assert_eq!(
            response.unwrap().status(),
            StatusCode::OK,
            "32 concurrent authorized reads"
        );
    }
    // Real thumbnail keys ensure alias positives cannot pass vacuously.
    for variant in ["jpg", "webp"] {
        bucket.objects.lock().await.insert(
            format!("{}/{hash}.thumb.{variant}", state.config.media.s3_bucket),
            Bytes::from_static(b"thumb bytes"),
        );
    }
    for path in [
        hash.clone(),
        format!("{hash}.{ext}"),
        format!("{hash}.thumb.jpg"),
        format!("{hash}.thumb.webp"),
    ] {
        for (method, range, expected) in [
            ("GET", None, StatusCode::OK),
            ("HEAD", None, StatusCode::OK),
            ("GET", Some("bytes=0-3"), StatusCode::PARTIAL_CONTENT),
        ] {
            assert_eq!(
                router(state.clone())
                    .oneshot(get(&member, &host, &hash, &path, method, range))
                    .await
                    .unwrap()
                    .status(),
                expected,
                "authorized {method} {path}"
            );
        }
    }
    state
        .db
        .remove_member(
            tenant.community(),
            channel,
            member.public_key().as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .unwrap();
    for path in [
        hash.clone(),
        format!("{hash}.{ext}"),
        format!("{hash}.thumb.jpg"),
        format!("{hash}.thumb.webp"),
    ] {
        for (method, range) in [
            ("GET", None),
            ("HEAD", None),
            ("GET", Some("bytes=0-3")),
            ("GET", Some("bytes=99999999-")),
        ] {
            let before = bucket.reads.load(Ordering::SeqCst);
            let response = router(state.clone())
                .oneshot(get(&member, &host, &hash, &path, method, range))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{method} {path} {range:?}"
            );
            assert!(!response.headers().contains_key(header::CONTENT_RANGE));
            assert_eq!(
                bucket.reads.load(Ordering::SeqCst),
                before + 1,
                "denial reads only the tenant sidecar"
            );
        }
    }
    for keys in [&owner, &survivor] {
        assert_eq!(
            router(state.clone())
                .oneshot(get(keys, &host, &hash, &hash, "GET", None))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    let wrong = Request::builder()
        .method("PUT")
        .uri("/upload")
        .header(header::HOST, &host)
        .header(
            header::AUTHORIZATION,
            proof(&member, &host, &hash, "upload"),
        )
        .header("X-SHA-256", &hash)
        .body(Body::from("wrong bytes"))
        .unwrap();
    assert_eq!(
        router(state.clone()).oneshot(wrong).await.unwrap().status(),
        StatusCode::UNAUTHORIZED,
        "deduplicated upload still verifies bytes before recording possession"
    );
    for mode in [
        super::media_access::MediaReadMode::Off,
        super::media_access::MediaReadMode::Shadow,
    ] {
        let rollout = fixture(&endpoint, mode).await;
        assert_eq!(
            router(rollout)
                .oneshot(get(&member, &host, &hash, &hash, "GET", None))
                .await
                .unwrap()
                .status(),
            StatusCode::OK,
            "rollout mode {mode:?}"
        );
    }
    let open = state
        .db
        .create_channel(
            tenant.community(),
            "open-media",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    for event in [
        EventBuilder::new(Kind::Custom(9033), "")
            .tags([Tag::parse(["icon", url]).unwrap()])
            .sign_with_keys(&member)
            .unwrap(),
        EventBuilder::new(
            Kind::Metadata,
            serde_json::json!({"picture":url}).to_string(),
        )
        .sign_with_keys(&member)
        .unwrap(),
        EventBuilder::new(Kind::Custom(9), "copy")
            .tags([
                Tag::parse(["h", &open.to_string()]).unwrap(),
                Tag::parse([
                    "imeta",
                    &format!("url {url}"),
                    &format!("x {hash}"),
                    "m application/octet-stream",
                    &format!("size {}", bytes.len()),
                ])
                .unwrap(),
            ])
            .sign_with_keys(&member)
            .unwrap(),
    ] {
        let error = post(&state, &tenant, &member, event)
            .await
            .err()
            .expect("copied URL denied");
        assert!(
            matches!(error, crate::handlers::ingest::IngestError::Rejected(ref reason) if reason == "restricted: attachment is not accessible")
        );
    }
    // First-party owners upload avatars before the registered agent signs kind:0.
    let agent = Keys::generate();
    state
        .db
        .ensure_user(tenant.community(), agent.public_key().as_bytes())
        .await
        .unwrap();
    state
        .db
        .set_agent_owner(
            tenant.community(),
            agent.public_key().as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .unwrap();
    state
        .db
        .add_member(
            tenant.community(),
            channel,
            agent.public_key().as_bytes(),
            MemberRole::Member,
            Some(owner.public_key().as_bytes()),
        )
        .await
        .unwrap();
    assert_eq!(
        router(state.clone())
            .oneshot(get(&agent, &host, &hash, &hash, "GET", None))
            .await
            .unwrap()
            .status(),
        StatusCode::OK,
        "owned agent has access before removal"
    );
    state
        .db
        .remove_member(
            tenant.community(),
            channel,
            agent.public_key().as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(
        router(state.clone())
            .oneshot(get(&agent, &host, &hash, &hash, "GET", None))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND,
        "owned agent loses access after removal"
    );
    let stolen_profile = EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({"picture":url}).to_string(),
    )
    .sign_with_keys(&agent)
    .unwrap();
    assert!(
        post(&state, &tenant, &agent, stolen_profile).await.is_err(),
        "removed owned agent cannot turn owner-uploaded private media into its avatar"
    );
    let avatar_bytes = b"new agent avatar";
    let avatar = hex::encode(Sha256::digest(avatar_bytes));
    state
        .db
        .record_media_upload(tenant.community(), &avatar, owner.public_key().as_bytes())
        .await
        .unwrap();
    let sidecar = bucket
        .objects
        .lock()
        .await
        .get(&format!(
            "{}/_meta/{}/{hash}.json",
            state.config.media.s3_bucket,
            tenant.community()
        ))
        .cloned()
        .unwrap();
    bucket.objects.lock().await.insert(
        format!(
            "{}/_meta/{}/{avatar}.json",
            state.config.media.s3_bucket,
            tenant.community()
        ),
        sidecar,
    );
    let picture = format!("http://{host}/media/{avatar}.{ext}");
    let forged = EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({"picture":picture}).to_string(),
    )
    .tags([Tag::parse([
        "imeta",
        &format!("url {url}"),
        &format!("x {hash}"),
        "m application/octet-stream",
        &format!("size {}", bytes.len()),
    ])
    .unwrap()])
    .sign_with_keys(&agent)
    .unwrap();
    assert!(
        post(&state, &tenant, &agent, forged).await.is_err(),
        "owner avatar delegation must not authorize unrelated attachments"
    );
    let profile = EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({"picture":picture}).to_string(),
    )
    .sign_with_keys(&agent)
    .unwrap();
    assert!(
        post(&state, &tenant, &agent, profile.clone())
            .await
            .is_err(),
        "agent must not borrow owner upload proof"
    );
    // The creation path transfers actual bytes with an agent-signed upload.
    let upload = Request::builder()
        .method("PUT")
        .uri("/upload")
        .header(header::HOST, &host)
        .header(
            header::AUTHORIZATION,
            proof(&agent, &host, &avatar, "upload"),
        )
        .header("X-SHA-256", &avatar)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(avatar_bytes.as_slice()))
        .unwrap();
    assert_eq!(
        router(state.clone())
            .oneshot(upload)
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let created_profile = EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({"picture":picture}).to_string(),
    )
    .sign_with_keys(&agent)
    .unwrap();
    post(&state, &tenant, &agent, created_profile)
        .await
        .expect("agent's own upload permits creation under enforce");
    let copy = EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({"picture":url}).to_string(),
    )
    .sign_with_keys(&member)
    .unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/events")
        .header(header::HOST, &host)
        .header("x-pubkey", member.public_key().to_hex())
        .body(Body::from(copy.as_json()))
        .unwrap();
    let response = crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 100_000).await.unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("attachment is not accessible"),
        "REST denial: {}",
        String::from_utf8_lossy(&body)
    );
    use buzz_workflow::action_sink::ActionSink;
    let sink = crate::workflow_sink::RelayActionSink::new(&state);
    let text = serde_json::json!({"picture":url}).to_string();
    let result = sink
        .send_message(
            tenant.community(),
            &open.to_string(),
            &text,
            &text,
            &member.public_key().to_hex(),
            None,
        )
        .await;
    assert!(
        matches!(result, Err(buzz_workflow::action_sink::ActionSinkError::InvalidInput(ref reason)) if reason.contains("attachment is not accessible")),
        "workflow copy must be denied"
    );
    // A legitimate readable second share preserves access after one removal.
    let shared = EventBuilder::new(Kind::Custom(9), "share")
        .tags([
            Tag::parse(["h", &open.to_string()]).unwrap(),
            Tag::parse(["imeta", &format!("url {url}"), "m application/octet-stream"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    state
        .db
        .insert_event(tenant.community(), &shared, Some(open))
        .await
        .unwrap();
    assert_eq!(
        router(state.clone())
            .oneshot(get(&member, &host, &hash, &hash, "GET", None))
            .await
            .unwrap()
            .status(),
        StatusCode::OK,
        "legacy URL-only share"
    );
    reference_budget_postgres_tests::run(&state, &tenant, &host, &owner, &member, channel).await;
    emoji_reference_postgres_tests::run(&state, &tenant, &host, &owner, &survivor).await;
    reference_formats_postgres_tests::run(
        &state, &tenant, &host, &owner, &member, &survivor, channel,
    )
    .await;
    // Every event kind has bounded publication work; shadow still accepts.
    let mut emoji_tags = vec![Tag::parse(["d", "large"]).unwrap()];
    for n in 1000..1513 {
        let emoji_hash = format!("{n:064x}");
        state
            .db
            .record_media_upload(
                tenant.community(),
                &emoji_hash,
                owner.public_key().as_bytes(),
            )
            .await
            .unwrap();
        let sidecar = bucket
            .objects
            .lock()
            .await
            .get(&format!(
                "{}/_meta/{}/{hash}.json",
                state.config.media.s3_bucket,
                tenant.community()
            ))
            .cloned()
            .unwrap();
        bucket.objects.lock().await.insert(
            format!(
                "{}/_meta/{}/{emoji_hash}.json",
                state.config.media.s3_bucket,
                tenant.community()
            ),
            sidecar,
        );
        emoji_tags.push(
            Tag::parse([
                "emoji",
                &format!("emoji{n}"),
                &format!("http://{host}/media/{emoji_hash}.{ext}"),
            ])
            .unwrap(),
        );
    }
    let shadow = fixture(&endpoint, super::media_access::MediaReadMode::Shadow).await;
    // Shadow counts sidecar lookup errors and lets the serving path decide
    // whether storage can supply bytes. Enforce still refuses these checks.
    let broken_hash = "8".repeat(64);
    let broken_key = format!(
        "{}/_meta/{}/{broken_hash}.json",
        state.config.media.s3_bucket,
        tenant.community()
    );
    for malformed in [false, true] {
        if malformed {
            bucket
                .objects
                .lock()
                .await
                .insert(broken_key.clone(), Bytes::from_static(b"not JSON"));
        }
        assert!(
            super::media_access::enforce_read(
                &shadow,
                &tenant,
                &broken_hash,
                owner.public_key().as_bytes(),
                buzz_media::auth::BlossomStrictness::Permissive,
            )
            .await
            .is_ok(),
            "shadow must not refuse a sidecar lookup error"
        );
        assert!(super::media_access::enforce_read(
            &state,
            &tenant,
            &broken_hash,
            owner.public_key().as_bytes(),
            buzz_media::auth::BlossomStrictness::Permissive
        )
        .await
        .is_err());
    }
    for kind in [10030, 30030] {
        let large_set = EventBuilder::new(Kind::Custom(kind), "")
            .tags(emoji_tags.clone())
            .sign_with_keys(&owner)
            .unwrap();
        let reads = bucket.reads.load(Ordering::SeqCst);
        let error =
            crate::handlers::media_access::validate_media_references(&tenant, &state, &large_set)
                .await
                .unwrap_err();
        assert!(
            matches!(error,crate::handlers::ingest::IngestError::Rejected(message) if message.contains("too many attachment"))
        );
        assert_eq!(
            bucket.reads.load(Ordering::SeqCst),
            reads,
            "over-limit emoji must stop before sidecar fan-out"
        );
        crate::handlers::media_access::validate_media_references(&tenant, &shadow, &large_set)
            .await
            .unwrap();
    }
    let bounded_set = EventBuilder::new(Kind::Custom(30030), "")
        .tags(emoji_tags.into_iter().take(513))
        .sign_with_keys(&owner)
        .unwrap();
    crate::handlers::media_access::validate_media_references(&tenant, &state, &bounded_set)
        .await
        .expect("512 emoji assets remain publishable");
    let many_tags: Vec<Tag> = (0..513)
        .map(|n| Tag::parse(["imeta", &format!("url http://{host}/media/{n:064x}.bin")]).unwrap())
        .collect();
    let oversized = EventBuilder::new(Kind::Custom(9), "")
        .tags(many_tags)
        .sign_with_keys(&owner)
        .unwrap();
    let error =
        crate::handlers::media_access::validate_media_references(&tenant, &state, &oversized)
            .await
            .unwrap_err();
    assert!(
        matches!(error, crate::handlers::ingest::IngestError::Rejected(message) if message.contains("too many attachment"))
    );
    crate::handlers::media_access::validate_media_references(&tenant, &shadow, &oversized)
        .await
        .expect("shadow counts and accepts publication cap");

    let stale_profile = EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({"picture":format!("http://{host}/media/{}.bin", "f".repeat(64))})
            .to_string(),
    )
    .sign_with_keys(&owner)
    .unwrap();
    crate::handlers::media_access::validate_media_references(&tenant, &shadow, &stale_profile)
        .await
        .expect("shadow accepts a missing sidecar");
    assert!(
        crate::handlers::media_access::validate_media_references(&tenant, &state, &stale_profile)
            .await
            .is_err(),
        "enforce rejects missing sidecars"
    );
    // Inject a lookup error after a real existing sidecar: authorization fails closed.
    sqlx::query("ALTER TABLE media_uploaders RENAME TO unavailable_media_uploaders")
        .execute(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        router(state.clone())
            .oneshot(get(&owner, &host, &hash, &hash, "GET", None))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    crate::handlers::media_access::validate_media_references(&tenant, &shadow, &bounded_set)
        .await
        .expect("shadow accepts database lookup failures");
    let upload = Request::builder()
        .method("PUT")
        .uri("/upload")
        .header(header::HOST, &host)
        .header(header::AUTHORIZATION, proof(&owner, &host, &hash, "upload"))
        .header("X-SHA-256", &hash)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes.as_slice()))
        .unwrap();
    assert_eq!(
        router(shadow.clone())
            .oneshot(upload)
            .await
            .unwrap()
            .status(),
        StatusCode::OK,
        "shadow upload accepts proof lookup failure"
    );
    sqlx::query("ALTER TABLE unavailable_media_uploaders RENAME TO media_uploaders")
        .execute(state.db.pool())
        .await
        .unwrap();
    sqlx::query("ALTER TABLE media_publications RENAME TO unavailable_media_publications")
        .execute(state.db.pool())
        .await
        .unwrap();
    crate::handlers::media_access::validate_media_references(&tenant, &shadow, &bounded_set)
        .await
        .expect("shadow accepts publication recording failure");
    assert!(
        crate::handlers::media_access::validate_media_references(&tenant, &state, &bounded_set)
            .await
            .is_err(),
        "enforce rejects recording failure"
    );
    let upload = Request::builder()
        .method("PUT")
        .uri("/upload")
        .header(header::HOST, &host)
        .header(header::AUTHORIZATION, proof(&owner, &host, &hash, "upload"))
        .header("X-SHA-256", &hash)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes.as_slice()))
        .unwrap();
    assert_eq!(
        router(shadow).oneshot(upload).await.unwrap().status(),
        StatusCode::OK,
        "shadow upload accepts marker recording failure"
    );
    let absent = "f".repeat(64);
    assert_eq!(
        router(state.clone())
            .oneshot(get(&owner, &host, &absent, &absent, "GET", None))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND,
        "missing sidecar precedes broken DB"
    );
    server.abort();
}
