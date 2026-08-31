//! Drive parsed REQs through the real handler and inspect outbound WS frames.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::Message;
use buzz_auth::{AuthContext, AuthMethod};
use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_db::Db;
use nostr::Keys;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::connection::{AuthState, ConnectionState};
use crate::protocol::ClientMessage;
use crate::state::AppState;

async fn state(db: Db, pool: sqlx::PgPool) -> Arc<AppState> {
    state_with_redis(db, pool, "redis://127.0.0.1:1".into()).await
}

async fn state_with_redis(db: Db, pool: sqlx::PgPool, redis_url: String) -> Arc<AppState> {
    let mut config = crate::config::Config::from_env().unwrap();
    config.require_relay_membership = false;
    config.redis_url = redis_url;
    config.require_auth_token = true;
    let redis = deadpool_redis::Config::from_url(&config.redis_url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .unwrap();
    let pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new(&config.redis_url, redis.clone())
            .await
            .unwrap(),
    );
    let audit = buzz_audit::AuditService::new(pool.clone());
    let auth = buzz_auth::AuthService::new(config.auth.clone());
    let search = buzz_search::SearchService::new(pool);
    let workflow = Arc::new(buzz_workflow::WorkflowEngine::new(
        db.clone(),
        Default::default(),
    ));
    let media = buzz_media::MediaStorage::new(&config.media).unwrap();
    let (state, _) = AppState::new(
        config,
        db,
        redis,
        audit,
        pubsub,
        auth,
        search,
        workflow,
        Keys::generate(),
        media,
    );
    Arc::new(state)
}

fn connection(
    state: &AppState,
    tenant: TenantContext,
    keys: &Keys,
) -> (Arc<ConnectionState>, mpsc::Receiver<Message>) {
    let (send_tx, rx) = mpsc::channel(2048);
    let (ctrl_tx, _) = mpsc::channel(8);
    state.accessible_channels_cache.insert(
        (tenant.community(), keys.public_key().to_bytes().to_vec()),
        vec![],
    );
    (
        Arc::new(ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant,
            remote_addr: "127.0.0.1:1".parse().unwrap(),
            auth_state: RwLock::new(AuthState::Authenticated(AuthContext {
                pubkey: keys.public_key(),
                scopes: vec![],
                channel_ids: None,
                auth_method: AuthMethod::Nip42,
                agent_owner_pubkey: None,
            })),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            cancel: CancellationToken::new(),
            backpressure_count: Arc::new(AtomicU8::new(0)),
            grace_limit: 3,
        }),
        rx,
    )
}

async fn req(
    state: &Arc<AppState>,
    conn: &Arc<ConnectionState>,
    rx: &mut mpsc::Receiver<Message>,
    filters: Vec<Value>,
) -> Vec<Value> {
    let mut wire = vec![json!("REQ"), json!("host-history")];
    wire.extend(filters);
    let ClientMessage::Req { sub_id, filters } =
        ClientMessage::parse(&Value::Array(wire).to_string()).unwrap()
    else {
        panic!("REQ")
    };
    crate::handlers::req::handle_req(sub_id, filters, conn.clone(), state.clone()).await;
    let mut frames = vec![];
    while let Ok(frame) = rx.try_recv() {
        let Message::Text(text) = frame else {
            panic!("text frame")
        };
        frames.push(serde_json::from_str(&text).unwrap());
    }
    frames
}

fn host_filter(keys: &Keys, label: &str) -> Value {
    json!({"kinds":[50000], "authors":[keys.public_key().to_hex()], "#p":[keys.public_key().to_hex()], "#L":["buzz.host.v1"], "#l":[label], "limit":1000})
}

/// A listening TCP endpoint counts attempted replica checkouts. The fence and
/// bounded-read budget are open; a control REQ proves routing reaches it. No PG
/// or Redis service is required and no real configuration credentials are used.
#[tokio::test]
async fn host_req_failure_is_closed_and_bypasses_eligible_replica() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let server = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            drop(socket);
        }
    });
    let writer = PgPoolOptions::new()
        .connect_lazy("postgres://test:test@127.0.0.1:1/test")
        .unwrap();
    writer.close().await; // Deterministic query failure, not access-cache failure.
    let reader = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(100))
        .connect_lazy(&format!(
            "postgres://test:test@{address}/test?sslmode=disable"
        ))
        .unwrap();
    let mut db = Db::from_pools(writer.clone(), reader);
    db.set_replica_read_max_age_for_tests(Some(Duration::from_secs(60)));
    db.fence().force_open_for_tests(chrono::Utc::now());
    let state = state(db, writer).await;
    let owner = Keys::generate();
    let tenant =
        TenantContext::resolved(CommunityId::from_uuid(Uuid::new_v4()), "host-history.test");
    let (conn, mut rx) = connection(&state, tenant, &owner);
    for filters in [
        vec![host_filter(&owner, "registration")],
        vec![host_filter(&owner, "report")],
        vec![json!({"ids":["aa".repeat(32)]})],
        // Failure in another filter must not turn a host-containing REQ into
        // partial success. The whole multi-filter history is one read outcome.
        vec![json!({"kinds":[0]}), host_filter(&owner, "registration")],
    ] {
        let before = attempts.load(Ordering::SeqCst);
        let mixed = filters.len() > 1;
        let frames = req(&state, &conn, &mut rx, filters).await;
        assert_eq!(
            frames,
            vec![json!(["CLOSED", "host-history", "error: database error"])]
        );
        assert!(conn.subscriptions.lock().await.is_empty());
        assert_eq!(state.sub_registry.total_subscriptions(), 0);
        if !mixed {
            assert_eq!(
                attempts.load(Ordering::SeqCst),
                before,
                "host reads must never attempt a replica checkout"
            );
        }
    }
    let before = attempts.load(Ordering::SeqCst);
    let frames = req(&state, &conn, &mut rx, vec![json!({"kinds":[0]})]).await;
    assert_eq!(frames, vec![json!(["EOSE", "host-history"])]);
    assert!(
        attempts.load(Ordering::SeqCst) > before,
        "negative control must attempt routed history"
    );
    assert_eq!(
        state.sub_registry.total_subscriptions(),
        1,
        "legacy unrelated error behavior retained"
    );
    server.abort();
}

