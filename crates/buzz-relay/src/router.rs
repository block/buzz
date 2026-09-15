//! axum routers — app (WebSocket + REST), health (K8s probes), metrics (Prometheus).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{ConnectInfo, FromRequest, State, WebSocketUpgrade},
    http::{header, HeaderMap, HeaderValue, Request, StatusCode},
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
use tower_http::trace::{HttpMakeClassifier, TraceLayer};

use crate::api;
use crate::audio;
use crate::connection::handle_connection;
use crate::metrics::track_metrics;
use crate::nip11::{nip11_document, relay_info_handler};
use crate::readiness::{self, DependencyReport, ReadinessReason};
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
        .route("/upload", put(api::media::upload_blob))
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
        .route("/_readiness", get(public_readiness_handler))
        // Nostr HTTP bridge (NIP-98 auth)
        .route("/events", post(api::bridge::submit_event))
        .route("/query", post(api::bridge::query_events))
        .route("/count", post(api::bridge::count_events))
        // Relay-owned third-party GIF metadata proxy (NIP-98 auth).
        .route(api::gifs::SEARCH_PATH, post(api::gifs::search))
        .route(api::gifs::SHARE_PATH, post(api::gifs::share))
        .route(
            "/workflows/{workflow_id}/runs",
            get(api::workflows::workflow_runs),
        )
        .route(
            "/workflows/{workflow_id}/runs/{run_id}/approvals",
            get(api::workflows::run_approvals),
        )
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
                        if is_admin_static_path(path) {
                            return files
                                .oneshot(req)
                                .await
                                .map(|response| with_admin_csp(response.into_response()));
                        }
                        if is_admin_spa_path(path) {
                            return Ok(with_admin_csp(read_spa_index(&index).await));
                        }
                    }
                    return Ok(with_admin_csp(StatusCode::NOT_FOUND.into_response()));
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
        .layer(http_trace_layer())
        .layer(build_cors_layer(&state.config.cors_origins))
}

fn http_trace_layer() -> TraceLayer<HttpMakeClassifier, fn(&Request<Body>) -> tracing::Span> {
    TraceLayer::new_for_http().make_span_with(make_http_span as fn(&Request<Body>) -> tracing::Span)
}

fn make_http_span(request: &Request<Body>) -> tracing::Span {
    tracing::info_span!(
        target: "buzz_relay",
        "http.request",
        otel.kind = "server",
        http.request.method = %request.method(),
    )
}

fn is_admin_spa_path(path: &str) -> bool {
    path == "/"
        || path == "/reports"
        || path.starts_with("/reports/")
        || path == "/feedback"
        || path.starts_with("/feedback/")
}

/// Files served from the admin bundle directory verbatim. `/assets/*` is the
/// hashed Vite output; `/favicon.svg` is the one root-level file the bundle
/// emits and the document links. Everything else on the admin host is a 404 —
/// the directory is not browsable.
fn is_admin_static_path(path: &str) -> bool {
    path.starts_with("/assets/") || path == "/favicon.svg"
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

/// The admin dashboard holds the operator token in `sessionStorage`, so its
/// documents and assets are locked to same-origin code with no framing. `blob:`
/// images are required: attachments are fetched with the token and rendered
/// from object URLs. Applied only to the admin host — the public bundle keeps
/// its own headers.
#[rustfmt::skip]
const ADMIN_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' blob:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'";

fn with_admin_csp(mut response: axum::response::Response) -> axum::response::Response {
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(ADMIN_CSP),
    );
    response
}

