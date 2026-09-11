//! axum routers — app (WebSocket + REST), health (K8s probes), metrics (Prometheus).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

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
use tower_http::timeout::{RequestBodyTimeoutLayer, TimeoutLayer};
use tower_http::trace::{HttpMakeClassifier, TraceLayer};

use crate::api;
use crate::audio;
use crate::connection::handle_connection;
use crate::metrics::track_metrics;
use crate::nip11::{nip11_document, relay_info_handler};
use crate::readiness::{self, ReadinessEvaluation, ReadinessReason};
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

/// Compatibility endpoint on the public listener. It evaluates dependencies
/// and preserves the existing response contract but never records rollout
/// telemetry.
async fn public_readiness_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if !state.readiness.public_evaluation_allowed() {
        return readiness_response(ReadinessEvaluation::shutting_down(), false);
    }

    let evaluation = state.readiness.evaluate(&state.db, &state.redis_pool).await;
    let evaluation = state.readiness.finish_public_evaluation(evaluation);
    readiness_response(evaluation, false)
}

/// Kubernetes health-listener endpoint. All rollout metrics flow through the
/// process-owned coordinator so shutdown and probe generations are ordered.
async fn kubernetes_readiness_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let readiness::ProbeStart::Evaluate(ticket) = state.readiness.begin_probe() else {
        return readiness_response(ReadinessEvaluation::shutting_down(), true);
    };

    let evaluation = state.readiness.evaluate(&state.db, &state.redis_pool).await;
    let evaluation = state.readiness.finish_probe(ticket, evaluation);
    readiness_response(evaluation, true)
}

