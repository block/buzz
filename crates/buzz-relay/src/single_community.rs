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
            || path == "/"
            || path == "/reports"
            || path.starts_with("/reports/")
            || path == "/feedback"
            || path.starts_with("/feedback/")
            || path.starts_with("/assets/")
            || path == "/favicon.svg";
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn single_mode_admission_covers_hosts_paths_and_cached_tenants() {
        let mut state = crate::state::tests::test_state_with_unreachable_database_url(
            "postgres://buzz@127.0.0.1:1/buzz",
        )
        .await;
        let config = Arc::make_mut(&mut Arc::get_mut(&mut state).expect("exclusive").config);
        config.multi_community_relay = false;
        config.relay_url = "wss://relay.example:8443".into();
        config.relay_operator_api_origin = Some("https://operator.example".into());
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".into(),
            auth: crate::config::AdminAuth::Nip98,
            web_dir: None,
        });
        state.tenant_host_cache.insert(
            "extra.example".into(),
            (
                buzz_core::CommunityId::from_uuid(uuid::Uuid::new_v4()),
                std::time::Instant::now(),
            ),
        );
        // A real router mounts the production middleware before any handler;
        // a sentinel downstream makes accidental admission unambiguous.
        let app = Router::new()
            .fallback(get(|| async { StatusCode::ACCEPTED }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                admit_request,
            ));
        for (host, allowed) in [
            ("relay.example:8443", true),
            ("RELAY.EXAMPLE:8443", true),
            ("relay.example", false),
            ("extra.example", false),
            ("", false),
        ] {
            for path in [
                "/",
                "/events",
                "/query",
                "/api/invites/claim",
                "/media/file",
                "/git/a/b/info/refs",
                "/huddle/id/audio",
                "/invite/token",
            ] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(path)
                            .header("host", host)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    if allowed {
                        StatusCode::ACCEPTED
                    } else {
                        StatusCode::NOT_FOUND
                    },
                    "{host} {path}"
                );
            }
        }
        for (path, authority) in [
            ("/operator/communities", "operator.example"),
            ("/api/admin/v1/probe", "admin.example"),
            ("/reports", "admin.example"),
        ] {
            for host in [authority, "extra.example"] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(path)
                            .header("host", host)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    if host == authority {
                        StatusCode::ACCEPTED
                    } else {
                        StatusCode::NOT_FOUND
                    },
                    "{host} {path}"
                );
            }
        }
        for path in [
            "/operatorx/communities",
            "/Operator/communities",
            "/operator%2Fcommunities",
            "/api/admin/v1x/probe",
            "/api/admin/V1/probe",
            "/api/admin/v1%2Fprobe",
            "/health/",
            "/internal/git/policy/",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("host", "extra.example")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
        for path in [
            "/health",
            "/_liveness",
            "/_readiness",
            "/internal/git/policy",
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::ACCEPTED);
        }
        // Default/multi mode is a strict pass-through even without a Host.
        let mut config = (*state.config).clone();
        config.multi_community_relay = true;
        let mut multi = (*state).clone();
        multi.config = Arc::new(config);
        let app = Router::new()
            .fallback(get(|| async { StatusCode::ACCEPTED }))
            .layer(axum::middleware::from_fn_with_state(
                Arc::new(multi),
                admit_request,
            ));
        assert_eq!(
            app.oneshot(Request::new(Body::empty()))
                .await
                .unwrap()
                .status(),
            StatusCode::ACCEPTED
        );
    }
}
