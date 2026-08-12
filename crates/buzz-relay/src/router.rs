//! axum routers — app (WebSocket + REST), health (K8s probes), metrics (Prometheus).

mod route_policy;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{FromRequest, MatchedPath, State, WebSocketUpgrade},
    http::{HeaderMap, Request, StatusCode},
    middleware::{self, Next},
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
use crate::authorization_runtime::ProviderFreeRuntimeMode;
use crate::connection::handle_connection;
use crate::metrics::track_metrics;
use crate::nip11::{nip11_document, relay_info_handler};
use crate::state::AppState;

struct DatabaseTrustedProxyReplay<'a>(&'a buzz_db::Db);

impl buzz_auth::TrustedProxyNonceReplayReader for DatabaseTrustedProxyReplay<'_> {
    fn is_committed<'a>(
        &'a self,
        authorization_domain: buzz_core::CommunityId,
        claim: &'a buzz_auth::TrustedProxyNonceClaim,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<bool, buzz_auth::TrustedProxyReplayReadError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.0
                .trusted_proxy_nonce_is_committed(authorization_domain, claim)
                .await
                .map_err(|_| buzz_auth::TrustedProxyReplayReadError)
        })
    }
}

/// Build the axum [`Router`] with all relay routes, middleware, and CORS configuration.
///
/// Pure Nostr protocol: WebSocket (NIP-01), HTTP bridge (NIP-98), media (Blossom),
/// git (smart HTTP), NIP-05, and health probes.
pub fn build_router(state: Arc<AppState>) -> Router {
    route_policy::assert_startup_inventory();
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
        // Every registered route must be present in the centralized policy
        // inventory. When the gate is enabled, an unclassified route fails
        // before its handler can perform reads or writes.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_nip_fi_route_inventory,
        ))
        .layer(middleware::from_fn(attach_machine_error_code))
        .layer(middleware::from_fn(track_metrics))
        .layer(http_trace_layer())
        .layer(build_cors_layer(&state.config.cors_origins))
}

async fn attach_machine_error_code(request: Request<Body>, next: Next) -> axum::response::Response {
    let mut response = next.run(request).await;
    if response.status().is_client_error() || response.status().is_server_error() {
        let code = crate::api::ApiErrorCode::for_status(response.status()).as_str();
        response.headers_mut().insert(
            axum::http::HeaderName::from_static("x-buzz-error-code"),
            axum::http::HeaderValue::from_static(code),
        );
    }
    response
}

async fn enforce_nip_fi_route_inventory(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> axum::response::Response {
    enforce_route_inventory_for_runtime(
        state.authorization_runtime.mode(),
        state.authorization_runtime.is_ready(),
        request,
        next,
    )
    .await
}

#[cfg(test)]
async fn enforce_route_inventory_for_mode(
    mode: ProviderFreeRuntimeMode,
    request: Request<Body>,
    next: Next,
) -> axum::response::Response {
    enforce_route_inventory_for_runtime(mode, true, request, next).await
}

async fn enforce_route_inventory_for_runtime(
    mode: ProviderFreeRuntimeMode,
    runtime_ready: bool,
    request: Request<Body>,
    next: Next,
) -> axum::response::Response {
    if mode == ProviderFreeRuntimeMode::Off {
        return next.run(request).await;
    }
    let matched_path = request.extensions().get::<MatchedPath>();
    let policy = matched_path
        .and_then(|path| route_policy::classify_matched_route(request.method(), path.as_str()));
    if let Some(policy) = policy {
        if matches!(
            policy,
            route_policy::CorporateIdentityRoutePolicy::Exempt(_)
        ) {
            return next.run(request).await;
        }
        return match mode {
            ProviderFreeRuntimeMode::Off => next.run(request).await,
            ProviderFreeRuntimeMode::Enforce if runtime_ready => next.run(request).await,
            ProviderFreeRuntimeMode::Enforce => (
                StatusCode::SERVICE_UNAVAILABLE,
                "protected route unavailable: canonical authority is not ready",
            )
                .into_response(),
            ProviderFreeRuntimeMode::DenyProtected => (
                StatusCode::FORBIDDEN,
                "protected route denied by emergency policy",
            )
                .into_response(),
        };
    }
    if matched_path.is_some_and(|path| route_policy::is_known_matched_path(path.as_str())) {
        // Axum's method fallback also runs route layers. Return its semantic
        // equivalent directly, while still preventing an accidentally added
        // unclassified method handler from executing.
        let allow = matched_path
            .and_then(|path| route_policy::allowed_methods(path.as_str()))
            .unwrap_or_default();
        return axum::response::Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header(axum::http::header::ALLOW, allow)
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::METHOD_NOT_ALLOWED.into_response());
    }
    tracing::error!(
        method = %request.method(),
        matched_path = matched_path.map(|path| path.as_str()).unwrap_or("<missing>"),
        "rejecting route missing provider-free authorization classification"
    );
    crate::api::coded_api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        crate::api::ApiErrorCode::DependencyUnavailable,
        "route unavailable: authorization policy is not configured",
    )
    .into_response()
}

