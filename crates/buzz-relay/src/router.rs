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
use tower::ServiceExt;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::api;
use crate::audio;
use crate::connection::handle_connection;
use crate::metrics::track_metrics;
use crate::nip11::{nip11_document, relay_info_handler};
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

    let admin_enabled = state.config.admin.is_some();
    let admin_web_dir = state
        .config
        .admin
        .as_ref()
        .and_then(|config| config.web_dir.clone());
    let admin_router = admin_enabled
        .then(|| Router::new().nest("/api/admin/v1", api::admin::router(state.clone())));

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
        .route(
            "/operator/communities",
            get(api::operator::list_owned_communities).post(api::operator::provision_community),
        )
        .route(
            "/operator/communities/archive",
            post(api::operator::archive_community),
        )
        .route(
            "/operator/communities/unarchive",
            post(api::operator::unarchive_community),
        )
        .route(
            "/operator/communities/availability",
            get(api::operator::community_availability),
        )
        .route(
            "/operator/communities/transfer",
            post(api::operator::transfer_community),
        )
        // Relay invites: mint (owner/admin) + claim (membership-gate exempt)
        .route("/api/invites", post(api::invites::mint_invite))
        .route("/api/join-policy", get(api::invites::join_policy))
        // Policy documents as standalone pages — desktop opens these in the
        // system browser instead of rendering the Markdown in-app.
        .route(
            "/api/join-policy/terms",
            get(api::invites::join_policy_terms),
        )
        .route(
            "/api/join-policy/privacy",
            get(api::invites::join_policy_privacy),
        )
        .route(
            "/api/invites/accept-policy",
            post(api::invites::accept_policy),
        )
        .route("/api/invites/claim", post(api::invites::claim_invite))
        // Moderation queue reads (NIP-98 auth + mod-authz gate, L6)
        .route("/moderation/reports", get(api::bridge::moderation_reports))
        .route("/moderation/audit", get(api::bridge::moderation_audit))
        .route(
            "/moderation/restricted",
            get(api::bridge::moderation_restricted),
        )
        // Webhook trigger (secret-authenticated, no NIP-98)
        .route("/hooks/{id}", post(api::bridge::workflow_webhook))
        // Mesh demo echo probe — testbed-only; 404 unless BUZZ_MESH=on and
        // BUZZ_MESH_DEMO_ECHO=on (see api::mesh_demo).
        .route("/_mesh/demo/echo", post(api::mesh_demo::demo_echo))
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
    if let Some(admin_router) = admin_router {
        merged = merged.merge(admin_router);
    }

    // Serve both bundles from one fallback. The admin host is checked first so
    // it can never fall through to the public web bundle.
    let web_dir = state.config.web_dir.clone();
    if admin_web_dir.is_some() || web_dir.is_some() {
        let admin_index = admin_web_dir.as_ref().map(|dir| dir.join("index.html"));
        let admin_files = admin_web_dir.map(ServeDir::new);
        let web_index = web_dir.as_ref().map(|dir| dir.join("index.html"));
        let web_files = web_dir.map(ServeDir::new);
        let serve_git_web_gui = state.config.serve_git_web_gui;
        let fallback_state = state.clone();
        let spa_fallback = tower::service_fn(move |req: axum::extract::Request| {
            let admin_index = admin_index.clone();
            let admin_files = admin_files.clone();
            let web_index = web_index.clone();
            let web_files = web_files.clone();
            let state = fallback_state.clone();
            async move {
                let path = req.uri().path();
                let admin_host = api::admin::is_admin_host(&state, req.headers());
                if admin_host {
                    if let (Some(index), Some(files)) = (admin_index, admin_files) {
                        if path.starts_with("/assets/") {
                            return files.oneshot(req).await.map(IntoResponse::into_response);
                        }
                        if is_admin_spa_path(path) {
                            return Ok(read_spa_index(&index).await);
                        }
                    }
                    return Ok(StatusCode::NOT_FOUND.into_response());
                }

                if let (Some(index), Some(files)) = (web_index, web_files) {
                    if path.starts_with("/assets/") {
                        return files.oneshot(req).await.map(IntoResponse::into_response);
                    }
                    if should_serve_spa(path, serve_git_web_gui) {
                        return Ok(read_spa_index(&index).await);
                    }
                }
                Ok(StatusCode::NOT_FOUND.into_response())
            }
        });
        merged = merged.fallback_service(spa_fallback);
    }

    merged
        .layer(middleware::from_fn(track_metrics))
        .layer(TraceLayer::new_for_http())
        .layer(build_cors_layer(&state.config.cors_origins))
}

fn is_admin_spa_path(path: &str) -> bool {
    path == "/"
        || path == "/reports"
        || path.starts_with("/reports/")
        || path == "/feedback"
        || path.starts_with("/feedback/")
}

fn is_invite_landing_path(path: &str) -> bool {
    path.strip_prefix("/invite/")
        .is_some_and(|code| !code.is_empty() && !code.contains('/'))
}

