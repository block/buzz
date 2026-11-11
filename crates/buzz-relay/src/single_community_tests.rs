use crate::single_community::admit_request;
use axum::{body::Body, routing::get, Router};
use axum::{extract::Request, http::StatusCode};
use std::sync::Arc;
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