/// Serve the admin bundle's `index.html` for a browser request to `/`. Any
/// non-HTML request to the admin authority is a 404: the relay protocol is not
/// exposed there.
async fn admin_spa_document(state: &AppState, accept: &str) -> axum::response::Response {
    let index = state
        .config
        .admin
        .as_ref()
        .and_then(|config| config.web_dir.as_ref())
        .filter(|_| accept.contains("text/html"))
        .map(|dir| dir.join("index.html"));
    match index {
        Some(index) => read_spa_index(&index).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Build the health-only router for K8s probes (port 8080 in CAKE).
///
/// No metrics middleware, no auth, no CORS, no body limit.
pub fn build_health_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/_liveness", get(liveness_handler))
        .route("/_readiness", get(kubernetes_readiness_handler))
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
        return with_admin_csp(admin_spa_document(&state, accept).await);
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

    let max_frame_bytes = state.config.max_frame_bytes;
    match WebSocketUpgrade::from_request(req, &state).await {
        Ok(ws) => {
            // Shutting down: refuse new sockets instead of accepting a
            // connection onto a dying pod. Readiness already returns 503, but
            // that only stops K8s routing — direct and in-flight upgrades
            // still reach here during the pre-drain grace window. Clients
            // treat the refusal as a normal dial failure and retry, landing
            // on a healthy pod.
            if state.shutting_down.load(Ordering::Relaxed) {
                return (StatusCode::SERVICE_UNAVAILABLE, "relay restarting").into_response();
            }
            limit_relay_websocket(ws, max_frame_bytes)
                .on_upgrade(move |socket| handle_connection(socket, state, addr, tenant))
                .into_response()
        }
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

fn limit_relay_websocket<F>(
    ws: WebSocketUpgrade<F>,
    max_frame_bytes: usize,
) -> WebSocketUpgrade<F> {
    // recv_loop keeps the application-level check as defense in depth, but
    // parser limits must be set before tungstenite assembles the message.
    ws.max_message_size(max_frame_bytes)
        .max_frame_size(max_frame_bytes)
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn liveness_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Compatibility endpoint on the public listener. Same lifecycle answer as the
/// probe, but public traffic must never move rollout telemetry.
async fn public_readiness_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    readiness_response(readiness_reason(&state))
}

/// Kubernetes health-listener endpoint — the only source of rollout readiness
/// telemetry.
async fn kubernetes_readiness_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let reason = readiness_reason(&state);
    readiness::record_readiness_probe(reason, || readiness_reason(&state).is_ready());
    readiness_response(reason)
}

/// Readiness answers for this process only.
///
/// Shared Postgres, Redis, and deletion-catalog health used to gate this
/// answer, which meant one shared outage removed every replica from the load
/// balancer simultaneously and left a reconnect burst with nowhere to land.
/// Those checks now report on `/_status`. The health listener does not bind
/// until the database, migrations, Redis, and pub/sub are up (see
/// `buzz-relay/src/main.rs`), so an answering process is a booted process and
/// needs no separate startup state.
fn readiness_reason(state: &AppState) -> ReadinessReason {
    if state.shutting_down.load(Ordering::Acquire) {
        ReadinessReason::ShuttingDown
    } else {
        ReadinessReason::Ready
    }
}

fn readiness_response(reason: ReadinessReason) -> axum::response::Response {
    match reason {
        ReadinessReason::Ready => {
            (StatusCode::OK, Json(json!({"status": "ready"}))).into_response()
        }
        ReadinessReason::ShuttingDown => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "shutting_down"})),
        )
            .into_response(),
    }
}

fn status_payload(uptime_secs: u64) -> serde_json::Value {
    json!({
        "service": "buzz-relay",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime_secs,
        "build": {
            "source_sha": crate::build_info::source_sha(),
            "id": crate::build_info::build_id(),
            "url": crate::build_info::build_url(),
        },
    })
}

/// The dependency fields the readiness body used to carry, now diagnostic only.
fn dependency_diagnostics_payload(report: &DependencyReport) -> serde_json::Value {
    json!({
        "postgres": report.postgres_ready(),
        "redis": report.redis_ready(),
        "deletion_catalog": report.deletion_catalog_ready(),
        "reason": report.reason.label(),
    })
}

