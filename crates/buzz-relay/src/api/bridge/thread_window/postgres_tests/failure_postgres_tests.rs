use super::*;
use std::{
    future::Future,
    sync::atomic::{AtomicUsize, Ordering},
    task::Poll,
};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

// Pause the HTTP future after the first real auxiliary page completes. This
// observes a production span rather than replacing the database or closure.
struct AuxPages(Arc<AtomicUsize>);
impl<S> Layer<S> for AuxPages
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_close(&self, id: tracing::Id, ctx: Context<'_, S>) {
        if ctx.span(&id).unwrap().metadata().name() == "thread_window_aux" {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_bridge_restarts_both_aux_hops_after_replica_failure() {
    let mut f = Fixture::new().await;
    let reply = f.reply(0).await;
    let reaction = f.aux(7, &reply, Some(f.channel)).await;
    // Two raw pages, so losing the snapshot after page one leaves a meaningful
    // old cursor. The writer edit inserted later sorts before that cursor.
    sqlx::query("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id) \
        SELECT community_id,decode(md5(n::text)||md5(('restart'||n)::text),'hex'),pubkey,created_at,kind,tags,'fixture',sig,received_at,channel_id \
        FROM events CROSS JOIN generate_series(1,1000) n WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid()).bind(reaction.id.to_bytes().to_vec()).execute(&f.pool).await.unwrap();
    let reader_name = format!("tw-reader-{}", Uuid::new_v4());
    let options: sqlx::postgres::PgConnectOptions =
        crate::test_support::database_url().parse().unwrap();
    let reader = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.application_name(&reader_name))
        .await
        .unwrap();
    let db = buzz_db::Db::from_pools(f.pool.clone(), reader.clone());
    db.fence()
        .force_open_for_tests(chrono::Utc::now() + chrono::Duration::seconds(10));
    Arc::make_mut(&mut f.state).db = db;
    let mut filter = f.filter();
    filter["until"] = json!(f.root.created_at.as_secs() + 1);
    filter["before_id"] = json!("00".repeat(32));
    let pages = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(AuxPages(pages.clone()));
    let mut request = Box::pin(
        f.post(&f.keys, "/query", json!([filter]))
            .with_subscriber(subscriber),
    );
    std::future::poll_fn(|cx| {
        assert!(
            request.as_mut().poll(cx).is_pending(),
            "request must pause inside closure"
        );
        if pages.load(Ordering::SeqCst) > 0 {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND xact_start IS NOT NULL")
        .bind(&reader_name).fetch_one(&f.pool).await.unwrap();
    assert_eq!(held, 1, "must actually hold the proved replica transaction");
    let edit = f.aux(40003, &reply, Some(f.channel)).await;
    let deletion = f.aux(5, &edit, None).await;
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(edit.id.to_bytes().to_vec())
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name=$1")
        .bind(&reader_name)
        .execute(&f.pool)
        .await
        .unwrap();
    let (status, body) = request.await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.bounds(&body, &filter)["has_more"], false);
    let ids: Vec<_> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&deletion.id.to_hex().as_str()),
        "restart must discover writer edit tombstone and its deletion"
    );
    assert!(!ids.contains(&edit.id.to_hex().as_str()));
    assert_eq!(
        ids.len(),
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        "discarded replica output must not duplicate events"
    );
    assert_eq!(ids.len(), 1004); // reply, 1001 reactions, deletion, bounds
    reader.close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_http_deadline_covers_authorization_wait() {
    let mut f = Fixture::new().await;
    // Disable only the optional DB lock budget via production configuration;
    // the shared HTTP deadline must still cover authorization in this mode.
    Arc::make_mut(&mut f.state).db = buzz_db::Db::new(&buzz_db::DbConfig {
        database_url: crate::test_support::database_url(),
        max_connections: 5,
        lock_timeout_ms: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut lock = f.pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE channel_members IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let (status, body) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, json!({"error":"thread window deadline exceeded"}));
    assert!(started.elapsed() >= DEADLINE);
    assert!(started.elapsed() < DEADLINE + Duration::from_secs(4));
    lock.rollback().await.unwrap();
    let (status, body) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.bounds(&body, &f.filter())["has_more"], false);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_bounds_rejected_by_ws_event_handler() {
    let f = Fixture::new().await;
    let forged = event(
        &f.keys,
        f.channel,
        39007,
        "{}",
        None,
        Timestamp::now().as_secs(),
    );
    let (send_tx, mut send_rx) = tokio::sync::mpsc::channel(10);
    let (ctrl_tx, _ctrl_rx) = tokio::sync::mpsc::channel(10);
    let conn = Arc::new(crate::connection::ConnectionState {
        conn_id: Uuid::new_v4(),
        tenant: buzz_core::TenantContext::resolved(f.community, f.host.clone()),
        remote_addr: "127.0.0.1:1234".parse().unwrap(),
        auth_state: std::sync::Mutex::new(crate::connection::AuthState::Authenticated(
            buzz_auth::AuthContext {
                pubkey: f.keys.public_key(),
                scopes: vec![],
                channel_ids: None,
                auth_method: buzz_auth::AuthMethod::Nip42,
                agent_owner_pubkey: None,
            },
        )),
        subscriptions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        send_tx,
        ctrl_tx,
        cancel: tokio_util::sync::CancellationToken::new(),
        backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
        grace_limit: 3,
    });
    crate::handlers::event::handle_event(forged.clone(), conn, f.state.clone()).await;
    let axum::extract::ws::Message::Text(text) = send_rx.try_recv().unwrap() else {
        panic!("expected ACK")
    };
    let ack: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(ack[0], "OK");
    assert_eq!(ack[1], forged.id.to_hex());
    assert_eq!(ack[2], false);
    assert!(ack[3].as_str().unwrap().contains("relay-only"), "{ack}");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_retries_aux_access_grant_during_closure() {
    assert_aux_access_change(true).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_retries_aux_access_revocation_during_closure() {
    assert_aux_access_change(false).await;
}

async fn assert_aux_access_change(grant: bool) {
    let f = Fixture::new().await;
    let reply = f.reply(0).await;
    // A visible first-hop event ensures the closure has a second hop where
    // polling can pause, even when the cross-channel edit is initially hidden.
    f.aux(7, &reply, Some(f.channel)).await;
    let aux_channel = Uuid::new_v4();
    f.state
        .db
        .create_channel_with_id(
            f.community,
            aux_channel,
            "aux-access-change",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            &f.keys.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap();
    let edit = event(
        &f.keys,
        aux_channel,
        40003,
        "cross-channel edit",
        Some(&reply),
        f.root.created_at.as_secs() + 60,
    );
    f.state
        .db
        .insert_event(f.community, &edit, Some(aux_channel))
        .await
        .unwrap();
    let set_access = "UPDATE channel_members SET removed_at=CASE WHEN $4 THEN NULL ELSE now() END \
                      WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3";
    sqlx::query(set_access)
        .bind(f.community.as_uuid())
        .bind(aux_channel)
        .bind(f.keys.public_key().to_bytes().to_vec())
        .bind(!grant)
        .execute(&f.pool)
        .await
        .unwrap();
    let contains_edit = |body: &Value| {
        body.as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == edit.id.to_hex())
    };
    let filter = f.filter();
    let (status, before) = f.post(&f.keys, "/query", json!([filter])).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    f.bounds(&before, &filter);
    assert_eq!(
        contains_edit(&before),
        !grant,
        "pre-transition visibility control"
    );

    let pages = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(AuxPages(pages.clone()));
    let mut request = Box::pin(
        f.post(&f.keys, "/query", json!([filter]))
            .with_subscriber(subscriber),
    );
    tokio::time::timeout(
        Duration::from_secs(5),
        std::future::poll_fn(|cx| {
            assert!(
                request.as_mut().poll(cx).is_pending(),
                "must pause before final authorization"
            );
            if pages.load(Ordering::SeqCst) > 0 {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }),
    )
    .await
    .expect("first auxiliary page must complete before transition");
    assert_eq!(
        pages.load(Ordering::SeqCst),
        1,
        "barrier must precede closure completion"
    );
    let changed = sqlx::query(set_access)
        .bind(f.community.as_uuid())
        .bind(aux_channel)
        .bind(f.keys.public_key().to_bytes().to_vec())
        .bind(grant)
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(changed.rows_affected(), 1);
    let current = f
        .state
        .db
        .get_accessible_channel_ids(f.community, &f.keys.public_key().to_bytes())
        .await
        .unwrap();
    assert!(
        current.contains(&f.channel),
        "requested channel stays authorized"
    );
    assert_eq!(current.contains(&aux_channel), grant);
    let (status, interrupted) = request.await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{interrupted}");
    assert_eq!(
        interrupted,
        json!({"error":"thread auxiliary authorization changed; retry window"})
    );

    let (status, after) = f.post(&f.keys, "/query", json!([filter])).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    f.bounds(&after, &filter);
    assert_eq!(
        contains_edit(&after),
        grant,
        "retry must use the complete new access set"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_production_authorization_lock_timeout_is_retryable() {
    let f = Fixture::new().await;
    let mut lock = f.pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE channel_members IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let (status, body) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(
        body,
        json!({"error":"thread database timeout; retry window"})
    );
    assert!(
        started.elapsed() >= Duration::from_millis(buzz_db::DbConfig::default().lock_timeout_ms)
    );
    assert!(started.elapsed() < DEADLINE);
    lock.rollback().await.unwrap();
    let (status, recovered) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert_eq!(status, StatusCode::OK, "{recovered}");
    assert_eq!(f.bounds(&recovered, &f.filter())["has_more"], false);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_database_timeout_classification_uses_sqlstate() {
    let f = Fixture::new().await;
    // The adapter handles the same DB failures at initial/final authorization,
    // selection and aux closure. Exercise actual PostgreSQL statement errors,
    // not string-matched synthetic errors; unrelated faults stay sanitized 500s.
    for (sql, expected) in [
        (
            "SET statement_timeout='25ms'; SELECT pg_sleep(1)",
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        ("SELECT 1/0", StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        let mut conn = f.pool.acquire().await.unwrap();
        let error = sqlx::raw_sql(sql).execute(&mut *conn).await.unwrap_err();
        let (status, body) = database_error("test", error.into());
        assert_eq!(status, expected, "{body:?}");
    }
    let (status, body) = database_error("pool", buzz_db::DbError::Sqlx(sqlx::Error::PoolTimedOut));
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body.0,
        json!({"error":"thread database timeout; retry window"})
    );
}
