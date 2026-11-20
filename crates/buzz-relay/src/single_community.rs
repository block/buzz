//! Deployment admission policy; management lookups deliberately remain unfiltered.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use buzz_core::tenant::{normalize_host, relay_url_authority};

use crate::{config::Config, state::AppState};

pub(crate) fn serves_host(config: &Config, host: &str) -> bool {
    config.multi_community_relay || {
        let canonical = relay_url_authority(&config.relay_url);
        !canonical.is_empty() && normalize_host(host) == canonical
    }
}

// Only these control-plane paths can bypass community serving admission. Their
// own authentication/authorization still runs. Keep authority and path checks
// together: a management-looking path on an extra tenant is not an exception.
fn control_plane(state: &AppState, request: &Request) -> bool {
    let path = request.uri().path();
    if matches!(
        path,
        "/health" | "/_liveness" | "/_readiness" | "/internal/git/policy"
    ) {
        // The git callback additionally requires localhost and an operation HMAC.
        // Its handler checks the HMAC-bound target community in single mode.
        return true;
    }
    // Management route components use literal ASCII paths and UUID/pubkey IDs.
    // Do not extend an exception across an encoded separator or dot segment.
    if path.contains('%') || path.split('/').any(|part| matches!(part, "." | "..")) {
        return false;
    }
    let host = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(normalize_host)
        .unwrap_or_default();
    if host.is_empty() {
        return false;
    }
    if path.starts_with("/operator/") {
        return state
            .config
            .relay_operator_api_origin
            .as_deref()
            .is_some_and(|origin| host == relay_url_authority(origin));
    }
    if crate::api::admin::is_admin_host(state, request.headers()) {
        return path.starts_with("/api/admin/v1/")
            || crate::router::is_admin_spa_path(path)
            || crate::router::is_admin_static_path(path);
    }
    // Signed issuer commands revoke sessions deployment-wide; they do not
    // provide ordinary community access. Serve them on the configured relay
    // authority (the ordinary admission below), not arbitrary tenant hosts.
    false
}

pub(crate) async fn admit_request(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if state.config.multi_community_relay || control_plane(&state, &request) {
        return next.run(request).await;
    }
    let host = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|host| host.to_str().ok())
        .unwrap_or("");
    if serves_host(&state.config, host) {
        next.run(request).await
    } else {
        // Same empty, generic rejection as an unmapped host; never reveal
        // whether a rejected hostname has a stored community row.
        StatusCode::NOT_FOUND.into_response()
    }
}