/// Status endpoint — service name, version, uptime, intrinsic build identity,
/// and shared-dependency diagnostics.
///
/// Health-listener only, and never wired to a Kubernetes probe: this is where
/// an operator looks to tell "the pod is fine, Postgres is not" apart from "the
/// pod is broken". It is the only endpoint that touches the shared pools.
async fn status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let report = state
        .dependency_diagnostics
        .evaluate(&state.db, &state.redis_pool)
        .await;
    let mut payload = status_payload(state.started_at.elapsed().as_secs());
    payload["dependencies"] = dependency_diagnostics_payload(&report);
    Json(payload)
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
    use std::collections::VecDeque;
    use std::sync::{Mutex, PoisonError};
    use std::time::Duration;

    use axum::{routing::get, Router};
    use futures_util::SinkExt;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use tower::ServiceBuilder;
    use tracing::Instrument as _;
    use tracing_subscriber::prelude::*;

    use super::*;

    struct ScriptedDependencyEvaluator {
        evaluations: Mutex<VecDeque<DependencyReport>>,
    }

    impl ScriptedDependencyEvaluator {
        fn new(evaluations: impl IntoIterator<Item = DependencyReport>) -> Self {
            Self {
                evaluations: Mutex::new(evaluations.into_iter().collect()),
            }
        }

        fn push(&self, evaluation: DependencyReport) {
            self.evaluations
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back(evaluation);
        }
    }

    #[async_trait::async_trait]
    impl readiness::DependencyEvaluator for ScriptedDependencyEvaluator {
        async fn evaluate(
            &self,
            _db: &buzz_db::Db,
            _redis_pool: &deadpool_redis::Pool,
        ) -> DependencyReport {
            self.evaluations
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .expect("scripted dependency report")
        }
    }

    fn dependency_report(
        postgres: readiness::PostgresOutcome,
        redis: readiness::RedisOutcome,
        deletion_catalog: readiness::DeletionCatalogOutcome,
    ) -> DependencyReport {
        DependencyReport::from_results(
            readiness::TimedOutcome::new(postgres, Duration::from_millis(35)),
            readiness::TimedOutcome::new(redis, Duration::from_millis(20)),
            readiness::TimedOutcome::new(deletion_catalog, Duration::from_millis(15)),
            Duration::from_millis(35),
        )
    }

    fn ready_report() -> DependencyReport {
        dependency_report(
            readiness::PostgresOutcome::Success,
            readiness::RedisOutcome::Success,
            readiness::DeletionCatalogOutcome::Success,
        )
    }

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

    /// Relay state serving both bundles: the admin SPA on `admin.example` and
    /// the public SPA on any other host.
    async fn spa_state(admin_dir: &std::path::Path, web_dir: &std::path::Path) -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.web_dir = Some(web_dir.to_path_buf());
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Disabled,
            web_dir: Some(admin_dir.to_path_buf()),
        });
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    async fn readiness_state(evaluator: Arc<dyn readiness::DependencyEvaluator>) -> Arc<AppState> {
        let mut state = unreachable_dependency_state().await;
        Arc::get_mut(&mut state)
            .expect("sole reference")
            .set_dependency_evaluator(evaluator);
        state
    }

    /// A relay process whose shared Postgres and Redis are both unroutable.
    async fn unreachable_dependency_state() -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.database_url = "postgres://buzz:buzz_dev@127.0.0.1:1/buzz".to_string(); // sadscan:disable np.postgres.1 -- local test-only credentials on a closed port
        config.redis_url = "redis://127.0.0.1:1".to_string();
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    async fn readiness_request(router: Router) -> (StatusCode, serde_json::Value) {
        let response = router
            .oneshot(
                Request::get("/_readiness")
                    .body(Body::empty())
                    .expect("readiness request"),
            )
            .await
            .expect("readiness response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("readiness response body");
        let payload = serde_json::from_slice(&body).expect("readiness JSON");
        (status, payload)
    }

    async fn status_request(router: Router) -> (StatusCode, serde_json::Value) {
        let response = router
            .oneshot(
                Request::get("/_status")
                    .body(Body::empty())
                    .expect("status request"),
            )
            .await
            .expect("status response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("status response body");
        let payload = serde_json::from_slice(&body).expect("status JSON");
        (status, payload)
    }

    /// The incident regression. Shared Postgres and Redis pressure took every
    /// replica out of the load balancer at once, so a reconnect burst had
    /// nowhere to land. Readiness answers for this process only: a pod whose
    /// shared dependencies are unreachable is still a healthy pod, and only a
    /// local shutdown may withdraw it.
    #[tokio::test]
    async fn readiness_answers_from_local_lifecycle_not_shared_dependencies() {
        let state = unreachable_dependency_state().await;

        for router in [
            build_health_router(state.clone()),
            build_router(state.clone()),
        ] {
            assert_eq!(
                readiness_request(router).await,
                (StatusCode::OK, json!({"status": "ready"})),
                "unreachable shared dependencies must not deroute a healthy pod"
            );
        }

        state.begin_shutdown();

        for router in [
            build_health_router(state.clone()),
            build_router(state.clone()),
        ] {
            assert_eq!(
                readiness_request(router).await,
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"status": "shutting_down"})
                ),
                "a draining pod must still withdraw itself"
            );
        }
    }

    /// Dependency health did not disappear with the probe — it moved to the
    /// diagnostic endpoint, which is never wired to a Kubernetes probe.
    #[tokio::test]
    async fn status_retains_dependency_diagnostics_off_the_probe_path() {
        let evaluator = Arc::new(ScriptedDependencyEvaluator::new([dependency_report(
            readiness::PostgresOutcome::Success,
            readiness::RedisOutcome::PoolTimeout,
            readiness::DeletionCatalogOutcome::Success,
        )]));
        let state = readiness_state(evaluator).await;

        let (status, payload) = status_request(build_health_router(state)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["service"], "buzz-relay");
        assert_eq!(
            payload["dependencies"],
            json!({
                "postgres": true,
                "redis": false,
                "deletion_catalog": true,
                "reason": "redis_pool_timeout"
            })
        );
    }

    fn readiness_metric_lines(rendered: &str) -> Vec<&str> {
        rendered
            .lines()
            .filter(|line| line.starts_with("buzz_readiness"))
            .collect()
    }

    fn metric_value(rendered: &str, exact_prefix: &str) -> f64 {
        rendered
            .lines()
            .find_map(|line| {
                line.strip_prefix(exact_prefix)
                    .and_then(|value| value.strip_prefix(' '))
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or_else(|| panic!("missing metric line: {exact_prefix}"))
    }

    /// The frozen telemetry contract for the health listener.
    ///
    /// Readiness is lifecycle-only: its counter carries exactly two reasons and
    /// its gauge follows shutdown, never a dependency. Dependency families are
    /// still exported, but only by the diagnostic `/_status` endpoint, and
    /// public-listener traffic moves nothing.
    #[test]
    fn production_health_routes_export_the_frozen_telemetry_contract() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        // Seeded with the first `/_status` evaluation only; the coverage loop
        // below pushes the rest, one per request, so the evaluator never
        // serves a report the assertions did not choose.
        let evaluator = Arc::new(ScriptedDependencyEvaluator::new([ready_report()]));
        let (recorder, handle) = crate::metrics::readiness_test_recorder();

        metrics::with_local_recorder(&recorder, || {
            crate::metrics::describe_readiness_metrics();
            runtime.block_on(async {
                let state = readiness_state(evaluator.clone()).await;
                let public = build_router(state.clone());
                let health = build_health_router(state.clone());

                for _ in 0..3 {
                    assert_eq!(
                        readiness_request(public.clone()).await,
                        (StatusCode::OK, json!({"status": "ready"}))
                    );
                }
                assert!(
                    readiness_metric_lines(&handle.render()).is_empty(),
                    "public compatibility requests must emit no readiness series"
                );

                assert_eq!(
                    readiness_request(health.clone()).await,
                    (StatusCode::OK, json!({"status": "ready"}))
                );
                let after_probe = handle.render();

                assert!(after_probe.contains("# TYPE buzz_readiness_checks_total counter"));
                assert!(after_probe.contains("# TYPE buzz_readiness_state gauge"));
                assert_eq!(
                    metric_value(&after_probe, "buzz_readiness_checks_total{reason=\"ready\"}"),
                    1.0
                );
                assert_eq!(
                    metric_value(&after_probe, "buzz_readiness_state{check=\"overall\"}"),
                    1.0
                );
                assert!(
                    !after_probe.contains("buzz_readiness_dependency_checks_total{"),
                    "the probe must not touch a shared dependency"
                );
                assert!(
                    !after_probe.contains("buzz_readiness_check_duration_seconds_count"),
                    "the probe must not record a dependency latency sample"
                );

                // Dependency telemetry now belongs to the diagnostic endpoint.
                let (status, payload) = status_request(health.clone()).await;
                assert_eq!(status, StatusCode::OK);
                assert_eq!(payload["dependencies"]["reason"], json!("ready"));
                let after_status = handle.render();
                assert!(
                    after_status.contains("# TYPE buzz_readiness_dependency_checks_total counter")
                );
                assert!(
                    after_status.contains("# TYPE buzz_readiness_check_duration_seconds histogram")
                );
                assert_eq!(
                    metric_value(
                        &after_status,
                        "buzz_readiness_dependency_checks_total{dependency=\"postgres\",outcome=\"success\"}"
                    ),
                    1.0
                );
                for bucket in ["2", "2.5", "+Inf"] {
                    assert!(after_status.contains(&format!(
                        "buzz_readiness_check_duration_seconds_bucket{{check=\"overall\",le=\"{bucket}\"}}"
                    )));
                }
                assert!(!after_status.contains("result="));
                assert!(!after_status
                    .lines()
                    .filter(|line| line.starts_with("buzz_readiness_check_duration_seconds"))
                    .any(|line| line.contains("outcome=")));
                for dependency in ["postgres", "redis", "deletion_catalog"] {
                    assert!(
                        !after_status
                            .contains(&format!("buzz_readiness_state{{check=\"{dependency}\"}}")),
                        "dependency health has no publishable readiness gauge"
                    );
                }

                // A failing dependency is reported and changes nothing about
                // whether this pod stays in the load balancer. This set also
                // covers every valid dependency/outcome pair, so the series
                // total below is exact rather than merely bounded.
                let coverage = [
                    (
                        readiness::PostgresOutcome::PoolTimeout,
                        readiness::RedisOutcome::PoolTimeout,
                        readiness::DeletionCatalogOutcome::OperationTimeout,
                    ),
                    (
                        readiness::PostgresOutcome::PoolError,
                        readiness::RedisOutcome::PoolError,
                        readiness::DeletionCatalogOutcome::OperationError,
                    ),
                    (
                        readiness::PostgresOutcome::QueryTimeout,
                        readiness::RedisOutcome::Success,
                        readiness::DeletionCatalogOutcome::Success,
                    ),
                    (
                        readiness::PostgresOutcome::QueryError,
                        readiness::RedisOutcome::Success,
                        readiness::DeletionCatalogOutcome::Success,
                    ),
                ];
                for (index, (postgres, redis, deletion_catalog)) in
                    coverage.into_iter().enumerate()
                {
                    evaluator.push(dependency_report(postgres, redis, deletion_catalog));
                    let (status, degraded) = status_request(health.clone()).await;
                    assert_eq!(status, StatusCode::OK);
                    if index == 0 {
                        assert_eq!(
                            degraded["dependencies"],
                            json!({
                                "postgres": false,
                                "redis": false,
                                "deletion_catalog": false,
                                "reason": "overall_timeout"
                            })
                        );
                    }
                    assert_eq!(
                        readiness_request(health.clone()).await,
                        (StatusCode::OK, json!({"status": "ready"})),
                        "a failing dependency must never deroute this pod"
                    );
                }

                let histogram_count_before = metric_value(
                    &handle.render(),
                    "buzz_readiness_check_duration_seconds_count{check=\"overall\"}",
                );
                state.begin_shutdown();
                assert_eq!(
                    readiness_request(public).await,
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        json!({"status": "shutting_down"})
                    )
                );
                assert!(handle
                    .render()
                    .lines()
                    .all(|line| !line.contains("reason=\"shutting_down\"")));

                assert_eq!(
                    readiness_request(health).await,
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        json!({"status": "shutting_down"})
                    )
                );
                let final_scrape = handle.render();
                assert_eq!(
                    metric_value(
                        &final_scrape,
                        "buzz_readiness_check_duration_seconds_count{check=\"overall\"}"
                    ),
                    histogram_count_before,
                    "shutdown must not fabricate a dependency latency sample"
                );
                assert_eq!(
                    metric_value(
                        &final_scrape,
                        "buzz_readiness_checks_total{reason=\"shutting_down\"}"
                    ),
                    1.0
                );
                assert_eq!(
                    metric_value(&final_scrape, "buzz_readiness_state{check=\"overall\"}"),
                    0.0
                );
                assert!(!final_scrape.contains("sensitive-sql-or-url"));

                let exported_reasons = final_scrape
                    .lines()
                    .filter(|line| line.starts_with("buzz_readiness_checks_total{"))
                    .count();
                assert_eq!(exported_reasons, readiness::READINESS_REASON_LABELS.len());
                assert_eq!(
                    readiness_metric_lines(&final_scrape).len(),
                    readiness::READINESS_RAW_SERIES_PER_POD,
                    "readiness series contract must stay at or below its 86-series cap"
                );
            });
        });
    }

    /// A minimal built SPA: an index document, one hashed asset, and the
    /// root-level favicon Vite copies out of `public/`.
    fn write_bundle(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("assets")).expect("assets dir");
        std::fs::write(dir.join("index.html"), "<!doctype html>").expect("index.html");
        std::fs::write(dir.join("assets/app.js"), "export {};").expect("bundle asset");
        std::fs::write(dir.join("favicon.svg"), "<svg/>").expect("favicon");
    }

    async fn spa_response(
        state: Arc<AppState>,
        host: &str,
        path: &str,
    ) -> axum::response::Response {
        build_router(state)
            .oneshot(
                Request::get(path)
                    .header(axum::http::header::HOST, host)
                    .header(axum::http::header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response")
    }

    #[tokio::test]
    async fn admin_spa_documents_and_assets_carry_the_admin_csp() {
        let admin_dir = tempfile::tempdir().expect("admin bundle dir");
        let web_dir = tempfile::tempdir().expect("public bundle dir");
        write_bundle(admin_dir.path());
        write_bundle(web_dir.path());
        let state = spa_state(admin_dir.path(), web_dir.path()).await;

        for path in [
            "/",
            "/reports",
            "/feedback/abc",
            "/assets/app.js",
            "/favicon.svg",
        ] {
            let response = spa_response(state.clone(), "admin.example", path).await;
            assert_eq!(
                response
                    .headers()
                    .get(header::CONTENT_SECURITY_POLICY)
                    .and_then(|value| value.to_str().ok()),
                Some(ADMIN_CSP),
                "{path} must carry the admin CSP"
            );
        }
    }

    #[tokio::test]
    async fn the_admin_host_serves_the_favicon_the_document_links() {
        let admin_dir = tempfile::tempdir().expect("admin bundle dir");
        let web_dir = tempfile::tempdir().expect("public bundle dir");
        write_bundle(admin_dir.path());
        write_bundle(web_dir.path());
        let state = spa_state(admin_dir.path(), web_dir.path()).await;

        let response = spa_response(state.clone(), "admin.example", "/favicon.svg").await;
        assert_eq!(response.status(), StatusCode::OK);

        // The bundle directory is not browsable: only the assets Vite emits at
        // the root are reachable, never arbitrary files beside them.
        for path in ["/index.html", "/nope.svg"] {
            let response = spa_response(state.clone(), "admin.example", path).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[test]
    fn the_admin_csp_never_allows_inline_or_eval() {
        assert!(
            !ADMIN_CSP.contains("unsafe-inline") && !ADMIN_CSP.contains("unsafe-eval"),
            "the dashboard performs signed admin requests — inline script or style must stay blocked"
        );
    }

    #[tokio::test]
    async fn the_public_spa_is_untouched_by_the_admin_csp() {
        let admin_dir = tempfile::tempdir().expect("admin bundle dir");
        let web_dir = tempfile::tempdir().expect("public bundle dir");
        write_bundle(admin_dir.path());
        write_bundle(web_dir.path());
        let state = spa_state(admin_dir.path(), web_dir.path()).await;

        for path in ["/invite/payload.mac", "/assets/app.js"] {
            let response = spa_response(state.clone(), "public.example", path).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert!(
                response
                    .headers()
                    .get(header::CONTENT_SECURITY_POLICY)
                    .is_none(),
                "{path} on the public host must keep its own headers"
            );
        }
    }

    #[test]
    fn status_payload_exposes_source_and_build_identity() {
        let payload = status_payload(42);

        assert_eq!(payload["service"], "buzz-relay");
        assert_eq!(payload["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(payload["uptime_seconds"], 42);
        for field in ["source_sha", "id", "url"] {
            assert!(
                payload["build"][field]
                    .as_str()
                    .is_some_and(|value| !value.is_empty()),
                "build.{field} must be a non-empty string"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_and_datastore_spans_are_exported_in_the_same_trace() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer()
                .with_tracer(provider.tracer("test"))
                .with_filter(crate::telemetry::otel_env_filter(None)),
        );
        let _subscriber_guard = tracing::subscriber::set_default(subscriber);
        let service = ServiceBuilder::new()
            .layer(http_trace_layer())
            .service(tower::service_fn(
                |_: axum::http::Request<axum::body::Body>| async {
                    async {}
                        .instrument(tracing::info_span!(
                            target: "buzz_datastore",
                            "SELECT",
                            otel.kind = "client",
                            db.system.name = "postgresql",
                        ))
                        .await;
                    Ok::<_, std::convert::Infallible>(axum::response::Response::new(
                        axum::body::Body::empty(),
                    ))
                },
            ));

        service
            .oneshot(
                axum::http::Request::get("/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        let http = spans
            .iter()
            .find(|span| span.name == "http.request")
            .unwrap();
        let datastore = spans.iter().find(|span| span.name == "SELECT").unwrap();

        assert_eq!(
            datastore.span_context.trace_id(),
            http.span_context.trace_id()
        );
        assert_eq!(datastore.parent_span_id, http.span_context.span_id());
    }

    async fn handler_receives_message_with_limit(limit: usize, size: usize) -> bool {
        let (received_tx, mut received_rx) = mpsc::unbounded_channel();
        let app = Router::new().route(
            "/",
            get(move |ws: WebSocketUpgrade| {
                let received_tx = received_tx.clone();
                async move {
                    limit_relay_websocket(ws, limit).on_upgrade(move |mut socket| async move {
                        let _ = received_tx.send(matches!(socket.recv().await, Some(Ok(_))));
                    })
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test WebSocket listener");
        let addr = listener.local_addr().expect("test listener address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test WebSocket server");
        });

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("connect test WebSocket client");
        client
            .send(Message::Text("x".repeat(size).into()))
            .await
            .expect("send test WebSocket message");

        let received = tokio::time::timeout(std::time::Duration::from_secs(2), received_rx.recv())
            .await
            .expect("server should process the test message")
            .expect("server should report whether it received the message");

        server.abort();
        let _ = server.await;

        received
    }

    #[tokio::test]
    async fn relay_websocket_parser_rejects_oversized_messages_before_handler_reads_them() {
        let limit = 64;

        assert!(
            handler_receives_message_with_limit(limit, limit).await,
            "messages at the relay limit should still reach the handler"
        );
        assert!(
            !handler_receives_message_with_limit(limit, limit + 1).await,
            "oversized messages must be rejected by the WebSocket parser before the handler sees them"
        );
    }
}
