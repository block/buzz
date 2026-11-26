//! Read-only gauges over the `search_outbox` queue: how much is waiting and
//! how long the oldest change has waited. Only the relay that holds the
//! indexer lease samples them, so the fleet reads the queue once per
//! interval, not once per relay.
//!
//! - `buzz_search_outbox_depth`: queued rows, counted up to
//!   [`DEPTH_COUNT_CAP`] so a long outage cannot make the count itself slow;
//! - `buzz_search_index_lag_seconds`: age of the oldest queued row, `0` when
//!   the queue is empty. Both reads use the queue's `created_at` index.

use std::time::Duration;

use sqlx::PgPool;

use crate::SearchError;

/// Rows counted at most; a deeper queue reports this value.
pub const DEPTH_COUNT_CAP: i64 = 100_000;

/// How often the relay samples the queue.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(15);

/// One reading of the queue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutboxStats {
    /// Queued rows, at most [`DEPTH_COUNT_CAP`].
    pub depth: i64,
    /// Age of the oldest queued row; zero when the queue is empty.
    pub lag: Duration,
}

/// Read the queue's depth and lag.
pub async fn read(pool: &PgPool) -> Result<OutboxStats, SearchError> {
    let (depth, lag_seconds): (i64, Option<f64>) = sqlx::query_as(
        "SELECT \
             (SELECT count(*) FROM (SELECT 1 FROM search_outbox LIMIT $1) AS capped), \
             (SELECT EXTRACT(EPOCH FROM now() - min(created_at))::float8 FROM search_outbox)",
    )
    .bind(DEPTH_COUNT_CAP)
    .fetch_one(pool)
    .await?;
    Ok(OutboxStats {
        depth,
        lag: Duration::from_secs_f64(lag_seconds.unwrap_or(0.0).max(0.0)),
    })
}

/// Read the queue and publish its gauges.
pub async fn record(pool: &PgPool) -> Result<OutboxStats, SearchError> {
    let stats = read(pool).await?;
    metrics::gauge!("buzz_search_outbox_depth").set(stats.depth as f64);
    metrics::gauge!("buzz_search_index_lag_seconds").set(stats.lag.as_secs_f64());
    Ok(stats)
}