fn should_serve_spa(path: &str, serve_git_web_gui: bool) -> bool {
    is_invite_landing_path(path) || (serve_git_web_gui && is_git_web_gui_path(path))
}

fn is_git_web_gui_path(path: &str) -> bool {
    path == "/" || path == "/repos" || path.starts_with("/repos/")
}

async fn read_spa_index(index: &std::path::Path) -> axum::response::Response {
    match tokio::fs::read(index).await {
        Ok(body) => axum::response::Html(body).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Build the health-only router for K8s probes (port 8080 in CAKE).
///
/// No metrics middleware, no auth, no CORS, no body limit.
pub fn build_health_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/_liveness", get(liveness_handler))
        .route("/_readiness", get(readiness_handler))
        .route("/_status", get(status_handler))
        .route("/_mesh", get(mesh_status_handler))
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

    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // `/` is an explicit relay route, so it never reaches the SPA fallback.
    // Short-circuit the exact admin authority here and never let it serve the
    // public web bundle, NIP-11 document, or WebSocket endpoint.
    if api::admin::is_admin_host(&state, &headers) {
        if !accept.contains("text/html") {
            return StatusCode::NOT_FOUND.into_response();
        }
        let Some(index) = state
            .config
            .admin
            .as_ref()
            .and_then(|config| config.web_dir.as_ref())
            .map(|dir| dir.join("index.html"))
        else {
            return StatusCode::NOT_FOUND.into_response();
        };
        return read_spa_index(&index).await;
    }

    if accept.contains("application/nostr+json") {
        return Json(nip11_document(&state, raw_host).await).into_response();
    }

    // Row zero: bind the connection to its community from the request host
    // BEFORE the WebSocket upgrade, so no frame is ever read on an unbound
    // connection. The host is the authoritative selector; an unmapped host or a
    // lookup failure fails closed with a generic rejection — never a default
    // tenant. NIP-11 above is served before binding and stays fail-open: an
    // unmapped host still gets the document (with host-scoped fields like
    // `icon` simply absent), so the doc cannot leak which hosts are mapped.
    let tenant = match crate::tenant::bind_community(&state.db, raw_host).await {
        Ok(ctx) => ctx,
        Err(_) => {
            // Generic rejection: do not distinguish "unmapped" from "lookup
            // error", and never echo the host, so an unauthenticated caller
            // cannot probe which communities exist on this deployment.
            return (
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
                .into_response();
        }
    };

    match WebSocketUpgrade::from_request(req, &state).await {
        Ok(ws) => ws
            .on_upgrade(move |socket| handle_connection(socket, state, addr, tenant))
            .into_response(),
        Err(_) => {
            // Browser requesting HTML and Git web GUI is enabled → serve SPA.
            if state.config.serve_git_web_gui {
                if let Some(ref dir) = state.config.web_dir {
                    if accept.contains("text/html") {
                        let index = dir.join("index.html");
                        if let Ok(body) = tokio::fs::read(&index).await {
                            return axum::response::Html(body).into_response();
                        }
                    }
                }
            }
            // Not a WS request and not asking for nostr+json — serve NIP-11 as fallback.
            Json(nip11_document(&state, raw_host).await).into_response()
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

/// `/_mesh` — live mesh status: peer table, connection/phi state, per-peer
/// counters, fence-rejection totals. Mesh-off reports `{"enabled": false}` so
/// operators can distinguish "off" from "on with zero peers".
async fn mesh_status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.mesh() {
        Some(handle) => Json(serde_json::to_value(handle.status()).unwrap_or_else(
            |e| json!({"enabled": true, "error": format!("status serialize: {e}")}),
        )),
        None => Json(json!({"enabled": false})),
    }
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

#[cfg(test)]
mod tests {
    use super::{is_git_web_gui_path, is_invite_landing_path, should_serve_spa};

    #[test]
    fn invite_landing_path_requires_exactly_one_nonempty_code_segment() {
        assert!(is_invite_landing_path("/invite/payload.mac"));
        assert!(!is_invite_landing_path("/invite/"));
        assert!(!is_invite_landing_path("/invite/code/extra"));
        assert!(!is_invite_landing_path("/repos"));
        assert!(!is_invite_landing_path("/"));
    }

    #[test]
    fn git_web_gui_paths_are_explicit() {
        assert!(is_git_web_gui_path("/"));
        assert!(is_git_web_gui_path("/repos"));
        assert!(is_git_web_gui_path("/repos/example"));
        assert!(!is_git_web_gui_path("/repository"));
        assert!(!is_git_web_gui_path("/arbitrary"));
        assert!(!is_git_web_gui_path("/api/invites"));
    }

    #[test]
    fn invite_is_always_served_but_git_gui_requires_opt_in() {
        assert!(should_serve_spa("/invite/payload.mac", false));
        assert!(should_serve_spa("/invite/payload.mac", true));
        assert!(!should_serve_spa("/", false));
        assert!(!should_serve_spa("/repos/example", false));
        assert!(should_serve_spa("/", true));
        assert!(should_serve_spa("/repos/example", true));
        assert!(!should_serve_spa("/arbitrary", true));
    }
}
