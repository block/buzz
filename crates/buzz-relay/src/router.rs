//! axum routers — app (WebSocket + REST), health (K8s probes), metrics (Prometheus).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{ConnectInfo, FromRequest, Path, State, WebSocketUpgrade},
    http::{header, uri::Authority, HeaderMap, Request, StatusCode},
    middleware,
    response::{Html, IntoResponse, Json, Response},
    routing::{get, post, put},
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
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
        // Keep invite links usable even when BUZZ_WEB_DIR is not configured.
        // Deployments with the web bundle still receive the full SPA page.
        .route("/invite/{code}", get(invite_landing_handler))
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

fn invite_relay_url(configured_relay_url: &str, headers: &HeaderMap) -> Option<String> {
    let raw_host = headers.get(header::HOST)?.to_str().ok()?.trim();
    let authority = raw_host.parse::<Authority>().ok()?;
    let scheme = if configured_relay_url.trim_start().starts_with("wss://") {
        "wss"
    } else {
        "ws"
    };
    Some(format!("{scheme}://{authority}"))
}

fn minimal_invite_landing_html(relay_url: &str, code: &str, script_nonce: &str) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("relay", relay_url)
        .append_pair("code", code)
        .finish();
    let app_link = format!("buzz://join?{query}").replace('&', "&amp;");

    const TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Join Buzz</title>
    <style>
      :root { color-scheme: light; font-family: ui-sans-serif, system-ui, sans-serif; }
      * { box-sizing: border-box; }
      body {
        align-items: center;
        background: linear-gradient(180deg, #d7d72e 0%, #d7e7f6 100%);
        display: flex;
        justify-content: center;
        margin: 0;
        min-height: 100vh;
        padding: 24px;
      }
      main {
        background: white;
        border-radius: 24px;
        box-shadow: 0 20px 60px rgb(0 0 0 / 12%);
        max-width: 480px;
        padding: 48px 32px;
        text-align: center;
        width: 100%;
      }
      h1 { font-size: 28px; margin: 0 0 12px; }
      p { color: rgb(0 0 0 / 62%); line-height: 1.5; margin: 0 0 28px; }
      a { display: block; }
      .policy {
        background: rgb(0 0 0 / 3%);
        border: 1px solid rgb(0 0 0 / 10%);
        border-radius: 12px;
        margin-bottom: 20px;
        padding: 16px;
        text-align: left;
      }
      .policy label { color: rgb(0 0 0 / 68%); display: flex; font-size: 13px; gap: 10px; line-height: 1.5; }
      .policy label + label { margin-top: 12px; }
      .policy input { flex: 0 0 auto; margin-top: 3px; }
      .policy a { color: black; display: inline; text-underline-offset: 3px; }
      .primary {
        background: black;
        border-radius: 10px;
        color: white;
        font-weight: 600;
        padding: 12px 18px;
        text-decoration: none;
      }
      .primary[aria-disabled="true"] { cursor: not-allowed; opacity: 0.45; }
      .download {
        color: black;
        font-size: 14px;
        margin-top: 18px;
        text-underline-offset: 3px;
      }
    </style>
  </head>
  <body>
    <main>
      <h1>You're invited to Buzz</h1>
      <p>Open Buzz to accept this invitation and join the community.</p>
      <section class="policy" id="join-policy" hidden>
        <label id="age-confirmation" hidden>
          <input id="age-confirmed" type="checkbox">
          <span>I am 18 years of age or older.</span>
        </label>
        <label id="agreement-confirmation" hidden>
          <input id="agreement-confirmed" type="checkbox">
          <span>
            I have reviewed and agree to the community
            <a href="/api/join-policy/terms" id="terms-link" rel="noopener noreferrer" target="_blank" hidden>Terms of Service</a><span id="policy-separator" hidden> and </span><a href="/api/join-policy/privacy" id="privacy-link" rel="noopener noreferrer" target="_blank" hidden>Privacy Policy</a>.
          </span>
        </label>
      </section>
      <a aria-disabled="true" class="primary" href="__APP_LINK__" id="accept-invite">Accept invite in Buzz</a>
      <p aria-live="polite" id="invite-status" role="status" hidden></p>
      <a class="download" href="https://github.com/block/buzz/releases/latest" rel="noreferrer">Download Buzz</a>
    </main>
    <script nonce="__SCRIPT_NONCE__">
      (() => {
        "use strict";

        const acceptLink = document.getElementById("accept-invite");
        const policyPanel = document.getElementById("join-policy");
        const ageRow = document.getElementById("age-confirmation");
        const ageCheckbox = document.getElementById("age-confirmed");
        const agreementRow = document.getElementById("agreement-confirmation");
        const agreementCheckbox = document.getElementById("agreement-confirmed");
        const termsLink = document.getElementById("terms-link");
        const privacyLink = document.getElementById("privacy-link");
        const separator = document.getElementById("policy-separator");
        const status = document.getElementById("invite-status");
        let policy;
        let opening = false;

        const updateDisabledState = () => {
          const needsAge = Boolean(policy?.age_attestation_required);
          const needsAgreement = Boolean(policy?.terms_markdown || policy?.privacy_markdown);
          const disabled =
            policy === undefined ||
            opening ||
            (needsAge && !ageCheckbox.checked) ||
            (needsAgreement && !agreementCheckbox.checked);
          acceptLink.setAttribute("aria-disabled", String(disabled));
        };

        ageCheckbox.addEventListener("change", updateDisabledState);
        agreementCheckbox.addEventListener("change", updateDisabledState);

        acceptLink.addEventListener("click", async (event) => {
          if (policy === null) return;
          event.preventDefault();
          if (policy === undefined || acceptLink.getAttribute("aria-disabled") === "true") return;

          opening = true;
          status.hidden = false;
          status.textContent = "Preparing your invitation…";
          updateDisabledState();
          try {
            const response = await fetch("/api/invites/accept-policy", {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({
                code: new URL(acceptLink.href).searchParams.get("code"),
                policy_version: policy.version,
                age_confirmed: ageCheckbox.checked,
              }),
            });
            if (!response.ok) throw new Error(`HTTP ${response.status}`);
            const payload = await response.json();
            if (typeof payload.receipt !== "string" || payload.receipt.length === 0) {
              throw new Error("Missing policy receipt");
            }
            const appUrl = new URL(acceptLink.href);
            appUrl.searchParams.set("policy_receipt", payload.receipt);
            window.location.assign(appUrl.toString());
          } catch (_error) {
            opening = false;
            status.textContent = "Could not accept the community policy. Please try again.";
            updateDisabledState();
          }
        });

        fetch("/api/join-policy")
          .then(async (response) => {
            if (!response.ok) throw new Error(`HTTP ${response.status}`);
            const payload = await response.json();
            policy = payload.policy ?? null;
            if (policy) {
              const hasTerms = Boolean(policy.terms_markdown);
              const hasPrivacy = Boolean(policy.privacy_markdown);
              policyPanel.hidden = false;
              ageRow.hidden = !policy.age_attestation_required;
              agreementRow.hidden = !(hasTerms || hasPrivacy);
              termsLink.hidden = !hasTerms;
              privacyLink.hidden = !hasPrivacy;
              separator.hidden = !(hasTerms && hasPrivacy);
            }
            updateDisabledState();
          })
          .catch(() => {
            status.hidden = false;
            status.textContent = "Could not load the community policy. Please try again.";
            updateDisabledState();
          });
      })();
    </script>
  </body>
</html>
"#;

    TEMPLATE
        .replace("__APP_LINK__", &app_link)
        .replace("__SCRIPT_NONCE__", script_nonce)
}

