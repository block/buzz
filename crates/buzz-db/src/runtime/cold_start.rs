//! Bounded cold database startup for run-once operator workers.
//!
//! A freshly scheduled worker pod (no mesh sidecar, cold DNS) can need more
//! than the relay's three-second acquisition budget to open its first
//! connection. Workers that start, do bounded work, and exit share this
//! connector so a slow first connection is waited out and transient transport
//! failures are retried, while configuration, authentication, TLS, and
//! protocol errors still fail immediately.
//!
//! Startup attempts are reported as JSON lines on stderr because these
//! command-line workers do not install a tracing subscriber.

use std::future::Future;
use std::io::ErrorKind;
use std::time::Duration;

use tokio::time::{sleep, timeout, Instant};

use super::{Db, DbConfig};
use crate::DbError;

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];

/// A run-once worker could not establish its database pool.
///
/// The message carries only a bounded classification: driver messages and
/// URLs can contain credentials. The driver error stays available as the
/// [`std::error::Error::source`].
#[derive(Debug, thiserror::Error)]
#[error(
    "{worker} database startup failed after {attempts} attempt(s) and {elapsed_ms}ms ({class})"
)]
pub struct ColdStartError {
    worker: &'static str,
    attempts: usize,
    elapsed_ms: u128,
    class: &'static str,
    #[source]
    source: DbError,
}

impl Db {
    /// Connect a run-once worker's pool with a bounded cold-start budget.
    ///
    /// The pool keeps the caller's size and session policy, opens no idle
    /// spare connections, and waits up to 30 seconds per acquisition. Startup
    /// timeouts and transient transport failures are retried at most twice,
    /// after two and five seconds, so startup waits at most 97 seconds.
    /// `worker` prefixes the stderr startup events, for example
    /// `deletion_db_connect_failed`.
    ///
    /// Only initial connection establishment is retried. Once a caller holds
    /// a session-bound lock or lease, reconnecting would lose that fence.
    pub async fn connect_cold_start(
        config: DbConfig,
        worker: &'static str,
    ) -> std::result::Result<Self, ColdStartError> {
        let config = cold_start_config(config);
        connect_with_retry(worker, || Db::new(&config)).await
    }
}

fn cold_start_config(config: DbConfig) -> DbConfig {
    DbConfig {
        // A run-once worker only needs connections it actually uses; idle
        // replacements would also contend with a slow cold start.
        min_connections: 0,
        acquire_timeout_secs: ACQUIRE_TIMEOUT.as_secs(),
        ..config
    }
}

async fn connect_with_retry<T, Connect, Attempt>(
    worker: &'static str,
    mut connect: Connect,
) -> std::result::Result<T, ColdStartError>
where
    Connect: FnMut() -> Attempt,
    Attempt: Future<Output = crate::Result<T>>,
{
    let started = Instant::now();
    // Three attempts of at most 30s, plus 2s and 5s backoffs: at most 97s.
    for attempt in 0..=RETRY_DELAYS.len() {
        let attempt_started = Instant::now();
        eprintln!(
            "{}",
            serde_json::json!({
                "event": format!("{worker}_db_connect_started"),
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
                        "event": format!("{worker}_db_connect_completed"),
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
                        "event": format!("{worker}_db_connect_failed"),
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
                        return Err(ColdStartError {
                            worker,
                            attempts: attempt + 1,
                            elapsed_ms: started.elapsed().as_millis(),
                            class,
                            source: error,
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
