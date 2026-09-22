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

async fn pause_after_aux_page<F: Future>(
    request: F,
) -> std::pin::Pin<Box<impl Future<Output = F::Output>>> {
    let pages = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(AuxPages(pages.clone()));
    let mut request = Box::pin(request.with_subscriber(subscriber));
    tokio::time::timeout(
        Duration::from_secs(5),
        std::future::poll_fn(|cx| {
            assert!(
                request.as_mut().poll(cx).is_pending(),
                "must pause inside closure"
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
    request
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_bridge_restarts_both_aux_hops_after_replica_failure() {
    let mut f = Fixture::new().await;
    let reply = f.reply(0).await;
    let reaction = f.aux(7, &reply, Some(f.channel)).await;
    // Two raw pages, so losing the snapshot after page one leaves a meaningful
    // old cursor. The writer edit inserted later sorts before that cursor.
    f.copy_aux(&reaction, 1000, "fixture").await;
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
    let request = pause_after_aux_page(f.post(&f.keys, "/query", json!([filter]))).await;
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND xact_start IS NOT NULL")
        .bind(&reader_name).fetch_one(&f.pool).await.unwrap();
    assert_eq!(held, 1, "must actually hold the proved replica transaction");
    let edit = f.aux(40003, &reply, Some(f.channel)).await;
    let deletion = f.aux(5, &edit, None).await;
    f.tombstone(&edit).await;
    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name=$1")
        .bind(&reader_name)
        .execute(&f.pool)
        .await
        .unwrap();
    let (status, body) = request.await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.bounds(&body, &filter)["has_more"], false);
    let ids = ids(&body, None);
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
    // The shared HTTP deadline still applies when the optional DB lock budget is disabled.
    Arc::make_mut(&mut f.state).db = production_db(buzz_db::DbConfig {
        lock_timeout_ms: 0,
        ..Default::default()
    })
    .await;
    assert_authorization_timeout(
        &f,
        "thread window deadline exceeded",
        DEADLINE,
        DEADLINE + Duration::from_secs(4),
    )
    .await;
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
    let auth = crate::connection::AuthState::Authenticated(buzz_auth::AuthContext {
        pubkey: f.keys.public_key(),
        scopes: vec![],
        channel_ids: None,
        auth_method: buzz_auth::AuthMethod::Nip42,
        agent_owner_pubkey: None,
    });
    let (mut conn, mut send_rx) = crate::connection::tests::test_conn_with_auth(auth);
    Arc::get_mut(&mut conn).unwrap().tenant =
        buzz_core::TenantContext::resolved(f.community, f.host.clone());
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
    private_channel(&f.state.db, f.community, aux_channel, &f.keys).await;
    let edit = f.aux(40003, &reply, Some(aux_channel)).await;
    set_access(&f, aux_channel, !grant).await;
    let contains_edit = |body: &Value| {
        body.as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == edit.id.to_hex())
    };
    let filter = f.filter();
    let before = f.query(&filter).await;
    f.bounds(&before, &filter);
    assert_eq!(
        contains_edit(&before),
        !grant,
        "pre-transition visibility control"
    );

    let request = pause_after_aux_page(f.post(&f.keys, "/query", json!([filter]))).await;
    set_access(&f, aux_channel, grant).await;
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

    let after = f.query(&filter).await;
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
    assert_authorization_timeout(
        &f,
        "thread database timeout; retry window",
        Duration::from_millis(buzz_db::DbConfig::default().lock_timeout_ms),
        DEADLINE,
    )
    .await;
}

async fn assert_authorization_timeout(f: &Fixture, message: &str, min: Duration, max: Duration) {
    let mut lock = f.pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE channel_members IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let (status, body) = f.post(&f.keys, "/query", json!([f.filter()])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, json!({"error":message}));
    assert!((min..max).contains(&started.elapsed()));
    lock.rollback().await.unwrap();
    let recovered = f.query(&f.filter()).await;
    assert_eq!(f.bounds(&recovered, &f.filter())["has_more"], false);
}

async fn set_access(f: &Fixture, channel: Uuid, allowed: bool) {
    let changed = sqlx::query(
        "UPDATE channel_members SET removed_at=CASE WHEN $4 THEN NULL ELSE now() END \
        WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
    )
    .bind(f.community.as_uuid())
    .bind(channel)
    .bind(f.keys.public_key().to_bytes().to_vec())
    .bind(allowed)
    .execute(&f.pool)
    .await
    .unwrap();
    assert_eq!(changed.rows_affected(), 1);
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
