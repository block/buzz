//! Bounded cold database startup for the run-once storage accounting worker.

use std::future::Future;
use std::io::ErrorKind;
use std::time::Duration;

use anyhow::{Context, Result};
use buzz_db::{Db, DbConfig, DbError};
use tokio::time::{sleep, timeout, Instant};

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];

/// Establish the worker's database pool before it acquires its accounting lease.
pub(super) async fn connect_db() -> Result<Db> {
    let config = worker_config(crate::db_config_from_env());
    connect_with_retry(|| Db::new(&config)).await
}

fn worker_config(config: DbConfig) -> DbConfig {
    DbConfig {
        max_connections: 1,
        // The worker detaches its lock-owning session. Do not open idle
        // replacements while that session scans S3 and publishes the result.
        min_connections: 0,
        acquire_timeout_secs: ACQUIRE_TIMEOUT.as_secs(),
        ..config
    }
}

// Only initial connection establishment is retried. Once the command owns
// the advisory lock, reconnecting would lose its publication fence.
async fn connect_with_retry<T, Connect, Attempt>(mut connect: Connect) -> Result<T>
where
    Connect: FnMut() -> Attempt,
    Attempt: Future<Output = buzz_db::Result<T>>,
{
    let started = Instant::now();
    // Three attempts of at most 30s, plus 2s and 5s backoffs: at most 97s.
    for attempt in 0..=RETRY_DELAYS.len() {
        let attempt_started = Instant::now();
        eprintln!(
            "{}",
            serde_json::json!({
                "event": "storage_snapshot_db_connect_started",
                "stage": "db_connect",
                "attempt": attempt + 1,
                "timeout_ms": ACQUIRE_TIMEOUT.as_millis(),
            })
        );
        // Bound the entire initialization future, including session setup.
        let result = timeout(ACQUIRE_TIMEOUT, connect())
            .await
            .unwrap_or_else(|_| Err(sqlx::Error::PoolTimedOut.into()));
        match result {
            Ok(db) => {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "storage_snapshot_db_connect_completed",
                        "stage": "db_connect",
                        "attempt": attempt + 1,
                        "attempt_elapsed_ms": attempt_started.elapsed().as_millis(),
                        "elapsed_ms": started.elapsed().as_millis(),
                    })
                );
                return Ok(db);
            }
            Err(error) => {
                let delay = RETRY_DELAYS
                    .get(attempt)
                    .copied()
                    .filter(|_| retryable(&error));
                let class = error_class(&error);
                // Avoid raw connection errors/URLs: configuration and driver
                // messages can contain credentials. Keep the source on the
                // returned error, but print only this bounded classification.
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "storage_snapshot_db_connect_failed",
                        "stage": "db_connect",
                        "attempt": attempt + 1,
                        "attempt_elapsed_ms": attempt_started.elapsed().as_millis(),
                        "elapsed_ms": started.elapsed().as_millis(),
                        "error_class": class,
                        "io_kind": match &error {
                            DbError::Sqlx(sqlx::Error::Io(error)) => Some(format!("{:?}", error.kind())),
                            _ => None,
                        },
                        "sqlstate": match &error {
                            DbError::Sqlx(sqlx::Error::Database(error)) => error.code(),
                            _ => None,
                        },
                        "retry_in_ms": delay.map(|value| value.as_millis()),
                    })
                );
                match delay {
                    Some(delay) => sleep(delay).await,
                    None => {
                        return Err(error).with_context(|| {
                            format!(
                                "storage snapshot database startup failed after {} attempt(s) and {}ms ({class})",
                                attempt + 1,
                                started.elapsed().as_millis(),
                            )
                        });
                    }
                }
            }
        }
    }
    unreachable!("the last connection attempt always returns")
}

fn retryable(error: &DbError) -> bool {
    match error {
        DbError::Sqlx(sqlx::Error::PoolTimedOut) => true,
        // SQLx already backs off on connection refusal and transient server
        // errors. Other transport/resolver failures may also be transient;
        // allow only the bounded startup retries, excluding local input and
        // permission errors. Unknown resolver errors are not labeled as DNS.
        DbError::Sqlx(sqlx::Error::Io(error)) => !matches!(
            error.kind(),
            ErrorKind::InvalidInput
                | ErrorKind::InvalidData
                | ErrorKind::PermissionDenied
                | ErrorKind::NotFound
                | ErrorKind::Unsupported
        ),
        // Includes authentication, TLS, URL configuration and protocol errors.
        _ => false,
    }
}

fn error_class(error: &DbError) -> &'static str {
    match error {
        DbError::Sqlx(sqlx::Error::PoolTimedOut) => "timeout",
        DbError::Sqlx(sqlx::Error::Io(_)) => "io",
        DbError::Sqlx(sqlx::Error::Tls(_)) => "tls",
        DbError::Sqlx(sqlx::Error::Configuration(_)) => "configuration",
        DbError::Sqlx(sqlx::Error::Database(error)) => {
            if error.code().is_some_and(|code| code.starts_with("28")) {
                "authentication"
            } else {
                "database"
            }
        }
        DbError::Sqlx(sqlx::Error::Protocol(_)) => "protocol",
        _ => "other",
    }
}

#[cfg(test)]
mod tests;