async fn invite_landing_handler(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> Response {
    if api::admin::is_admin_host(&state, &headers) {
        return StatusCode::NOT_FOUND.into_response();
    }

    if let Some(web_dir) = state.config.web_dir.as_ref() {
        return read_spa_index(&web_dir.join("index.html")).await;
    }

    let Some(relay_url) = invite_relay_url(&state.config.relay_url, &headers) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let nonce_bytes: [u8; 16] = rand::random();
    let script_nonce = URL_SAFE_NO_PAD.encode(nonce_bytes);
    let csp = format!(
        "default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-{script_nonce}'; \
         connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    );
    let Ok(csp) = csp.parse::<header::HeaderValue>() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = Html(minimal_invite_landing_html(
        &relay_url,
        &code,
        &script_nonce,
    ))
    .into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response_headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    response_headers.insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("no-referrer"),
    );
    response_headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    response
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

    #[test]
    fn minimal_invite_page_builds_a_safe_app_deep_link() {
        let html = minimal_invite_landing_html(
            "wss://relay.example.com",
            "v2.payload\"><script>alert(1)</script>",
            "test-nonce",
        );

        assert!(html.contains(
            "buzz://join?relay=wss%3A%2F%2Frelay.example.com&amp;code=v2.payload%22%3E%3Cscript%3Ealert%281%29%3C%2Fscript%3E"
        ));
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("<script nonce=\"test-nonce\">"));
        assert!(html.contains("fetch(\"/api/join-policy\")"));
        assert!(html.contains("fetch(\"/api/invites/accept-policy\""));
        assert!(html.contains("appUrl.searchParams.set(\"policy_receipt\", payload.receipt)"));
    }

    #[test]
    fn invite_relay_url_uses_the_request_host_and_configured_tls_posture() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "community.example.com:8443".parse().unwrap());

        assert_eq!(
            invite_relay_url("wss://relay.internal", &headers).as_deref(),
            Some("wss://community.example.com:8443")
        );
        assert_eq!(
            invite_relay_url("ws://localhost:3000", &headers).as_deref(),
            Some("ws://community.example.com:8443")
        );
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
