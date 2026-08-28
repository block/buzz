//! Durable state for multi-agent workflow runs.
//!
//! Extends legacy workflow runs with phase CAS, idempotent tasks and artifacts,
//! resumable checkpoints, and an append-only transition ledger.

use crate::error::{DbError, Result};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::{fmt, str::FromStr};
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 1_000;

/// Durable state of an agent task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    /// Created but not dispatched.
    Pending,
    /// Assigned to an agent.
    Assigned,
    /// Agent is actively working.
    Running,
    /// Waiting for an external dependency.
    Waiting,
    /// Eligible for retry after its delay.
    RetryScheduled,
    /// Finished with validated output.
    Completed,
    /// Finished unsuccessfully.
    Failed,
    /// Explicitly cancelled.
    Cancelled,
    /// Cannot proceed without intervention.
    Blocked,
}

impl fmt::Display for AgentTaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pending => "pending",
            Self::Assigned => "assigned",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::RetryScheduled => "retry_scheduled",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
        })
    }
}

impl FromStr for AgentTaskStatus {
    type Err = DbError;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "assigned" => Ok(Self::Assigned),
            "running" => Ok(Self::Running),
            "waiting" => Ok(Self::Waiting),
            "retry_scheduled" => Ok(Self::RetryScheduled),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "blocked" => Ok(Self::Blocked),
            other => Err(DbError::InvalidData(format!(
                "unknown agent task status: {other}"
            ))),
        }
    }
}

/// Phase and optimistic-lock state attached to a workflow run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRunState {
    /// Parent workflow run.
    pub run_id: Uuid,
    /// Current workflow-defined phase.
    pub phase: String,
    /// Optimistic-lock version.
    pub state_version: i64,
    /// Immutable manifest SHA-256.
    pub manifest_hash: Option<Vec<u8>>,
    /// Channel thread root event id.
    pub thread_root_event_id: Option<Vec<u8>>,
    /// Optional run deadline.
    pub deadline: Option<DateTime<Utc>>,
    /// Workflow-specific metadata.
    pub metadata: Value,
    /// Last update.
    pub updated_at: DateTime<Utc>,
}

/// Parameters used to initialize durable run state.
pub struct EnsureAgentRunState<'a> {
    /// Parent workflow run.
    pub run_id: Uuid,
    /// Initial phase.
    pub phase: &'a str,
    /// Optional manifest hash.
    pub manifest_hash: Option<&'a [u8]>,
    /// Optional thread root event id.
    pub thread_root_event_id: Option<&'a [u8]>,
    /// Optional deadline.
    pub deadline: Option<DateTime<Utc>>,
    /// Initial metadata.
    pub metadata: &'a Value,
}

/// Durable agent task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTask {
    /// Task id.
    pub id: Uuid,
    /// Parent run.
    pub run_id: Uuid,
    /// Stable workflow task key.
    pub task_key: String,
    /// Workflow phase.
    pub phase: String,
    /// Assigned Nostr pubkey.
    pub agent_pubkey: Option<Vec<u8>>,
    /// Current status.
    pub status: AgentTaskStatus,
    /// Current attempt number.
    pub attempt: i32,
    /// Maximum attempts.
    pub max_attempts: i32,
    /// Structured input.
    pub input: Value,
    /// Optional output schema.
    pub output_schema: Option<Value>,
    /// Stable idempotency key.
    pub idempotency_key: String,
    /// Optional parent task.
    pub parent_task_id: Option<Uuid>,
    /// JSON array of dependency keys.
    pub depends_on: Value,
    /// Earliest claim time.
    pub not_before: Option<DateTime<Utc>>,
    /// Start time.
    pub started_at: Option<DateTime<Utc>>,
    /// Completion time.
    pub completed_at: Option<DateTime<Utc>>,
    /// Machine-readable failure code.
    pub error_code: Option<String>,
    /// Human-readable failure detail.
    pub error_message: Option<String>,
    /// Optimistic-lock version.
    pub version: i64,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last update time.
    pub updated_at: DateTime<Utc>,
}

