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
use crate::nip_fi_http::http_denial;
use crate::readiness::{self, ReadinessEvaluation, ReadinessReason};
use crate::state::AppState;

// ── NIP-FI fail-closed assertion guard ───────────────────────────────────────
//
// ## Purpose
//
// This middleware is the crypto backstop for NIP-FI route classification.
// It runs *over the entire merged router*: in Enforce or DenyProtected mode,
// any request whose path does not start with a prefix in
// `NIP_FI_EXEMPT_PREFIXES` must carry the
// `Nostr-Federated-Identity: Bearer …` assertion header with a
// cryptographically valid signature — or it is denied before reaching the
// handler.
//
// The handler-level admission authority is `admit_nip_fi_http_on_state` in
// `nip_fi_http.rs`.  Protected handlers call it with a NIP-98 extraction
// closure and it delegates to `admit_nip_fi_http`, which runs NIP-98
// extraction → assertion verify → pairing → deny-map (Enforce) and alone
// constructs a `NipFiAdmission`.  The private constructor does not force a
// handler to make the call.
//
// What is guaranteed: in Enforce, this guard rejects a missing or invalid
// assertion on every non-exempt route; in DenyProtected it returns 503
// without verifying; in Off it is transparent.  Key pairing
// (`asserted_key == proven_pubkey`) and the deny map run only in handlers
// that call `admit_nip_fi_http_on_state`.  In Enforce, a handler that omits
// the call and does its own NIP-98 is still subject to the assertion guard,
// but a request with a valid assertion passes without pairing or a deny check.
//
// ## Adding a new route
//
// * **Protected (NIP-98-authenticated):** call `admit_nip_fi_http_on_state`
//   with a NIP-98 extraction closure.  No action needed here.
//
// * **Public / exempt (no NIP-FI requirement):** add the path or prefix to
//   `NIP_FI_EXEMPT_PREFIXES` below.  Failure to do so will deny the route in
//   Enforce mode, which is intentional: the default is DENY; public status is
//   explicit.
//
// ## Relationship to Off mode
//
// When `NipFiMode::Off` the guard is fully transparent — no request is
// touched.  [FI-INV-15]
//
// ## What this guard checks (and does NOT check)
//
// The guard performs the full offline assertion verification (transport
// extraction + JWT signature + issuer + expiry + claims).  This means:
//
//   • Absent header                         → 401 MissingEvidence
//   • Junk / non-Bearer value               → 403 EvidenceRejected
//   • Repeated / comma-combined fields      → 403 EvidenceRejected
//   • Structurally malformed / bad sig      → 403 EvidenceRejected
//   • Unknown issuer / expired / bad claims → 403 EvidenceRejected
//   • No verifier yet (startup race)        → 503 AuthorizationUnavailable
//   • Cryptographically valid assertion     → forward to handler
//
// The guard does NOT check key pairing or deny-map: those require the NIP-98
// `proven_pubkey` from each handler's closure, which is not available in
// middleware.  `admit_nip_fi_http_on_state` performs the full sequence.
//
// [FI-TRACE-AUTHORITY-UNIFORM] Both the guard and `admit_nip_fi_http_on_state`
// delegate to `nip_fi_http.rs`; the guard fires first.

/// Path prefixes that are exempt from NIP-FI assertion enforcement.
///
/// Every route in this relay is NIP-FI-protected by default.  Routes that
/// should NOT require the `Nostr-Federated-Identity` header in Enforce mode
/// MUST appear in this list; omission means the guard denies the route.
///
/// **Matching rules:**
/// - Entries ending with `/` match any path with that prefix (subtree match).
/// - All other entries match exactly (the request path must equal the entry
///   or start with the entry followed by `/`, `?`, or `#`).
///
/// When adding a new public or pre-auth route, add its path or prefix here
/// and include the NIP-FI classification comment in `build_router`.
const NIP_FI_EXEMPT_PREFIXES: &[&str] = &[
    // WebSocket upgrade + NIP-11 relay info (public; WS-NIP-FI governs WS)
    "/",
    // NIP-11 relay info — exact path
    "/info",
    // NIP-05 — exact path
    "/.well-known/nostr.json",
    // K8s / health probes (no auth) — exact paths
    "/health",
    "/_liveness",
    "/_readiness",
    // Pre-membership enrollment door — identity not yet issued
    "/api/invites/claim",
    // Pre-membership policy gate — no NIP-98 principal yet
    "/api/invites/accept-policy",
    // Public policy documents — exact path + subtree
    "/api/join-policy",
    // Webhook trigger — secret-header auth; subtree for /hooks/{id}
    "/hooks/",
    // Huddle audio WebSocket — WS-NIP-FI governs WebSocket; subtree
    "/huddle/",
    // Testbed-only mesh probe — no auth; subtree
    "/_mesh/",
    // Operator admin plane — keypair-in-config auth; subtree
    "/operator/",
    // Admin SPA backend — operator-credential gated; subtree
    "/api/admin/",
    // Static assets served by the SPA fallback; subtree
    "/assets/",
    "/favicon.svg",
    // Invite landing page (SPA) — subtree
    "/invite/",
    // Git web GUI (SPA) — exact + subtree
    "/repos",
    // Internal HMAC/localhost control-plane endpoint for the pre-receive hook.
    // Already protected by `require_localhost` middleware + signed operation
    // payload; does not carry a NIP-FI assertion.  Listed by exact path —
    // sub-paths (if any) are equally harmless since no routes exist there.
    "/internal/git/policy",
];

