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
                    result = tokio::time::timeout(ENQUEUE_TIMEOUT, enqueue(&pool, &job, #[cfg(test)] None)) => result,
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

#[cfg(test)]
type GateBarrier = (
    tokio::sync::oneshot::Sender<i32>,
    tokio::sync::oneshot::Receiver<()>,
);

/// Pause the real enqueue transaction after it acquires the production gate.
#[cfg(test)]
pub(crate) async fn enqueue_with_gate_barrier(
    pool: &PgPool,
    community: CommunityId,
    event_id: Vec<u8>,
    barrier: GateBarrier,
) -> crate::Result<()> {
    enqueue(
        pool,
        &Job {
            community,
            event_id,
            barrier: None,
        },
        Some(barrier),
    )
    .await
}

async fn enqueue(
    pool: &PgPool,
    job: &Job,
    #[cfg(test)] gate_barrier: Option<GateBarrier>,
) -> crate::Result<()> {
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
    #[cfg(test)]
    if let Some((held, release)) = gate_barrier {
        let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await?;
        held.send(pid)
            .expect("race test observes enqueue transaction");
        release
            .await
            .expect("race test releases enqueue transaction");
    }
    // Compare recorded receipt/update times without imposing commit ordering.
    // Registration overlapping message arrival may send or suppress a wake.
    // This bounded job never scans historical messages on lease activation.
    sqlx::query(
        "INSERT INTO push_match_queue (community_id, event_id) \
         SELECT $1, $2 FROM events e \
         WHERE e.community_id = $1 AND e.id = $2 AND e.deleted_at IS NULL \
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

    mod postgres_tests {
        use super::*;

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn enqueue_admits_active_tenant_and_rejects_quiescing_tenant() {
            let (db, communities) = crate::test_support::admission_fixture().await;
            crate::test_support::reject_legacy_sql_fence_predicate(&db.pool).await;
            for (index, community) in communities[..2].iter().enumerate() {
                sqlx::query(
                    "INSERT INTO events \
                     (community_id, id, pubkey, created_at, kind, tags, content, sig) \
                     VALUES ($1, $2, $3, to_timestamp(1), 9, '[]', '', $4)",
                )
                .bind(community.as_uuid())
                .bind(vec![index as u8; 32])
                .bind(vec![1_u8; 32])
                .bind(vec![2_u8; 64])
                .execute(&db.pool)
                .await
                .expect("seed event");
                sqlx::query(
                    "INSERT INTO push_leases (community_id, author, installation_id, \
                     source_event_id, source_created_at, generation, active, endpoint_hash, \
                     endpoint_grant, max_class, subscriptions, expires_at, updated_at) \
                     VALUES ($1, $2, 'test', $3, 1, 1, true, $4, 'grant', 'default', '[]', \
                             EXTRACT(EPOCH FROM now())::bigint + 3600, now() - interval '1 hour')",
                )
                .bind(community.as_uuid())
                .bind(vec![3_u8; 32])
                .bind(vec![4_u8; 32])
                .bind(vec![5_u8; 32])
                .execute(&db.pool)
                .await
                .expect("seed lease");
            }
            let active = Job {
                community: communities[0],
                event_id: vec![0_u8; 32],
                barrier: None,
            };
            let quiescing = Job {
                community: communities[1],
                event_id: vec![1_u8; 32],
                barrier: None,
            };
            let err = enqueue(&db.pool, &quiescing, None)
                .await
                .expect_err("quiescing enqueue");
            assert!(crate::test_support::is_admission_rejection(&err));
            enqueue(&db.pool, &active, None)
                .await
                .expect("admitted enqueue");
            for (community, expected) in [(communities[0], 1), (communities[1], 0)] {
                let count: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM push_match_queue WHERE community_id = $1",
                )
                .bind(community.as_uuid())
                .fetch_one(&db.pool)
                .await
                .expect("read enqueue state");
                assert_eq!(count, expected, "queue state for {community}");
            }
        }
    }
}