/// Parameters for idempotently creating a task.
pub struct CreateAgentTask<'a> {
    /// Parent run.
    pub run_id: Uuid,
    /// Stable task key.
    pub task_key: &'a str,
    /// Workflow phase.
    pub phase: &'a str,
    /// Optional assignee pubkey.
    pub agent_pubkey: Option<&'a [u8]>,
    /// Maximum attempts.
    pub max_attempts: i32,
    /// Structured input.
    pub input: &'a Value,
    /// Optional output schema.
    pub output_schema: Option<&'a Value>,
    /// Stable idempotency key.
    pub idempotency_key: &'a str,
    /// Optional parent task.
    pub parent_task_id: Option<Uuid>,
    /// JSON array of dependency keys.
    pub depends_on: &'a Value,
}

/// Versioned output or evidence produced by a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentArtifact {
    /// Artifact id.
    pub id: Uuid,
    /// Parent run.
    pub run_id: Uuid,
    /// Producing task.
    pub task_id: Option<Uuid>,
    /// Artifact kind.
    pub kind: String,
    /// Artifact version.
    pub version: i32,
    /// Media type.
    pub content_type: String,
    /// External immutable location.
    pub uri: Option<String>,
    /// SHA-256 digest.
    pub sha256: Vec<u8>,
    /// Small inline payload.
    pub inline_content: Option<Value>,
    /// Additional metadata.
    pub metadata: Value,
    /// Producing Nostr pubkey.
    pub created_by: Option<Vec<u8>>,
    /// Stable idempotency key.
    pub idempotency_key: String,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// Parameters for idempotently storing an artifact.
pub struct CreateAgentArtifact<'a> {
    /// Parent run.
    pub run_id: Uuid,
    /// Producing task.
    pub task_id: Option<Uuid>,
    /// Artifact kind.
    pub kind: &'a str,
    /// Artifact version.
    pub version: i32,
    /// Media type.
    pub content_type: &'a str,
    /// External location.
    pub uri: Option<&'a str>,
    /// SHA-256 digest.
    pub sha256: &'a [u8],
    /// Small inline payload.
    pub inline_content: Option<&'a Value>,
    /// Metadata.
    pub metadata: &'a Value,
    /// Producing pubkey.
    pub created_by: Option<&'a [u8]>,
    /// Stable idempotency key.
    pub idempotency_key: &'a str,
}

/// Resumable task checkpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentCheckpoint {
    /// Checkpoint id.
    pub id: Uuid,
    /// Parent run.
    pub run_id: Uuid,
    /// Parent task.
    pub task_id: Uuid,
    /// Monotonic sequence.
    pub sequence: i64,
    /// Structured resume state.
    pub state: Value,
    /// Optional artifact snapshot.
    pub artifact_id: Option<Uuid>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// Append-only run transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRunTransition {
    /// Transition id.
    pub id: Uuid,
    /// Parent run.
    pub run_id: Uuid,
    /// Monotonic sequence.
    pub sequence: i64,
    /// Previous phase.
    pub from_phase: Option<String>,
    /// New phase.
    pub to_phase: String,
    /// Previous status.
    pub from_status: Option<String>,
    /// New status.
    pub to_status: String,
    /// Optional reason.
    pub reason: Option<String>,
    /// Actor pubkey.
    pub actor_pubkey: Option<Vec<u8>>,
    /// Structured metadata.
    pub metadata: Value,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// Ensure durable state exists, returning the existing row on replay.
pub async fn ensure_run_state(
    pool: &PgPool,
    community: CommunityId,
    p: EnsureAgentRunState<'_>,
) -> Result<AgentRunState> {
    sqlx::query("INSERT INTO workflow_run_state (community_id,run_id,phase,manifest_hash,thread_root_event_id,deadline,metadata) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (community_id,run_id) DO NOTHING")
        .bind(community.as_uuid()).bind(p.run_id).bind(p.phase).bind(p.manifest_hash)
        .bind(p.thread_root_event_id).bind(p.deadline).bind(p.metadata).execute(pool).await?;
    get_run_state(pool, community, p.run_id)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("workflow run state {}", p.run_id)))
}

/// Fetch durable state for a workflow run.
pub async fn get_run_state(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
) -> Result<Option<AgentRunState>> {
    sqlx::query("SELECT run_id,phase,state_version,manifest_hash,thread_root_event_id,deadline,metadata,updated_at FROM workflow_run_state WHERE community_id=$1 AND run_id=$2")
        .bind(community.as_uuid()).bind(run_id).fetch_optional(pool).await?.map(map_run_state).transpose()
}