/// Middleware: full offline assertion guard for NIP-FI protected paths.
///
/// Fires before any handler.  In Enforce mode, if the request path is not
/// covered by [`NIP_FI_EXEMPT_PREFIXES`] the guard performs the full offline
/// NIP-FI assertion verification (transport extraction + JWT signature +
/// issuer + expiry + claims) via the relay's `FederatedAssertionVerifier`:
///
/// - Absent header               → 401 `authentication required\n`
/// - Junk / non-Bearer value     → 403 `evidence rejected\n`
/// - Repeated / comma-combined   → 403 `evidence rejected\n`
/// - Bad signature / claims      → 403 `evidence rejected\n`
/// - No verifier (startup race)  → 503 `authorization unavailable\n`
/// - Cryptographically valid     → forward to handler
///
/// A "forgotten gate" handler — one that omits its own
/// `admit_nip_fi_http_on_state` call — cannot admit with an invalidly signed
/// assertion because the guard rejects it here before the handler fires.
/// Only a cryptographically verified assertion reaches the handler; the
/// handler then performs the key pairing and deny-map checks via
/// `admit_nip_fi_http_on_state`.
///
/// In Off mode the middleware is fully transparent.
async fn nip_fi_assertion_guard(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: middleware::Next,
) -> axum::response::Response {
    use crate::nip_fi_http::extract_bearer_token;
    use buzz_auth::NipFiMode;

    // Off mode: fully transparent. [FI-INV-15]
    if matches!(state.config.nip_fi.mode, NipFiMode::Off) {
        return next.run(request).await;
    }

    let path = request.uri().path();

    // Exempt paths bypass the assertion-token check.
    let exempt = NIP_FI_EXEMPT_PREFIXES.iter().any(|pattern| {
        if *pattern == "/" {
            // Exact root match only.
            return path == "/";
        }
        if pattern.ends_with('/') {
            // Subtree match: path must start with this prefix.
            return path.starts_with(pattern);
        }
        // Exact-or-subtree match: path equals the pattern, or path starts
        // with the pattern followed by a path separator or query character.
        // This prevents "/info" from matching "/info-extra".
        if path == *pattern {
            return true;
        }
        if let Some(rest) = path.strip_prefix(pattern) {
            return rest.starts_with('/') || rest.starts_with('?') || rest.starts_with('#');
        }
        false
    });

    if exempt {
        return next.run(request).await;
    }

    // Admin SPA document routes are exempt when the request is on the admin
    // host.  The admin SPA serves its own documents at bare paths (`/reports`,
    // `/reports/<id>`, `/feedback`) — the browser navigates there directly.
    // These paths carry no NIP-FI-protected tenant data; the actual data calls
    // go to `/api/admin/v1/...` (already exempt via the `/api/admin/` prefix).
    //
    // The exemption is host-qualified: `/reports` on a tenant host is NOT
    // exempt and stays protected.  Off mode is already handled above.
    // [FI-TRACE-AUTHORITY-UNIFORM]
    if is_admin_spa_path(path) && api::admin::is_admin_host(&state, request.headers()) {
        return next.run(request).await;
    }

    // Non-exempt path in Enforce or DenyProtected mode.
    //
    // DenyProtected: unconditional 503 regardless of assertion presence.
    // (`admit_nip_fi_http_on_state` also does this; the guard is the backstop.)
    if matches!(state.config.nip_fi.mode, NipFiMode::DenyProtected) {
        return http_denial(buzz_auth::DenialClass::AuthorizationUnavailable);
    }

    // Enforce mode: full offline assertion verification.
    //
    // Step 1 — transport: extract the Bearer token.  Rejects absent, junk,
    // repeated, comma-combined, empty, and whitespace-containing values.
    // [FI-TRACE-TRANSPORT-CLOSED]
    let token = match extract_bearer_token(request.headers()) {
        Ok(t) => t,
        Err(class) => return http_denial(class),
    };

    // Step 2 — cryptographic: verify signature, issuer, expiry, and claims.
    // A forgotten-gate handler that omits `admit_nip_fi_http_on_state` can
    // only be reached with a cryptographically valid assertion.  Key pairing
    // and deny-map are performed by `admit_nip_fi_http_on_state` in the
    // handler, not here.  [FI-TRACE-AUTHORITY-UNIFORM]
    let verifier = match state.nip_fi_verifier.as_deref() {
        Some(v) => v,
        None => {
            // Verifier not yet constructed (startup race); fail closed.
            return http_denial(buzz_auth::DenialClass::AuthorizationUnavailable);
        }
    };
    match verifier.verify_assertion(token) {
        Ok(_) => next.run(request).await,
        Err(e) => http_denial(e.denial_class()),
    }
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
        .layer(middleware::from_fn_with_state(
            state.clone(),
            nip_fi_assertion_guard,
        ))
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

    // NIP-FI assertion gate at WebSocket upgrade.
    //
    // Strategy: belt-and-suspenders. Two fire-points cover the two ways an
    // HTTP request can become a WebSocket upgrade request on this handler:
    //
    // HTTP/1.1 path (RFC 6455, currently the only live WebSocket path):
    // detected by the `Upgrade: websocket` + `Connection: Upgrade` header pair.
    // The gate fires BEFORE `WebSocketUpgrade::from_request` so denial is
    // returned on the raw HTTP connection. This also keeps the gate independently
    // testable via tower `oneshot` (which provides no real hyper `OnUpgrade`
    // extension and would cause the extractor to return
    // `ConnectionNotUpgradable`).
    //
    // HTTP/2 extended-CONNECT (latent — workspace Axum does not enable
    // `http2`; the `/` route uses `get()` and Axum requires CONNECT routing
    // for h2 WebSockets): not currently reachable. The gate inside `Ok(ws)`
    // below is structural hardening for when `http2` is enabled. [F3-H2-GATE]
    //
    // Together these two fire-points ensure that every shape the extractor
    // accepts is also gated — no hand-rolled predicate can diverge from the
    // extractor's accepted shapes when `http2` is eventually enabled.
    //
    // Zero DB cost invariant: both fire-points run before `bind_community`,
    // so denied upgrades pay zero DB cost [FI-TRACE-TRANSPORT-CLOSED], and
    // tests that assert 401/503 are not pre-empted by a 404 from an unseeded
    // DB — the gate exercises its own seam without coupling to host-resolution
    // fixture state.
    //
    // Keying on the header pair (not on `Accept`) means an HTML Accept header
    // on a real WS upgrade is still gated correctly.
    let nip_fi_assertion = {
        let is_h1_ws_upgrade = headers
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
        if is_h1_ws_upgrade {
            use crate::nip_fi_upgrade::{check_nip_fi_at_upgrade, NipFiUpgradeOutcome};
            let mode = state.config.nip_fi.mode;
            let verifier = state.nip_fi_verifier.as_deref();
            match check_nip_fi_at_upgrade(&headers, verifier, mode) {
                NipFiUpgradeOutcome::NotRequired => None,
                NipFiUpgradeOutcome::Admitted(assertion) => Some(assertion),
                NipFiUpgradeOutcome::Denied(resp) => return resp.into_response(),
            }
        } else {
            // Not an HTTP/1.1 WS upgrade — could be an HTTP/2 extended-CONNECT,
            // a NIP-11 request, or a plain browser GET. Do not gate here; the
            // `Ok(ws)` arm below gates any extractor-accepted h2 upgrade. [F3-H2-GATE]
            None
        }
    };

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
            // [F3-H2-GATE] Structural hardening for HTTP/2 extended-CONNECT
            // WebSocket upgrades. H2 CONNECT is currently latent (workspace
            // Axum does not enable `http2` and the route uses `get()` rather
            // than CONNECT routing), but the gate here future-proofs against
            // enabling h2: if the extractor ever accepts an h2 shape that the
            // pre-extractor predicate missed (no `Upgrade` header on CONNECT),
            // the gate fires here instead of admitting the upgrade silently.
            // For HTTP/1.1 requests, `nip_fi_assertion` was already set above
            // and this block is unreachable (the h1 denial is returned before
            // we get here).
            let nip_fi_assertion = if nip_fi_assertion.is_none() {
                // Only re-check if the pre-extractor gate did not fire (h2 path).
                use crate::nip_fi_upgrade::{check_nip_fi_at_upgrade, NipFiUpgradeOutcome};
                let mode = state.config.nip_fi.mode;
                let verifier = state.nip_fi_verifier.as_deref();
                match check_nip_fi_at_upgrade(&headers, verifier, mode) {
                    NipFiUpgradeOutcome::NotRequired => None,
                    NipFiUpgradeOutcome::Admitted(assertion) => Some(assertion),
                    NipFiUpgradeOutcome::Denied(resp) => return resp.into_response(),
                }
            } else {
                nip_fi_assertion
            };

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
        let mut config = crate::config::Config::for_test(); // [FI-TRACE-ENV-RACE]
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
        let mut config = crate::config::Config::for_test(); // [FI-TRACE-ENV-RACE]
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

    /// Write a distinct admin bundle with a unique sentinel so tests can
    /// assert the exact expected bytes and distinguish admin from public HTML.
    fn write_admin_bundle(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("assets")).expect("assets dir");
        std::fs::write(
            dir.join("index.html"),
            "<!doctype html><html data-bundle=\"admin\"></html>",
        )
        .expect("admin index.html");
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
    // production: deleting either gate call (the pre-extractor h1 block in
    // `nip11_or_ws_handler`, or at the top of `ws_audio_handler` in
    // `audio/handler.rs`) causes the request to proceed past the pre-101 check
    // and receive a 404 (tenant not found) instead of the expected denial,
    // turning these tests red.
    //
    // ## F3 structural proof
    //
    // The NIP-FI gate uses a belt-and-suspenders approach. The HTTP/1.1
    // path is the only currently live WebSocket upgrade shape (workspace
    // Axum does not enable `http2`; the route uses `get()` not CONNECT routing):
    //
    // HTTP/1.1 WebSocket (RFC 6455, currently live): the gate fires BEFORE
    // `WebSocketUpgrade::from_request` using the `Upgrade: websocket` +
    // `Connection: Upgrade` header predicate. These tests drive this path via
    // tower `oneshot` — `oneshot` provides no real hyper `OnUpgrade` extension
    // so the extractor would return `ConnectionNotUpgradable`; the pre-extractor
    // gate catches the denial first and returns it before the extractor runs.
    //
    // HTTP/2 extended-CONNECT (latent, future-proofing): Axum's `http2`
    // feature is NOT currently enabled (workspace `axum = { features = ["ws",
    // "macros"] }` — no `http2`). The `[F3-H2-GATE]` backstop inside `Ok(ws)`
    // is structural hardening: if `http2` is ever enabled, any h2 CONNECT that
    // the extractor accepts but the pre-extractor predicate misses (no `Upgrade`
    // header) is caught at the backstop. A live integration test for h2 CONNECT
    // is not provided because the path is currently latent.
    //
    // Mutation evidence:
    //   A) Delete the pre-extractor gate call in `nip11_or_ws_handler` → root
    //      request returns 404 (no community) instead of 401/503 → assert_eq
    //      panics.
    //   B) Delete the [F3-H2-GATE] backstop in the `Ok(ws)` arm → h2 extended-
    //      CONNECT upgrades would bypass the gate when `http2` is eventually
    //      enabled; h1 tests still pass but the latent path loses its safety net.
    //   C) Delete the gate call in `ws_audio_handler` → audio request returns
    //      404 (no community) instead of 401/503 → assert_eq panics.
    //   D) Switch `Enforce` to `Off` in the test state → both ingresses skip
    //      the gate and return 404 (no community) → status assertions panic.

    /// Build AppState with NIP-FI enforce mode and no verifier (simulates
    /// startup with no JWKS yet warmed). The verifier is `None` because
    /// `jwks_configs` is empty and `ProductionJwksSource::new` returns `None`
    /// for an empty list; the mode field is set directly so no env is needed.
    ///
    /// The NIP-FI gate fires in the pre-extractor h1 block (before
    /// `bind_community`), so these tests exercise the gate seam independently
    /// of DB / host-resolution state. The lazy PG pool is kept so
    /// `AppState::new` compiles; it is never queried by any of these router
    /// tests.
    async fn nip_fi_enforce_state() -> Arc<AppState> {
        nip_fi_state(buzz_auth::NipFiMode::Enforce).await
    }

    /// Off-mode twin of [`nip_fi_enforce_state`]: the mode is set directly on
    /// `config.nip_fi`, so no env is involved.
    async fn nip_fi_off_state() -> Arc<AppState> {
        nip_fi_state(buzz_auth::NipFiMode::Off).await
    }

    async fn nip_fi_state(mode: buzz_auth::NipFiMode) -> Arc<AppState> {
        use crate::nip_fi_config::NipFiRelayConfig;
        use buzz_auth::IssuerRegistry;

        // Fix 5: use Config::for_test() which holds NIP_FI_ENV_LOCK internally,
        // so this fixture never races nip_fi_config's own tests. [FI-TRACE-ENV-RACE]
        let mut config = crate::config::Config::for_test();
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        // No issuers configured — the verifier is None (no JWKS source). In
        // Enforce that is the startup-race condition that must return 503 for
        // a token-carrying request.
        config.nip_fi = NipFiRelayConfig {
            mode,
            registry: IssuerRegistry::new(),
            jwks_configs: vec![],
            max_connection_lifetime_secs: 3600,
        };

        // Unreachable database: port 1 refuses every connection, so each
        // request that reaches `bind_community` fails the same way (generic
        // 404) regardless of local database contents or host load. sqlx
        // retries refused connects until the acquire timeout, so keep it short.
        config.database_url = "postgres://buzz:buzz_dev@127.0.0.1:1/buzz".to_string();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(100))
            .connect_lazy(&config.database_url)
            .expect("lazy pg pool");
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
            .status()
    }

    /// Off mode reads no identity header: an upgrade with no header and one
    /// with a malformed header get the same status, outside {401, 403, 503}.
    /// This proves the NIP-FI gate is bypassed, not that the upgrade succeeds:
    /// without a database the request can stop later (e.g. tenant lookup 404).
    async fn assert_off_mode_ignores_header(path: &str, malformed: bool) {
        let absent = nip_fi_gate_status(nip_fi_off_state().await, path, None, None).await;
        let status = if malformed {
            nip_fi_gate_status(
                nip_fi_off_state().await,
                path,
                Some("Nostr-Federated-Identity"),
                Some("Basic not-a-bearer-token"),
            )
            .await
        } else {
            absent
        };
        for s in [absent, status] {
            assert!(
                !matches!(s.as_u16(), 401 | 403 | 503),
                "Off mode must not gate {path} (malformed={malformed}); got {s}"
            );
        }
        assert_eq!(status, absent, "Off mode must ignore the header on {path}");
    }

    #[tokio::test]
    async fn nip_fi_off_root_passes_without_header() {
        assert_off_mode_ignores_header("/", false).await;
    }

    #[tokio::test]
    async fn nip_fi_off_root_ignores_malformed_header() {
        assert_off_mode_ignores_header("/", true).await;
    }

    #[tokio::test]
    async fn nip_fi_off_audio_passes_without_header() {
        let path = format!("/huddle/{}/audio", uuid::Uuid::new_v4());
        assert_off_mode_ignores_header(&path, false).await;
    }

    #[tokio::test]
    async fn nip_fi_off_audio_ignores_malformed_header() {
        let path = format!("/huddle/{}/audio", uuid::Uuid::new_v4());
        assert_off_mode_ignores_header(&path, true).await;
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
    // Because the gate runs before bind_community, the WS-upgrade 401/503 tests
    // above are DB-free. Plain-GET requests, however, do reach bind_community
    // (the gate's non-upgrade else-branch skips the gate and falls through).
    // With an unseeded lazy pool, bind_community returns 404 — but that is NOT
    // a gate denial. These tests assert that the response is neither 401 nor 503
    // (gate denial codes), which holds regardless of host resolution state.
    //
    // Fix 6: corrected the DB-free comment (bind_community is reached by plain
    // GETs; only WS-upgrade requests pay zero DB cost via the pre-gate path).
    //
    // Mutation evidence:
    //   A) Move the NIP-FI gate to fire on plain GETs too → response becomes
    //      401/503 → assertion `status != 401 && status != 503` panics.
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
    async fn nip_fi_enforce_plain_get_not_gated_401_or_503() {
        let state = nip_fi_enforce_state().await;
        // A plain GET / without WS upgrade headers is not a WebSocket upgrade.
        // In enforce mode the NIP-FI gate must NOT intercept it — the response
        // must not be a gate denial (401/503). It may be a 404 from bind_community
        // (unseeded host) or 200 (NIP-11) with a seeded host; the gate invariant
        // holds either way.
        let status = nip_fi_non_upgrade_status(state, "/", None).await;
        assert_ne!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "plain GET / in enforce mode must not be gated 401"
        );
        assert_ne!(
            status,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "plain GET / in enforce mode must not be gated 503"
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

    // ── F6: document fallback (postgres-only) ───────────────────────────────────
    //
    // A no-`Accept` plain GET / to a successfully mapped host must bypass the
    // NIP-FI gate, pass `bind_community`, and reach the NIP-11 document fallback
    // at `router.rs:493`. The test seeds a community, fires a plain GET with the
    // community's host, and asserts 200 + NIP-11 JSON content.
    //
    // A lazy-pool state cannot seed the community — this test belongs in the
    // isolated postgres lane so it has a real DB. It is gated `#[ignore]` so it
    // does not run in the unit-test lane where no DB is available.
    mod postgres_tests {
        use super::*;
        use std::sync::Arc;

        async fn real_db_state() -> Option<Arc<AppState>> {
            let db_url = crate::test_support::database_url();
            let pool = sqlx::PgPool::connect(&db_url).await.ok()?;

            let mut config = crate::config::Config::for_test(); // [FI-TRACE-ENV-RACE]
            config.require_relay_membership = false;
            config.redis_url = "redis://127.0.0.1:1".to_string();
            config.database_url = db_url;
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .expect("redis pool");
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .expect("pubsub manager"),
            );
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let audit = buzz_audit::AuditService::new(pool.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage =
                buzz_media::MediaStorage::new(&config.media).expect("media storage");
            let (state, _shutdown) = AppState::new(
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
            Some(Arc::new(state))
        }

        /// F6: a no-Accept plain GET to a mapped host returns 200 + NIP-11 JSON.
        ///
        /// The test seeds a community, sends a plain GET with the community's
        /// host (no Accept header), and asserts 200. This proves the no-Accept
        /// path reaches the NIP-11 document fallback (`router.rs:493`) and that
        /// the NIP-FI gate does not intercept plain GET traffic.
        ///
        /// ## Mutation oracle
        ///
        /// A) Move the document fallback behind an additional NIP-FI gate check →
        ///    plain GET is denied (401/503) → assertion panics.
        ///
        /// B) Remove `bind_community` from the router path → every plain GET
        ///    returns 404 regardless of the host → 200 assertion panics.
        ///
        /// C) Serve plain GET from a different code path (e.g., gate fires before
        ///    `bind_community`) → 401 is returned → 200 assertion panics.
        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn f6_plain_get_mapped_host_returns_nip11_200() {
            use axum::body::Body;
            use axum::http::Request;
            use tower::ServiceExt;
            use uuid::Uuid;

            let state = real_db_state()
                .await
                .expect("F6: PostgreSQL must be available — set BUZZ_TEST_DATABASE_URL or start local postgres");
            let pool = state.db.pool().clone();

            // Seed a community with a unique host.
            let community_id = Uuid::new_v4();
            let host = format!("f6-test-{}.example", community_id.simple());
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(community_id)
                .bind(&host)
                .execute(&pool)
                .await
                .expect("F6: seed community");

            // Plain GET / with the community's host — no Accept header.
            let req = Request::get("/")
                .header(axum::http::header::HOST, &host)
                .body(Body::empty())
                .expect("F6: build request");

            let response = build_router(state)
                .oneshot(req)
                .await
                .expect("F6: router response");

            assert_eq!(
                response.status(),
                axum::http::StatusCode::OK,
                "F6: plain GET to a mapped host must return 200 (NIP-11 document fallback);\n                 Mutation oracle A: gate intercepts plain GET → 401/503 → panics.\n                 Mutation oracle B: bind_community removed → 404 → panics."
            );

            // Assert the body is NIP-11 JSON (has `supported_nips` field).
            let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
                .await
                .expect("F6: read body");
            let body: serde_json::Value =
                serde_json::from_slice(&body_bytes).expect("F6: body must be valid JSON");
            assert!(
                body.get("supported_nips").is_some(),
                "F6: response body must be NIP-11 JSON with `supported_nips` field; got {body}"
            );
        }
    }

    // ── nip_fi_assertion_guard: fail-closed classification tests ─────────────
    //
    // ## What these tests prove
    //
    // `nip_fi_assertion_guard` is the crypto backstop for NIP-FI route
    // classification.  These unit tests directly verify the exempt-prefix
    // matching logic that determines whether a request is guarded or not.
    //
    // The key property: a non-exempt path with no assertion header must be
    // denied in Enforce mode, even if the handler does NOT call
    // `admit_nip_fi_http_on_state`.  This is the belt — a handler that omits
    // its gate cannot bypass the missing/invalid-assertion rejection.
    // Key pairing and the deny map are NOT covered by this guard: they run
    // only in handlers that call `admit_nip_fi_http_on_state`.
    //
    // ## Dummy-route failure-mode demonstration (for code review)
    //
    // To confirm the failure mode is dead:
    //   1. In `build_router` add a handler with no NIP-FI gate:
    //      `.route("/dummy-unclassified", get(|| async { "hello" }))`
    //      (Do NOT add "/dummy-unclassified" to NIP_FI_EXEMPT_PREFIXES.)
    //   2. Deploy with NIP-FI in Enforce mode.
    //   3. Send `GET /dummy-unclassified` with valid NIP-98 but no assertion.
    //   4. Response: 401 `authentication required\n` from the guard.
    //   5. Revert the dummy route.
    //
    // This is the mechanism the tests below exercise at the unit level.

    /// Returns true when `path` is exempt per `NIP_FI_EXEMPT_PREFIXES`.
    /// Mirrors the matching logic in `nip_fi_assertion_guard`.
    fn is_exempt(path: &str) -> bool {
        NIP_FI_EXEMPT_PREFIXES.iter().any(|pattern| {
            if *pattern == "/" {
                return path == "/";
            }
            if pattern.ends_with('/') {
                return path.starts_with(pattern);
            }
            if path == *pattern {
                return true;
            }
            if let Some(rest) = path.strip_prefix(pattern) {
                return rest.starts_with('/') || rest.starts_with('?') || rest.starts_with('#');
            }
            false
        })
    }

    // Exempt paths — guard must pass these through in Enforce mode.
    #[test]
    fn exempt_paths_are_recognized() {
        // Root exact match
        assert!(is_exempt("/"), "/ must be exempt (WS + NIP-11)");
        assert!(!is_exempt("/events"), "POST /events must NOT be exempt");
        // Exact-match entries must not bleed into adjacent paths
        assert!(
            !is_exempt("/info-extra"),
            "/info-extra must NOT match /info"
        );
        assert!(!is_exempt("/healthz"), "/healthz must NOT match /health");

        // Probes
        assert!(is_exempt("/health"));
        assert!(is_exempt("/_liveness"));
        assert!(is_exempt("/_readiness"));

        // Pre-membership
        assert!(is_exempt("/api/invites/claim"));
        assert!(is_exempt("/api/invites/accept-policy"));

        // Public docs
        assert!(is_exempt("/api/join-policy"));
        assert!(is_exempt("/api/join-policy/terms"));
        assert!(is_exempt("/api/join-policy/privacy"));

        // Webhook (prefix)
        assert!(is_exempt("/hooks/abc123"));
        assert!(!is_exempt("/hooksnot"), "/hooksnot must not match /hooks/");

        // Operator / admin subtrees
        assert!(is_exempt("/operator/communities"));
        assert!(is_exempt("/api/admin/v1/something"));

        // SPA / assets
        assert!(is_exempt("/assets/main.js"));
        assert!(is_exempt("/invite/abc"));
        assert!(is_exempt("/repos"));
        assert!(is_exempt("/repos/owner/name"));
    }

    // Protected paths — guard must deny these in Enforce mode.
    #[test]
    fn protected_paths_are_not_exempt() {
        assert!(!is_exempt("/events"), "POST /events must be protected");
        assert!(!is_exempt("/query"), "POST /query must be protected");
        assert!(!is_exempt("/count"), "POST /count must be protected");
        assert!(
            !is_exempt("/gifs/search"),
            "POST /gifs/search must be protected"
        );
        assert!(
            !is_exempt("/gifs/share"),
            "POST /gifs/share must be protected"
        );
        assert!(
            !is_exempt("/workflows/abc/runs"),
            "GET /workflows must be protected"
        );
        assert!(
            !is_exempt("/moderation/reports"),
            "GET /moderation/reports must be protected"
        );
        assert!(
            !is_exempt("/moderation/audit"),
            "GET /moderation/audit must be protected"
        );
        assert!(
            !is_exempt("/moderation/restricted"),
            "GET /moderation/restricted must be protected"
        );
        assert!(
            !is_exempt("/api/invites"),
            "POST /api/invites (mint) must be protected"
        );
        assert!(!is_exempt("/upload"), "PUT /upload must be protected");
        assert!(
            !is_exempt("/media/upload"),
            "PUT /media/upload must be protected"
        );
        assert!(
            !is_exempt("/media/deadbeef.bin"),
            "GET /media/{{sha}} must be protected"
        );
    }

    // Regression: a newly added unclassified path must NOT be exempt by default.
    // If a developer adds a route and forgets to add it to NIP_FI_EXEMPT_PREFIXES,
    // `is_exempt` returns false → the guard denies in Enforce mode.
    // This test proves that the default is DENY, not ADMIT.
    #[test]
    fn unclassified_path_is_not_exempt_by_default() {
        // A path that looks plausibly authenticated but was just added:
        assert!(
            !is_exempt("/api/new-feature/data"),
            "newly added unclassified path must default to NOT exempt; \
             if this fails, NIP_FI_EXEMPT_PREFIXES has an overly broad entry"
        );
        assert!(
            !is_exempt("/api/invites/new-endpoint"),
            "a new invite sub-path must not be exempt just because /api/invites/ exists; \
             only /api/invites/claim and /api/invites/accept-policy are explicitly exempt"
        );
    }

    // ── F6: admin SPA document paths are NOT broadly exempt ─────────────────
    //
    // Admin SPA document routes (`/reports`, `/reports/<id>`, `/feedback`) are
    // served by the SPA fallback on the admin host.  They are NOT in
    // `NIP_FI_EXEMPT_PREFIXES` — the broad exempt list would make `/reports`
    // exempt on tenant hosts too, which is unintentional.  Instead, the guard
    // exempts them conditionally via a host-qualified `is_admin_spa_path` +
    // `is_admin_host` check (see `nip_fi_assertion_guard`).
    //
    // This test proves two things:
    // 1. `is_admin_spa_path` recognises the admin document routes.
    // 2. These paths are NOT broadly exempt (no entry in NIP_FI_EXEMPT_PREFIXES)
    //    so tenant hosts remain protected.
    //
    // Mutation evidence: adding "/reports" to NIP_FI_EXEMPT_PREFIXES makes
    // `is_exempt("/reports")` return true and the assertion below panics.
    #[test]
    fn admin_spa_paths_are_not_broadly_exempt_but_are_admin_spa_paths() {
        // /reports and /feedback ARE admin SPA document paths.
        assert!(
            is_admin_spa_path("/reports"),
            "/reports must be an admin SPA path (for host-qualified exemption)"
        );
        assert!(
            is_admin_spa_path("/reports/abc-123"),
            "/reports/<id> must be an admin SPA path"
        );
        assert!(
            is_admin_spa_path("/feedback"),
            "/feedback must be an admin SPA path"
        );
        assert!(
            is_admin_spa_path("/feedback/abc"),
            "/feedback/<id> must be an admin SPA path"
        );

        // But they are NOT in NIP_FI_EXEMPT_PREFIXES (not broadly exempt).
        // The guard exempts them only when the request is on the admin host.
        assert!(
            !is_exempt("/reports"),
            "/reports must NOT be broadly exempt; exemption is host-qualified in the guard"
        );
        assert!(
            !is_exempt("/reports/abc-123"),
            "/reports/<id> must NOT be broadly exempt"
        );
        assert!(
            !is_exempt("/feedback"),
            "/feedback must NOT be broadly exempt"
        );
        assert!(
            !is_exempt("/feedback/abc"),
            "/feedback/<id> must NOT be broadly exempt"
        );
    }

    // ── F6: build_router guard navigation — host-qualified admin exemption ───
    //
    // Proves that the `is_admin_spa_path(path) && is_admin_host(...)` check in
    // `nip_fi_assertion_guard` (router.rs:215-217) does exactly what it says:
    //
    //   • `/reports` on the admin host → HTML (guard exempts it, SPA fallback serves it)
    //   • `/reports` on a tenant host → 503 (guard NOT exempted; DenyProtected denies it)
    //
    // Falsifying mutation: remove the `is_admin_spa_path(path) && ...` branch at
    // router.rs:215-217.  The admin-host request then reaches the DenyProtected
    // branch and returns 503 — the assertion below panics instead of returning
    // HTML.  The path-classification tests (`admin_spa_paths_are_not_broadly_exempt_*`)
    // would still pass because they only test the helper functions, not the guard.
    //
    // DenyProtected is used here because it denies unconditionally without
    // needing a verifier, making the test self-contained and infrastructure-free.
    #[tokio::test]
    async fn build_router_admin_spa_path_exempt_on_admin_host_denied_on_tenant_host() {
        use buzz_auth::NipFiMode;

        let admin_dir = tempfile::tempdir().expect("admin bundle dir");
        let web_dir = tempfile::tempdir().expect("public bundle dir");
        write_bundle(admin_dir.path());
        write_bundle(web_dir.path());

        // Build a DenyProtected-mode state using the same SPA helper, but with
        // the NIP-FI mode overridden after config construction.
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.web_dir = Some(web_dir.path().to_path_buf());
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Disabled,
            web_dir: Some(admin_dir.path().to_path_buf()),
        });
        config.nip_fi.mode = NipFiMode::DenyProtected;

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
        let state = Arc::new(state);

        // Admin host: /reports must be exempted (guard lets it through → SPA serves HTML).
        let admin_response = spa_response(state.clone(), "admin.example", "/reports").await;
        assert_ne!(
            admin_response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "/reports on the admin host must NOT be denied 503 in DenyProtected mode; \
             the host-qualified admin SPA exemption in nip_fi_assertion_guard must fire. \
             Falsifying mutation: remove `is_admin_spa_path(path) && is_admin_host(...)` at router.rs:215-217"
        );
        // The SPA fallback serves the index document.
        assert_eq!(
            admin_response.status(),
            axum::http::StatusCode::OK,
            "/reports on the admin host must be served as an SPA document"
        );

        // Tenant host: /reports is NOT exempt (guard denies it with 503 DenyProtected).
        let tenant_response = spa_response(state.clone(), "tenant.example", "/reports").await;
        assert_eq!(
            tenant_response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "/reports on a tenant host must be denied 503 in DenyProtected mode; \
             the guard exemption must NOT fire for non-admin hosts. \
             Falsifying mutation: remove the is_admin_host check — tenant host would \
             then match is_admin_spa_path and bypass the guard"
        );
    }

    // ── F6 (extended): complete admin SPA matrix ─────────────────────────────
    //
    // Proves that the `is_admin_spa_path(path) && is_admin_host(...)` exemption
    // covers all admin document routes (`/reports`, `/reports/<id>`, `/feedback`)
    // in both Enforce and DenyProtected modes, and that none of these are
    // accidentally exempted on tenant hosts.
    //
    // The existing `build_router_admin_spa_path_exempt_on_admin_host_denied_on_tenant_host`
    // test covers `/reports` in DenyProtected.  This test fills the matrix.
    //
    // Falsifying mutation (coverage of all paths): replacing `is_admin_spa_path`
    // with a hardcoded `/reports`-only check would cause the `/reports/<id>` and
    // `/feedback` rows to return 503 on the admin host → assertions fire.
    #[tokio::test]
    async fn build_router_admin_spa_full_matrix() {
        use buzz_auth::NipFiMode;

        // Helper: build a state with a given mode.
        // Returns (state, _admin_dir, _web_dir) — callers must keep the TempDirs
        // alive for the lifetime of the test; they are dropped at end of scope.
        async fn state_with_mode(
            mode: NipFiMode,
            admin_dir: &std::path::Path,
            web_dir: &std::path::Path,
        ) -> Arc<AppState> {
            write_admin_bundle(admin_dir);
            write_bundle(web_dir);

            let mut config = crate::config::Config::from_env().expect("default config loads");
            config.require_relay_membership = false;
            config.redis_url = "redis://127.0.0.1:1".to_string();
            config.web_dir = Some(web_dir.to_path_buf());
            config.admin = Some(crate::config::AdminConfig {
                host: "admin.matrix.example".to_string(),
                auth: crate::config::AdminAuth::Disabled,
                web_dir: Some(admin_dir.to_path_buf()),
            });
            config.nip_fi.mode = mode;

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
            let media_storage =
                buzz_media::MediaStorage::new(&config.media).expect("media storage");
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

        // Admin SPA paths to test.
        let admin_paths = ["/reports", "/reports/abc-123-id", "/feedback"];

        // ── DenyProtected matrix ──────────────────────────────────────────────
        //
        // Admin host + DenyProtected: guard exempts admin SPA paths → 200 HTML.
        // Tenant host + DenyProtected: guard NOT exempted → 503 + exact body.
        let deny_admin_dir = tempfile::tempdir().expect("deny admin bundle dir");
        let deny_web_dir = tempfile::tempdir().expect("deny public bundle dir");
        let deny_state = state_with_mode(
            NipFiMode::DenyProtected,
            deny_admin_dir.path(),
            deny_web_dir.path(),
        )
        .await;
        for path in &admin_paths {
            let admin_resp = spa_response(deny_state.clone(), "admin.matrix.example", path).await;
            let admin_status = admin_resp.status();
            let admin_ct = admin_resp
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let admin_body = axum::body::to_bytes(admin_resp.into_body(), 8192)
                .await
                .unwrap_or_default();
            assert_eq!(
                admin_status,
                axum::http::StatusCode::OK,
                "DenyProtected: {path} on admin host must be 200 (SPA exemption). \
                 Falsifying mutation: remove is_admin_spa_path || restrict to /reports only → 503"
            );
            assert_eq!(
                admin_body.as_ref(),
                b"<!doctype html><html data-bundle=\"admin\"></html>",
                "DenyProtected: {path} on admin host 200 must serve the exact admin HTML body; \
                 distinct content distinguishes admin bundle from public bundle."
            );
            assert_eq!(
                admin_ct, "text/html; charset=utf-8",
                "DenyProtected: {path} on admin host 200 Content-Type must be \
                 'text/html; charset=utf-8'; got '{admin_ct}'"
            );

            let tenant_resp = spa_response(deny_state.clone(), "tenant.matrix.example", path).await;
            let tenant_status = tenant_resp.status();
            let tenant_resp_headers = tenant_resp.headers().clone();
            let tenant_body = axum::body::to_bytes(tenant_resp.into_body(), 8192)
                .await
                .unwrap_or_default();
            assert_eq!(
                tenant_status,
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "DenyProtected: {path} on tenant host must be 503. \
                 Exemption must not apply to non-admin hosts."
            );
            assert_eq!(
                tenant_body.as_ref(),
                b"authorization unavailable\n",
                "DenyProtected: {path} on tenant host 503 must have exact body \
                 'authorization unavailable\\n' (AuthorizationUnavailable contract)."
            );
            let tenant_ct = tenant_resp_headers
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert_eq!(
                tenant_ct, "text/plain; charset=utf-8",
                "DenyProtected: {path} on tenant host 503 Content-Type must be \
                 'text/plain; charset=utf-8'; got '{tenant_ct}'"
            );
            assert!(
                tenant_resp_headers.get("www-authenticate").is_none(),
                "DenyProtected: {path} on tenant host 503 MUST NOT carry WWW-Authenticate \
                 (DenyProtected unconditionally denies; challenge absent)"
            );
        }

        // ── Enforce matrix ────────────────────────────────────────────────────
        //
        // Admin host + Enforce + no verifier (None) → NIP-FI admission guard
        // fires for non-exempt paths.  Admin SPA paths are host-qualified exempt
        // → 200 HTML.  Tenant host → 401 (missing assertion, no verifier configured)
        // with exact body "authentication required\n" and WWW-Authenticate: Nostr.
        //
        // Important: asserting only `!= 200` for the tenant case is insufficient —
        // the fallthrough public 404 path also returns non-200.  Exact 401 + header
        // + body distinguishes the NIP-FI guard from a public 404 fallback.
        let enforce_admin_dir = tempfile::tempdir().expect("enforce admin bundle dir");
        let enforce_web_dir = tempfile::tempdir().expect("enforce public bundle dir");
        let enforce_state = state_with_mode(
            NipFiMode::Enforce,
            enforce_admin_dir.path(),
            enforce_web_dir.path(),
        )
        .await;
        for path in &admin_paths {
            let admin_resp =
                spa_response(enforce_state.clone(), "admin.matrix.example", path).await;
            let admin_status = admin_resp.status();
            let admin_ct = admin_resp
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let admin_body = axum::body::to_bytes(admin_resp.into_body(), 8192)
                .await
                .unwrap_or_default();
            assert_eq!(
                admin_status,
                axum::http::StatusCode::OK,
                "Enforce: {path} on admin host must be 200 (SPA exemption active in Enforce too). \
                 Falsifying mutation: remove is_admin_spa_path exemption from Enforce branch → non-200"
            );
            assert_eq!(
                admin_body.as_ref(),
                b"<!doctype html><html data-bundle=\"admin\"></html>",
                "Enforce: {path} on admin host 200 must serve the exact admin HTML body."
            );
            assert_eq!(
                admin_ct, "text/html; charset=utf-8",
                "Enforce: {path} on admin host 200 Content-Type must be \
                 'text/html; charset=utf-8'; got '{admin_ct}'"
            );

            let tenant_resp =
                spa_response(enforce_state.clone(), "tenant.matrix.example", path).await;
            let tenant_status = tenant_resp.status();
            let tenant_headers = tenant_resp.headers().clone();
            let tenant_body = axum::body::to_bytes(tenant_resp.into_body(), 8192)
                .await
                .unwrap_or_default();
            // Must be 401, not just != 200.  A 404 fallback would also satisfy != 200
            // but would indicate the NIP-FI guard was bypassed.
            assert_eq!(
                tenant_status,
                axum::http::StatusCode::UNAUTHORIZED,
                "Enforce: {path} on tenant host must be 401 MissingEvidence (not 200, not 404). \
                 The NIP-FI guard must fire and produce an exact denial, not fall through to \
                 the public 404 route. \
                 Falsifying mutation: remove the guard call → 404 → assertion fires."
            );
            assert_eq!(
                tenant_body.as_ref(),
                b"authentication required\n",
                "Enforce: {path} on tenant host 401 must have exact body \
                 'authentication required\\n' (MissingEvidence contract)."
            );
            let www_auth = tenant_headers
                .get("WWW-Authenticate")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert_eq!(
                www_auth,
                "Nostr",
                "Enforce: {path} on tenant host 401 must have WWW-Authenticate: Nostr header. \
                 Falsifying mutation: remove challenge from MissingEvidence denial → assertion fires."
            );
            let tenant_ct = tenant_headers
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert_eq!(
                tenant_ct, "text/plain; charset=utf-8",
                "Enforce: {path} on tenant host 401 Content-Type must be \
                 'text/plain; charset=utf-8'; got '{tenant_ct}'"
            );
        }
    }

    // ── T1-IMP1: adversarial guard — junk/non-Bearer assertion is denied ──────
    //
    // Before this fix the guard called `headers.contains_key(CLIENT_ATTACHED_HEADER)`,
    // so `Nostr-Federated-Identity: junk` would pass because the header is
    // present.  After the fix the guard performs full offline assertion
    // verification (transport extraction + JWT signature + issuer + expiry):
    //
    //   • Junk / non-Bearer value      → transport extraction fails → 403
    //   • Empty Bearer token           → transport extraction fails → 403
    //   • Structurally valid token     → transport extraction passes → crypto verify → 403 if bad sig
    //
    // This test proves the transport-extraction cases.  The crypto-verification
    // case (structurally valid but bad signature) is proven by the production-
    // router test `nip_fi_guard_rejects_crypto_invalid_assertion_before_handler_fires`
    // in bridge.rs, which has a falsifying mutation: removing
    // `verifier.verify_assertion(token)` from the guard turns the expected
    // 403 into 401 (handler's NIP-98 auth fires instead).
    #[test]
    fn guard_rejects_junk_assertion_not_just_absent_header() {
        use crate::nip_fi_http::extract_bearer_token;
        use axum::http::HeaderMap;
        use buzz_auth::CLIENT_ATTACHED_HEADER;

        // Case 1: bare junk value (not Bearer-prefixed).
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            "junk".parse().expect("valid header value"),
        );
        assert!(
            extract_bearer_token(&headers).is_err(),
            "guard calls extract_bearer_token: bare 'junk' must be rejected (EvidenceRejected)"
        );

        // Case 2: valid-looking Bearer prefix but empty token.
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            "Bearer ".parse().expect("valid header value"),
        );
        assert!(
            extract_bearer_token(&headers).is_err(),
            "guard calls extract_bearer_token: 'Bearer ' with empty token must be rejected"
        );

        // Case 3: structurally valid compact JWS (three Base64url-separated dots)
        // passes transport extraction.  The guard then calls
        // `verifier.verify_assertion()` which would reject it as
        // InvalidSignatureOrClaims → EvidenceRejected (403) in the full guard.
        // This test only exercises transport extraction; the full-guard crypto
        // falsifier is in bridge.rs::nip_fi_guard_rejects_crypto_invalid_assertion_before_handler_fires.
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            "Bearer a.b.c".parse().expect("valid header value"),
        );
        assert!(
            extract_bearer_token(&headers).is_ok(),
            "structurally-valid compact JWS passes transport extraction; \
             guard then proceeds to crypto verification"
        );
    }

    // ── T1-IMP2 exemption classification ─────────────────────────────────────
    //
    // `/internal/git/policy` must be exempt so the pre-receive hook callback
    // reaches its own `require_localhost` + HMAC authorization layer in active
    // NIP-FI mode.  Only the exact path and sub-paths are exempt — the broader
    // `/internal/` subtree is NOT exempted (no catch-all entry exists).
    //
    // Matching semantics: a non-`/`-ending pattern matches exact OR sub-paths
    // (path equals pattern, or path starts with `pattern/`).  This is safe
    // because no routes exist under `/internal/git/policy/*` — any sub-path
    // passes through the guard to Axum, which returns 404.
    //
    // Falsifying mutation: remove the "/internal/git/policy" entry from
    // NIP_FI_EXEMPT_PREFIXES.  `is_exempt("/internal/git/policy")` returns
    // false, and the guard would return 401 in Enforce mode (every git push
    // would be rejected by the hook callback failing).
    #[test]
    fn internal_git_policy_is_exempt_but_internal_subtree_is_not() {
        assert!(
            is_exempt("/internal/git/policy"),
            "/internal/git/policy must be exempt: pre-receive hook calls it without \
             a NIP-FI assertion; blocking it breaks git push in Enforce/DenyProtected mode"
        );
        // No catch-all /internal/ entry exists — only the specific path is
        // listed, so unrelated /internal/* paths are not exempt.
        assert!(
            !is_exempt("/internal/"),
            "the /internal/ subtree must NOT be broadly exempt; \
             only the specific hook-callback path is exempted"
        );
        assert!(
            !is_exempt("/internal/other"),
            "/internal/other must NOT be exempt (no /internal/ subtree entry)"
        );
    }
}
