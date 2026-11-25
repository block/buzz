//! The indexer: drains `search_outbox` into the OpenSearch index.
//!
//! One relay at a time holds the indexer lease (a session advisory lock);
//! when it stops, its lock connection closes and another relay takes over.
//! Rows are deleted only once the index holds their events, so a failed
//! request leaves them queued.
//!
//! A batch holds its community's admission open for up to one request
//! timeout. Community deletion's quiesce step waits for that batch, and once
//! the community is quiescing no new batch for it can start.

use std::time::Duration;

use buzz_db::{search_outbox, Db, DbError};
use tracing::{info, warn};

use crate::opensearch::SearchIndex;
use crate::SearchError;
use buzz_core::CommunityId;

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
    db: Db,
    index: SearchIndex,
}

impl Indexer {
    /// An indexer reading the outbox from `db` and writing `index`.
    pub fn new(db: Db, index: SearchIndex) -> Self {
        Self { db, index }
    }

    /// Index batches until none is left to claim. A community whose batch
    /// the cluster rejects is skipped for the rest of the pass and reported
    /// once the others are done; any other failure ends the pass.
    pub async fn drain(&self) -> Result<(), IndexerError> {
        let mut skipped: Vec<CommunityId> = Vec::new();
        let mut rejection = None;
        while let Some(batch) =
            search_outbox::claim_batch(self.db.pool(), BATCH_SIZE, &skipped).await?
        {
            let community = batch.community();
            match self
                .index
                .write(community, &batch.documents, &batch.removed)
                .await
            {
                Ok(()) => batch.complete().await?,
                Err(error @ SearchError::Rejected(_)) => {
                    warn!(%community, error = %error.kind(), "search index rejected a batch; its rows stay queued");
                    skipped.push(community);
                    rejection.get_or_insert(error);
                }
                Err(error) => return Err(error.into()),
            }
        }
        rejection.map_or(Ok(()), |error| Err(error.into()))
    }

    /// One leader pass: make sure the index exists, then drain. Checking
    /// every pass (one request when the alias exists) recreates an index
    /// that went away, and creating it here rather than at startup means an
    /// unreachable cluster never delays serving.
    async fn pass(&self) -> Result<(), IndexerError> {
        self.index.ensure().await?;
        self.drain().await
    }

    /// Hold the indexer lease and keep the index up to date, forever. Abort
    /// the task to stop; that closes the lease connection and lets another
    /// instance take over.
    pub async fn run(self) {
        // Spread lease attempts of instances that start together.
        tokio::time::sleep(POLL_INTERVAL.mul_f64(rand::random::<f64>())).await;
        let mut leader: Option<buzz_db::SessionLeader> = None;
        let mut failures = 0u32;
        loop {
            if let Some(guard) = leader.as_mut() {
                if !guard.is_live().await {
                    warn!("search indexer lease session lost; re-electing");
                    leader = None;
                }
            }
            if leader.is_none() {
                match self.db.try_lock_session_leader(INDEXER_LOCK_KEY).await {
                    Ok(Some(acquired)) => {
                        info!("acquired the search indexer lease");
                        leader = Some(acquired);
                    }
                    Ok(None) => {}
                    Err(error) => warn!(%error, "search indexer lease attempt failed"),
                }
            }
            if leader.is_some() {
                match self.pass().await {
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