/// Compare-and-swap the phase and metadata of a run.
pub async fn cas_run_state(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    expected: i64,
    phase: &str,
    metadata: &Value,
    deadline: Option<DateTime<Utc>>,
) -> Result<Option<AgentRunState>> {
    sqlx::query("UPDATE workflow_run_state SET phase=$4,metadata=$5,deadline=$6,state_version=state_version+1,updated_at=NOW() WHERE community_id=$1 AND run_id=$2 AND state_version=$3 RETURNING run_id,phase,state_version,manifest_hash,thread_root_event_id,deadline,metadata,updated_at")
        .bind(community.as_uuid()).bind(run_id).bind(expected).bind(phase).bind(metadata).bind(deadline)
        .fetch_optional(pool).await?.map(map_run_state).transpose()
}

/// Idempotently create an agent task.
pub async fn create_task(
    pool: &PgPool,
    community: CommunityId,
    p: CreateAgentTask<'_>,
) -> Result<AgentTask> {
    map_task(
        sqlx::query(
            "INSERT INTO workflow_run_tasks \
             (community_id,run_id,task_key,phase,agent_pubkey,max_attempts,input,output_schema,\
              idempotency_key,parent_task_id,depends_on) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) \
             ON CONFLICT (community_id,run_id,idempotency_key) DO UPDATE \
             SET idempotency_key=EXCLUDED.idempotency_key \
             RETURNING id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,\
              output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,\
              completed_at,error_code,error_message,version,created_at,updated_at",
        )
        .bind(community.as_uuid())
        .bind(p.run_id)
        .bind(p.task_key)
        .bind(p.phase)
        .bind(p.agent_pubkey)
        .bind(p.max_attempts)
        .bind(p.input)
        .bind(p.output_schema)
        .bind(p.idempotency_key)
        .bind(p.parent_task_id)
        .bind(p.depends_on)
        .fetch_one(pool)
        .await?,
    )
}

