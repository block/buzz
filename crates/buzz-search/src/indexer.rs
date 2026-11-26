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
//!
//! Only the lease holder backfills (see `buzz_db::search_backfill`), so a
//! fleet queues each stored event once, and only while the outbox has room,
//! so live rows wait behind a bounded number of backfill rows.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use buzz_db::{search_backfill, search_outbox, Db, DbError};
use tracing::{debug, error, info, warn};

use crate::opensearch::SearchIndex;
use crate::SearchError;
use buzz_core::CommunityId;

/// Deployment-global advisory lock electing the indexing instance.
const INDEXER_LOCK_KEY: i64 = 0x4255_5A5A_5345_4152;

/// Outbox rows per batch, and so documents per bulk request.
const BATCH_SIZE: i64 = 500;

/// Backfill queues keys only while the outbox holds fewer rows than this.
const BACKFILL_OUTBOX_BOUND: i64 = 4 * BATCH_SIZE;

/// How often an idle leader checks the queue, and a follower the lease.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// How long after its creation a new index is left alone before the first
/// write, whichever relay holds the lease. On OpenSearch
/// Serverless a write sent within two seconds of creating an index that
/// replaces a deleted one can be acknowledged by the deleted index and lost,
/// under the same name or another, and requests can fail for ten seconds.
const NEW_INDEX_SETTLE: Duration = Duration::from_secs(30);

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
    /// OpenSearch throttled a write (HTTP 429). Not a failure: back off.
    #[error("index write throttled")]
    Throttled,
}

/// What a drain indexed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainReport {
    /// Bulk requests whose rows were completed.
    pub batches: u64,
    /// Documents written.
    pub written: u64,
    /// Documents removed.
    pub removed: u64,
    /// Items that failed; their rows stay queued.
    pub failed: u64,
}

impl DrainReport {
    fn add(&mut self, other: DrainReport) {
        self.batches += other.batches;
        self.written += other.written;
        self.removed += other.removed;
        self.failed += other.failed;
    }
}

/// Moves queued events from `search_outbox` into a [`SearchIndex`].
#[derive(Debug, Clone)]
pub struct Indexer {
    db: Db,
    index: SearchIndex,
    batch_size: i64,
    backfill_bound: i64,
    workers: usize,
    new_index_settle: Duration,
}

impl Indexer {
    /// An indexer reading the outbox from `db` and writing `index`.
    pub fn new(db: Db, index: SearchIndex) -> Self {
        Self {
            db,
            index,
            batch_size: BATCH_SIZE,
            backfill_bound: BACKFILL_OUTBOX_BOUND,
            workers: 1,
            new_index_settle: NEW_INDEX_SETTLE,
        }
    }

    /// Leave a new index alone for `settle` after its creation instead of
    /// [`NEW_INDEX_SETTLE`]. For tests on a cluster that needs no wait.
    pub fn with_new_index_settle(mut self, settle: Duration) -> Self {
        self.new_index_settle = settle;
        self
    }

    /// Run `workers` (at least one) batches at once when draining, of one
    /// community or several. This also bounds the bulk requests in flight.
    pub fn with_workers(mut self, workers: usize) -> Self {
        self.workers = workers.max(1);
        self
    }

    /// Claim at most `rows` outbox rows per bulk request, and backfill as
    /// many event keys per step (at least one).
    pub fn with_batch_size(mut self, rows: i64) -> Self {
        self.batch_size = rows.max(1);
        self
    }

    /// Backfill only while the outbox holds fewer than `rows` rows.
    pub fn with_backfill_bound(mut self, rows: i64) -> Self {
        self.backfill_bound = rows;
        self
    }

