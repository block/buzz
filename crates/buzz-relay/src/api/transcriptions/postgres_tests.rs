//! Full-router transcription test: NIP-98 admission, community binding,
//! request validation, the in-flight bound and the shared rate limit, against
//! a local stub upstream. Requires PostgreSQL and Redis.

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    response::Json,
    routing::post,
    Router,
};
use nostr::Keys;
use tower::ServiceExt;

use super::{tests::serve, TRANSCRIPTIONS_PATH};
use crate::{
    config::{TranscriptionConfig, TRANSCRIPTION_MAX_IN_FLIGHT},
    state::AppState,
    test_support::database_url,
};

async fn app_state(transcription: Option<TranscriptionConfig>, per_minute: u64) -> Arc<AppState> {
    let mut config = crate::config::Config::for_test();
    config.database_url = database_url();
    config.redis_url =
        std::env::var("REDIS_URL").expect("REDIS_URL must point to the isolated test Redis");
    config.relay_url = "wss://transcription-test.invalid".to_owned();
    config.require_relay_membership = false;
    config.transcription = transcription;
    config.auth.rate_limits.transcriptions_per_min = per_minute;

    let pool = sqlx::PgPool::connect(&config.database_url)
        .await
        .expect("connect PostgreSQL");
    let db = buzz_db::Db::from_pool(pool.clone());
    let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).expect("Redis pool");
    let pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
            .await
            .expect("pubsub"),
    );
    let auth = buzz_auth::AuthService::new(config.auth.clone());
    let search = buzz_search::SearchService::new(pool.clone());
    let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
        db.clone(),
        buzz_workflow::WorkflowConfig::default(),
    ));
    let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
    let (state, _audit) = AppState::new(
        config,
        db,
        redis_pool,
        None::<buzz_audit::AuditService>,
        pubsub,
        auth,
        search,
        workflow_engine,
        nostr::Keys::generate(),
        media_storage,
    );
    Arc::new(state)
}

fn nip98(keys: &Keys, url: &str, body: &[u8]) -> String {
    crate::nip98::nip98_header(keys, url, body).expect("sign NIP-98")
}

async fn post_audio(
    state: Arc<AppState>,
    host: &str,
    auth: Option<String>,
    content_type: &str,
    body: &[u8],
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::post(TRANSCRIPTIONS_PATH)
        .header(header::HOST, host)
        .header(header::CONTENT_TYPE, content_type);
    if let Some(auth) = auth {
        request = request.header(header::AUTHORIZATION, auth);
    }
    let response = crate::router::build_router(state)
        .oneshot(request.body(Body::from(body.to_vec())).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn community(state: &AppState) -> String {
    let host = format!("stt-{}.local", uuid::Uuid::new_v4().simple());
    state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community");
    host
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Redis"]
async fn transcription_route_end_to_end_through_the_router() {
    let (base, server) = serve(Router::new().route(
        "/v1/audio/transcriptions",
        post(|| async { Json(serde_json::json!({ "text": "dictated words" })) }),
    ))
    .await;
    let config = TranscriptionConfig::from_values(Some("stub-key"), Some(&base), None)
        .expect("config")
        .expect("enabled");
    let state = app_state(Some(config.clone()), 2).await;
    let host = community(&state).await;
    let url = format!("https://{host}{TRANSCRIPTIONS_PATH}");
    let keys = Keys::generate();
    let audio = b"fake m4a audio".to_vec();

    // No proof: refused before anything else runs.
    let (status, _) = post_audio(state.clone(), &host, None, "audio/mp4", &audio).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A proof over a different body does not cover this audio.
    let (status, _) = post_audio(
        state.clone(),
        &host,
        Some(nip98(&keys, &url, b"other audio")),
        "audio/mp4",
        &audio,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Authenticated but not audio.
    let (status, _) = post_audio(
        state.clone(),
        &host,
        Some(nip98(&keys, &url, &audio)),
        "application/json",
        &audio,
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // Every one of this relay's upstream slots busy (the config clone shares
    // them with the state): a generic 503 that spends no quota.
    let busy = config
        .upstream_slots()
        .acquire_many(TRANSCRIPTION_MAX_IN_FLIGHT as u32)
        .await
        .expect("hold every slot");
    let (status, body) = post_audio(
        state.clone(),
        &host,
        Some(nip98(&keys, &url, &audio)),
        "audio/mp4",
        &audio,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "transcription is busy; try again shortly");
    drop(busy);

    let (status, body) = post_audio(
        state.clone(),
        &host,
        Some(nip98(&keys, &url, &audio)),
        "audio/mp4",
        &audio,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, serde_json::json!({ "text": "dictated words" }));

    // The per-minute quota is 2: the second admitted request is the last.
    let (status, _) = post_audio(
        state.clone(),
        &host,
        Some(nip98(&keys, &url, &audio)),
        "audio/mp4",
        &audio,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post_audio(
        state,
        &host,
        Some(nip98(&keys, &url, &audio)),
        "audio/mp4",
        &audio,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    server.abort();
}
