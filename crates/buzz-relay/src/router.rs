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
use crate::readiness::{self, ReadinessEvaluation, ReadinessReason};
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
        // NIP-FI admin command API — authenticated by signed command JWT,
        // NOT by NIP-98.  Self-contained auth inside the handler.
        .route("/api/nip-fi/disconnect", post(api::nip_fi::disconnect))
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

    // NIP-FI assertion check at upgrade — runs BEFORE `bind_community` so that:
    // (a) a denied upgrade pays zero DB cost [FI-TRACE-TRANSPORT-CLOSED], and
    // (b) tests that assert 401/503 are not pre-empted by a 404 from an
    //     unseeded DB — the gate exercises its own seam without coupling to
    //     host-resolution fixture state.
    //
    // Gated to genuine WebSocket upgrade requests (requests carrying both
    // `Upgrade: websocket` and a `Connection` header with the `Upgrade` token)
    // so plain browser GET / and NIP-11 fallback requests are never intercepted
    // by the enforcement gate.  Requiring both headers matches RFC 6455 §4.1
    // and avoids intercepting a request that carries only one header and would
    // be rejected by Axum's WebSocketUpgrade extractor anyway.
    //
    // Keying on the header pair (not on `Accept`) means an HTML Accept header
    // on a real WS upgrade is still gated correctly.
    //
    // This pre-check runs BEFORE `WebSocketUpgrade::from_request` so that the
    // denial response is returned on the raw HTTP connection, not inside the
    // upgrade callback.
    let nip_fi_assertion = {
        let is_ws_upgrade = headers
            .get(axum::http::header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false)
            && headers
                .get(axum::http::header::CONNECTION)
                .and_then(|v| v.to_str().ok())
                .map(|v| {
                    // Connection header is a comma-separated token list; per RFC 7230
                    // each token is case-insensitive. A genuine WS upgrade carries
                    // "Upgrade" (or "keep-alive, Upgrade") as a Connection token.
                    v.split(',')
                        .any(|t| t.trim().eq_ignore_ascii_case("upgrade"))
                })
                .unwrap_or(false);
        if is_ws_upgrade {
            use crate::nip_fi_upgrade::{check_nip_fi_at_upgrade, NipFiUpgradeOutcome};
            let mode = state.config.nip_fi.mode;
            let verifier = state.nip_fi_verifier.as_deref();
            match check_nip_fi_at_upgrade(&headers, verifier, mode) {
                NipFiUpgradeOutcome::NotRequired => None,
                NipFiUpgradeOutcome::Admitted(assertion) => Some(assertion),
                NipFiUpgradeOutcome::Denied(resp) => return resp.into_response(),
            }
        } else {
            None
        }
    };

    // S4 deny-map early-bounce check: runs after assertion validation, before bind_community.
    //
    // This is an OPTIMIZATION (early HTTP bounce), not the correctness mechanism.
    // Correctness is enforced in step 6 of the spec (NIP-FI.md:217-233):
    // NIP-42 proof → key equality → register proven k → deny check → admit.
    // That normative sequence runs in handlers/auth.rs after set_authenticated_pubkey.
    //
    // This pre-upgrade check provides a cheap bounce for keys already in the deny
    // map before the connection is upgraded — pays zero DB cost and rejects before
    // tungstenite hands the socket to the application. It is NOT race-free against
    // a concurrent disconnect (the session isn't registered yet), which is why the
    // normative post-registration check in auth.rs is the correctness gate.
    //
    // Off-mode: `nip_fi_deny_map` is `None` → the entire block is a no-op;
    // `asserted_key` is `None` → no key to check → pass through.
    // [FI-TRACE-DENY-SET]
    if let Some(assertion) = &nip_fi_assertion {
        if let Some(key) = assertion.asserted_key() {
            if let Some(deny_map) = state.nip_fi_deny_map.as_deref() {
                if deny_map.is_denied(assertion.identity().issuer(), &key, chrono::Utc::now()) {
                    return crate::nip_fi_upgrade::denial_response(
                        buzz_auth::DenialClass::AuthorizationDenied,
                    )
                    .into_response();
                }
            }
        }
    }

    // Row zero: bind the connection to its community from the request host
    // BEFORE the WebSocket upgrade, so no frame is ever read on an unbound
    // connection. The host is the authoritative selector; an unmapped host or a
    // lookup failure fails closed with a generic rejection — never a default
    // tenant. NIP-11 above is served before binding and stays fail-open: an
    // unmapped host still gets the document (with host-scoped fields like
    // `icon` simply absent), so the doc cannot leak which hosts are mapped.
    //
    // NIP-FI gate runs above (before bind_community) so denied upgrades pay
    // zero DB cost and the gate seam is testable without a seeded-DB fixture.
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
            // Capture the upgrade instant here — before the on_upgrade callback
            // fires — so the NIP-FI session partition is rooted at the HTTP
            // handshake, not the post-community-active-check instant.
            // [FI-TRACE-LEASE-BOUND]
            let connection_time = chrono::Utc::now();
            limit_relay_websocket(ws, max_frame_bytes)
                .on_upgrade(move |socket| {
                    handle_connection(
                        socket,
                        state,
                        addr,
                        tenant,
                        nip_fi_assertion,
                        connection_time,
                    )
                })
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
            // Not a WS upgrade request — serve NIP-11 as fallback.
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

    // ── NIP-FI built-router gate: both WS ingresses ───────────────────────────
    //
    // Drive the REAL built router (via tower `oneshot`) for both the root `/`
    // and the huddle audio `/huddle/{id}/audio` WebSocket ingresses in NIP-FI
    // enforce mode. These tests prove that both gate call sites live in
    // production: deleting either gate call (at the top of `nip11_or_ws_handler`
    // in `router.rs` or at the top of `ws_audio_handler` in `audio/handler.rs`)
    // causes the request to proceed past the pre-101 check and receive a
    // 404 (tenant not found) instead of the expected denial, turning these
    // tests red.
    //
    // Mutation evidence:
    //   A) Delete the gate call in `nip11_or_ws_handler` → root request
    //      returns 404 (no community) instead of 401/503 → assert_eq panics.
    //   B) Delete the gate call in `ws_audio_handler` → audio request returns
    //      404 (no community) instead of 401/503 → assert_eq panics.
    //   C) Switch `Enforce` to `Off` in the test state → both ingresses skip
    //      the gate and return 404 (no community) → status assertions panic.

    /// Build AppState with NIP-FI enforce mode and no verifier (simulates
    /// startup with no JWKS yet warmed). The verifier is `None` because
    /// `jwks_configs` is empty and `ProductionJwksSource::new` returns `None`
    /// for an empty list; the mode field is set directly so no env is needed.
    ///
    /// The NIP-FI gate runs before `bind_community`, so these tests exercise
    /// the gate seam independently of DB / host-resolution state. The lazy
    /// PG pool is kept so `AppState::new` compiles; it is never queried by
    /// any of these router tests.
    async fn nip_fi_enforce_state() -> Arc<AppState> {
        use crate::nip_fi_config::NipFiRelayConfig;
        use buzz_auth::{IssuerRegistry, NipFiMode};

        // Build config directly without env mutation — the nip_fi field is
        // constructed explicitly below, so reading NIP-FI env vars is irrelevant
        // and mutating them would race the config-module tests (separate statics).
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        // Override NIP-FI mode to Enforce with no issuers configured — the
        // verifier will be None (no JWKS source), which is the startup-race
        // condition that must return 503 for a token-carrying request.
        config.nip_fi = NipFiRelayConfig {
            mode: NipFiMode::Enforce,
            registry: IssuerRegistry::new(),
            jwks_configs: vec![],
            command_configs: vec![],
            max_connection_lifetime_secs: 3600,
        };

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

    /// Drive a request through the real built router. Returns the HTTP status code.
    /// For WebSocket upgrade paths, sends proper upgrade headers so axum's
    /// WebSocketUpgrade extractor doesn't reject with 400 before the handler runs.
    async fn nip_fi_gate_status(
        state: Arc<AppState>,
        path: &str,
        extra_header_name: Option<&str>,
        extra_header_value: Option<&str>,
    ) -> axum::http::StatusCode {
        nip_fi_gate_response(state, path, extra_header_name, extra_header_value)
            .await
            .status()
    }

    /// Drive a request through the real built router. Returns the full response.
    async fn nip_fi_gate_response(
        state: Arc<AppState>,
        path: &str,
        extra_header_name: Option<&str>,
        extra_header_value: Option<&str>,
    ) -> axum::response::Response {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let mut builder = Request::get(path)
            .header(axum::http::header::HOST, "relay.example")
            // WebSocket upgrade headers so axum's WebSocketUpgrade extractor
            // doesn't reject with 400/426 before the handler body runs.
            .header("Upgrade", "websocket")
            .header("Connection", "Upgrade")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
        if let (Some(name), Some(value)) = (extra_header_name, extra_header_value) {
            builder = builder.header(name, value);
        }
        let req = builder.body(Body::empty()).expect("request");
        build_router(state)
            .oneshot(req)
            .await
            .expect("router response")
    }

    #[tokio::test]
    async fn nip_fi_enforce_root_denies_missing_assertion_401() {
        let state = nip_fi_enforce_state().await;
        let status = nip_fi_gate_status(state, "/", None, None).await;
        assert_eq!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "root WebSocket upgrade without assertion must be denied 401 in enforce mode"
        );
    }

    #[tokio::test]
    async fn nip_fi_enforce_audio_denies_missing_assertion_401() {
        let state = nip_fi_enforce_state().await;
        let channel_id = uuid::Uuid::new_v4();
        let path = format!("/huddle/{channel_id}/audio");
        let status = nip_fi_gate_status(state, &path, None, None).await;
        assert_eq!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "audio WebSocket upgrade without assertion must be denied 401 in enforce mode"
        );
    }

    #[tokio::test]
    async fn nip_fi_enforce_root_denies_token_when_no_verifier_503() {
        let state = nip_fi_enforce_state().await;
        // A plausible but unverifiable bearer token on the correct header —
        // verifier is None (no JWKS). Expect 503 authorization unavailable.
        let status = nip_fi_gate_status(
            state,
            "/",
            Some("Nostr-Federated-Identity"),
            Some("Bearer eyJhbGciOiJFUzI1NiJ9.e30.sig"),
        )
        .await;
        assert_eq!(
            status,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "root WebSocket upgrade with token but no verifier must be denied 503 in enforce mode"
        );
    }

    #[tokio::test]
    async fn nip_fi_enforce_audio_denies_token_when_no_verifier_503() {
        let state = nip_fi_enforce_state().await;
        let channel_id = uuid::Uuid::new_v4();
        let path = format!("/huddle/{channel_id}/audio");
        let status = nip_fi_gate_status(
            state,
            &path,
            Some("Nostr-Federated-Identity"),
            Some("Bearer eyJhbGciOiJFUzI1NiJ9.e30.sig"),
        )
        .await;
        assert_eq!(
            status,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "audio WebSocket upgrade with token but no verifier must be denied 503 in enforce mode"
        );
    }

    // ── B4: non-upgrade document requests bypass the NIP-FI gate ─────────────
    //
    // A plain browser GET / or a NIP-11 content-negotiated request must reach
    // the NIP-11 fallback path, never the enforcement gate. The gate fires only
    // on genuine WebSocket upgrades (Connection/Upgrade headers present).
    //
    // Because the gate runs before bind_community, these tests are DB-free —
    // the lazy pool is never queried and no host seeding is required. Adding a
    // DB-dependent fixture here would hide a regression where the gate fires
    // only because the unseeded-host 404 has not yet been reached.
    //
    // Mutation evidence:
    //   A) Move the NIP-FI gate back after bind_community → without a seeded
    //      DB, plain-GET tests return 404 (not 200); the assertion panics.
    //      With a seeded DB the gate returns 401/503, also panics.
    //   B) Key the gate on the Accept header → a WS request with Accept:
    //      text/html bypasses it → the 401/503 test below returns 101 → panics.

    /// Drive a plain (non-WS) GET request through the built router. Returns
    /// the HTTP status and, for NIP-11 responses, validates the JSON content.
    async fn nip_fi_non_upgrade_status(
        state: Arc<AppState>,
        path: &str,
        accept: Option<&str>,
    ) -> axum::http::StatusCode {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let mut builder = Request::get(path).header(axum::http::header::HOST, "relay.example");
        if let Some(accept_value) = accept {
            builder = builder.header("Accept", accept_value);
        }
        let req = builder.body(Body::empty()).expect("request");
        build_router(state)
            .oneshot(req)
            .await
            .expect("router response")
            .status()
    }

    #[tokio::test]
    async fn nip_fi_enforce_plain_get_serves_nip11_not_401() {
        let state = nip_fi_enforce_state().await;
        // A plain GET / without WS upgrade headers is not a WebSocket upgrade.
        // In enforce mode the NIP-FI gate must NOT intercept it — the response
        // must be the NIP-11 JSON fallback (200), not a denial (401/403/503).
        let status = nip_fi_non_upgrade_status(state, "/", None).await;
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "plain GET / in enforce mode must fall through to NIP-11 (200), not be gated (401/503)"
        );
    }

    #[tokio::test]
    async fn nip_fi_enforce_nip11_content_negotiation_serves_200_not_401() {
        let state = nip_fi_enforce_state().await;
        // application/nostr+json short-circuits before the WS check; the
        // NIP-FI gate must never intercept it regardless of mode.
        let status = nip_fi_non_upgrade_status(state, "/", Some("application/nostr+json")).await;
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "NIP-11 content-negotiated GET in enforce mode must return 200"
        );
    }

    #[tokio::test]
    async fn nip_fi_enforce_ws_upgrade_with_html_accept_is_gated_401() {
        let state = nip_fi_enforce_state().await;
        // A genuine WS upgrade request that also carries Accept: text/html
        // must still be gated. The gate must NOT key on Accept — it must key
        // on the Connection/Upgrade headers that make it a real WS upgrade.
        let status = nip_fi_gate_status(state, "/", Some("Accept"), Some("text/html")).await;
        assert_eq!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "WS upgrade with Accept: text/html in enforce mode must still be denied 401"
        );
    }

    // ── B4 negative: single-header requests bypass the NIP-FI gate ───────────
    //
    // The gate fires ONLY when BOTH `Upgrade: websocket` AND a `Connection`
    // header carrying the `upgrade` token are present. A request with only one
    // of the two headers is not a valid WebSocket upgrade and must not be
    // intercepted by the NIP-FI enforcement gate.
    //
    // Mutation evidence:
    //   A) Change the gate to key on `Upgrade: websocket` alone (drop the
    //      Connection check) → the Upgrade-only test gets denied 401 instead of
    //      passing through → the assertion panics.
    //   B) Change the gate to key on `Connection: Upgrade` alone (drop the
    //      Upgrade check) → the Connection-only test gets denied 401 → panics.

    /// Drive a request that carries exactly `Upgrade: websocket` but no
    /// `Connection` header. Must not be gated — returns whatever the NIP-11
    /// or HTTP handler produces (not 401/503 from the NIP-FI gate).
    async fn nip_fi_upgrade_only_status(
        state: Arc<AppState>,
        path: &str,
    ) -> axum::http::StatusCode {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let req = Request::get(path)
            .header(axum::http::header::HOST, "relay.example")
            .header("Upgrade", "websocket")
            // Deliberately omit Connection header.
            .body(Body::empty())
            .expect("request");
        build_router(state)
            .oneshot(req)
            .await
            .expect("router response")
            .status()
    }

    /// Drive a request that carries `Connection: Upgrade` but no `Upgrade`
    /// header. Must not be gated by the NIP-FI enforcement logic.
    async fn nip_fi_connection_only_status(
        state: Arc<AppState>,
        path: &str,
    ) -> axum::http::StatusCode {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let req = Request::get(path)
            .header(axum::http::header::HOST, "relay.example")
            .header("Connection", "Upgrade")
            // Deliberately omit Upgrade header.
            .body(Body::empty())
            .expect("request");
        build_router(state)
            .oneshot(req)
            .await
            .expect("router response")
            .status()
    }

    #[tokio::test]
    async fn b4_upgrade_only_no_connection_header_not_gated() {
        let state = nip_fi_enforce_state().await;
        // Upgrade: websocket present, Connection absent → not a valid WS
        // upgrade handshake → must NOT be denied by the NIP-FI gate.
        // The request falls through to the NIP-11 / HTTP handler, which
        // returns 200 (NIP-11 JSON) or 426 (Upgrade Required) — not 401/503.
        let status = nip_fi_upgrade_only_status(state, "/").await;
        assert_ne!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "B4: Upgrade-only request (no Connection header) must not be denied 401 by NIP-FI gate"
        );
        assert_ne!(
            status,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "B4: Upgrade-only request (no Connection header) must not be denied 503 by NIP-FI gate"
        );
    }

    #[tokio::test]
    async fn b4_connection_upgrade_only_no_upgrade_header_not_gated() {
        let state = nip_fi_enforce_state().await;
        // Connection: Upgrade present, Upgrade absent → not a valid WS
        // upgrade handshake → must NOT be denied by the NIP-FI gate.
        let status = nip_fi_connection_only_status(state, "/").await;
        assert_ne!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "B4: Connection-only request (no Upgrade header) must not be denied 401 by NIP-FI gate"
        );
        assert_ne!(
            status,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "B4: Connection-only request (no Upgrade header) must not be denied 503 by NIP-FI gate"
        );
    }

    // ── S4 deny-map admission check ──────────────────────────────────────────
    //
    // A key with a live deny entry is refused at WS admission with 403
    // `authorization_denied`, even when the assertion JWT is otherwise valid.
    //
    // Placement: the check runs after `check_nip_fi_at_upgrade` returns
    // `Admitted(assertion)` and before `bind_community`, so no DB query is
    // made for denied keys.  The test drives the REAL built router via
    // `tower::oneshot`, with a `ProductionJwksSource` seeded with a test JWKS
    // snapshot so the assertion verifier runs the full JWT pipeline.
    //
    // Mutation evidence:
    //   A) Delete the deny-map check block in `nip11_or_ws_handler` → the
    //      denied key is not refused at the pre-101 HTTP gate → the upgrade
    //      proceeds until `bind_community` returns 404 (test host not seeded) →
    //      the assertion `assert_eq!(status, 403)` below panics.
    //   B) Flip the `is_denied` condition to `!is_denied` → admitted keys
    //      are refused and denied keys are admitted → this test panics (no
    //      deny entry yet, so the negated check admits nothing, status 403
    //      for the wrong key or 404 for admitted).
    //   C) Remove the `nip_fi_deny_map` assignment from `nip_fi_deny_state`
    //      → the map is `None` → the block is a no-op → upgrade proceeds to
    //      404 (no community) → assertion panics.

    // ES256 key pair — same as command.rs / api/nip_fi.rs test material.
    const DENY_TEST_PRIVATE_KEY_PEM: &str =
        "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\nWZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\nzhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n-----END PRIVATE KEY-----\n";

    const DENY_TEST_ISS: &str = "https://nip-fi-deny-test.example.com";
    const DENY_TEST_AUD: &str = "https://relay.example";
    const DENY_TEST_KID: &str = "deny-test-key-1";

    fn deny_test_public_jwk() -> jsonwebtoken::jwk::Jwk {
        serde_json::from_value(serde_json::json!({
            "kty": "EC",
            "crv": "P-256",
            "x": "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI",
            "y": "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA",
            "alg": "ES256",
            "use": "sig",
            "kid": DENY_TEST_KID
        }))
        .expect("valid deny-test JWK")
    }

    /// Build an AppState with a seeded NIP-FI assertion verifier (`Enforce`
    /// mode, test issuer) and a populated deny map containing `denied_key`.
    async fn nip_fi_deny_state(denied_key: &nostr::PublicKey) -> Arc<AppState> {
        use crate::nip_fi_config::NipFiRelayConfig;
        use buzz_auth::{
            FederatedAssertionVerifier, FreshnessClass, HttpJwksFetcher, IssuerCapacity,
            IssuerRegistry, JwksSourceContract, NipFiDenyMap, NipFiMode, ProductionJwksSource,
            TokenClass,
        };

        // Build config in enforce mode.
        let mut config = crate::config::Config::hermetic_for_test();
        config.require_relay_membership = false;

        let jwks_contract =
            JwksSourceContract::new(format!("{DENY_TEST_ISS}/.well-known/jwks.json"), 300, 86400)
                .expect("valid JWKS contract");
        let issuer_policy = buzz_auth::IssuerPolicy::new(
            DENY_TEST_ISS.to_owned(),
            vec![DENY_TEST_AUD.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![buzz_auth::JwtAlgorithm::ES256],
            30,
            3600,
            None,
            jwks_contract.clone(),
        )
        .expect("valid test issuer policy");

        let jwks_config = buzz_auth::IssuerJwksConfig {
            issuer: DENY_TEST_ISS.to_owned(),
            contract: jwks_contract,
        };

        config.nip_fi = NipFiRelayConfig {
            mode: NipFiMode::Enforce,
            registry: {
                let mut r = IssuerRegistry::new();
                r.insert(issuer_policy);
                r
            },
            jwks_configs: vec![jwks_config],
            command_configs: vec![],
            max_connection_lifetime_secs: 3600,
        };

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

        // Wire the NIP-FI assertion verifier with a seeded JWKS snapshot so the
        // full JWT pipeline runs without any HTTP call. The seeded JWKS contains
        // the test public key that signs tokens in `mint_deny_test_token`.
        let jwks = jsonwebtoken::jwk::JwkSet {
            keys: vec![deny_test_public_jwk()],
        };
        let key_source = Arc::new(
            ProductionJwksSource::new(
                vec![buzz_auth::IssuerJwksConfig {
                    issuer: DENY_TEST_ISS.to_owned(),
                    contract: buzz_auth::JwksSourceContract::new(
                        format!("{DENY_TEST_ISS}/.well-known/jwks.json"),
                        300,
                        86400,
                    )
                    .expect("valid contract"),
                }],
                HttpJwksFetcher::new(),
            )
            .expect("key source"),
        );
        key_source.seed_snapshot_for_test(DENY_TEST_ISS, jwks).await;
        let verifier = Arc::new(FederatedAssertionVerifier::new(
            state.config.nip_fi.registry.clone(),
            Arc::clone(&key_source),
        ));
        state.nip_fi_verifier = Some(verifier);
        state.nip_fi_jwks_source = Some(Arc::clone(&key_source));

        // Populate the deny map with a live entry for the denied key.
        let deny_map = Arc::new(NipFiDenyMap::new(
            16,
            vec![IssuerCapacity {
                issuer: DENY_TEST_ISS.to_owned(),
                capacity: 16,
            }],
        ));
        let until = chrono::Utc::now() + chrono::Duration::seconds(3600);
        let merge_result =
            deny_map.merge_cross_pod_deny(DENY_TEST_ISS, denied_key, until, chrono::Utc::now());
        assert!(
            matches!(merge_result, buzz_auth::CrossPodMergeResult::Merged),
            "deny entry must be inserted for test setup"
        );
        state.nip_fi_deny_map = Some(deny_map);

        Arc::new(state)
    }

    /// Mint a valid ES256 `nip-fi+jwt` assertion for `nostr_pubkey = key_hex`,
    /// signed by the deny-test key pair.
    fn mint_deny_test_token(key_hex: &str) -> String {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": DENY_TEST_ISS,
            "aud": DENY_TEST_AUD,
            "sub": "test-subject",
            "iat": now,
            "exp": now + 600,
            "nostr_pubkey": key_hex,
        });
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("nip-fi+jwt".to_owned());
        header.kid = Some(DENY_TEST_KID.to_owned());
        let key = EncodingKey::from_ec_pem(DENY_TEST_PRIVATE_KEY_PEM.as_bytes())
            .expect("valid test EC key");
        encode(&header, &claims, &key).expect("sign deny-test token")
    }

    #[tokio::test]
    async fn deny_map_blocks_ws_admission_for_live_entry() {
        // A key with a live deny entry is refused 403 `authorization_denied`
        // at WS admission even when the bearer JWT is otherwise valid.
        //
        // Full wire contract assertion: status 403, Content-Type text/plain,
        // exact body "authorization denied\n", no WWW-Authenticate header.
        // This distinguishes AuthorizationDenied from EvidenceRejected (also 403)
        // and from AuthorizationUnavailable (503). [FI-TRACE-DENIAL-ORACLE]
        //
        // Mutation evidence (A–C in build comments above):
        //   A) Delete the deny-map check → 404 not 403 → status assert panics.
        //   B) Use DenialClass::EvidenceRejected → body is "evidence rejected\n"
        //      → body assert panics.
        //   C) Remove nip_fi_deny_map from state → map None → 404 → status panics.
        let denied_key = nostr::Keys::generate().public_key();
        let state = nip_fi_deny_state(&denied_key).await;
        let token = mint_deny_test_token(&denied_key.to_hex());
        let bearer = format!("Bearer {token}");

        let resp =
            nip_fi_gate_response(state, "/", Some("Nostr-Federated-Identity"), Some(&bearer)).await;

        assert_eq!(
            resp.status(),
            axum::http::StatusCode::FORBIDDEN,
            "WS admission for a key with a live deny entry must be refused 403 \
             authorization_denied [FI-TRACE-DENY-SET]"
        );
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; charset=utf-8"),
            "authorization_denied response must carry text/plain; charset=utf-8"
        );
        assert!(
            resp.headers().get("WWW-Authenticate").is_none(),
            "authorization_denied must NOT carry WWW-Authenticate (that is MissingEvidence only)"
        );
        let body = axum::body::to_bytes(resp.into_body(), 64)
            .await
            .expect("body bytes");
        assert_eq!(
            body.as_ref(),
            b"authorization denied\n",
            "authorization_denied wire body must be exactly 'authorization denied\\n' \
             [FI-TRACE-DENIAL-ORACLE]"
        );
    }

    #[tokio::test]
    async fn deny_map_admits_key_not_in_map() {
        // A key NOT in the deny map passes the check and proceeds to
        // bind_community (which returns 404 — test host is not seeded).
        // This proves the Off-path (no deny entry → pass through) and guards
        // against an inverted condition.
        let clean_key = nostr::Keys::generate().public_key();
        // Build state with a DIFFERENT denied key so clean_key is not in the map.
        let other_key = nostr::Keys::generate().public_key();
        let state = nip_fi_deny_state(&other_key).await;
        let token = mint_deny_test_token(&clean_key.to_hex());
        let bearer = format!("Bearer {token}");

        let status =
            nip_fi_gate_status(state, "/", Some("Nostr-Federated-Identity"), Some(&bearer)).await;

        // The key is not denied — the pre-101 gate passes and the request
        // proceeds to `bind_community`, which returns 404 (test host not seeded).
        // A 403 here means the deny check fired incorrectly for a non-denied key.
        assert_eq!(
            status,
            axum::http::StatusCode::NOT_FOUND,
            "WS admission for a key NOT in the deny map must pass the deny check \
             and proceed to bind_community (404 — test host not seeded)"
        );
    }
}
