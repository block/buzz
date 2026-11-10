//! Stage records, handler outcomes, and atomic transition persistence.
use super::leases::*;
use super::*;
use std::time::Duration;

/// A reached stage. Retries will update the same row, not append attempts.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct EntityStage {
    /// Stable stage-history row identifier.
    pub id: Uuid,
    /// Parent workflow; application transactions enforce this relationship.
    pub request_id: Uuid,
    /// Community provenance, or null for a request-wide stage.
    pub target_community_id: Option<Uuid>,
    /// Action to dispatch.
    pub stage: String,
    /// `pending`, `running`, `failed`, or `completed`.
    pub status: String,
    /// Whether this is the workflow's resume position.
    pub is_current: bool,
    /// First execution time; pending stages have not started.
    pub started_at: Option<DateTime<Utc>>,
    /// Successful completion time.
    pub finished_at: Option<DateTime<Utc>>,
    /// Stage-specific saved progress.
    pub metadata_json: serde_json::Value,
    /// Most recent failure, if any.
    pub last_error: Option<String>,
    /// Last stage update.
    pub updated_at: DateTime<Utc>,
}

impl EntityDeletionStore {
    /// Start the supplied current stage, preserving its first start time on retry.
    pub async fn start_stage(&self, lease: &EntityLease, stage_id: Uuid) -> Result<EntityStage> {
        let mut tx = self.pool.begin().await?;
        let request = lock_owned(&mut tx, lease).await?;
        validate_approved_inventory(&mut tx, &request).await?;
        let stage = current_stage(&mut tx, lease.request_id).await?;
        if stage.id != stage_id
            || !matches!(stage.status.as_str(), "pending" | "running" | "failed")
        {
            return Err(DbError::DeletionSafety(
                "stage is no longer executable".into(),
            ));
        }
        let due: bool = sqlx::query_scalar(
            "SELECT next_attempt_at IS NULL OR next_attempt_at <= clock_timestamp()
             FROM entity_deletion_requests WHERE id = $1",
        )
        .bind(lease.request_id)
        .fetch_one(&mut *tx)
        .await?;
        if !due {
            return Err(DbError::DeletionSafety("stage retry is not due".into()));
        }
        sqlx::query("UPDATE entity_deletion_requests SET status = 'running', next_attempt_at = NULL WHERE id = $1")
            .bind(lease.request_id).execute(&mut *tx).await?;
        let stage = sqlx::query_as(
            "UPDATE entity_deletion_stage_history
             SET status = 'running', started_at = COALESCE(started_at, clock_timestamp()),
                 updated_at = clock_timestamp()
             WHERE id = $1
             RETURNING id, request_id, target_community_id, stage, status, is_current,
                       started_at, finished_at, metadata_json, last_error, updated_at",
        )
        .bind(stage.id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(stage)
    }

    /// Record failure and its retry deadline; get_next_stage returns WaitUntil.
    /// Ownership is released by the worker after it observes that outcome.
    pub async fn fail_stage(
        &self,
        lease: &EntityLease,
        stage_id: Uuid,
        message: &str,
        retry_delay: Duration,
    ) -> Result<()> {
        let millis = duration_millis(retry_delay)?;
        let message: String = message.chars().take(2048).collect();
        let mut tx = self.pool.begin().await?;
        lock_owned(&mut tx, lease).await?;
        require_running_stage(&mut tx, lease, stage_id).await?;
        sqlx::query(
            "UPDATE entity_deletion_stage_history
             SET status = 'failed', last_error = $2, updated_at = clock_timestamp()
             WHERE id = $1",
        )
        .bind(stage_id)
        .bind(&message)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE entity_deletion_requests
             SET status = 'retry_wait', retry_count = retry_count + 1, last_error = $2,
                 next_attempt_at = clock_timestamp() + $3 * interval '1 millisecond',
                 updated_at = clock_timestamp()
             WHERE id = $1",
        )
        .bind(lease.request_id)
        .bind(message)
        .bind(millis)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Save a handler's result without switching stages. A completed current row
    /// is the durable journal for a transition that has not committed yet.
    pub async fn save_stage_success(
        &self,
        lease: &EntityLease,
        stage_id: Uuid,
        result: serde_json::Value,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let request = lock_owned(&mut tx, lease).await?;
        validate_approved_inventory(&mut tx, &request).await?;
        require_running_stage(&mut tx, lease, stage_id).await?;
        sqlx::query(
            "UPDATE entity_deletion_stage_history SET status = 'completed',
             finished_at = clock_timestamp(), last_error = NULL,
             metadata_json = jsonb_set(metadata_json - 'waiting', '{result}', $2),
             updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(stage_id)
        .bind(result)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
pub(super) async fn current_stage(tx: &mut Tx<'_>, id: Uuid) -> Result<EntityStage> {
    let stage: EntityStage = sqlx::query_as(
        "SELECT id, request_id, target_community_id, stage, status, is_current,
                started_at, finished_at, metadata_json, last_error, updated_at
         FROM entity_deletion_stage_history WHERE request_id = $1 AND is_current",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| DbError::DeletionSafety("unfinished request has no current stage".into()))?;
    Ok(stage)
}

pub(super) async fn require_running_stage(
    tx: &mut Tx<'_>,
    lease: &EntityLease,
    stage_id: Uuid,
) -> Result<EntityStage> {
    let stage = current_stage(tx, lease.request_id).await?;
    if stage.id != stage_id || stage.status != "running" {
        return Err(DbError::DeletionSafety(
            "stage is no longer current and running".into(),
        ));
    }
    Ok(stage)
}

/// Inputs for a stage chosen by the caller's workflow policy.
pub struct NewEntityStage {
    /// Handler name; interpreted by the deletion worker, not the store.
    pub stage: String,
    /// Community provenance, absent for request-wide work.
    pub community: Option<Uuid>,
    /// Saved handler arguments.
    pub inputs: serde_json::Value,
}

/// A workflow decision applied while the request's database lock is held.
pub enum StageTransition {
    /// Create the first stage of an approved request.
    Initialize(NewEntityStage),
    /// Return the current stage, or its retry deadline if not yet due.
    Resume(Uuid),
    /// Retire a completed current stage and create its successor, or finish.
    Advance {
        /// The completed stage whose result was interpreted.
        previous: Uuid,
        /// Workflow-specific decision retained for recovery and audit.
        decision: serde_json::Value,
        /// None records successful request completion.
        successor: Option<NewEntityStage>,
    },
    /// Return an already completed request without changing it.
    Finished,
}

/// The saved position returned to a deletion worker.
#[derive(Debug)]
pub enum NextStage {
    /// Full saved stage row, including its inputs.
    Stage(EntityStage),
    /// Request completion has been saved.
    Finished,
    /// Saved retry deadline has not arrived.
    WaitUntil(DateTime<Utc>),
}

pub(super) fn invalid(message: &str) -> DbError {
    DbError::DeletionSafety(message.into())
}

impl EntityDeletionStore {
    /// Lock the request, let the caller decide from its saved state, and persist
    /// that decision atomically. The synchronous callback must only compute a
    /// decision; it must not perform external work. No stage policy lives here.
    pub async fn transition_stage<F>(
        &self,
        lease: &EntityLease,
        previous_stage_id: Option<Uuid>,
        decide: F,
    ) -> Result<NextStage>
    where
        F: FnOnce(&EntityRequest, &[EntityStage]) -> Result<StageTransition> + Send,
    {
        let mut tx = self.pool.begin().await?;
        let request = lock_owned(&mut tx, lease).await?;
        validate_approved_inventory(&mut tx, &request).await?;
        let history: Vec<EntityStage> = sqlx::query_as(
            "SELECT id, request_id, target_community_id, stage, status, is_current,
                    started_at, finished_at, metadata_json, last_error, updated_at
             FROM entity_deletion_stage_history WHERE request_id = $1 LIMIT 10001",
        )
        .bind(request.id)
        .fetch_all(&mut *tx)
        .await?;
        if history.len() > 10000 {
            return Err(invalid("workflow exceeds 10000 reached stages"));
        }
        if previous_stage_id.is_some_and(|id| !history.iter().any(|row| row.id == id)) {
            return Err(invalid("previous stage does not belong to this request"));
        }
        let current = history.iter().find(|row| row.is_current);
        let next = match decide(&request, &history)? {
            StageTransition::Finished => {
                if request.status != "completed" || current.is_some() {
                    return Err(invalid("request is not completed"));
                }
                return Ok(NextStage::Finished);
            }
            StageTransition::Initialize(stage) => {
                if request.status != "approved" || !history.is_empty() {
                    return Err(invalid(
                        "only an approved request without history can initialize",
                    ));
                }
                let row = insert_stage(&mut tx, request.id, None, stage).await?;
                sqlx::query(
                    "UPDATE entity_deletion_requests
                     SET status = 'running', updated_at = clock_timestamp() WHERE id = $1",
                )
                .bind(request.id)
                .execute(&mut *tx)
                .await?;
                NextStage::Stage(row)
            }
            StageTransition::Resume(id) => {
                let row = current
                    .filter(|row| row.id == id)
                    .ok_or_else(|| invalid("resume stage is not current"))?;
                match row.status.as_str() {
                    "pending" | "running" if request.status == "running" => {
                        return Ok(NextStage::Stage(row.clone()));
                    }
                    "failed" => {
                        let retry: Option<(DateTime<Utc>, bool)> = sqlx::query_as(
                            "SELECT next_attempt_at, next_attempt_at > clock_timestamp()
                             FROM entity_deletion_requests
                             WHERE id = $1 AND status = 'retry_wait' AND next_attempt_at IS NOT NULL",
                        )
                        .bind(request.id)
                        .fetch_optional(&mut *tx)
                        .await?;
                        let (deadline, waiting) =
                            retry.ok_or_else(|| invalid("failed stage has no retry record"))?;
                        return Ok(if waiting {
                            NextStage::WaitUntil(deadline)
                        } else {
                            NextStage::Stage(row.clone())
                        });
                    }
                    _ => return Err(invalid("current stage is not executable")),
                }
            }
            StageTransition::Advance {
                previous,
                decision,
                successor,
            } => {
                let row = current
                    .filter(|row| row.id == previous && row.status == "completed")
                    .ok_or_else(|| invalid("only a completed current stage can advance"))?;
                if request.status != "running" {
                    return Err(invalid("completed stage has inconsistent parent status"));
                }
                sqlx::query(
                    "UPDATE entity_deletion_stage_history SET is_current = false,
                     metadata_json = jsonb_set(metadata_json, '{transition}', $2),
                     updated_at = clock_timestamp() WHERE id = $1",
                )
                .bind(row.id)
                .bind(decision)
                .execute(&mut *tx)
                .await?;
                let next = match successor {
                    Some(stage) => NextStage::Stage(
                        insert_stage(&mut tx, request.id, Some(row.id), stage).await?,
                    ),
                    None => NextStage::Finished,
                };
                sqlx::query(
                    "UPDATE entity_deletion_requests
                     SET status = CASE WHEN $2 THEN 'completed' ELSE 'running' END,
                         completed_at = CASE WHEN $2 THEN clock_timestamp() ELSE NULL END,
                         retry_count = 0, next_attempt_at = NULL, last_error = NULL,
                         updated_at = clock_timestamp() WHERE id = $1",
                )
                .bind(request.id)
                .bind(matches!(next, NextStage::Finished))
                .execute(&mut *tx)
                .await?;
                next
            }
        };
        tx.commit().await?;
        Ok(next)
    }
}

async fn insert_stage(
    tx: &mut Tx<'_>,
    request_id: Uuid,
    previous: Option<Uuid>,
    stage: NewEntityStage,
) -> Result<EntityStage> {
    let metadata = serde_json::json!({"inputs": stage.inputs, "previous_stage_id": previous});
    Ok(sqlx::query_as(
        "INSERT INTO entity_deletion_stage_history (request_id, target_community_id, stage, metadata_json)
         VALUES ($1, $2, $3, $4)
         RETURNING id, request_id, target_community_id, stage, status, is_current,
                   started_at, finished_at, metadata_json, last_error, updated_at",
    )
    .bind(request_id)
    .bind(stage.community)
    .bind(stage.stage)
    .bind(metadata)
    .fetch_one(&mut **tx)
    .await?)
}

#[cfg(test)]
#[path = "stages_postgres_tests.rs"]
mod postgres_tests;