/// List tasks for a run in creation order.
pub async fn list_tasks(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    limit: Option<i64>,
) -> Result<Vec<AgentTask>> {
    sqlx::query(
        "SELECT id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,\
         output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,\
         completed_at,error_code,error_message,version,created_at,updated_at \
         FROM workflow_run_tasks WHERE community_id=$1 AND run_id=$2 \
         ORDER BY created_at,id LIMIT $3",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind(bounded(limit))
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(map_task)
    .collect()
}

/// Fetch one task by id within its tenant.
pub async fn get_task(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
) -> Result<Option<AgentTask>> {
    sqlx::query(
        "SELECT id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,\
         output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,\
         completed_at,error_code,error_message,version,created_at,updated_at \
         FROM workflow_run_tasks WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .fetch_optional(pool)
    .await?
    .map(map_task)
    .transpose()
}

/// Claim a task using optimistic locking.
pub async fn claim_task(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected: i64,
    agent: &[u8],
) -> Result<Option<AgentTask>> {
    sqlx::query(
        "UPDATE workflow_run_tasks AS task \
         SET status='running',agent_pubkey=$4,attempt=attempt+1,\
         started_at=NOW(),version=version+1,updated_at=NOW() \
         WHERE task.community_id=$1 AND task.id=$2 AND task.version=$3 \
           AND task.status IN ('pending','assigned','retry_scheduled') \
           AND (task.not_before IS NULL OR task.not_before<=NOW()) \
           AND task.attempt<task.max_attempts \
           AND NOT EXISTS (\
             SELECT 1 FROM jsonb_array_elements_text(task.depends_on) dependency(task_key) \
             LEFT JOIN workflow_run_tasks prerequisite \
               ON prerequisite.community_id=task.community_id \
              AND prerequisite.run_id=task.run_id \
              AND prerequisite.task_key=dependency.task_key \
             WHERE prerequisite.id IS NULL OR prerequisite.status<>'completed'\
           ) \
         RETURNING id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,\
          output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,\
          completed_at,error_code,error_message,version,created_at,updated_at",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .bind(expected)
    .bind(agent)
    .fetch_optional(pool)
    .await?
    .map(map_task)
    .transpose()
}

/// Atomically complete an eligible ingestion task, persist its immutable
/// manifest artifact, and bind the manifest hash to run state.
#[allow(clippy::too_many_arguments)]
pub async fn complete_ingestion_task(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected: i64,
    kind: &str,
    artifact_sha256: &[u8],
    manifest_hash: &[u8],
    inline_content: &Value,
    metadata: &Value,
    idempotency_key: &str,
) -> Result<Option<(AgentTask, AgentArtifact)>> {
    let mut transaction = pool.begin().await?;
    let Some(task_row) = sqlx::query(
        "UPDATE workflow_run_tasks AS task \
         SET status='completed',completed_at=NOW(),version=version+1,updated_at=NOW() \
         WHERE task.community_id=$1 AND task.id=$2 AND task.version=$3 \
           AND task.status='pending' \
           AND NOT EXISTS (\
             SELECT 1 FROM jsonb_array_elements_text(task.depends_on) dependency(task_key) \
             LEFT JOIN workflow_run_tasks prerequisite \
               ON prerequisite.community_id=task.community_id \
              AND prerequisite.run_id=task.run_id \
              AND prerequisite.task_key=dependency.task_key \
             WHERE prerequisite.id IS NULL OR prerequisite.status<>'completed'\
           ) \
         RETURNING id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,\
          output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,\
          completed_at,error_code,error_message,version,created_at,updated_at",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .bind(expected)
    .fetch_optional(&mut *transaction)
    .await?
    else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let task = map_task(task_row)?;
    let artifact_row = sqlx::query(
        "INSERT INTO workflow_run_artifacts \
         (community_id,run_id,task_id,kind,version,content_type,uri,sha256,inline_content,metadata,created_by,idempotency_key) \
         VALUES ($1,$2,$3,$4,1,'application/json',NULL,$5,$6,$7,NULL,$8) \
         ON CONFLICT (community_id,run_id,idempotency_key) DO UPDATE \
         SET idempotency_key=EXCLUDED.idempotency_key \
         RETURNING id,run_id,task_id,kind,version,content_type,uri,sha256,inline_content,metadata,created_by,idempotency_key,created_at",
    )
    .bind(community.as_uuid())
    .bind(task.run_id)
    .bind(task.id)
    .bind(kind)
    .bind(artifact_sha256)
    .bind(inline_content)
    .bind(metadata)
    .bind(idempotency_key)
    .fetch_one(&mut *transaction)
    .await?;
    let artifact = map_artifact(artifact_row)?;
    if artifact.task_id != Some(task.id)
        || artifact.kind != kind
        || artifact.version != 1
        || artifact.content_type != "application/json"
        || artifact.uri.is_some()
        || artifact.sha256 != artifact_sha256
        || artifact.inline_content.as_ref() != Some(inline_content)
        || artifact.metadata != *metadata
        || artifact.created_by.is_some()
    {
        transaction.rollback().await?;
        return Err(DbError::InvalidData(
            "ingestion artifact conflicts with persisted blueprint".into(),
        ));
    }
    let state_updated = sqlx::query(
        "UPDATE workflow_run_state SET manifest_hash=$3,updated_at=NOW() \
         WHERE community_id=$1 AND run_id=$2 \
           AND (manifest_hash IS NULL OR manifest_hash=$3)",
    )
    .bind(community.as_uuid())
    .bind(task.run_id)
    .bind(manifest_hash)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if state_updated != 1 {
        transaction.rollback().await?;
        return Err(DbError::InvalidData(
            "document manifest hash conflicts with durable run state".into(),
        ));
    }
    transaction.commit().await?;
    Ok(Some((task, artifact)))
}

/// Defer an eligible coordinator task after a transient adapter failure.
pub async fn defer_ready_task(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected: i64,
    not_before: DateTime<Utc>,
    code: &str,
    message: &str,
) -> Result<Option<AgentTask>> {
    sqlx::query(
        "UPDATE workflow_run_tasks AS task \
         SET attempt=attempt+1,\
             status=CASE WHEN attempt+1>=max_attempts THEN 'failed' ELSE 'retry_scheduled' END,\
             not_before=CASE WHEN attempt+1>=max_attempts THEN NULL ELSE $4 END,\
             error_code=$5,error_message=$6,\
             completed_at=CASE WHEN attempt+1>=max_attempts THEN NOW() ELSE NULL END,\
             version=version+1,updated_at=NOW() \
         WHERE task.community_id=$1 AND task.id=$2 AND task.version=$3 \
           AND task.status IN ('pending','retry_scheduled') \
           AND (task.not_before IS NULL OR task.not_before<=NOW()) \
           AND NOT EXISTS (\
             SELECT 1 FROM jsonb_array_elements_text(task.depends_on) dependency(task_key) \
             LEFT JOIN workflow_run_tasks prerequisite \
               ON prerequisite.community_id=task.community_id \
              AND prerequisite.run_id=task.run_id \
              AND prerequisite.task_key=dependency.task_key \
             WHERE prerequisite.id IS NULL OR prerequisite.status<>'completed'\
           ) \
         RETURNING id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,\
          output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,\
          completed_at,error_code,error_message,version,created_at,updated_at",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .bind(expected)
    .bind(not_before)
    .bind(code)
    .bind(message)
    .fetch_optional(pool)
    .await?
    .map(map_task)
    .transpose()
}
/// Block an unsupported coordinator task only after every dependency completes.
pub async fn block_ready_task(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected: i64,
    code: &str,
    message: &str,
) -> Result<Option<AgentTask>> {
    sqlx::query(
        "UPDATE workflow_run_tasks AS task \
         SET status='blocked',error_code=$4,error_message=$5,completed_at=NOW(),\
             version=version+1,updated_at=NOW() \
         WHERE task.community_id=$1 AND task.id=$2 AND task.version=$3 \
           AND task.status='pending' \
           AND NOT EXISTS (\
             SELECT 1 FROM jsonb_array_elements_text(task.depends_on) dependency(task_key) \
             LEFT JOIN workflow_run_tasks prerequisite \
               ON prerequisite.community_id=task.community_id \
              AND prerequisite.run_id=task.run_id \
              AND prerequisite.task_key=dependency.task_key \
             WHERE prerequisite.id IS NULL OR prerequisite.status<>'completed'\
           ) \
         RETURNING id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,\
          output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,\
          completed_at,error_code,error_message,version,created_at,updated_at",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .bind(expected)
    .bind(code)
    .bind(message)
    .fetch_optional(pool)
    .await?
    .map(map_task)
    .transpose()
}

/// Complete a coordinator task only when every declared dependency is complete.
///
/// Dependency eligibility and the status transition are evaluated atomically in
/// one SQL statement, so concurrent workers cannot open a barrier early.
pub async fn complete_ready_task(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected: i64,
) -> Result<Option<AgentTask>> {
    sqlx::query(
        "UPDATE workflow_run_tasks AS task \
         SET status='completed',completed_at=NOW(),version=version+1,updated_at=NOW() \
         WHERE task.community_id=$1 AND task.id=$2 AND task.version=$3 \
           AND task.status IN ('pending','retry_scheduled') \
           AND (task.not_before IS NULL OR task.not_before<=NOW()) \
           AND NOT EXISTS (\
             SELECT 1 FROM jsonb_array_elements_text(task.depends_on) dependency(task_key) \
             LEFT JOIN workflow_run_tasks prerequisite \
               ON prerequisite.community_id=task.community_id \
              AND prerequisite.run_id=task.run_id \
              AND prerequisite.task_key=dependency.task_key \
             WHERE prerequisite.id IS NULL OR prerequisite.status<>'completed'\
           ) \
         RETURNING id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,\
          output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,\
          completed_at,error_code,error_message,version,created_at,updated_at",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .bind(expected)
    .fetch_optional(pool)
    .await?
    .map(map_task)
    .transpose()
}

/// Complete a task using optimistic locking.
pub async fn complete_task(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected: i64,
) -> Result<Option<AgentTask>> {
    update_terminal_task(pool, community, task_id, expected, "completed", None, None).await
}

/// Schedule a retry using optimistic locking without consuming another attempt.
pub async fn schedule_retry(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected: i64,
    not_before: DateTime<Utc>,
    code: &str,
    message: &str,
) -> Result<Option<AgentTask>> {
    sqlx::query(
        "UPDATE workflow_run_tasks SET status='retry_scheduled',not_before=$4,\
         error_code=$5,error_message=$6,completed_at=NULL,version=version+1,updated_at=NOW() \
         WHERE community_id=$1 AND id=$2 AND version=$3 \
           AND status IN ('running','waiting') AND attempt<max_attempts \
         RETURNING id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,\
          output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,\
          completed_at,error_code,error_message,version,created_at,updated_at",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .bind(expected)
    .bind(not_before)
    .bind(code)
    .bind(message)
    .fetch_optional(pool)
    .await?
    .map(map_task)
    .transpose()
}

/// Fail or block a task using optimistic locking.
pub async fn fail_task(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected: i64,
    blocked: bool,
    code: &str,
    message: &str,
) -> Result<Option<AgentTask>> {
    update_terminal_task(
        pool,
        community,
        task_id,
        expected,
        if blocked { "blocked" } else { "failed" },
        Some(code),
        Some(message),
    )
    .await
}

async fn update_terminal_task(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    expected: i64,
    status: &str,
    code: Option<&str>,
    message: Option<&str>,
) -> Result<Option<AgentTask>> {
    sqlx::query(
        "UPDATE workflow_run_tasks SET status=$4,error_code=$5,error_message=$6,\
         completed_at=NOW(),version=version+1,updated_at=NOW() \
         WHERE community_id=$1 AND id=$2 AND version=$3 \
           AND status NOT IN ('completed','cancelled') \
         RETURNING id,run_id,task_key,phase,agent_pubkey,status,attempt,max_attempts,input,\
          output_schema,idempotency_key,parent_task_id,depends_on,not_before,started_at,\
          completed_at,error_code,error_message,version,created_at,updated_at",
    )
    .bind(community.as_uuid())
    .bind(task_id)
    .bind(expected)
    .bind(status)
    .bind(code)
    .bind(message)
    .fetch_optional(pool)
    .await?
    .map(map_task)
    .transpose()
}

/// Idempotently store a versioned artifact.
pub async fn create_artifact(
    pool: &PgPool,
    community: CommunityId,
    p: CreateAgentArtifact<'_>,
) -> Result<AgentArtifact> {
    map_artifact(sqlx::query("INSERT INTO workflow_run_artifacts (community_id,run_id,task_id,kind,version,content_type,uri,sha256,inline_content,metadata,created_by,idempotency_key) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT (community_id,run_id,idempotency_key) DO UPDATE SET idempotency_key=EXCLUDED.idempotency_key RETURNING id,run_id,task_id,kind,version,content_type,uri,sha256,inline_content,metadata,created_by,idempotency_key,created_at")
        .bind(community.as_uuid()).bind(p.run_id).bind(p.task_id).bind(p.kind).bind(p.version)
        .bind(p.content_type).bind(p.uri).bind(p.sha256).bind(p.inline_content).bind(p.metadata)
        .bind(p.created_by).bind(p.idempotency_key).fetch_one(pool).await?)
}

/// List artifacts for a run.
pub async fn list_artifacts(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    limit: Option<i64>,
) -> Result<Vec<AgentArtifact>> {
    sqlx::query("SELECT id,run_id,task_id,kind,version,content_type,uri,sha256,inline_content,metadata,created_by,idempotency_key,created_at FROM workflow_run_artifacts WHERE community_id=$1 AND run_id=$2 ORDER BY created_at,id LIMIT $3")
        .bind(community.as_uuid()).bind(run_id).bind(bounded(limit)).fetch_all(pool).await?
        .into_iter().map(map_artifact).collect()
}

/// Append an idempotent monotonic task checkpoint.
pub async fn append_checkpoint(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    task_id: Uuid,
    sequence: i64,
    state: &Value,
    artifact_id: Option<Uuid>,
) -> Result<AgentCheckpoint> {
    map_checkpoint(sqlx::query("INSERT INTO workflow_run_checkpoints (community_id,run_id,task_id,sequence,state,artifact_id) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (community_id,run_id,task_id,sequence) DO UPDATE SET sequence=EXCLUDED.sequence RETURNING id,run_id,task_id,sequence,state,artifact_id,created_at")
        .bind(community.as_uuid()).bind(run_id).bind(task_id).bind(sequence).bind(state).bind(artifact_id).fetch_one(pool).await?)
}

/// Fetch the latest checkpoint for a task.
pub async fn latest_checkpoint(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    task_id: Uuid,
) -> Result<Option<AgentCheckpoint>> {
    sqlx::query("SELECT id,run_id,task_id,sequence,state,artifact_id,created_at FROM workflow_run_checkpoints WHERE community_id=$1 AND run_id=$2 AND task_id=$3 ORDER BY sequence DESC LIMIT 1")
        .bind(community.as_uuid()).bind(run_id).bind(task_id).fetch_optional(pool).await?.map(map_checkpoint).transpose()
}

/// Append an idempotent run transition.
#[allow(clippy::too_many_arguments)]
pub async fn append_transition(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    sequence: i64,
    from_phase: Option<&str>,
    to_phase: &str,
    from_status: Option<&str>,
    to_status: &str,
    reason: Option<&str>,
    actor: Option<&[u8]>,
    metadata: &Value,
) -> Result<AgentRunTransition> {
    map_transition(sqlx::query("INSERT INTO workflow_run_transitions (community_id,run_id,sequence,from_phase,to_phase,from_status,to_status,reason,actor_pubkey,metadata) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT (community_id,run_id,sequence) DO UPDATE SET sequence=EXCLUDED.sequence RETURNING id,run_id,sequence,from_phase,to_phase,from_status,to_status,reason,actor_pubkey,metadata,created_at")
        .bind(community.as_uuid()).bind(run_id).bind(sequence).bind(from_phase).bind(to_phase)
        .bind(from_status).bind(to_status).bind(reason).bind(actor).bind(metadata).fetch_one(pool).await?)
}

/// Append the next monotonic run transition under a transaction-scoped lock.
///
/// Concurrent pods serialize by tenant and run before deriving the next
/// sequence, so every committed transition receives a unique increasing value.
#[allow(clippy::too_many_arguments)]
pub async fn append_next_transition(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    from_phase: Option<&str>,
    to_phase: &str,
    from_status: Option<&str>,
    to_status: &str,
    reason: Option<&str>,
    actor: Option<&[u8]>,
    metadata: &Value,
) -> Result<AgentRunTransition> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text || ':' || $2::text, 0))")
        .bind(community.as_uuid())
        .bind(run_id)
        .execute(&mut *transaction)
        .await?;
    let row = sqlx::query(
        "INSERT INTO workflow_run_transitions \
         (community_id,run_id,sequence,from_phase,to_phase,from_status,to_status,reason,actor_pubkey,metadata) \
         SELECT $1,$2,COALESCE(MAX(sequence),-1)+1,$3,$4,$5,$6,$7,$8,$9 \
         FROM workflow_run_transitions WHERE community_id=$1 AND run_id=$2 \
         RETURNING id,run_id,sequence,from_phase,to_phase,from_status,to_status,reason,actor_pubkey,metadata,created_at",
    )
    .bind(community.as_uuid())
    .bind(run_id)
    .bind(from_phase)
    .bind(to_phase)
    .bind(from_status)
    .bind(to_status)
    .bind(reason)
    .bind(actor)
    .bind(metadata)
    .fetch_one(&mut *transaction)
    .await?;
    let transition = map_transition(row)?;
    transaction.commit().await?;
    Ok(transition)
}

/// List transitions for a run.
pub async fn list_transitions(
    pool: &PgPool,
    community: CommunityId,
    run_id: Uuid,
    limit: Option<i64>,
) -> Result<Vec<AgentRunTransition>> {
    sqlx::query("SELECT id,run_id,sequence,from_phase,to_phase,from_status,to_status,reason,actor_pubkey,metadata,created_at FROM workflow_run_transitions WHERE community_id=$1 AND run_id=$2 ORDER BY sequence LIMIT $3")
        .bind(community.as_uuid()).bind(run_id).bind(bounded(limit)).fetch_all(pool).await?
        .into_iter().map(map_transition).collect()
}

fn bounded(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}
fn map_run_state(r: sqlx::postgres::PgRow) -> Result<AgentRunState> {
    Ok(AgentRunState {
        run_id: r.try_get("run_id")?,
        phase: r.try_get("phase")?,
        state_version: r.try_get("state_version")?,
        manifest_hash: r.try_get("manifest_hash")?,
        thread_root_event_id: r.try_get("thread_root_event_id")?,
        deadline: r.try_get("deadline")?,
        metadata: r.try_get("metadata")?,
        updated_at: r.try_get("updated_at")?,
    })
}
pub(crate) fn map_task(r: sqlx::postgres::PgRow) -> Result<AgentTask> {
    let s: String = r.try_get("status")?;
    Ok(AgentTask {
        id: r.try_get("id")?,
        run_id: r.try_get("run_id")?,
        task_key: r.try_get("task_key")?,
        phase: r.try_get("phase")?,
        agent_pubkey: r.try_get("agent_pubkey")?,
        status: s.parse()?,
        attempt: r.try_get("attempt")?,
        max_attempts: r.try_get("max_attempts")?,
        input: r.try_get("input")?,
        output_schema: r.try_get("output_schema")?,
        idempotency_key: r.try_get("idempotency_key")?,
        parent_task_id: r.try_get("parent_task_id")?,
        depends_on: r.try_get("depends_on")?,
        not_before: r.try_get("not_before")?,
        started_at: r.try_get("started_at")?,
        completed_at: r.try_get("completed_at")?,
        error_code: r.try_get("error_code")?,
        error_message: r.try_get("error_message")?,
        version: r.try_get("version")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}
pub(crate) fn map_artifact(r: sqlx::postgres::PgRow) -> Result<AgentArtifact> {
    Ok(AgentArtifact {
        id: r.try_get("id")?,
        run_id: r.try_get("run_id")?,
        task_id: r.try_get("task_id")?,
        kind: r.try_get("kind")?,
        version: r.try_get("version")?,
        content_type: r.try_get("content_type")?,
        uri: r.try_get("uri")?,
        sha256: r.try_get("sha256")?,
        inline_content: r.try_get("inline_content")?,
        metadata: r.try_get("metadata")?,
        created_by: r.try_get("created_by")?,
        idempotency_key: r.try_get("idempotency_key")?,
        created_at: r.try_get("created_at")?,
    })
}
pub(crate) fn map_checkpoint(r: sqlx::postgres::PgRow) -> Result<AgentCheckpoint> {
    Ok(AgentCheckpoint {
        id: r.try_get("id")?,
        run_id: r.try_get("run_id")?,
        task_id: r.try_get("task_id")?,
        sequence: r.try_get("sequence")?,
        state: r.try_get("state")?,
        artifact_id: r.try_get("artifact_id")?,
        created_at: r.try_get("created_at")?,
    })
}
pub(crate) fn map_transition(r: sqlx::postgres::PgRow) -> Result<AgentRunTransition> {
    Ok(AgentRunTransition {
        id: r.try_get("id")?,
        run_id: r.try_get("run_id")?,
        sequence: r.try_get("sequence")?,
        from_phase: r.try_get("from_phase")?,
        to_phase: r.try_get("to_phase")?,
        from_status: r.try_get("from_status")?,
        to_status: r.try_get("to_status")?,
        reason: r.try_get("reason")?,
        actor_pubkey: r.try_get("actor_pubkey")?,
        metadata: r.try_get("metadata")?,
        created_at: r.try_get("created_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_status_round_trips() {
        for status in [
            AgentTaskStatus::Pending,
            AgentTaskStatus::Assigned,
            AgentTaskStatus::Running,
            AgentTaskStatus::Waiting,
            AgentTaskStatus::RetryScheduled,
            AgentTaskStatus::Completed,
            AgentTaskStatus::Failed,
            AgentTaskStatus::Cancelled,
            AgentTaskStatus::Blocked,
        ] {
            let parsed = status.to_string().parse::<AgentTaskStatus>();
            assert!(matches!(parsed, Ok(value) if value == status));
        }
    }
    #[test]
    fn list_limit_is_bounded() {
        assert_eq!(bounded(None), DEFAULT_LIMIT);
        assert_eq!(bounded(Some(0)), 1);
        assert_eq!(bounded(Some(10_000)), MAX_LIMIT);
    }
}
