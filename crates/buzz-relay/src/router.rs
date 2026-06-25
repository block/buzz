//! axum routers — app (WebSocket + REST), health (K8s probes), metrics (Prometheus).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, FromRequest, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    middleware,
    response::{IntoResponse, Json},
    routing::{get, post, put},
    Router,
};
use serde_json::json;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::api;
use crate::audio;
use crate::connection::handle_connection;
use crate::metrics::track_metrics;
use crate::nip11::{nip11_facts, relay_info_handler, RelayInfo};
use crate::state::AppState;

/// Build the axum [`Router`] with all relay routes, middleware, and CORS configuration.
///
/// Pure Nostr protocol: WebSocket (NIP-01), HTTP bridge (NIP-98), media (Blossom),
/// git (smart HTTP), NIP-05, and health probes.
pub fn build_router(state: Arc<AppState>) -> Router {
    let media_body_limit = state
        .config
        .media
        .max_image_bytes
        .max(state.config.media.max_video_bytes) as usize;
    let media_router = Router::new()
        .route("/media/upload", put(api::media::upload_blob))
        .route(
            "/media/{sha256_ext}",
            get(api::media::get_blob).head(api::media::head_blob),
        )
        .layer(RequestBodyLimitLayer::new(media_body_limit))
        .with_state(state.clone());

    let git_router = api::git::git_router(state.clone());

    let git_policy_router = api::git::git_policy_router(state.clone());

    let api_router = Router::new()
        // WebSocket + NIP-11
        .route("/", get(nip11_or_ws_handler))
        .route("/info", get(relay_info_handler))
        .route("/.well-known/nostr.json", get(api::nip05::nostr_nip05))
        // Health endpoints
        .route("/health", get(health_handler))
        .route("/_liveness", get(liveness_handler))
        .route("/_readiness", get(readiness_handler))
        // Nostr HTTP bridge (NIP-98 auth)
        .route("/events", post(api::bridge::submit_event))
        .route("/query", post(api::bridge::query_events))
        .route("/count", post(api::bridge::count_events))
        // Webhook trigger (secret-authenticated, no NIP-98)
        .route("/hooks/{id}", post(api::bridge::workflow_webhook))
        // Huddle audio WebSocket route
        .route(
            "/huddle/{channel_id}/audio",
            get(audio::handler::ws_audio_handler),
        )
        // Reject request bodies larger than 1 MB to prevent resource exhaustion.
        .layer(RequestBodyLimitLayer::new(1024 * 1024))
        .with_state(state.clone());

    // Merge — each sub-router carries its own body limit.
    // Metrics → Trace → CORS applied once over the combined router.
    let mut merged = api_router
        .merge(media_router)
        .merge(git_router)
        .merge(git_policy_router);

    // When BUZZ_WEB_DIR is set, serve the SPA as a fallback for unmatched routes.
    if let Some(ref web_dir) = state.config.web_dir {
        let index_path = web_dir.join("index.html");
        let spa_fallback = ServeDir::new(web_dir).not_found_service(tower::service_fn(
            move |req: axum::extract::Request| {
                let index = index_path.clone();
                async move {
                    let path = req.uri().path();
                    // Reserved API prefixes must 404 normally, not serve index.html.
                    let reserved = path.starts_with("/api/")
                        || path.starts_with("/media/")
                        || path.starts_with("/git/")
                        || path.starts_with("/internal/")
                        || path.starts_with("/.well-known/")
                        || path.starts_with("/huddle/")
                        || path == "/health"
                        || path == "/_liveness"
                        || path == "/_readiness"
                        || path == "/_status"
                        || path == "/info";
                    // Files with extensions (e.g. /assets/missing.js) should 404.
                    let has_ext = path.rsplit('/').next().is_some_and(|seg| seg.contains('.'));
                    if reserved || has_ext {
                        Ok(StatusCode::NOT_FOUND.into_response())
                    } else {
                        // SPA client-side route → serve index.html
                        match tokio::fs::read(&index).await {
                            Ok(body) => Ok(axum::response::Html(body).into_response()),
                            Err(_) => Ok(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
                        }
                    }
                }
            },
        ));
        merged = merged.fallback_service(spa_fallback);
    }

    merged
        .layer(middleware::from_fn(track_metrics))
        .layer(TraceLayer::new_for_http())
        .layer(build_cors_layer(&state.config.cors_origins))
}

/// Build the health-only router for K8s probes (port 8080 in CAKE).
///
/// No metrics middleware, no auth, no CORS, no body limit.
pub fn build_health_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/_liveness", get(liveness_handler))
        .route("/_readiness", get(readiness_handler))
        .route("/_status", get(status_handler))
        .with_state(state)
}

