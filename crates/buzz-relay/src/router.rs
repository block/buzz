//! axum routers — app (WebSocket + REST), health (K8s probes), metrics (Prometheus).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{ConnectInfo, FromRequest, State, WebSocketUpgrade},
    http::{HeaderMap, Request, StatusCode},
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
use tower_http::timeout::{RequestBodyTimeoutLayer, TimeoutLayer};
use tower_http::trace::{HttpMakeClassifier, TraceLayer};

use crate::api;
use crate::audio;
use crate::connection::handle_connection;
use crate::metrics::track_metrics;
use crate::nip11::{nip11_document, relay_info_handler};
use crate::state::AppState;

/// Deadline for one API request's service future — from routing until
/// response headers are produced, including request-body collection inside
/// extractors and handlers. Streaming response bodies are *not* bounded once
/// headers are sent, and the two WebSocket routes are handshake-bounded
/// only: after the 101 upgrade the established session escapes this future.
const API_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Idle deadline for a media request body: the longest the relay waits for
/// the *next* body byte, not a bound on the whole upload. Large uploads over
/// slow links are legitimate (the video auth window is 3600s precisely so
/// they can finish), so media must NOT get a tight wall-clock deadline like
/// [`API_REQUEST_TIMEOUT`] — a 500 MiB body under such a bound would
/// require a minimum sustained uplink and cut off real slow uploads. A
/// progressing upload delivers a byte well inside 60s and completes as long
/// as it finishes within [`MEDIA_UPLOAD_CEILING`]; only a withheld body
/// (the parked-task attack) trips this bound.
const MEDIA_BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Wall-clock ceiling on a media *upload* request's whole service future.
/// This is not the upload budget — the idle bound above does that work — it
/// backstops the post-body phase (storage writes, thumbnailing) so a hung
/// S3 call cannot park the task forever. 3600s matches the Blossom auth
/// window: an upload still in flight past its own auth expiry was dead
/// anyway.
///
/// Named cost (measured by Sami, `RESEARCH/PR4401_MEDIA_TIMEOUT_SHAPES.md`):
/// an authed client trickling one byte per <60s never trips the idle bound
/// and holds an upload permit for up to this ceiling — 12x longer than the
/// previous 300s wall-clock. Accepted tradeoff: it is still bounded (main
/// had no media deadline at all, so a trickler held a permit forever), and
/// it sits behind Blossom auth, relay membership, 30/min rate limiting, and
/// the 2-per-pubkey concurrency cap, so exhausting the global permit pool
/// of 8 requires 4 distinct authorized pubkeys.
const MEDIA_UPLOAD_CEILING: Duration = Duration::from_secs(3600);

/// Deadline for media *read* routes (`GET`/`HEAD /media/{sha256_ext}`).
/// These carry no request body, so the idle body timeout is inapplicable —
/// without their own bound, a hung storage read would park a task forever
/// (the same shape as #4424, on the read side). Wall-clock is the correct
/// instrument here: it only covers until response headers are produced — a
/// streaming blob download escapes it once headers are sent, so large/slow
/// downloads are never truncated.
///
/// 300s preserves the read bound these routes already had when the previous
/// shared media deadline shipped — splitting uploads off must not silently
/// tighten reads. It also covers the multi-call pre-header path: the read
/// handler awaits several *sequential* storage calls before headers (sidecar
/// MIME read, ext cross-check, HEAD, then GET/range), each independently
/// allowed up to 60s by rust-s3's per-call default, so a 60s request
/// deadline could cancel a sequence whose individual calls are all within
/// their own dependency budgets.
const MEDIA_READ_TIMEOUT: Duration = Duration::from_secs(300);

