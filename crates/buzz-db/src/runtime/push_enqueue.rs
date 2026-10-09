//! Bounded best-effort push production, strictly after event commit.

use std::sync::Arc;
use std::time::Duration;

use buzz_core::CommunityId;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(crate) const CAPACITY: usize = 256;
const ENQUEUE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub(crate) struct PushEnqueue {
    sender: mpsc::Sender<Job>,
    cancel: CancellationToken,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

#[derive(Debug)]
struct Job {
    community: CommunityId,
    event_id: Vec<u8>,
    #[cfg(test)]
    barrier: Option<tokio::sync::oneshot::Sender<()>>,
}

impl PushEnqueue {
    pub(crate) fn new(writer: &PgPool) -> Self {
        // No connection from the serving pool is ever consumed by push work.
        // This lazy pool cannot make relay startup depend on push availability.
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(ENQUEUE_TIMEOUT)
            .connect_lazy_with(
                (*writer.connect_options())
                    .clone()
                    .application_name("buzz-push-enqueue"),
            );
        let (sender, mut receiver) = mpsc::channel::<Job>(CAPACITY);
        let cancel = CancellationToken::new();
        let stopped = cancel.clone();
        let task = tokio::spawn(async move {
            loop {
                let job = tokio::select! {
                    biased;
                    _ = stopped.cancelled() => break,
                    job = receiver.recv() => match job {
                        Some(job) => job,
                        None => break,
                    },
                };
                #[cfg(test)]
                if let Some(barrier) = job.barrier {
                    let _ = barrier.send(());
                    continue;
                }
                let result = tokio::select! {
                    biased;
                    _ = stopped.cancelled() => {
                        record_drop("shutdown");
                        break;
                    },
                    result = tokio::time::timeout(ENQUEUE_TIMEOUT, enqueue(&pool, &job)) => result,
                };
                match result {
                    Ok(Ok(())) => {
                        metrics::counter!("buzz_push_enqueue_total", "result" => "completed")
                            .increment(1);
                    }
                    Ok(Err(error)) => {
                        record_drop("error");
                        tracing::warn!(%error, community = %job.community, "push enqueue failed after message commit");
                    }
                    Err(_) => record_drop("timeout"),
                }
            }
            receiver.close();
            while receiver.try_recv().is_ok() {
                record_drop("shutdown");
            }
            if tokio::time::timeout(ENQUEUE_TIMEOUT, pool.close())
                .await
                .is_err()
            {
                tracing::warn!("push enqueue pool close exceeded shutdown deadline");
            }
        });
        Self {
            sender,
            cancel,
            task: Arc::new(Mutex::new(Some(task))),
        }
    }

    pub(crate) fn submit(&self, community: CommunityId, event_id: Vec<u8>) {
        if self.cancel.is_cancelled() {
            record_drop("shutdown");
            return;
        }
        if let Err(error) = self.sender.try_send(Job {
            community,
            event_id,
            #[cfg(test)]
            barrier: None,
        }) {
            record_drop(match error {
                mpsc::error::TrySendError::Full(_) => "full",
                mpsc::error::TrySendError::Closed(_) => "closed",
            });
        }
    }

    #[cfg(test)]
    pub(crate) async fn flush(&self) {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.sender
            .send(Job {
                community: CommunityId::from_uuid(uuid::Uuid::nil()),
                event_id: Vec::new(),
                barrier: Some(sender),
            })
            .await
            .expect("producer open");
        receiver.await.expect("producer completed preceding work");
    }

    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
    }

    pub(crate) async fn join(&self) {
        if let Some(task) = self.task.lock().await.take() {
            if let Err(error) = task.await {
                tracing::warn!(%error, "push enqueue worker failed");
            }
        }
    }
}

pub(crate) fn record_drop(reason: &'static str) {
    metrics::counter!("buzz_push_enqueue_total", "result" => reason).increment(1);
}

async fn enqueue(pool: &PgPool, job: &Job) -> crate::Result<()> {
    let mut tx = pool.begin().await?;
    // Server-side deadlines also bound work if the client disconnects or the
    // task is cancelled. These apply only to the dedicated push connection.
    sqlx::query("SELECT set_config('lock_timeout', '500ms', true), set_config('statement_timeout', '1500ms', true)")
        .execute(&mut *tx).await?;
    crate::deletion::DeletionStore::new(pool.clone())
        .guard_transaction(&mut tx, job.community)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
        .bind(crate::push::push_gate_lock_key(job.community))
        .execute(&mut *tx)
        .await?;
    // Compare recorded receipt/update times without imposing commit ordering.
    // Registration overlapping message arrival may send or suppress a wake.
    // This bounded job never scans historical messages on lease activation.
    sqlx::query(
        "INSERT INTO push_match_queue (community_id, event_id) \
         SELECT $1, $2 FROM events e \
         WHERE e.community_id = $1 AND e.id = $2 AND e.deleted_at IS NULL \
           AND community_write_allowed($1) \
           AND EXISTS (SELECT 1 FROM push_leases l \
             WHERE l.community_id = $1 AND l.active AND l.endpoint_enabled \
               AND l.updated_at <= e.received_at \
               AND l.expires_at > EXTRACT(EPOCH FROM now())::bigint) \
         ON CONFLICT DO NOTHING",
    )
    .bind(job.community.as_uuid())
    .bind(&job.event_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn saturation_and_shutdown_never_wait_for_a_receiver() {
        let (sender, mut receiver) = mpsc::channel(CAPACITY);
        let producer = PushEnqueue {
            sender,
            cancel: CancellationToken::new(),
            task: Arc::new(Mutex::new(None)),
        };
        let community = CommunityId::from_uuid(uuid::Uuid::new_v4());
        for _ in 0..CAPACITY + 1 {
            producer.submit(community, vec![1; 32]);
        }
        assert_eq!(receiver.len(), CAPACITY);
        receiver.try_recv().unwrap();
        producer.cancel();
        producer.submit(community, vec![2; 32]);
        assert_eq!(receiver.len(), CAPACITY - 1);
        producer.join().await;
    }
}
