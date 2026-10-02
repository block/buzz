use std::cell::Cell;

use super::*;

#[test]
fn worker_pool_overrides_do_not_change_relay_defaults_or_session_policy() {
    let defaults = DbConfig::default();
    let worker = worker_config(DbConfig {
        lock_timeout_ms: 123,
        idle_txn_timeout_ms: 456,
        statement_timeout_ms: 789,
        ..defaults.clone()
    });
    assert_eq!(worker.max_connections, 1);
    assert_eq!(worker.min_connections, 0);
    assert_eq!(worker.acquire_timeout_secs, 30);
    assert_eq!(worker.lock_timeout_ms, 123);
    assert_eq!(worker.idle_txn_timeout_ms, 456);
    assert_eq!(worker.statement_timeout_ms, 789);
    assert_eq!(defaults.max_connections, 20);
    assert_eq!(defaults.min_connections, 2);
    assert_eq!(defaults.acquire_timeout_secs, 3);
}

#[tokio::test(start_paused = true)]
async fn cold_connection_can_exceed_the_old_three_second_budget() {
    let started = Instant::now();
    connect_with_retry(|| async {
        sleep(Duration::from_secs(6)).await;
        Ok(())
    })
    .await
    .expect("cold startup succeeds");
    assert_eq!(started.elapsed(), Duration::from_secs(6));
}

#[tokio::test(start_paused = true)]
async fn transient_failure_backs_off_before_a_successful_attempt() {
    let attempts = Cell::new(0);
    let started = Instant::now();
    let result = connect_with_retry(|| {
        attempts.set(attempts.get() + 1);
        let attempt = attempts.get();
        async move {
            if attempt == 1 {
                Err(sqlx::Error::Io(ErrorKind::ConnectionReset.into()).into())
            } else {
                Ok(42)
            }
        }
    })
    .await;
    assert_eq!(result.expect("second attempt succeeds"), 42);
    assert_eq!(attempts.get(), 2);
    assert_eq!(started.elapsed(), Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn retry_exhaustion_propagates_the_last_error_after_three_attempts() {
    let attempts = Cell::new(0);
    let started = Instant::now();
    let error = connect_with_retry(|| {
        attempts.set(attempts.get() + 1);
        async { Err::<(), _>(sqlx::Error::Io(ErrorKind::ConnectionReset.into()).into()) }
    })
    .await
    .expect_err("exhausted startup must fail");
    assert_eq!(attempts.get(), 3);
    assert_eq!(started.elapsed(), Duration::from_secs(7));
    assert!(error.to_string().contains("3 attempt(s)"));
    assert!(matches!(
        error.downcast_ref::<DbError>(),
        Some(DbError::Sqlx(sqlx::Error::Io(_)))
    ));
}

#[tokio::test(start_paused = true)]
async fn hung_initialization_is_bounded_to_97_seconds_including_backoff() {
    let attempts = Cell::new(0);
    let started = Instant::now();
    let error = connect_with_retry(|| {
        attempts.set(attempts.get() + 1);
        std::future::pending::<buzz_db::Result<()>>()
    })
    .await
    .expect_err("hung connection setup must time out");
    assert_eq!(attempts.get(), 3);
    assert_eq!(started.elapsed(), Duration::from_secs(97));
    assert!(matches!(
        error.downcast_ref::<DbError>(),
        Some(DbError::Sqlx(sqlx::Error::PoolTimedOut))
    ));
}

#[tokio::test(start_paused = true)]
async fn permanent_errors_fail_immediately_and_do_not_expose_connection_details() {
    let attempts = Cell::new(0);
    let started = Instant::now();
    for driver_error in [
        sqlx::Error::Configuration("sensitive-url".into()),
        sqlx::Error::Tls("sensitive-certificate".into()),
        sqlx::Error::Protocol("sensitive-server-reply".into()),
        sqlx::Error::Io(ErrorKind::PermissionDenied.into()),
    ] {
        let mut error = Some(driver_error);
        let result = connect_with_retry(|| {
            attempts.set(attempts.get() + 1);
            let error = error.take().expect("permanent errors must not retry");
            async move { Err::<(), _>(error.into()) }
        })
        .await
        .expect_err("permanent error");
        assert!(!result.to_string().contains("sensitive"));
    }
    assert_eq!(attempts.get(), 4);
    assert_eq!(started.elapsed(), Duration::ZERO);
}
