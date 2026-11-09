use axum::{body::to_bytes, http::StatusCode};
use std::{sync::Arc, time::Duration};

/// Exercise the real router and SQL lookup with a checked-out sole connection.
/// An authoritative missing host remains 404 before and after the outage;
/// saturation is retryable even for a host whose existence is not yet known.
/// Mutation: collapsing Lookup into Unmapped makes the saturated step fail.
#[sqlx::test(migrations = false)]
#[ignore = "requires Postgres"]
async fn pool_timeout_is_unavailable_but_an_authoritative_host_miss_is_not_found(
    fixture: sqlx::PgPool,
) {
    sqlx::raw_sql(include_str!(
        "../../../../schema/tables/public/communities.sql"
    ))
    .execute(&fixture)
    .await
    .unwrap();
    let community = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, 'relay.example')")
        .bind(community)
        .execute(&fixture)
        .await
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(250))
        .connect_with((*fixture.connect_options()).clone())
        .await
        .unwrap();
    let mut state =
        (*crate::state::tests::test_state_with_database_pool(pool.clone()).await).clone();
    Arc::make_mut(&mut state.config).nip_fi.mode = buzz_auth::NipFiMode::Off;
    let state = Arc::new(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = crate::router::build_router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    for saturated in [false, true, false] {
        // A fresh cache each phase forces the same real lookup after recovery.
        state.invalidate_community_host(&buzz_core::TenantContext::resolved(
            buzz_core::CommunityId::from_uuid(community),
            "relay.example",
        ));
        let held = if saturated {
            Some(pool.acquire().await.unwrap())
        } else {
            None
        };
        for host in ["relay.example", "unknown.example"] {
            for path in ["/query", "/count", "/events", "/api/invites"] {
                let response = client
                    .post(format!("http://{address}{path}"))
                    .header("host", host)
                    .header("content-type", "application/json")
                    .body("{}")
                    .send()
                    .await
                    .unwrap();
                let status = response.status();
                let body = response.bytes().await.unwrap();
                if saturated {
                    assert_eq!(
                        status,
                        StatusCode::SERVICE_UNAVAILABLE,
                        "{path} {host}: {body:?}"
                    );
                    assert_eq!(
                        &body[..],
                        br#"{"error":"relay: service temporarily unavailable"}"#
                    );
                } else if host == "unknown.example" {
                    assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body:?}");
                    assert_eq!(
                        &body[..],
                        br#"{"error":"relay: no community is configured for this host"}"#
                    );
                } else {
                    // Known host gets past row zero to authentication/validation.
                    assert!(
                        ![StatusCode::NOT_FOUND, StatusCode::SERVICE_UNAVAILABLE].contains(&status),
                        "{path}: {body:?}"
                    );
                }
            }
        }
        for mode in [buzz_auth::NipFiMode::Off, buzz_auth::NipFiMode::Shadow] {
            let mut upgrade_state = (*state).clone();
            Arc::make_mut(&mut upgrade_state.config).nip_fi.mode = mode;
            for host in ["relay.example", "unknown.example", ""] {
                let result = crate::nip_fi_policy::bind_for_upgrade(&upgrade_state, host).await;
                if !saturated && host == "relay.example" {
                    let (tenant, _) = result.expect("known community binds after recovery");
                    assert_eq!(tenant.community().as_uuid(), &community);
                } else {
                    let response = result.expect_err("unresolved community cannot upgrade");
                    let status = response.status();
                    let body = to_bytes(response.into_body(), 4096).await.unwrap();
                    let unavailable = saturated && !host.is_empty();
                    assert_eq!(
                        status,
                        if unavailable {
                            StatusCode::SERVICE_UNAVAILABLE
                        } else {
                            StatusCode::NOT_FOUND
                        },
                        "{mode:?} {host}: {body:?}"
                    );
                    assert_eq!(
                        &body[..],
                        if unavailable {
                            b"relay: service temporarily unavailable".as_slice()
                        } else {
                            b"relay: no community is configured for this host".as_slice()
                        }
                    );
                }
            }
        }
        drop(held);
    }
    server.abort();
    pool.close().await;
}
