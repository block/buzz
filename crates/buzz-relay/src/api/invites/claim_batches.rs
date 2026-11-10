use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use buzz_core::{CommunityId, TenantContext};
use buzz_db::relay_invite::{
    ClaimOutcome, ClaimRequest, ClaimResult, EnqueuedClaim, ProcessedInviteBatch,
    ProcessedInviteClaim, StoredClaimResult, INVITE_CLAIM_TTL,
};
use tokio::sync::{oneshot, Mutex, Notify, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use uuid::Uuid;

use crate::handlers::side_effects::{
    auto_join_default_channels_batch, fan_out_nip43_membership_list, join_effect_concurrency,
    publish_nip43_member_added,
};
use crate::state::AppState;

const MAX_PENDING: usize = 2_048;
const MAX_WORKERS_PER_PHASE: usize = 8;
const ENQUEUE_BUDGET: Duration = Duration::from_secs(2);
const RESULT_POLL_INTERVAL: Duration = Duration::from_millis(50);
// Read committed outcomes after admission expiry, including a stopped processor.
const RESULT_READ_GRACE: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub(super) enum ClaimError {
    Unavailable,
    Database(Arc<buzz_db::DbError>),
}

struct PendingClaim {
    id: Uuid,
    reply: oneshot::Sender<Result<ClaimResult, ClaimError>>,
    deadline: Instant,
    _permit: OwnedSemaphorePermit,
}

#[derive(Default)]
struct LocalQueue {
    pending: VecDeque<PendingClaim>,
    wake: Arc<Notify>,
}

/// Local waiters for the database's shared, ordered invitation queue.
/// Only active requests poll, and each community has one poller per process.
pub(crate) struct ClaimBatches {
    queues: Mutex<HashMap<CommunityId, LocalQueue>>,
    pending: Arc<Semaphore>,
    admission_workers: Arc<Semaphore>,
    follow_up_workers: Arc<Semaphore>,
}

impl Default for ClaimBatches {
    fn default() -> Self {
        Self {
            queues: Mutex::default(),
            pending: Arc::new(Semaphore::new(MAX_PENDING)),
            admission_workers: Arc::new(Semaphore::new(MAX_WORKERS_PER_PHASE)),
            follow_up_workers: Arc::new(Semaphore::new(MAX_WORKERS_PER_PHASE)),
        }
    }
}

impl ClaimBatches {
    pub(super) async fn submit(
        self: &Arc<Self>,
        state: &Arc<AppState>,
        tenant: &TenantContext,
        request: ClaimRequest,
    ) -> Result<ClaimResult, ClaimError> {
        let permit = Arc::clone(&self.pending)
            .try_acquire_owned()
            .map_err(|_| ClaimError::Unavailable)?;
        let queued = tokio::time::timeout(
            ENQUEUE_BUDGET,
            state.db.enqueue_invite_claim(tenant.community(), &request),
        )
        .await
        .map_err(|_| ClaimError::Unavailable)?
        .map_err(|error| ClaimError::Database(Arc::new(error)))?;
        let id = match queued {
            EnqueuedClaim::Invalid => return Ok(ClaimResult::V2(ClaimOutcome::Invalid.into())),
            EnqueuedClaim::Full => return Err(ClaimError::Unavailable),
            EnqueuedClaim::Queued(id) => id,
        };
        let (reply, response) = oneshot::channel();
        let mut queues = self.queues.lock().await;
        let start_worker = !queues.contains_key(&tenant.community());
        let queue = queues.entry(tenant.community()).or_default();
        queue.pending.push_back(PendingClaim {
            id,
            reply,
            deadline: Instant::now() + INVITE_CLAIM_TTL + RESULT_READ_GRACE,
            _permit: permit,
        });
        queue.wake.notify_one();
        if start_worker {
            let (batches, state, tenant) = (Arc::clone(self), Arc::clone(state), tenant.clone());
            tokio::spawn(async move { batches.run(state, tenant).await });
        }
        drop(queues);
        response.await.map_err(|_| ClaimError::Unavailable)?
    }

    async fn run(self: Arc<Self>, state: Arc<AppState>, tenant: TenantContext) {
        let mut active = Vec::new();
        let mut admission: Option<tokio::task::JoinHandle<()>> = None;
        let mut follow_up: Option<tokio::task::JoinHandle<()>> = None;
        loop {
            let wake = {
                let mut queues = self.queues.lock().await;
                let Some(queue) = queues.get_mut(&tenant.community()) else {
                    return;
                };
                active.extend(queue.pending.drain(..));
                if active.is_empty() {
                    queues.remove(&tenant.community());
                    return;
                }
                Arc::clone(&queue.wake)
            };
            let now = Instant::now();
            let mut live = Vec::with_capacity(active.len());
            for job in active.drain(..) {
                if job.deadline <= now {
                    let _ = job.reply.send(Err(ClaimError::Unavailable));
                } else {
                    live.push(job);
                }
            }
            active = live;
            let Some(deadline) = active.iter().map(|job| job.deadline).min() else {
                continue;
            };
            let ids: Vec<_> = active.iter().map(|job| job.id).collect();
            // Polling stays independent of a processor working on another batch.
            match tokio::time::timeout_at(
                deadline,
                state.db.invite_claim_results(tenant.community(), &ids),
            )
            .await
            {
                Ok(Ok(completed)) => {
                    let mut results: HashMap<_, _> = completed
                        .into_iter()
                        .map(|claim| (claim.id, claim.result))
                        .collect();
                    let mut waiting = Vec::new();
                    let mut consumed = Vec::new();
                    for job in active.drain(..) {
                        if let Some(result) = results.remove(&job.id) {
                            let _ = job.reply.send(claim_result(result));
                            consumed.push(job.id);
                        } else {
                            waiting.push(job);
                        }
                    }
                    active = waiting;
                    if !consumed.is_empty() {
                        // Cleanup is best effort; retained receipts remain bounded
                        // by the normal expiry sweep after errors or cancellation.
                        let _ = tokio::time::timeout(
                            RESULT_POLL_INTERVAL,
                            state
                                .db
                                .acknowledge_invite_claim_results(tenant.community(), &consumed),
                        )
                        .await;
                    }
                }
                Ok(Err(error)) => {
                    tracing::warn!(error_kind = %error.kind(), "invite result read failed");
                }
                Err(_) => {}
            }
            for (worker, slots, phase) in [
                (
                    &mut admission,
                    &self.admission_workers,
                    ClaimPhase::Admission,
                ),
                (
                    &mut follow_up,
                    &self.follow_up_workers,
                    ClaimPhase::FollowUp,
                ),
            ] {
                if worker.as_ref().is_some_and(|task| task.is_finished()) {
                    if let Some(task) = worker.take() {
                        if let Err(error) = task.await {
                            tracing::error!(?phase, error = %error, "invite processor task failed");
                        }
                    }
                }
                if !active.is_empty() && worker.is_none() {
                    if let Ok(permit) = Arc::clone(slots).try_acquire_owned() {
                        let (state, tenant) = (Arc::clone(&state), tenant.clone());
                        *worker = Some(tokio::spawn(async move {
                            let _permit = permit;
                            // Includes pool acquisition before the batch supplies its deadline.
                            let operation = phase.process(&state, &tenant);
                            match tokio::time::timeout(INVITE_CLAIM_TTL + ENQUEUE_BUDGET, operation)
                                .await
                            {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => {
                                    tracing::warn!(?phase, error_kind = %error.kind(), "invite processor attempt failed")
                                }
                                Err(_) => tracing::warn!(
                                    ?phase,
                                    "invite processor timed out; committed work remains retryable"
                                ),
                            }
                        }));
                    }
                }
            }
            if !active.is_empty() {
                tokio::select! {
                    _ = wake.notified() => {},
                    _ = tokio::time::sleep(RESULT_POLL_INTERVAL) => {},
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ClaimPhase {
    Admission,
    FollowUp,
}

impl ClaimPhase {
    async fn process(self, state: &Arc<AppState>, tenant: &TenantContext) -> buzz_db::Result<()> {
        match self {
            Self::Admission => {
                if let Some(write) = state
                    .db
                    .try_admit_invite_claims(tenant.community(), &state.relay_keypair)
                    .await?
                {
                    announce_committed_admission(state, tenant, write).await;
                }
            }
            Self::FollowUp => {
                if let Some(batch) = state
                    .db
                    .try_finish_invite_claims(tenant.community())
                    .await?
                {
                    finish_committed_batch(state, tenant, batch).await;
                }
            }
        }
        Ok(())
    }
}

pub(super) async fn announce_committed_admission(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    write: buzz_db::relay_members::MembershipWrite<Vec<ProcessedInviteClaim>>,
) {
    metrics::histogram!("buzz_invite_claim_batch_size").record(write.outcome.len() as f64);
    for claim in &write.outcome {
        let (users_created, owner_link_created) = match &claim.result {
            StoredClaimResult::Success(ClaimResult::V2(result)) => {
                (result.users_created, result.owner_link_created)
            }
            StoredClaimResult::Success(ClaimResult::Legacy(result)) => {
                (result.users_created, result.owner_link_created)
            }
            _ => continue,
        };
        metrics::counter!("buzz_users_created_total", "community" => tenant.host().to_owned())
            .increment(users_created);
        if owner_link_created {
            state.disconnect_unowned_agent_clusterwide(tenant, &claim.pubkey);
        }
    }
    let started = Instant::now();
    if tokio::time::timeout(
        INVITE_CLAIM_TTL,
        fan_out_nip43_membership_list(tenant, state, write.snapshot),
    )
    .await
    .is_err()
    {
        tracing::warn!("committed community roster delivery timed out");
    }
    tracing::debug!(
        members = write.outcome.len(),
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "community roster delivered"
    );
}

pub(super) async fn finish_committed_batch(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    batch: ProcessedInviteBatch,
) {
    let deadline = Instant::from_std(batch.deadline);
    let mut members = Vec::new();
    let mut added = Vec::new();
    for claim in &batch.claims {
        let (joined, admitted, owner_link_created) = match &claim.result {
            StoredClaimResult::Success(ClaimResult::V2(result)) => (
                matches!(result.outcome, ClaimOutcome::Joined { .. }),
                matches!(
                    result.outcome,
                    ClaimOutcome::Joined { .. } | ClaimOutcome::AlreadyMember { .. }
                ),
                result.owner_link_created,
            ),
            StoredClaimResult::Success(ClaimResult::Legacy(result)) => {
                (result.inserted, true, result.owner_link_created)
            }
            _ => continue,
        };
        if owner_link_created {
            state.disconnect_unowned_agent_clusterwide(tenant, &claim.pubkey);
        }
        let pubkey = hex::encode(claim.pubkey);
        if joined {
            added.push(pubkey.clone());
        }
        if admitted {
            members.push(pubkey);
        }
    }
    let follow_up = async {
        use futures_util::stream::{self, StreamExt};
        let started = std::time::Instant::now();
        let (task_state, task_tenant) = (Arc::clone(state), tenant.clone());
        stream::iter(added)
            .for_each_concurrent(join_effect_concurrency(state), move |pubkey| {
                let (state, tenant) = (Arc::clone(&task_state), task_tenant.clone());
                async move {
                    if let Err(error) = publish_nip43_member_added(&tenant, &state, &pubkey).await {
                        tracing::warn!(error = %error, "member-added announcement failed after claim");
                    }
                }
            })
            .await;
        tracing::debug!(
            members = batch.claims.len(),
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "member-added announcements finished"
        );
        auto_join_default_channels_batch(tenant, state, &members).await;
        // Completion releases the guard so the next follow-up batch can start.
        batch.complete().await
    };
    match tokio::time::timeout_at(deadline, follow_up).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(error_kind = %error.kind(), "invite completion remains retryable")
        }
        Err(_) => {
            tracing::warn!("invite follow-up timed out; default-channel work remains retryable");
            metrics::counter!("buzz_auto_join_failures_total").increment(1);
        }
    }
}

fn claim_result(result: StoredClaimResult) -> Result<ClaimResult, ClaimError> {
    match result {
        StoredClaimResult::Success(result) => Ok(result),
        StoredClaimResult::OwnerMismatch => Err(ClaimError::Database(Arc::new(
            buzz_db::DbError::InviteOwnerMismatch,
        ))),
        StoredClaimResult::Restricted => Err(ClaimError::Database(Arc::new(
            buzz_db::DbError::InviteRestricted,
        ))),
        StoredClaimResult::Expired => Err(ClaimError::Unavailable),
    }
}