/// Content-negotiated: NIP-11 JSON for plain HTTP, WebSocket upgrade otherwise.
async fn nip11_or_ws_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    req: axum::extract::Request,
) -> impl IntoResponse {
    let addr = req
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0)
        .unwrap_or_else(|| std::net::SocketAddr::from(([0, 0, 0, 0], 0)));

    let accept = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let (relay_self, advertise_nip43) = nip11_facts(&state);

    if accept.contains("application/nostr+json") {
        let info = RelayInfo::build(
            relay_self.as_deref(),
            advertise_nip43,
            state.config.max_frame_bytes,
        );
        return Json(info).into_response();
    }

    match WebSocketUpgrade::from_request(req, &state).await {
        Ok(ws) => ws
            .on_upgrade(move |socket| handle_connection(socket, state, addr))
            .into_response(),
        Err(_) => {
            // Browser requesting HTML and web UI is configured → serve SPA.
            if let Some(ref dir) = state.config.web_dir {
                if accept.contains("text/html") {
                    let index = dir.join("index.html");
                    if let Ok(body) = tokio::fs::read(&index).await {
                        return axum::response::Html(body).into_response();
                    }
                }
            }
            // Not a WS request and not asking for nostr+json — serve NIP-11 as fallback.
            let info = RelayInfo::build(
                relay_self.as_deref(),
                advertise_nip43,
                state.config.max_frame_bytes,
            );
            Json(info).into_response()
        }
    }
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn liveness_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Readiness probe — checks shutdown flag, Postgres, and Redis connectivity.
async fn readiness_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use std::time::Duration;

    if state.shutting_down.load(Ordering::Relaxed) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "shutting_down"})),
        )
            .into_response();
    }

    let check = async {
        let (pg_ok, redis_ok) = tokio::join!(state.db.ping(), async {
            state.redis_pool.get().await.is_ok()
        },);
        (pg_ok, redis_ok)
    };

    let (pg_ok, redis_ok) = tokio::time::timeout(Duration::from_secs(2), check)
        .await
        .unwrap_or((false, false));

    if pg_ok && redis_ok {
        (StatusCode::OK, Json(json!({"status": "ready"}))).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "not_ready", "postgres": pg_ok, "redis": redis_ok})),
        )
            .into_response()
    }
}

/// Status endpoint — service name, version, uptime.
async fn status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime_secs = state.started_at.elapsed().as_secs();
    Json(json!({
        "service": "buzz-relay",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime_secs,
    }))
}

/// Build a CORS layer from the configured origins list.
fn build_cors_layer(cors_origins: &[String]) -> CorsLayer {
    if cors_origins.is_empty() {
        return CorsLayer::permissive();
    }

    let origins: Vec<axum::http::HeaderValue> = cors_origins
        .iter()
        .filter_map(|o| o.parse::<axum::http::HeaderValue>().ok())
        .collect();

    if origins.is_empty() {
        tracing::error!(
            "BUZZ_CORS_ORIGINS set but no valid origins could be parsed — \
             refusing to fall back to permissive CORS. Fix the origins or unset \
             the variable for development mode."
        );
        return CorsLayer::new();
    }

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}