/// Apply a sub-router's request-body limit with a request deadline **outside**
/// it, so a stalled body is cancelled by the timeout instead of sitting inside
/// the body-limit middleware indefinitely. On expiry the client receives an
/// empty `408 Request Timeout` response ([`TimeoutLayer::with_status_code`]).
///
/// This is the right instrument for the API router, whose bodies are small
/// JSON documents (≤1 MiB): a wall-clock bound on the whole service future
/// cannot misfire on a legitimate request. Do NOT use it for media — see
/// [`with_media_body_guards`]. The admin router, git policy router, SPA
/// fallback, and health listener are not routed through either helper, and
/// header-read deadlines before routing are out of scope here (tracked in
/// #4424).
fn with_request_deadline<S>(router: Router<S>, body_limit: usize, timeout: Duration) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .layer(RequestBodyLimitLayer::new(body_limit))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            timeout,
        ))
}

/// Apply the media *upload* sub-router's request-body limit plus an
/// **idle-based** body timeout ([`RequestBodyTimeoutLayer`]): the deadline
/// resets on every body frame, so a slow-but-progressing upload within the
/// wall-clock ceiling completes
/// while a withheld body still fails closed. Unlike [`with_request_deadline`]
/// the layer does not synthesize the response itself — the timeout surfaces
/// as a body read error ([`tower_http::timeout::TimeoutError`] in the source
/// chain), which `upload_blob` maps to `408 Request Timeout` at every
/// body-consumption path (see [`buzz_media::classify_body_error`]).
///
/// A generous wall-clock `ceiling` ([`TimeoutLayer`]) wraps everything to
/// backstop the post-body phase (hung storage writes). Uploads and reads are
/// two different timeout semantics: media *read* routes take a separate,
/// tight [`with_request_deadline`] instead — do not route them through this
/// helper, whose ceiling is sized for a 500 MiB slow upload.
///
/// Layer order (outermost first): ceiling, idle body timeout, body limit —
/// so the handler polls `Limited<TimeoutBody<Body>>`. Both body guards apply
/// to the body the handler actually reads, and an oversized `Content-Length`
/// is still rejected up front with `413` by the limit layer.
pub(crate) fn with_media_body_guards<S>(
    router: Router<S>,
    body_limit: usize,
    idle_timeout: Duration,
    ceiling: Duration,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .layer(RequestBodyLimitLayer::new(body_limit))
        .layer(RequestBodyTimeoutLayer::new(idle_timeout))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            ceiling,
        ))
}

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
    // Uploads and reads carry different timeout semantics, so the media
    // router is split by route class before merging: uploads get body
    // guards + a generous ceiling; body-less reads get a tight wall-clock
    // deadline (see the constants above for why neither fits the other).
    let media_upload_router = Router::new()
        .route("/upload", put(api::media::upload_blob))
        .route("/media/upload", put(api::media::upload_blob));
    let media_upload_router = with_media_body_guards(
        media_upload_router,
        media_body_limit,
        MEDIA_BODY_IDLE_TIMEOUT,
        MEDIA_UPLOAD_CEILING,
    );
    let media_read_router = Router::new().route(
        "/media/{sha256_ext}",
        get(api::media::get_blob).head(api::media::head_blob),
    );
    let media_read_router =
        with_request_deadline(media_read_router, media_body_limit, MEDIA_READ_TIMEOUT);
    let media_router = media_upload_router
        .merge(media_read_router)
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
        );
    // Reject request bodies larger than 1 MB to prevent resource exhaustion,
    // and bound the request future so a withheld body cannot park a task
    // (WebSocket routes: handshake bounded, established session unaffected).
    let api_router = with_request_deadline(api_router, 1024 * 1024, API_REQUEST_TIMEOUT)
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
        let (pg_ok, redis_ok, deletion_catalog_ok) = tokio::join!(
            state.db.ping(),
            async { state.redis_pool.get().await.is_ok() },
            async { state.db.validate_deletion_serving_catalog().await.is_ok() },
        );
        (pg_ok, redis_ok, deletion_catalog_ok)
    };

    let (pg_ok, redis_ok, deletion_catalog_ok) =
        tokio::time::timeout(Duration::from_secs(2), check)
            .await
            .unwrap_or((false, false, false));

    if pg_ok && redis_ok && deletion_catalog_ok {
        (StatusCode::OK, Json(json!({"status": "ready"}))).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "not_ready",
                "postgres": pg_ok,
                "redis": redis_ok,
                "deletion_catalog": deletion_catalog_ok
            })),
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

    /// A request body that never produces its bytes — the wire shape of a
    /// client that sends headers and then withholds the body forever.
    fn stalled_body() -> Body {
        Body::from_stream(futures_util::stream::pending::<
            Result<bytes::Bytes, std::io::Error>,
        >())
    }

    /// A test router shaped like the production sub-routers: a handler that
    /// collects its request body, wrapped by [`with_request_deadline`] with a
    /// millisecond deadline so tests do not sleep for production constants.
    /// `completed` is set only if the handler finishes collecting the body.
    fn deadline_test_router(
        timeout: Duration,
        completed: Arc<std::sync::atomic::AtomicBool>,
    ) -> Router {
        let router = Router::new().route(
            "/collect",
            axum::routing::post(move |request: axum::extract::Request| {
                let completed = completed.clone();
                async move {
                    let _ = axum::body::to_bytes(request.into_body(), usize::MAX).await;
                    completed.store(true, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        );
        with_request_deadline(router, 1024, timeout)
    }

    /// A test router shaped like the production media router **after the
    /// upload/read split and merge**: `/upload` (POST) and the legacy
    /// `/media/upload` alias (PUT, as in production — the literal that must
    /// keep winning over the read router's `/media/{sha256_ext}` param
    /// capture) stream their request body and classify read errors the way
    /// the test handler below does, wrapped by [`with_media_body_guards`];
    /// `/hang` and `GET /media/{sha256_ext}` (no request body, stall without
    /// ever polling one — a hung storage read) are wrapped by
    /// [`with_request_deadline`]; the two are merged like `build_router`
    /// does. `received` counts body bytes the upload handlers actually
    /// observed.
    fn media_guards_test_router(
        idle_timeout: Duration,
        upload_ceiling: Duration,
        read_timeout: Duration,
        body_limit: usize,
        received: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Router {
        let upload_handler = move |request: axum::extract::Request| {
            let received = received.clone();
            async move {
                use futures_util::StreamExt;
                let mut stream = request.into_body().into_data_stream();
                while let Some(next) = stream.next().await {
                    match next {
                        Ok(chunk) => {
                            received.fetch_add(chunk.len(), Ordering::SeqCst);
                        }
                        Err(error) => {
                            return match buzz_media::classify_body_error(&error) {
                                buzz_media::BodyErrorKind::IdleTimeout => {
                                    StatusCode::REQUEST_TIMEOUT
                                }
                                buzz_media::BodyErrorKind::LengthLimit => {
                                    StatusCode::PAYLOAD_TOO_LARGE
                                }
                                buzz_media::BodyErrorKind::Other => {
                                    StatusCode::INTERNAL_SERVER_ERROR
                                }
                            };
                        }
                    }
                }
                StatusCode::OK
            }
        };
        let upload_router = Router::new()
            .route("/upload", axum::routing::post(upload_handler.clone()))
            .route("/media/upload", axum::routing::put(upload_handler));
        let hang_handler = || async {
            std::future::pending::<()>().await;
            StatusCode::OK
        };
        let read_router = Router::new()
            .route("/hang", get(hang_handler))
            .route("/media/{sha256_ext}", get(hang_handler));
        with_media_body_guards(upload_router, body_limit, idle_timeout, upload_ceiling)
            .merge(with_request_deadline(read_router, body_limit, read_timeout))
    }

    /// A body that delivers every declared byte, just paced: `chunks` chunks
    /// of `chunk_len` bytes with `gap` between them. The legitimate
    /// slow-uplink wire shape — the one `stalled_body()` cannot express.
    fn paced_body(chunks: usize, chunk_len: usize, gap: Duration) -> Body {
        Body::from_stream(futures_util::stream::unfold(
            0usize,
            move |sent| async move {
                if sent >= chunks {
                    return None;
                }
                if sent > 0 {
                    tokio::time::sleep(gap).await;
                }
                Some((
                    Ok::<_, std::io::Error>(bytes::Bytes::from(vec![0u8; chunk_len])),
                    sent + 1,
                ))
            },
        ))
    }

    #[tokio::test]
    async fn slow_but_progressing_media_body_completes_past_the_idle_bound() {
        // THE row that separates an idle deadline from a wall-clock one: the
        // client sends every byte, just slower than the bound in total. A
        // wall-clock deadline (the old media TimeoutLayer) cuts this upload
        // off; the idle deadline must let it finish because every chunk gap
        // is under the bound. The read deadline is deliberately TIGHTER than
        // the total upload duration: it must bound only read routes, never
        // leak onto the merged upload route.
        let received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let router = media_guards_test_router(
            Duration::from_millis(50),
            Duration::from_secs(60),
            Duration::from_millis(50),
            64 * 1024,
            received.clone(),
        );

        // 10 x 100 bytes, 20ms apart: total ~180ms, well past the 50ms bound;
        // each inter-chunk gap comfortably inside it.
        let request = Request::post("/upload")
            .body(paced_body(10, 100, Duration::from_millis(20)))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a progressing body must never be cut off by the idle deadline"
        );
        assert_eq!(
            received.load(Ordering::SeqCst),
            1000,
            "every declared byte must reach the handler"
        );
    }

    #[tokio::test]
    async fn withheld_media_body_fails_closed_with_408_and_frees_the_task() {
        // The attack shape: headers sent, body withheld forever. The idle
        // deadline must surface a typed body error that classifies to 408,
        // and the handler must *return* (task freed) instead of parking.
        // The ceiling is generous, so it is the idle bound that fires.
        let received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let router = media_guards_test_router(
            Duration::from_millis(50),
            Duration::from_secs(60),
            Duration::from_secs(60),
            64 * 1024,
            received.clone(),
        );

        let request = Request::post("/upload").body(stalled_body()).unwrap();
        let started = std::time::Instant::now();
        let response = router.oneshot(request).await.unwrap();

        // The handler produced this response itself — its task is released.
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "idle deadline must fire at the configured bound, not hang"
        );
        assert_eq!(received.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn media_body_over_the_limit_is_still_rejected_with_413() {
        // Layer-order regression: the body limit must still guard the body
        // the handler reads. Declared oversize is rejected up front by the
        // limit layer; an undeclared oversize stream errors mid-read and
        // classifies to 413 (never 408, never 500).
        let received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let router = media_guards_test_router(
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(60),
            512,
            received.clone(),
        );

        let declared = Request::post("/upload")
            .header("content-length", "1024")
            .body(Body::from(vec![0u8; 1024]))
            .unwrap();
        let response = router.clone().oneshot(declared).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let undeclared = Request::post("/upload")
            .body(paced_body(4, 256, Duration::from_millis(1)))
            .unwrap();
        let response = router.oneshot(undeclared).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn stalled_no_body_media_route_is_bounded_by_the_read_deadline() {
        // Sami's fifth row: the idle body timeout only guards routes that
        // read a request body. A GET with no body that stalls (hung storage
        // read) must be bounded by the read routes' own wall-clock deadline
        // — the generous upload ceiling must NOT be what bounds it (here the
        // ceiling is 60s and the test asserts a sub-5s cutoff), and with no
        // bound at all this parks a task forever, reintroducing #4424 on
        // the read side.
        let received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let router = media_guards_test_router(
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_millis(50),
            64 * 1024,
            received,
        );

        let request = Request::get("/hang").body(Body::empty()).unwrap();
        let started = std::time::Instant::now();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::REQUEST_TIMEOUT,
            "a stalled no-body route must be cut off by the read deadline, not parked"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "read deadline must fire at its own bound, not the upload ceiling"
        );
    }

    #[tokio::test]
    async fn legacy_media_upload_alias_stays_under_the_upload_guards() {
        // Route-precedence regression (Sami's alias seam): `/media/upload`
        // lives in the *upload* sub-router while `/media/{sha256_ext}` lives
        // in the *read* sub-router — two routers sharing a path prefix,
        // merged. If axum ever resolved the literal under the param capture,
        // the alias would inherit the tight read wall-clock and the commit-3
        // regression would survive on exactly one route, invisible to tests
        // that only use `/upload`. The read deadline here is deliberately
        // TIGHTER than the total upload duration so misrouting is fatal to
        // the test, not silent.
        let received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let router = media_guards_test_router(
            Duration::from_millis(50),
            Duration::from_secs(60),
            Duration::from_millis(50),
            64 * 1024,
            received.clone(),
        );

        // 10 x 100 bytes, 20ms apart: total ~180ms — past the 50ms read
        // deadline, every gap inside the 50ms idle bound.
        let request = Request::put("/media/upload")
            .body(paced_body(10, 100, Duration::from_millis(20)))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a slow-but-progressing PUT /media/upload must complete under the \
             upload guards, not die at the read deadline"
        );
        assert_eq!(received.load(Ordering::SeqCst), 1000);

        // Discriminator: GET on the literal must be 405 (method not allowed
        // on the upload router's literal route), NOT a 408 from the read
        // deadline — proving the static literal still wins over the read
        // router's `{sha256_ext}` param capture. The status alone
        // discriminates: the param route's stalling handler can only ever
        // produce 408.
        let request = Request::get("/media/upload").body(Body::empty()).unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "GET /media/upload must resolve to the upload router's literal \
             (405), not the read router's param capture"
        );
    }

    #[tokio::test]
    async fn stalled_request_body_times_out_with_408_and_cancels_handler() {
        // Layer-order regression: the deadline must sit *outside* the body
        // limit, so a withheld body is cancelled by the timeout instead of
        // sitting inside the body-limit middleware indefinitely.
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let router = deadline_test_router(Duration::from_millis(50), completed.clone());

        let request = Request::post("/collect").body(stalled_body()).unwrap();
        let started = std::time::Instant::now();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timeout must fire at the configured bound, not hang"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(body.is_empty(), "tower-http timeout responses are empty");
        // The handler future was dropped mid-body-collection.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !completed.load(Ordering::SeqCst),
            "cancelled handler must never complete"
        );
    }

    #[tokio::test]
    async fn dropped_stalled_request_never_completes_handler() {
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let router = deadline_test_router(Duration::from_secs(60), completed.clone());

        let request = Request::post("/collect").body(stalled_body()).unwrap();
        let response_future = router.oneshot(request);
        tokio::select! {
            _ = response_future => panic!("stalled request must not produce a response yet"),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
        // `response_future` was dropped by the select; the handler must not
        // complete afterwards.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !completed.load(Ordering::SeqCst),
            "dropped request future must not complete the handler"
        );
    }

    #[tokio::test]
    async fn websocket_session_survives_request_timeout() {
        // The deadline bounds only the handshake response future. Once the
        // 101 upgrade is returned, the established session escapes it.
        let timeout = Duration::from_millis(200);
        let (received_tx, mut received_rx) = mpsc::unbounded_channel();
        let router = Router::new().route(
            "/",
            get(move |ws: WebSocketUpgrade| {
                let received_tx = received_tx.clone();
                async move {
                    ws.on_upgrade(move |mut socket| async move {
                        let _ = received_tx.send(matches!(socket.recv().await, Some(Ok(_))));
                    })
                }
            }),
        );
        let app = with_request_deadline(router, 1024, timeout);

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
        // Outlive the request deadline several times over, then prove the
        // established session still works.
        tokio::time::sleep(timeout * 4).await;
        client
            .send(Message::Text("still alive".into()))
            .await
            .expect("send on established session after the deadline");

        let received = tokio::time::timeout(Duration::from_secs(2), received_rx.recv())
            .await
            .expect("server should process the message")
            .expect("server should report receipt");
        assert!(
            received,
            "established WebSocket session must survive the request deadline"
        );

        server.abort();
        let _ = server.await;
    }
}