#[tokio::test]
async fn host_req_authorization_and_search_fail_before_history() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://test:test@127.0.0.1:1/test")
        .unwrap();
    pool.close().await;
    let state = state(Db::from_pool(pool.clone()), pool).await;
    let owner = Keys::generate();
    let other = Keys::generate();
    let tenant =
        TenantContext::resolved(CommunityId::from_uuid(Uuid::new_v4()), "host-history.test");
    let (conn, mut rx) = connection(&state, tenant, &other);
    let frames = req(
        &state,
        &conn,
        &mut rx,
        vec![host_filter(&owner, "registration")],
    )
    .await;
    assert_eq!(frames[0][0], "CLOSED");
    assert!(frames[0][2].as_str().unwrap().starts_with("restricted:"));
    let mut search = host_filter(&other, "registration");
    search["search"] = json!("test");
    assert_eq!(
        req(&state, &conn, &mut rx, vec![search]).await,
        vec![json!([
            "CLOSED",
            "host-history",
            "error: host history search is unsupported"
        ])]
    );
    assert_eq!(state.sub_registry.total_subscriptions(), 0);
}

/// Opt-in transport integration against an isolated, migrated database. Does
/// not silently skip or use the configured/shared relay database.
#[tokio::test]
#[ignore = "requires isolated migrated Postgres: MULTIVERSE_TEST_DATABASE_URL"]
async fn host_req_primary_exact_tags_precede_limit_and_private_results_are_safe() {
    let url =
        std::env::var("MULTIVERSE_TEST_DATABASE_URL").expect("isolated database URL required");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let db = Db::from_pool(pool.clone());
    let host_name = format!("host-history-{}.test", Uuid::new_v4());
    let community = db.ensure_configured_community(&host_name).await.unwrap();
    let tenant = TenantContext::resolved(community.id, host_name);
    let owner = Keys::generate();
    let registration = buzz_core::host::registration(
        &owner,
        Keys::generate().public_key(),
        nostr::Timestamp::now().as_secs(),
    )
    .unwrap();
    // Deliberately omit event_mentions. It is not authoritative registration state.
    buzz_db::event::insert_event(&pool, tenant.community(), &registration, None)
        .await
        .unwrap();
    let state = state(db, pool.clone()).await;
    let (conn, mut rx) = connection(&state, tenant.clone(), &owner);
    let frames = req(
        &state,
        &conn,
        &mut rx,
        vec![host_filter(&owner, "registration")],
    )
    .await;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0][2]["id"], registration.id.to_hex());
    assert_eq!(frames[1], json!(["EOSE", "host-history"]));

    // Unrelated registrations cannot consume a report page before LIMIT.
    let mut capped = host_filter(&owner, "report");
    capped["limit"] = json!(1);
    assert_eq!(
        req(&state, &conn, &mut rx, vec![capped]).await,
        vec![json!(["EOSE", "host-history"])]
    );
    assert_eq!(state.sub_registry.total_subscriptions(), 1);

    // Known-ID requests still apply owner-only visibility on every returned row.
    for keys in [&owner, &Keys::generate()] {
        let (conn, mut rx) = connection(&state, tenant.clone(), keys);
        let frames = req(
            &state,
            &conn,
            &mut rx,
            vec![json!({"ids":[registration.id.to_hex()], "limit":1})],
        )
        .await;
        if keys.public_key() == owner.public_key() {
            assert_eq!(frames.len(), 2);
            assert_eq!(frames[0][2]["id"], registration.id.to_hex());
        } else {
            assert_eq!(frames, vec![json!(["EOSE", "host-history"])]);
        }
    }
    // This fixture uses a unique community; leave service lifecycle to its owner.
    pool.close().await;
}

mod paging;

mod mixed_tags;