    /// Queue backfill batches, one community at a time in turn, while the
    /// outbox holds fewer rows than the backfill bound. Returns the number of
    /// event keys queued: zero when every community's backfill is complete
    /// or the outbox is full.
    ///
    /// Call it only on the lease holder, as [`Indexer::run`] does, so a
    /// fleet queues each stored event once.
    pub async fn backfill(&self) -> Result<u64, IndexerError> {
        let pool = self.db.pool();
        let mut queued = 0u64;
        // With several workers, queue two batches per worker, so each has
        // full batches and the serial backfill runs less often.
        let bound = if self.workers > 1 {
            self.backfill_bound
                .max(2 * self.workers as i64 * self.batch_size)
        } else {
            self.backfill_bound
        };
        while search_backfill::outbox_below(pool, bound).await? {
            let Some(step) = search_backfill::step(pool, self.batch_size, &[]).await? else {
                break;
            };
            let community = step.community.to_string();
            metrics::counter!("buzz_search_backfill_queued_total", "community" => community.clone())
                .increment(step.queued);
            if let Some(cursor) = step.cursor {
                metrics::gauge!("buzz_search_backfill_cursor_timestamp_seconds", "community" => community.clone())
                    .set(cursor.timestamp() as f64);
            }
            metrics::gauge!("buzz_search_backfill_complete", "community" => community)
                .set(if step.completed { 1.0 } else { 0.0 });
            if step.completed {
                info!(community = %step.community, queued = step.total_queued, "search backfill complete");
            } else {
                debug!(community = %step.community, queued = step.queued, cursor = ?step.cursor, "search backfill step");
            }
            queued += step.queued;
        }
        Ok(queued)
    }

    /// Index batches with the configured number of workers until none is
    /// left to claim. A community whose batch had a failed item is skipped
    /// for the rest of the pass by that worker, unless every failure was a
    /// refused item and the rest of the batch succeeded. Throttling stops every worker
    /// and returns [`IndexerError::Throttled`]; another failure ends its
    /// worker's drain and is returned once the others are done. Items the
    /// cluster refuses for good are logged and counted, not returned.
    ///
    /// Ends with an info log of the documents written and removed and the
    /// elapsed time, for measuring throughput.
    pub async fn drain(&self) -> Result<DrainReport, IndexerError> {
        let started = Instant::now();
        let stop = Arc::new(AtomicBool::new(false));
        let mut report = DrainReport::default();
        let mut result = Ok(());
        let mut keep = |worker_report: DrainReport, outcome: Result<(), IndexerError>| {
            report.add(worker_report);
            // Keep the first failure; it outranks throttling.
            if let Err(error) = outcome {
                if !matches!(
                    result,
                    Err(IndexerError::Outbox(_) | IndexerError::Index(_))
                ) {
                    result = Err(error);
                }
            }
        };
        if self.workers == 1 {
            let mut worker_report = DrainReport::default();
            let outcome = self.drain_worker(&mut worker_report, &stop).await;
            keep(worker_report, outcome);
        } else {
            let mut workers = tokio::task::JoinSet::new();
            for _ in 0..self.workers {
                let (indexer, stop) = (self.clone(), Arc::clone(&stop));
                workers.spawn(async move {
                    let mut worker_report = DrainReport::default();
                    let outcome = indexer.drain_worker(&mut worker_report, &stop).await;
                    (worker_report, outcome)
                });
            }
            while let Some(joined) = workers.join_next().await {
                match joined {
                    Ok((worker_report, outcome)) => keep(worker_report, outcome),
                    Err(error) => keep(
                        DrainReport::default(),
                        Err(SearchError::Engine(format!("indexer worker ended: {error}")).into()),
                    ),
                }
            }
        }
        if report.batches > 0 {
            let elapsed = started.elapsed();
            let indexed = report.written + report.removed;
            info!(
                workers = self.workers,
                batches = report.batches,
                written = report.written,
                removed = report.removed,
                failed = report.failed,
                elapsed_ms = elapsed.as_millis() as u64,
                documents_per_second =
                    (indexed as f64 / elapsed.as_secs_f64().max(1e-3)).round() as u64,
                "search indexer drained the outbox"
            );
        }
        result.map(|()| report)
    }