fn readiness_response(
    evaluation: ReadinessEvaluation,
    include_reason: bool,
) -> axum::response::Response {
    if evaluation.reason == ReadinessReason::ShuttingDown {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "shutting_down"})),
        )
            .into_response();
    }

    let pg_ok = evaluation.postgres_ready();
    let redis_ok = evaluation.redis_ready();
    let deletion_catalog_ok = evaluation.deletion_catalog_ready();

    if evaluation.is_ready() {
        (StatusCode::OK, Json(json!({"status": "ready"}))).into_response()
    } else {
        let mut payload = json!({
            "status": "not_ready",
            "postgres": pg_ok,
            "redis": redis_ok,
            "deletion_catalog": deletion_catalog_ok
        });
        if include_reason {
            payload["reason"] = json!(evaluation.reason.label());
        }
        (StatusCode::SERVICE_UNAVAILABLE, Json(payload)).into_response()
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

/// Status endpoint — service name, version, uptime, and intrinsic build identity.
async fn status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(status_payload(state.started_at.elapsed().as_secs()))
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
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::{Mutex, PoisonError};
    use std::time::Duration;

    use axum::{routing::get, Router};
    use futures_util::SinkExt;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tokio::net::TcpListener;
    use tokio::sync::{mpsc, Notify};
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use tower::ServiceBuilder;
    use tracing::Instrument as _;
    use tracing_subscriber::prelude::*;

    use super::*;

    struct ScriptedReadinessEvaluator {
        evaluations: Mutex<VecDeque<ReadinessEvaluation>>,
    }

    impl ScriptedReadinessEvaluator {
        fn new(evaluations: impl IntoIterator<Item = ReadinessEvaluation>) -> Self {
            Self {
                evaluations: Mutex::new(evaluations.into_iter().collect()),
            }
        }

        fn push(&self, evaluation: ReadinessEvaluation) {
            self.evaluations
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back(evaluation);
        }
    }

    #[async_trait::async_trait]
    impl readiness::ReadinessEvaluator for ScriptedReadinessEvaluator {
        async fn evaluate(
            &self,
            _db: &buzz_db::Db,
            _redis_pool: &deadpool_redis::Pool,
        ) -> ReadinessEvaluation {
            self.evaluations
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .expect("scripted readiness evaluation")
        }
    }

    struct BarrierReadinessEvaluator {
        calls: AtomicUsize,
        first_started: Notify,
        release_first: Notify,
        first: ReadinessEvaluation,
        second: ReadinessEvaluation,
    }

    impl BarrierReadinessEvaluator {
        fn new(first: ReadinessEvaluation, second: ReadinessEvaluation) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                first_started: Notify::new(),
                release_first: Notify::new(),
                first,
                second,
            }
        }
    }

    #[async_trait::async_trait]
    impl readiness::ReadinessEvaluator for BarrierReadinessEvaluator {
        async fn evaluate(
            &self,
            _db: &buzz_db::Db,
            _redis_pool: &deadpool_redis::Pool,
        ) -> ReadinessEvaluation {
            if self.calls.fetch_add(1, AtomicOrdering::SeqCst) == 0 {
                self.first_started.notify_waiters();
                self.release_first.notified().await;
                self.first
            } else {
                self.second
            }
        }
    }

    fn readiness_evaluation(
        postgres: readiness::PostgresOutcome,
        redis: readiness::RedisOutcome,
        deletion_catalog: readiness::DeletionCatalogOutcome,
    ) -> ReadinessEvaluation {
        ReadinessEvaluation::from_results(
            readiness::TimedOutcome::new(postgres, Duration::from_millis(35)),
            readiness::TimedOutcome::new(redis, Duration::from_millis(20)),
            readiness::TimedOutcome::new(deletion_catalog, Duration::from_millis(15)),
            Duration::from_millis(35),
        )
    }

    fn ready_evaluation() -> ReadinessEvaluation {
        readiness_evaluation(
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

    async fn readiness_state(evaluator: Arc<dyn readiness::ReadinessEvaluator>) -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.database_url = "postgres://buzz:buzz_dev@127.0.0.1:1/buzz".to_string();
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
        let (mut state, _audit_shutdown) = AppState::new(
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
        state.set_readiness_evaluator(evaluator);
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

    fn readiness_metric_lines(rendered: &str) -> Vec<&str> {
        rendered
            .lines()
            .filter(|line| line.starts_with("buzz_readiness"))
            .collect()
    }

    fn sorted_readiness_metric_lines(rendered: &str) -> Vec<String> {
        let mut lines = readiness_metric_lines(rendered)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        lines.sort();
        lines
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

    #[test]
    fn production_readiness_routes_export_the_frozen_health_only_contract() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let evaluator = Arc::new(ScriptedReadinessEvaluator::new(std::iter::repeat_n(
            ready_evaluation(),
            4,
        )));
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
                let first_scrape = handle.render();

                assert!(first_scrape.contains("# TYPE buzz_readiness_checks_total counter"));
                assert!(first_scrape
                    .contains("# TYPE buzz_readiness_dependency_checks_total counter"));
                assert!(first_scrape
                    .contains("# TYPE buzz_readiness_check_duration_seconds histogram"));
                assert!(first_scrape.contains("# TYPE buzz_readiness_state gauge"));
                assert_eq!(
                    metric_value(
                        &first_scrape,
                        "buzz_readiness_checks_total{reason=\"ready\"}"
                    ),
                    1.0
                );
                assert_eq!(
                    metric_value(
                        &first_scrape,
                        "buzz_readiness_dependency_checks_total{dependency=\"postgres\",outcome=\"success\"}"
                    ),
                    1.0
                );
                assert_eq!(
                    metric_value(
                        &first_scrape,
                        "buzz_readiness_state{check=\"overall\"}"
                    ),
                    1.0
                );
                for bucket in ["2", "2.5", "+Inf"] {
                    assert!(first_scrape.contains(&format!(
                        "buzz_readiness_check_duration_seconds_bucket{{check=\"overall\",le=\"{bucket}\"}}"
                    )));
                }
                assert!(!first_scrape.contains("result="));
                assert!(!first_scrape
                    .lines()
                    .filter(|line| line.starts_with("buzz_readiness_check_duration_seconds"))
                    .any(|line| line.contains("outcome=")));

                let before_public_failure = sorted_readiness_metric_lines(&first_scrape);
                evaluator.push(readiness_evaluation(
                    readiness::PostgresOutcome::Success,
                    readiness::RedisOutcome::PoolTimeout,
                    readiness::DeletionCatalogOutcome::Success,
                ));
                assert_eq!(
                    readiness_request(public.clone()).await,
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        json!({
                            "status": "not_ready",
                            "postgres": true,
                            "redis": false,
                            "deletion_catalog": true
                        })
                    )
                );
                assert_eq!(
                    sorted_readiness_metric_lines(&handle.render()),
                    before_public_failure
                );

                let contract_evaluations = [
                    readiness_evaluation(
                        readiness::PostgresOutcome::PoolTimeout,
                        readiness::RedisOutcome::Success,
                        readiness::DeletionCatalogOutcome::Success,
                    ),
                    readiness_evaluation(
                        readiness::PostgresOutcome::PoolError,
                        readiness::RedisOutcome::Success,
                        readiness::DeletionCatalogOutcome::Success,
                    ),
                    readiness_evaluation(
                        readiness::PostgresOutcome::QueryTimeout,
                        readiness::RedisOutcome::Success,
                        readiness::DeletionCatalogOutcome::Success,
                    ),
                    readiness_evaluation(
                        readiness::PostgresOutcome::QueryError,
                        readiness::RedisOutcome::Success,
                        readiness::DeletionCatalogOutcome::Success,
                    ),
                    readiness_evaluation(
                        readiness::PostgresOutcome::Success,
                        readiness::RedisOutcome::PoolTimeout,
                        readiness::DeletionCatalogOutcome::Success,
                    ),
                    readiness_evaluation(
                        readiness::PostgresOutcome::Success,
                        readiness::RedisOutcome::PoolError,
                        readiness::DeletionCatalogOutcome::Success,
                    ),
                    readiness_evaluation(
                        readiness::PostgresOutcome::Success,
                        readiness::RedisOutcome::Success,
                        readiness::DeletionCatalogOutcome::OperationTimeout,
                    ),
                    readiness_evaluation(
                        readiness::PostgresOutcome::Success,
                        readiness::RedisOutcome::Success,
                        readiness::DeletionCatalogOutcome::OperationError,
                    ),
                    readiness_evaluation(
                        readiness::PostgresOutcome::PoolTimeout,
                        readiness::RedisOutcome::PoolTimeout,
                        readiness::DeletionCatalogOutcome::OperationTimeout,
                    ),
                    readiness_evaluation(
                        readiness::PostgresOutcome::PoolError,
                        readiness::RedisOutcome::PoolError,
                        readiness::DeletionCatalogOutcome::Success,
                    ),
                ];
                for evaluation in contract_evaluations {
                    evaluator.push(evaluation);
                    let (status, payload) = readiness_request(health.clone()).await;
                    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
                    assert_eq!(payload["reason"], json!(evaluation.reason.label()));
                }

                let before_shutdown = handle.render();
                let histogram_counts_before = ["overall", "postgres", "redis", "deletion_catalog"]
                    .map(|check| {
                        metric_value(
                            &before_shutdown,
                            &format!(
                                "buzz_readiness_check_duration_seconds_count{{check=\"{check}\"}}"
                            ),
                        )
                    });
                state.begin_shutdown();
                assert_eq!(
                    readiness_request(public).await,
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        json!({"status": "shutting_down"})
                    )
                );
                let after_public_shutdown = handle.render();
                assert!(after_public_shutdown
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
                let histogram_counts_after = ["overall", "postgres", "redis", "deletion_catalog"]
                    .map(|check| {
                        metric_value(
                            &final_scrape,
                            &format!(
                                "buzz_readiness_check_duration_seconds_count{{check=\"{check}\"}}"
                            ),
                        )
                    });
                assert_eq!(histogram_counts_after, histogram_counts_before);
                assert_eq!(
                    metric_value(
                        &final_scrape,
                        "buzz_readiness_checks_total{reason=\"shutting_down\"}"
                    ),
                    1.0
                );
                assert_eq!(
                    metric_value(
                        &final_scrape,
                        "buzz_readiness_state{check=\"overall\"}"
                    ),
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
                    "readiness series contract must stay at or below its 99-series cap"
                );
            });
        });
    }

    fn run_out_of_order_route_case(
        first: ReadinessEvaluation,
        second: ReadinessEvaluation,
    ) -> (serde_json::Value, serde_json::Value, String) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let evaluator = Arc::new(BarrierReadinessEvaluator::new(first, second));
        let (recorder, handle) = crate::metrics::readiness_test_recorder();

        metrics::with_local_recorder(&recorder, || {
            runtime.block_on(async {
                let state = readiness_state(evaluator.clone()).await;
                let health = build_health_router(state);
                let first_started = evaluator.first_started.notified();
                let slow_first = tokio::spawn(readiness_request(health.clone()));
                first_started.await;

                let (_, second_payload) = readiness_request(health).await;
                evaluator.release_first.notify_one();
                let (_, first_payload) = slow_first.await.expect("slow first probe task");
                (first_payload, second_payload, handle.render())
            })
        })
    }

    #[test]
    fn real_health_route_generation_fence_covers_both_completion_orders() {
        let failure = readiness_evaluation(
            readiness::PostgresOutcome::Success,
            readiness::RedisOutcome::PoolTimeout,
            readiness::DeletionCatalogOutcome::Success,
        );

        let (older_failure, newer_success, success_scrape) =
            run_out_of_order_route_case(failure, ready_evaluation());
        assert_eq!(older_failure["reason"], json!("redis_pool_timeout"));
        assert_eq!(newer_success, json!({"status": "ready"}));
        assert_eq!(
            metric_value(&success_scrape, "buzz_readiness_state{check=\"overall\"}"),
            1.0
        );
        assert_eq!(
            metric_value(&success_scrape, "buzz_readiness_state{check=\"redis\"}"),
            1.0
        );

        let (older_success, newer_failure, failure_scrape) =
            run_out_of_order_route_case(ready_evaluation(), failure);
        assert_eq!(older_success, json!({"status": "ready"}));
        assert_eq!(newer_failure["reason"], json!("redis_pool_timeout"));
        assert_eq!(
            metric_value(&failure_scrape, "buzz_readiness_state{check=\"overall\"}"),
            0.0
        );
        assert_eq!(
            metric_value(&failure_scrape, "buzz_readiness_state{check=\"redis\"}"),
            0.0
        );
        for scrape in [&success_scrape, &failure_scrape] {
            assert_eq!(
                metric_value(scrape, "buzz_readiness_checks_total{reason=\"ready\"}"),
                1.0
            );
            assert_eq!(
                metric_value(
                    scrape,
                    "buzz_readiness_checks_total{reason=\"redis_pool_timeout\"}"
                ),
                1.0
            );
        }
    }

    #[test]
    fn real_health_route_shutdown_fence_dominates_an_in_flight_success() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let evaluator = Arc::new(BarrierReadinessEvaluator::new(
            ready_evaluation(),
            ready_evaluation(),
        ));
        let (recorder, handle) = crate::metrics::readiness_test_recorder();

        metrics::with_local_recorder(&recorder, || {
            runtime.block_on(async {
                let state = readiness_state(evaluator.clone()).await;
                let health = build_health_router(state.clone());
                let first_started = evaluator.first_started.notified();
                let in_flight = tokio::spawn(readiness_request(health));
                first_started.await;

                state.begin_shutdown();
                evaluator.release_first.notify_one();
                assert_eq!(
                    in_flight.await.expect("in-flight readiness task"),
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        json!({"status": "shutting_down"})
                    )
                );

                let scrape = handle.render();
                assert_eq!(
                    metric_value(&scrape, "buzz_readiness_state{check=\"overall\"}"),
                    0.0
                );
                assert!(scrape
                    .lines()
                    .all(|line| !line.starts_with("buzz_readiness_state{check=\"postgres\"}")));
                assert_eq!(
                    metric_value(
                        &scrape,
                        "buzz_readiness_checks_total{reason=\"shutting_down\"}"
                    ),
                    1.0
                );
                assert_eq!(
                    metric_value(
                        &scrape,
                        "buzz_readiness_dependency_checks_total{dependency=\"postgres\",outcome=\"success\"}"
                    ),
                    1.0
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
