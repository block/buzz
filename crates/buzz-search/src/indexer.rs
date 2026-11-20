//! The indexer: drains `search_outbox` into the OpenSearch index.
//!
//! One relay instance holds the indexer lease (a session advisory lock) at a
//! time; when it stops, its lock connection closes and another instance takes
//! over on its next attempt. Rows are claimed with `SKIP LOCKED`, so more
//! workers can run without a redesign.
//!
//! Each batch re-reads its events from Postgres, writes them (or removes the
//! documents of events that are gone) in one bulk request, and only then
//! deletes the rows it claimed. A failed request leaves the rows queued for
//! the next attempt.

use std::time::Duration;

use buzz_db::{search_outbox, Db, DbError};
use sqlx::PgPool;
use tracing::{info, warn};

use crate::opensearch::SearchIndex;
use crate::SearchError;

/// Deployment-global advisory lock electing the indexing instance.
const INDEXER_LOCK_KEY: i64 = 0x4255_5A5A_5345_4152;

/// Outbox rows per batch, and so documents per bulk request.
const BATCH_SIZE: i64 = 500;

/// How often an idle leader checks the queue, and a follower the lease.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Longest wait after repeated failures.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Why a batch was not indexed. Its rows stay queued.
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    /// Claiming, reading or completing the batch failed.
    #[error("outbox: {0}")]
    Outbox(#[from] DbError),
    /// The index write failed.
    #[error("index write: {}", .0.kind())]
    Index(#[from] SearchError),
}

/// Moves queued events from `search_outbox` into a [`SearchIndex`].
#[derive(Debug, Clone)]
pub struct Indexer {
    pool: PgPool,
    index: SearchIndex,
}

impl Indexer {
    /// An indexer reading the outbox through `pool` and writing `index`.
    pub fn new(pool: PgPool, index: SearchIndex) -> Self {
        Self { pool, index }
    }

    /// Index one batch of one community. Returns `false` when there was
    /// nothing to claim.
    pub async fn index_next_batch(&self) -> Result<bool, IndexerError> {
        let Some(batch) = search_outbox::claim_batch(&self.pool, BATCH_SIZE).await? else {
            return Ok(false);
        };
        self.index
            .write(batch.community(), &batch.documents, &batch.removed)
            .await?;
        batch.complete().await?;
        Ok(true)
    }

    /// Index batches until none is left to claim.
    pub async fn drain(&self) -> Result<(), IndexerError> {
        while self.index_next_batch().await? {}
        Ok(())
    }

    /// Hold the indexer lease and keep the index up to date, forever. Abort
    /// the task to stop; that closes the lease connection and lets another
    /// instance take over.
    pub async fn run(self, db: Db) {
        // Spread lease attempts of instances that start together.
        tokio::time::sleep(POLL_INTERVAL.mul_f64(rand::random::<f64>())).await;
        let mut leader: Option<buzz_db::SessionLeader> = None;
        let mut index_ready = false;
        let mut failures = 0u32;
        loop {
            if let Some(guard) = leader.as_mut() {
                if !guard.is_live().await {
                    warn!("search indexer lease session lost; re-electing");
                    leader = None;
                }
            }
            if leader.is_none() {
                match db.try_lock_session_leader(INDEXER_LOCK_KEY).await {
                    Ok(Some(acquired)) => {
                        info!("acquired the search indexer lease");
                        leader = Some(acquired);
                    }
                    Ok(None) => {}
                    Err(error) => warn!(%error, "search indexer lease attempt failed"),
                }
            }
            if leader.is_some() {
                // Create the index here rather than at startup, so an
                // unreachable cluster never delays serving.
                let result = if index_ready {
                    self.drain().await
                } else {
                    match self.index.ensure().await {
                        Ok(()) => {
                            index_ready = true;
                            self.drain().await
                        }
                        Err(error) => Err(error.into()),
                    }
                };
                match result {
                    Ok(()) => failures = 0,
                    Err(error) => {
                        failures = failures.saturating_add(1);
                        warn!(%error, failures, "search indexing failed; rows stay queued");
                    }
                }
            }
            tokio::time::sleep(backoff(failures)).await;
        }
    }
}

/// The wait before the next attempt: the poll interval, doubled per
/// consecutive failure up to [`MAX_BACKOFF`], with jitter after a failure.
fn backoff(failures: u32) -> Duration {
    if failures == 0 {
        return POLL_INTERVAL;
    }
    let ceiling = POLL_INTERVAL
        .saturating_mul(1 << failures.min(4))
        .min(MAX_BACKOFF);
    ceiling.mul_f64(0.5 + rand::random::<f64>() / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_to_a_cap() {
        assert_eq!(backoff(0), POLL_INTERVAL);
        for failures in 1..10 {
            let wait = backoff(failures);
            assert!(wait <= MAX_BACKOFF, "{failures}: {wait:?}");
            assert!(wait >= POLL_INTERVAL, "{failures}: {wait:?}");
        }
        assert!(backoff(8) >= MAX_BACKOFF / 2);
    }
}