    /// One worker's share of a drain.
    async fn drain_worker(
        &self,
        report: &mut DrainReport,
        stop: &AtomicBool,
    ) -> Result<(), IndexerError> {
        let mut skipped: Vec<CommunityId> = Vec::new();
        let mut failure = None;
        while !stop.load(Ordering::Relaxed) {
            let Some(batch) = search_outbox::claim_batch_share(
                self.db.pool(),
                self.batch_size,
                &skipped,
                self.workers,
            )
            .await?
            else {
                break;
            };
            let community = batch.community();
            let outcome = match self
                .index
                .write(community, &batch.documents, &batch.removed, batch.version)
                .await
            {
                Ok(outcome) => outcome,
                Err(error @ SearchError::Rejected(_)) => {
                    // The whole request was refused; its rows stay queued.
                    warn!(%community, error = %error.kind(), "search index rejected a batch; its rows stay queued");
                    metrics::counter!("buzz_search_index_rejected_total", "community" => community.to_string())
                        .increment((batch.documents.len() + batch.removed.len()) as u64);
                    skipped.push(community);
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let items = (batch.documents.len() + batch.removed.len()) as u64;
            let written = batch
                .documents
                .iter()
                .filter(|document| !outcome.failed.contains(&document.event_id))
                .count() as u64;
            let removed = batch
                .removed
                .iter()
                .filter(|event| !outcome.failed.contains(event))
                .count() as u64;
            batch.complete_except(&outcome.failed).await?;
            let label = community.to_string();
            metrics::counter!("buzz_search_indexed_documents_total", "community" => label.clone(), "op" => "write")
                .increment(written);
            metrics::counter!("buzz_search_indexed_documents_total", "community" => label.clone(), "op" => "remove")
                .increment(removed);
            report.batches += 1;
            report.written += written;
            report.removed += removed;
            if outcome.failed.is_empty() {
                continue;
            }
            report.failed += outcome.failed.len() as u64;
            let reason = outcome.reason.as_deref().unwrap_or("unknown");
            if outcome.throttled {
                stop.store(true, Ordering::Relaxed);
                return Err(IndexerError::Throttled);
            }
            if outcome.rejected > 0 {
                metrics::counter!("buzz_search_index_rejected_total", "community" => label.clone())
                    .increment(outcome.rejected as u64);
            }
            if outcome.conflicts > 0 {
                // This batch is its events' newest writer, so the index can
                // hold a newer version only if the version sequence handed
                // out a value again, as after a Postgres crash or promoting
                // an asynchronous replica. Keep the rows: the retry carries a later version
                // once the sequence passes the one held.
                error!(%community, items = outcome.conflicts, "search index holds a newer version than the batch writing it; the version sequence went back; its rows stay queued");
                metrics::counter!("buzz_search_index_version_conflicts_total", "community" => label)
                    .increment(outcome.conflicts as u64);
            }
            if outcome.rejected == outcome.failed.len() {
                warn!(%community, items = outcome.rejected, reason, "search index rejected documents; their rows stay queued");
                // Refused items are claimed again with the next batch; keep
                // going while the rest of each batch succeeds.
                if written + removed == 0 && items > 0 {
                    skipped.push(community);
                }
            } else {
                skipped.push(community);
                failure.get_or_insert(SearchError::Engine(format!(
                    "{} bulk items failed ({reason})",
                    outcome.failed.len()
                )));
            }
        }
        failure.map_or(Ok(()), |error| Err(error.into()))
    }

    /// Set `buzz_search_backfill_complete` for every community from stored
    /// progress. A backfill step sets it only for the community it advanced,
    /// so a relay that takes the lease after a backfill finished would never
    /// report it.
    async fn publish_backfill_progress(&self) {
        match search_backfill::progress(self.db.pool()).await {
            Ok(progress) => {
                for (community, completed) in progress {
                    metrics::gauge!("buzz_search_backfill_complete", "community" => community.to_string())
                        .set(if completed { 1.0 } else { 0.0 });
                }
            }
            Err(error) => warn!(%error, "could not read search backfill progress"),
        }
    }

    /// One leader pass: make sure the index exists, top the outbox up with
    /// backfill, then drain. Checking the index every pass (one request when
    /// the alias exists) recreates an index that went away, and creating it
    /// here rather than at startup means an unreachable cluster never delays
    /// serving. Backfilling before draining keeps it going while a rejected
    /// batch's rows stay queued. Returns whether backfill queued anything, in
    /// which case the next pass should follow at once.
    ///
    /// `settled` is false until this lease holder has made sure the index is
    /// at least the settle time old.
    async fn pass(&self, settled: &mut bool) -> Result<bool, IndexerError> {
        let mut created = false;
        if self.index.missing().await? {
            // A new index is empty, whatever the stored progress says (the
            // old one was deleted or lost): backfill every community. Clear
            // the progress before creating the index, so a crash between the
            // two leaves the index missing and the next pass clears again.
            let reset = search_backfill::reset(self.db.pool()).await?;
            info!(
                communities = reset,
                "search index missing; backfilling every community again"
            );
            // Also when no row was left to remove, as after a crash before
            // the create, so no community still reports a complete backfill.
            self.publish_backfill_progress().await;
            *settled = false;
            created = self.index.create().await?;
        }
        if created {
            tokio::time::sleep(self.new_index_settle).await;
        } else if !*settled {
            // The relay that created the index may have lost the lease before
            // its wait ended. Without the creation time, wait the whole time.
            let age = self.index.age().await.unwrap_or_else(|error| {
                warn!(error = %error.kind(), "could not read when the search index was created");
                Duration::ZERO
            });
            tokio::time::sleep(self.new_index_settle.saturating_sub(age)).await;
        }
        *settled = true;
        let backfilled = self.backfill().await?;
        self.drain().await?;
        Ok(backfilled > 0)
    }

    /// Hold the indexer lease and keep the index up to date, forever. Abort
    /// the task to stop; that closes the lease connection and lets another
    /// instance take over.
    ///
    /// Every wait is jittered, so instances restarted together spread their
    /// lease attempts instead of polling Postgres in step. Failed lease
    /// attempts and failed passes back off up to [`MAX_BACKOFF`]; so does
    /// throttling, which is counted and logged apart from failures.
    pub async fn run(self) {
        tokio::time::sleep(POLL_INTERVAL.mul_f64(rand::random::<f64>())).await;
        let mut leader: Option<buzz_db::SessionLeader> = None;
        let mut settled = false;
        let mut failures = 0u32;
        let mut throttled = 0u32;
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
                        settled = false;
                        failures = 0;
                        self.publish_backfill_progress().await;
                    }
                    Ok(None) => failures = 0,
                    Err(error) => {
                        failures = failures.saturating_add(1);
                        warn!(%error, failures, "search indexer lease attempt failed");
                    }
                }
            }
            let mut backfilling = false;
            if leader.is_some() {
                match self.pass(&mut settled).await {
                    Ok(more) => {
                        failures = 0;
                        throttled = 0;
                        backfilling = more;
                    }
                    Err(IndexerError::Throttled) => {
                        throttled = throttled.saturating_add(1);
                        metrics::counter!("buzz_search_indexer_throttled_total").increment(1);
                        info!(throttled, "search index throttled the indexer; backing off");
                    }
                    Err(error) => {
                        failures = failures.saturating_add(1);
                        warn!(%error, failures, "search indexing failed; rows stay queued");
                    }
                }
            }
            if backfilling {
                // Next backfill batch now, after re-checking the lease.
                tokio::task::yield_now().await;
                continue;
            }
            tokio::time::sleep(backoff(failures.max(throttled))).await;
        }
    }
}

/// The wait before the next attempt: the poll interval, doubled per
/// consecutive failure up to [`MAX_BACKOFF`], scaled by a random factor
/// between one half and one.
fn backoff(failures: u32) -> Duration {
    let ceiling = POLL_INTERVAL
        .saturating_mul(1 << failures.min(4))
        .min(MAX_BACKOFF);
    ceiling.mul_f64(0.5 + rand::random::<f64>() / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_to_a_cap_with_jitter() {
        let idle: Vec<Duration> = (0..64).map(|_| backoff(0)).collect();
        assert!(idle
            .iter()
            .all(|wait| *wait >= POLL_INTERVAL / 2 && *wait <= POLL_INTERVAL));
        assert!(
            idle.iter().any(|wait| *wait != idle[0]),
            "idle waits are jittered: {idle:?}"
        );
        for failures in 1..10 {
            let wait = backoff(failures);
            assert!(wait <= MAX_BACKOFF, "{failures}: {wait:?}");
            assert!(wait >= POLL_INTERVAL, "{failures}: {wait:?}");
        }
        assert!(backoff(8) >= MAX_BACKOFF / 2);
    }
}