fn http_trace_layer() -> TraceLayer<HttpMakeClassifier, fn(&Request<Body>) -> tracing::Span> {
    TraceLayer::new_for_http().make_span_with(make_http_span as fn(&Request<Body>) -> tracing::Span)
}

fn make_http_span(request: &Request<Body>) -> tracing::Span {
    let authorization_policy = request
        .extensions()
        .get::<MatchedPath>()
        .and_then(|path| route_policy::classify_matched_route(request.method(), path.as_str()))
        .map(route_policy::CorporateIdentityRoutePolicy::trace_label)
        .unwrap_or("unclassified");
    tracing::info_span!(
        target: "buzz_relay",
        "http.request",
        otel.kind = "server",
        http.request.method = %request.method(),
        buzz.nip_fi.route_policy = authorization_policy,
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
    let corporate_identity_jwt = if state.config.nip_fi_mode == buzz_auth::NipFiMode::Enforce {
        None
    } else {
        crate::corporate_identity::identity_jwt_from_headers(
            &headers,
            &state.config.corporate_identity,
        )
    };

    let canonical_transport_evidence = if state.config.nip_fi_mode == buzz_auth::NipFiMode::Enforce
    {
        let verifier = match state.authorization_runtime.trusted_proxy_verifier() {
            Ok(verifier) => verifier,
            Err(_) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "relay: canonical transport authority is unavailable",
                )
                    .into_response();
            }
        };
        let path_and_query = req
            .uri()
            .path_and_query()
            .map_or("/", axum::http::uri::PathAndQuery::as_str);
        let request = match buzz_auth::TrustedProxyRequest::from_server_request(
            tenant.community(),
            buzz_auth::ProofTransport::Nip42,
            req.method().as_str(),
            raw_host,
            path_and_query,
            &[],
        ) {
            Ok(request) => request,
            Err(_) => return StatusCode::FORBIDDEN.into_response(),
        };
        let mut fields = Vec::new();
        for name in headers.keys() {
            for value in headers.get_all(name).iter() {
                fields.push(buzz_auth::HttpHeaderField::new(
                    name.as_str(),
                    value.as_bytes(),
                ));
            }
        }
        match verifier
            .verify(
                &fields,
                &request,
                chrono::Utc::now(),
                &DatabaseTrustedProxyReplay(&state.db),
            )
            .await
        {
            Ok(evidence) if evidence.authenticated_client_peer().is_some() => Some(evidence),
            Ok(_) => return StatusCode::FORBIDDEN.into_response(),
            Err(buzz_auth::TrustedProxyError::ReplayUnavailable) => {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            Err(_) => return StatusCode::FORBIDDEN.into_response(),
        }
    } else {
        None
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
                .on_upgrade(move |socket| {
                    handle_connection(
                        socket,
                        state,
                        tenant,
                        corporate_identity_jwt,
                        canonical_transport_evidence,
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

/// Readiness probe — checks shutdown, authorization, Postgres, and Redis.
async fn readiness_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use std::time::Duration;

    if state.shutting_down.load(Ordering::Relaxed) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "shutting_down"})),
        )
            .into_response();
    }

    if !state.authorization_runtime.is_ready() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "not_ready",
                "authorization_runtime": "unavailable"
            })),
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
        "authorization_runtime": {
            "mode": format!("{:?}", state.authorization_runtime.mode()),
            "ready": state.authorization_runtime.is_ready(),
        },
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
    use axum::{
        routing::{get, post},
        Router,
    };
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

    async fn enforce_provider_free_inventory(
        request: Request<Body>,
        next: Next,
    ) -> axum::response::Response {
        enforce_route_inventory_for_mode(ProviderFreeRuntimeMode::Enforce, request, next).await
    }

    async fn off_provider_free_inventory(
        request: Request<Body>,
        next: Next,
    ) -> axum::response::Response {
        enforce_route_inventory_for_mode(ProviderFreeRuntimeMode::Off, request, next).await
    }

    async fn deny_provider_free_inventory(
        request: Request<Body>,
        next: Next,
    ) -> axum::response::Response {
        enforce_route_inventory_for_mode(ProviderFreeRuntimeMode::DenyProtected, request, next)
            .await
    }

    async fn unavailable_provider_free_inventory(
        request: Request<Body>,
        next: Next,
    ) -> axum::response::Response {
        enforce_route_inventory_for_runtime(ProviderFreeRuntimeMode::Enforce, false, request, next)
            .await
    }

    #[tokio::test]
    async fn healthy_enforce_reaches_handlers_and_preserves_public_exemptions() {
        let app = Router::new()
            .route("/events", post(|| async { StatusCode::OK }))
            .route("/api/invites", post(|| async { StatusCode::OK }))
            .route("/operator/communities", get(|| async { StatusCode::OK }))
            .route("/api/join-policy", get(|| async { StatusCode::OK }))
            .route("/new-unclassified-route", get(|| async { StatusCode::OK }))
            .route_layer(middleware::from_fn(enforce_provider_free_inventory));

        let event = app
            .clone()
            .oneshot(Request::post("/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(event.status(), StatusCode::OK);

        let relay_invite = app
            .clone()
            .oneshot(Request::post("/api/invites").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(relay_invite.status(), StatusCode::OK);

        let operator = app
            .clone()
            .oneshot(
                Request::get("/operator/communities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(operator.status(), StatusCode::OK);

        let public_policy = app
            .clone()
            .oneshot(
                Request::get("/api/join-policy")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(public_policy.status(), StatusCode::OK);

        let unsupported = app
            .clone()
            .oneshot(Request::delete("/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unsupported.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            unsupported.headers().get(axum::http::header::ALLOW),
            Some(&axum::http::HeaderValue::from_static("POST"))
        );

        let unclassified = app
            .oneshot(
                Request::get("/new-unclassified-route")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unclassified.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn off_mode_preserves_protected_and_unclassified_handler_reachability() {
        let app = Router::new()
            .route("/events", post(|| async { StatusCode::OK }))
            .route("/new-unclassified-route", get(|| async { StatusCode::OK }))
            .route_layer(middleware::from_fn(off_provider_free_inventory));
        let protected = app
            .clone()
            .oneshot(Request::post("/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(protected.status(), StatusCode::OK);
        let unclassified = app
            .oneshot(
                Request::get("/new-unclassified-route")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unclassified.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn deny_protected_and_missing_provider_close_before_handlers() {
        let deny = Router::new()
            .route("/events", post(|| async { StatusCode::OK }))
            .route("/api/join-policy", get(|| async { StatusCode::OK }))
            .route_layer(middleware::from_fn(deny_provider_free_inventory));
        let protected = deny
            .clone()
            .oneshot(Request::post("/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(protected.status(), StatusCode::FORBIDDEN);
        let exempt = deny
            .oneshot(
                Request::get("/api/join-policy")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(exempt.status(), StatusCode::OK);

        let unavailable = Router::new()
            .route("/events", post(|| async { StatusCode::OK }))
            .route("/api/join-policy", get(|| async { StatusCode::OK }))
            .route_layer(middleware::from_fn(unavailable_provider_free_inventory));
        let protected = unavailable
            .clone()
            .oneshot(Request::post("/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(protected.status(), StatusCode::SERVICE_UNAVAILABLE);
        let exempt = unavailable
            .oneshot(
                Request::get("/api/join-policy")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(exempt.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn every_http_error_status_carries_the_stable_machine_code() {
        let app = Router::new()
            .route(
                "/status/{code}",
                get(
                    |axum::extract::Path(code): axum::extract::Path<u16>| async move {
                        StatusCode::from_u16(code).expect("valid test status")
                    },
                ),
            )
            .layer(middleware::from_fn(attach_machine_error_code));

        for (status, expected) in [
            (StatusCode::BAD_REQUEST, "invalid_request"),
            (StatusCode::UNAUTHORIZED, "authentication_required"),
            (StatusCode::FORBIDDEN, "authorization_denied"),
            (StatusCode::NOT_FOUND, "resource_not_found"),
            (StatusCode::CONFLICT, "conflict"),
            (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            (StatusCode::SERVICE_UNAVAILABLE, "dependency_unavailable"),
            (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::get(format!("/status/{}", status.as_u16()))
                        .body(Body::empty())
                        .expect("status request"),
                )
                .await
                .expect("status response");
            assert_eq!(response.status(), status);
            assert_eq!(
                response
                    .headers()
                    .get("x-buzz-error-code")
                    .and_then(|value| value.to_str().ok()),
                Some(expected),
                "{status}"
            );
        }

        let success = app
            .oneshot(
                Request::get("/status/200")
                    .body(Body::empty())
                    .expect("success request"),
            )
            .await
            .expect("success response");
        assert!(success.headers().get("x-buzz-error-code").is_none());
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
