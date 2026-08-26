//! Workflow CRUD -- workflows, workflow_runs, and workflow_approvals tables.
//!
//! All IDs are native Postgres UUID columns. Never uses string interpolation
//! for query values -- all user data goes through bind parameters.
//!
//! Security notes:
//! - Approval tokens are stored as SHA-256 hashes (never plaintext).
//! - All list queries have a bounded LIMIT to prevent unbounded scans.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use nostr::{EventId, PublicKey};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use buzz_core::workflow_delivery::{
    WorkflowDeliveryBinding, WorkflowDeliveryCause, WorkflowDeliveryId,
};
use buzz_core::CommunityId;

use crate::error::{DbError, Result};

// -- Token hashing ------------------------------------------------------------

/// Default maximum rows returned by list queries. Callers may request fewer.
pub const LIST_DEFAULT_LIMIT: i64 = 100;
/// Hard cap on rows returned by list queries.
pub const LIST_MAX_LIMIT: i64 = 1000;

/// SHA-256 hash of a raw approval token. Returns the 32-byte digest.
///
/// Approval tokens are stored hashed so that a DB read does not expose
/// the raw token (same pattern as API tokens in buzz-auth).
fn hash_approval_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

// -- Status enums -------------------------------------------------------------

/// Status of a workflow definition. Stored as ENUM('active','disabled','archived').
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowStatus {
    /// Workflow is live and will fire on matching events.
    Active,
    /// Workflow is paused and will not fire.
    Disabled,
    /// Workflow has been retired.
    Archived,
}

impl fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkflowStatus::Active => write!(f, "active"),
            WorkflowStatus::Disabled => write!(f, "disabled"),
            WorkflowStatus::Archived => write!(f, "archived"),
        }
    }
}

impl FromStr for WorkflowStatus {
    type Err = DbError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "active" => Ok(WorkflowStatus::Active),
            "disabled" => Ok(WorkflowStatus::Disabled),
            "archived" => Ok(WorkflowStatus::Archived),
            other => Err(DbError::InvalidData(format!(
                "unknown workflow status: {other}"
            ))),
        }
    }
}

/// Status of a workflow run. Stored as ENUM in workflow_runs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Run is queued but not yet started.
    Pending,
    /// Run is actively executing steps.
    Running,
    /// Run is suspended waiting for an approval gate.
    WaitingApproval,
    /// Run finished successfully.
    Completed,
    /// Run terminated with an error.
    Failed,
    /// Run was cancelled before completion.
    Cancelled,
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunStatus::Pending => write!(f, "pending"),
            RunStatus::Running => write!(f, "running"),
            RunStatus::WaitingApproval => write!(f, "waiting_approval"),
            RunStatus::Completed => write!(f, "completed"),
            RunStatus::Failed => write!(f, "failed"),
            RunStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl FromStr for RunStatus {
    type Err = DbError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(RunStatus::Pending),
            "running" => Ok(RunStatus::Running),
            "waiting_approval" => Ok(RunStatus::WaitingApproval),
            "completed" => Ok(RunStatus::Completed),
            "failed" => Ok(RunStatus::Failed),
            "cancelled" => Ok(RunStatus::Cancelled),
            other => Err(DbError::InvalidData(format!("unknown run status: {other}"))),
        }
    }
}

/// Status of an approval request. Stored as ENUM in workflow_approvals.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalStatus {
    /// Approval has been requested but not yet acted on.
    Pending,
    /// Approval was granted; the run may proceed.
    Granted,
    /// Approval was denied; the run should fail.
    Denied,
    /// The approval window elapsed without a decision.
    Expired,
}

impl fmt::Display for ApprovalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApprovalStatus::Pending => write!(f, "pending"),
            ApprovalStatus::Granted => write!(f, "granted"),
            ApprovalStatus::Denied => write!(f, "denied"),
            ApprovalStatus::Expired => write!(f, "expired"),
        }
    }
}

impl FromStr for ApprovalStatus {
    type Err = DbError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(ApprovalStatus::Pending),
            "granted" => Ok(ApprovalStatus::Granted),
            "denied" => Ok(ApprovalStatus::Denied),
            "expired" => Ok(ApprovalStatus::Expired),
            other => Err(DbError::InvalidData(format!(
                "unknown approval status: {other}"
            ))),
        }
    }
}

// -- Record types -------------------------------------------------------------

/// A workflow definition record. Run-state columns live in `workflow_runs`.
#[derive(Debug, Clone)]
pub struct WorkflowRecord {
    /// Unique workflow identifier.
    pub id: Uuid,
    /// Server-resolved community that owns this workflow.
    pub community_id: CommunityId,
    /// Human-readable workflow name.
    pub name: String,
    /// Compressed public key bytes of the workflow owner.
    pub owner_pubkey: Vec<u8>,
    /// Channel this workflow is scoped to, if any.
    pub channel_id: Option<Uuid>,
    /// Canonical JSON of the workflow definition.
    pub definition: serde_json::Value,
    /// SHA-256 hash of the canonical definition JSON.
    pub definition_hash: Vec<u8>,
    /// Exact owner-signed kind:30620 event that materialized this revision.
    pub definition_event_id: Option<Vec<u8>>,
    /// Current lifecycle status of the workflow definition.
    pub status: WorkflowStatus,
    /// Whether the workflow will fire on matching events.
    pub enabled: bool,
    /// When the workflow was created.
    pub created_at: DateTime<Utc>,
    /// When the workflow was last updated.
    pub updated_at: DateTime<Utc>,
}

/// A single execution of a workflow.
#[derive(Debug, Clone)]
pub struct WorkflowRunRecord {
    /// Unique run identifier.
    pub id: Uuid,
    /// Server-resolved community this run (and its workflow) belongs to.
    ///
    /// `workflow_runs` is keyed `(community_id, id)`; the same run/workflow
    /// UUID is allowed across communities, so every run carries its owning
    /// community and downstream execution (side-effect sink, scoped lookups)
    /// runs under it rather than re-deriving a tenant from the deployment host.
    pub community_id: CommunityId,
    /// The workflow definition that was executed.
    pub workflow_id: Uuid,
    /// Exact owner-signed kind:30620 revision this run executes.
    ///
    /// NULL is retained only for runs created before the revision-binding
    /// migration; execution and resume paths must fail those rows closed.
    pub definition_event_id: Option<Vec<u8>>,
    /// Current execution status of this run.
    pub status: RunStatus,
    /// Raw event ID bytes that triggered this run, if any.
    pub trigger_event_id: Option<Vec<u8>>,
    /// Index of the step currently executing (0-based).
    pub current_step: i32,
    /// JSON execution trace -- one entry per completed step.
    pub execution_trace: serde_json::Value,
    /// Serialized `TriggerContext` captured at workflow start.
    /// NULL for runs created before this column was added (backwards-compatible).
    pub trigger_context: Option<serde_json::Value>,
    /// When execution began.
    pub started_at: Option<DateTime<Utc>>,
    /// When execution finished (success or failure).
    pub completed_at: Option<DateTime<Utc>>,
    /// Redacted human-readable diagnostic for failed or cancelled runs.
    pub error_message: Option<String>,
    /// Stable machine-readable failure or cancellation classification.
    /// Kept separate from `error_message` so callers never parse diagnostics.
    pub error_code: Option<String>,
    /// When the run record was created.
    pub created_at: DateTime<Utc>,
}

/// A winning scheduled workflow fire claim.
///
/// The primary identity is `(workflow_id, scheduled_for)`. `community_id` is
/// resolved from the workflow row inside the claim SQL and returned for scoped
/// audit/logging; callers never supply it as a claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledWorkflowFireClaim {
    /// Community that owns this scheduled fire.
    pub community_id: CommunityId,
    /// Workflow definition that should run.
    pub workflow_id: Uuid,
    /// Authoritative schedule instant this claim represents.
    pub scheduled_for: DateTime<Utc>,
    /// Database timestamp for when this pod won the claim.
    pub claimed_at: DateTime<Utc>,
}

/// A pending or resolved approval gate for a workflow step.
#[derive(Debug, Clone)]
pub struct ApprovalRecord {
    /// Token hash as stored in the DB (BYTEA).
    pub token: Vec<u8>,
    /// The workflow this approval belongs to.
    pub workflow_id: Uuid,
    /// The run waiting on this approval.
    pub run_id: Uuid,
    /// The step ID that requested approval.
    pub step_id: String,
    /// Zero-based index of the step in the workflow.
    pub step_index: i32,
    /// Who may approve (user mention or role spec).
    pub approver_spec: String,
    /// Current status of this approval request.
    pub status: ApprovalStatus,
    /// Compressed public key bytes of the user who acted on this approval.
    pub approver_pubkey: Option<Vec<u8>>,
    /// Optional note left by the approver.
    pub note: Option<String>,
    /// When this approval request expires.
    pub expires_at: DateTime<Utc>,
    /// When the approval record was created.
    pub created_at: DateTime<Utc>,
}

// -- Workflow CRUD ------------------------------------------------------------

/// Insert a new workflow record. Returns the new workflow's UUID.
/// New workflows start as `active` and `enabled = TRUE`.
///
/// NOTE: see the cache-invalidation note on [`update_workflow`]. The relay's
/// creation path is [`upsert_workflow`] via event ingest. (No current callers.)
pub async fn create_workflow(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Option<Uuid>,
    owner_pubkey: &[u8],
    name: &str,
    definition_json: &str,
    definition_hash: &[u8],
) -> Result<Uuid> {
    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO workflows
            (id, community_id, name, owner_pubkey, channel_id, definition, definition_hash, status, enabled)
        VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, 'active', TRUE)
        "#,
    )
    .bind(id)
    .bind(community_id.as_uuid())
    .bind(name)
    .bind(owner_pubkey)
    .bind(channel_id)
    .bind(definition_json)
    .bind(definition_hash)
    .execute(pool)
    .await?;

    Ok(id)
}

/// Insert or update a workflow at the caller-supplied NIP-33 `d`-tag UUID.
///
/// Updates are allowed only when the existing row has the same owner and
/// channel. That keeps a learned workflow UUID from becoming a cross-user or
/// cross-channel overwrite primitive while still making retries idempotent.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_workflow(
    conn: &mut sqlx::PgConnection,
    community_id: CommunityId,
    id: Uuid,
    channel_id: Option<Uuid>,
    owner_pubkey: &[u8],
    name: &str,
    definition_json: &str,
    definition_hash: &[u8],
    definition_event_id: &[u8],
) -> Result<()> {
    let row = sqlx::query(
        r#"
        INSERT INTO workflows
            (community_id, id, name, owner_pubkey, channel_id, definition, definition_hash, definition_event_id, status, enabled)
        VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8, 'active', TRUE)
        ON CONFLICT (community_id, id) DO UPDATE
        SET name = EXCLUDED.name,
            definition = EXCLUDED.definition,
            definition_hash = EXCLUDED.definition_hash,
            definition_event_id = EXCLUDED.definition_event_id,
            updated_at = NOW()
        WHERE workflows.owner_pubkey = EXCLUDED.owner_pubkey
          AND workflows.channel_id IS NOT DISTINCT FROM EXCLUDED.channel_id
        RETURNING id
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .bind(name)
    .bind(owner_pubkey)
    .bind(channel_id)
    .bind(definition_json)
    .bind(definition_hash)
    .bind(definition_event_id)
    .fetch_optional(conn)
    .await?;

    if row.is_none() {
        return Err(DbError::AccessDenied(format!(
            "workflow {id} belongs to a different owner or channel"
        )));
    }

    Ok(())
}

/// Fetch a single workflow by ID, scoped to its community.
///
/// `workflows` is keyed `(community_id, id)`; the same workflow UUID can exist
/// in two communities, so a request-scoped lookup must bind both. The caller
/// supplies the server-resolved community (host-bound tenant for request paths,
/// the run's own community for execution paths) — never a client-supplied id.
pub async fn get_workflow(
    pool: &PgPool,
    community_id: CommunityId,
    id: Uuid,
) -> Result<WorkflowRecord> {
    let row = sqlx::query(
        r#"
        SELECT id, community_id, name, owner_pubkey, channel_id, definition, definition_hash, definition_event_id,
               status::text AS status, enabled, created_at, updated_at
        FROM workflows
        WHERE community_id = $1 AND id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("workflow {id}")))?;

    row_to_workflow_record(row)
}

/// Fetch and lock one workflow on the caller's transaction.
///
/// The shared row lock is held through commit. Definition replacement takes an
/// update lock on the same row, so callers can validate an exact revision and
/// create dependent rows without a replacement committing between those steps.
pub async fn get_workflow_for_share_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community_id: CommunityId,
    id: Uuid,
) -> Result<WorkflowRecord> {
    let row = sqlx::query(
        r#"
        SELECT id, community_id, name, owner_pubkey, channel_id, definition, definition_hash, definition_event_id,
               status::text AS status, enabled, created_at, updated_at
        FROM workflows
        WHERE community_id = $1 AND id = $2
        FOR SHARE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("workflow {id}")))?;

    row_to_workflow_record(row)
}

/// List workflows for a channel, ordered newest first.
///
/// `limit` is capped at [`LIST_MAX_LIMIT`]. Pass `None` to use [`LIST_DEFAULT_LIMIT`].
/// `offset` enables pagination (0-based row offset).
pub async fn list_channel_workflows(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<WorkflowRecord>> {
    let limit = limit.unwrap_or(LIST_DEFAULT_LIMIT).clamp(1, LIST_MAX_LIMIT);
    let offset = offset.unwrap_or(0).max(0);

    let rows = sqlx::query(
        r#"
        SELECT id, community_id, name, owner_pubkey, channel_id, definition, definition_hash, definition_event_id,
               status::text AS status, enabled, created_at, updated_at
        FROM workflows
        WHERE community_id = $1 AND channel_id = $2
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_workflow_record).collect()
}

/// List active, enabled workflows for a channel.
/// Used by the trigger-matching path to find workflows that should fire.
/// Only returns workflows with status = 'active' AND enabled = TRUE.
///
/// Bounded to [`LIST_MAX_LIMIT`] rows -- the trigger path should not process
/// an unbounded number of workflows per event.
pub async fn list_enabled_channel_workflows(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<Vec<WorkflowRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, community_id, name, owner_pubkey, channel_id, definition, definition_hash, definition_event_id,
               status::text AS status, enabled, created_at, updated_at
        FROM workflows
        WHERE community_id = $1
          AND channel_id = $2
          AND status = 'active'
          AND enabled = TRUE
        ORDER BY created_at DESC
        LIMIT $3
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(LIST_MAX_LIMIT)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_workflow_record).collect()
}

/// List all active, enabled workflows with a `schedule` trigger across all channels.
///
/// Used by the cron scheduler. Filters by trigger type in SQL to avoid loading
/// event-triggered workflows that the cron loop would immediately discard.
/// Results are bounded to [`LIST_MAX_LIMIT`] rows.
pub async fn list_all_enabled_workflows(pool: &PgPool) -> Result<Vec<WorkflowRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT w.id, w.community_id, w.name, w.owner_pubkey, w.channel_id, w.definition, w.definition_hash, w.definition_event_id,
               w.status::text AS status, w.enabled, w.created_at, w.updated_at
        FROM workflows w
        JOIN communities c ON c.id = w.community_id
        WHERE w.status = 'active'
          AND w.enabled = TRUE
          AND w.definition->'trigger'->>'on' = 'schedule'
          AND c.archived_at IS NULL
        ORDER BY w.created_at ASC
        LIMIT $1
        "#,
    )
    .bind(LIST_MAX_LIMIT)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_workflow_record).collect()
}

/// Claim a scheduled workflow fire for an authoritative schedule instant.
///
/// Returns `Some` only for the first pod that claims `(community_id,
/// workflow_id, scheduled_for)`. All other pods receive `None` and must skip
/// creating a workflow run. The `scheduled_for` value must come from an
/// external schedule anchor (cron expression) or DB-authoritative interval
/// anchor; a per-pod in-memory timestamp is not safe because different pods
/// can compute different claim keys.
///
/// `community_id` is server provenance — for the global scheduler scan it is
/// the `workflow.community_id` returned by [`list_all_enabled_workflows`], not
/// any client-supplied value. It is required because `workflows` is keyed
/// `(community_id, id)`: duplicate workflow UUIDs across communities are
/// allowed, so resolving the owning community from `id` alone is ambiguous and
/// would fan a single claim across every community holding that UUID. Binding
/// `(community_id, id)` confines the claim — and its `SELECT`/`INSERT` row — to
/// exactly the intended tenant.
pub async fn claim_scheduled_workflow_fire(
    pool: &PgPool,
    community_id: CommunityId,
    workflow_id: Uuid,
    scheduled_for: DateTime<Utc>,
) -> Result<Option<ScheduledWorkflowFireClaim>> {
    let row = sqlx::query(
        r#"
        INSERT INTO scheduled_workflow_fires (community_id, workflow_id, scheduled_for)
        SELECT w.community_id, w.id, $3
        FROM workflows w
        WHERE w.community_id = $1 AND w.id = $2
        ON CONFLICT (community_id, workflow_id, scheduled_for) DO NOTHING
        RETURNING community_id, workflow_id, scheduled_for, claimed_at
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(workflow_id)
    .bind(scheduled_for)
    .fetch_optional(pool)
    .await?;

    row.map(|row| {
        let community_id: Uuid = row.try_get("community_id")?;
        Ok(ScheduledWorkflowFireClaim {
            community_id: CommunityId::from_uuid(community_id),
            workflow_id: row.try_get("workflow_id")?,
            scheduled_for: row.try_get("scheduled_for")?,
            claimed_at: row.try_get("claimed_at")?,
        })
    })
    .transpose()
}

/// Fetch the greatest claimed schedule instant for a workflow.
///
/// Interval schedulers use this as their DB-authoritative `last_fired` anchor.
/// It makes all pods compute the same next interval instant after a successful
/// claim, and preserves the interval clock across pod restarts. This intentionally
/// reads from `scheduled_workflow_fires`, not `workflow_runs`, because the claim
/// row is the source of truth for schedule deduplication.
pub async fn latest_scheduled_workflow_fire(
    pool: &PgPool,
    community_id: CommunityId,
    workflow_id: Uuid,
) -> Result<Option<DateTime<Utc>>> {
    let row = sqlx::query(
        r#"
        SELECT MAX(scheduled_for) AS scheduled_for
        FROM scheduled_workflow_fires
        WHERE community_id = $1 AND workflow_id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(workflow_id)
    .fetch_one(pool)
    .await?;

    row.try_get("scheduled_for").map_err(Into::into)
}

/// Link a won scheduled-fire claim to the workflow run it created.
///
/// This is for ops/audit forensics only; the claim row remains the dedupe
/// boundary. If run creation succeeds, callers should attach the run id before
/// spawning execution. If run creation fails, leaving `workflow_run_id` NULL is
/// intentional: the schedule instant was claimed and must not duplicate later.
pub async fn attach_scheduled_workflow_run(
    pool: &PgPool,
    community_id: CommunityId,
    workflow_id: Uuid,
    scheduled_for: DateTime<Utc>,
    workflow_run_id: Uuid,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE scheduled_workflow_fires
        SET workflow_run_id = $4
        WHERE community_id = $1
          AND workflow_id = $2
          AND scheduled_for = $3
          AND workflow_run_id IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(workflow_id)
    .bind(scheduled_for)
    .bind(workflow_run_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}

/// Delete old scheduled workflow fire claims for retention.
///
/// Schedule claim rows are correctness metadata, but they grow with every fire.
/// The relay/ops janitor should retain enough history for audits and interval
/// anchoring: the cutoff must be older than the largest interval schedule the
/// deployment supports, or interval workflows can lose their DB-authoritative
/// anchor after pruning.
pub async fn prune_scheduled_workflow_fires_before(
    pool: &PgPool,
    older_than: DateTime<Utc>,
) -> Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM scheduled_workflow_fires
        WHERE claimed_at < $1
          AND community_write_allowed(community_id)
        "#,
    )
    .bind(older_than)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Update a workflow's name, definition, and definition_hash.
///
/// NOTE: the relay's `WorkflowEngine` caches enabled workflows per
/// `(community_id, channel_id)`; a caller mutating trigger behavior must
/// invalidate via `WorkflowEngine::invalidate_channel_workflows` or trigger
/// matching lags the change by up to the cache TTL. (No current callers.)
pub async fn update_workflow(
    pool: &PgPool,
    community_id: CommunityId,
    id: Uuid,
    name: &str,
    definition_json: &str,
    definition_hash: &[u8],
) -> Result<()> {
    let affected = sqlx::query(
        r#"
        UPDATE workflows
        SET name = $1, definition = $2::jsonb, definition_hash = $3
        WHERE community_id = $4 AND id = $5
        "#,
    )
    .bind(name)
    .bind(definition_json)
    .bind(definition_hash)
    .bind(community_id.as_uuid())
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound(format!("workflow {id}")));
    }
    Ok(())
}

/// Update a workflow's status (active -> disabled -> archived).
///
/// NOTE: status gates trigger eligibility; see the cache-invalidation note on
/// [`update_workflow`]. (No current callers.)
pub async fn update_workflow_status(
    pool: &PgPool,
    community_id: CommunityId,
    id: Uuid,
    status: WorkflowStatus,
) -> Result<()> {
    let affected = sqlx::query(
        r#"
        UPDATE workflows
        SET status = $1::workflow_status
        WHERE community_id = $2 AND id = $3
        "#,
    )
    .bind(status.to_string())
    .bind(community_id.as_uuid())
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound(format!("workflow {id}")));
    }
    Ok(())
}

/// Enable or disable a workflow without changing its status.
///
/// NOTE: `enabled` gates trigger eligibility; see the cache-invalidation note
/// on [`update_workflow`]. (No current callers.)
pub async fn set_workflow_enabled(
    pool: &PgPool,
    community_id: CommunityId,
    id: Uuid,
    enabled: bool,
) -> Result<()> {
    let affected = sqlx::query(
        r#"
        UPDATE workflows
        SET enabled = $1
        WHERE community_id = $2 AND id = $3
        "#,
    )
    .bind(enabled)
    .bind(community_id.as_uuid())
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound(format!("workflow {id}")));
    }
    Ok(())
}

/// Disable all of `owner_pubkey`'s workflows in a channel (SEC-006).
///
/// Called when the owner loses channel membership (kind 9001 removal or kind
/// 9022 leave) so their workflows stop firing durably — across pods and
/// restarts — rather than only until the per-fire authority gate happens to
/// run. Idempotent; returns the number of workflows disabled so the caller
/// can decide whether a trigger-cache invalidation is needed.
pub async fn disable_workflows_for_owner_in_channel(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    owner_pubkey: &[u8],
) -> Result<u64> {
    let affected = sqlx::query(
        r#"
        UPDATE workflows
        SET enabled = FALSE
        WHERE community_id = $1 AND channel_id = $2 AND owner_pubkey = $3 AND enabled = TRUE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(owner_pubkey)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(affected)
}

/// Delete a workflow and all its runs/approvals (CASCADE).
///
/// NOTE: see the cache-invalidation note on [`update_workflow`]. The relay's
/// deletion path uses [`delete_workflow_for_owner`], which returns the
/// `channel_id` needed for invalidation. (No current callers.)
pub async fn delete_workflow(pool: &PgPool, community_id: CommunityId, id: Uuid) -> Result<()> {
    let affected = sqlx::query("DELETE FROM workflows WHERE community_id = $1 AND id = $2")
        .bind(community_id.as_uuid())
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound(format!("workflow {id}")));
    }
    Ok(())
}

/// Delete a workflow only when it belongs to `owner_pubkey`.
///
/// Used by event-driven deletion paths where the workflow UUID is attacker
/// controlled. Keeping the owner predicate in the DELETE statement avoids a
/// check-then-delete race and ensures a caller cannot delete another user's
/// workflow just by learning its UUID.
///
/// Returns the deleted workflow's `channel_id` so the caller can invalidate
/// the per-channel trigger cache without a separate lookup.
pub async fn delete_workflow_for_owner(
    pool: &PgPool,
    community_id: CommunityId,
    id: Uuid,
    owner_pubkey: &[u8],
) -> Result<Option<Uuid>> {
    let row = sqlx::query(
        "DELETE FROM workflows WHERE community_id = $1 AND id = $2 AND owner_pubkey = $3 \
         RETURNING channel_id",
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .bind(owner_pubkey)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => Ok(row.try_get("channel_id")?),
        None => Err(DbError::NotFound(format!("workflow {id}"))),
    }
}

// -- Workflow Run CRUD --------------------------------------------------------

/// Insert a new workflow run on the caller's transaction.
///
/// Command handlers use this with the transaction that persisted the trigger
/// event so both rows commit or roll back together.
pub async fn create_workflow_run_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community_id: CommunityId,
    workflow_id: Uuid,
    definition_event_id: &[u8],
    trigger_event_id: Option<&[u8]>,
    trigger_context: Option<&serde_json::Value>,
) -> Result<Uuid> {
    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO workflow_runs
            (community_id, id, workflow_id, definition_event_id, status, trigger_event_id, current_step, execution_trace, trigger_context)
        VALUES ($1, $2, $3, $4, 'pending', $5, 0, '[]', $6)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .bind(workflow_id)
    .bind(definition_event_id)
    .bind(trigger_event_id)
    .bind(trigger_context)
    .execute(&mut **tx)
    .await?;

    Ok(id)
}

/// Insert a new workflow run. Returns the new run's UUID.
///
/// `trigger_context` is the serialized `TriggerContext` for this run. It is stored
/// so that post-approval resume steps can restore the original trigger data and
/// correctly resolve `{{trigger.*}}` template variables.
pub async fn create_workflow_run(
    pool: &PgPool,
    community_id: CommunityId,
    workflow_id: Uuid,
    definition_event_id: &[u8],
    trigger_event_id: Option<&[u8]>,
    trigger_context: Option<&serde_json::Value>,
) -> Result<Uuid> {
    let id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO workflow_runs
            (community_id, id, workflow_id, definition_event_id, status, trigger_event_id, current_step, execution_trace, trigger_context)
        VALUES ($1, $2, $3, $4, 'pending', $5, 0, '[]', $6)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .bind(workflow_id)
    .bind(definition_event_id)
    .bind(trigger_event_id)
    .bind(trigger_context)
    .execute(pool)
    .await?;

    Ok(id)
}

/// Fetch a single workflow run by ID, scoped to its community.
pub async fn get_workflow_run(
    pool: &PgPool,
    community_id: CommunityId,
    id: Uuid,
) -> Result<WorkflowRunRecord> {
    let row = sqlx::query(
        r#"
        SELECT community_id, id, workflow_id, definition_event_id, status::text AS status, trigger_event_id, current_step,
               execution_trace, trigger_context, started_at, completed_at, error_message, error_code, created_at
        FROM workflow_runs
        WHERE community_id = $1 AND id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("workflow_run {id}")))?;

    row_to_run_record(row)
}

/// List runs for a workflow using a stable newest-first keyset.
///
/// Rows are ordered by `(created_at DESC, id DESC)`. A cursor is valid only
/// when both `before` and `before_id` are supplied; callers should pass the
/// final row from the previous page. `limit` is clamped to the shared list
/// bounds.
pub async fn list_workflow_runs_page(
    pool: &PgPool,
    community_id: CommunityId,
    workflow_id: Uuid,
    before: Option<DateTime<Utc>>,
    before_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<WorkflowRunRecord>> {
    let limit = limit.clamp(1, LIST_MAX_LIMIT);
    let rows = sqlx::query(
        r#"
        SELECT community_id, id, workflow_id, definition_event_id, status::text AS status, trigger_event_id, current_step,
               execution_trace, trigger_context, started_at, completed_at, error_message, error_code, created_at
        FROM workflow_runs
        WHERE community_id = $1 AND workflow_id = $2
          AND (
              $3::timestamptz IS NULL
              OR $4::uuid IS NULL
              OR (created_at, id) < ($3, $4)
          )
        ORDER BY created_at DESC, id DESC
        LIMIT $5
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(workflow_id)
    .bind(before)
    .bind(before_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_run_record).collect()
}

/// List runs for a workflow, newest first, up to `limit` rows.
pub async fn list_workflow_runs(
    pool: &PgPool,
    community_id: CommunityId,
    workflow_id: Uuid,
    limit: i64,
) -> Result<Vec<WorkflowRunRecord>> {
    list_workflow_runs_page(pool, community_id, workflow_id, None, None, limit).await
}

/// Structured failure persisted for a workflow run.
#[derive(Debug, Clone, Copy)]
pub struct WorkflowRunFailure<'a> {
    /// Stable machine-readable failure code.
    pub code: &'a str,
    /// Human-readable failure detail.
    pub message: &'a str,
}

/// Update run status, current step, execution trace, and optional failure.
///
/// Fix C3: `started_at` is set when the NEW status is 'running' and `started_at`
/// has not yet been stamped (IS NULL). The original code read `status` from the
/// column AFTER `SET status = ?` had already changed it, so the condition was
/// always false. We now check the bind parameter directly.
pub async fn update_workflow_run(
    pool: &PgPool,
    community_id: CommunityId,
    id: Uuid,
    status: RunStatus,
    current_step: i32,
    trace: &serde_json::Value,
    failure: Option<WorkflowRunFailure<'_>>,
) -> Result<()> {
    let status_str = status.to_string();
    let (error_code, error) = failure
        .map(|failure| (Some(failure.code), Some(failure.message)))
        .unwrap_or((None, None));
    let affected = sqlx::query(
        r#"
        UPDATE workflow_runs
        SET status        = $1::run_status,
            current_step  = $2,
            execution_trace = $3,
            error_code    = $4,
            error_message = $5,
            started_at    = CASE WHEN $6 = 'running' AND started_at IS NULL
                                 THEN NOW() ELSE started_at END,
            completed_at  = CASE WHEN $7 IN ('completed','failed','cancelled')
                                 THEN NOW() ELSE completed_at END
        WHERE community_id = $8 AND id = $9
        "#,
    )
    .bind(&status_str)
    .bind(current_step)
    .bind(trace)
    .bind(error_code)
    .bind(error)
    .bind(&status_str) // for started_at CASE
    .bind(&status_str) // for completed_at CASE
    .bind(community_id.as_uuid())
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound(format!("workflow_run {id}")));
    }
    Ok(())
}

// -- Approval CRUD ------------------------------------------------------------

/// Parameters for creating a new approval request.
pub struct CreateApprovalParams<'a> {
    /// Server-resolved community that owns the workflow/run this approval gates.
    pub community_id: CommunityId,
    /// Raw approval token (will be hashed before storage).
    pub token: &'a str,
    /// The workflow this approval belongs to.
    pub workflow_id: Uuid,
    /// The run waiting on this approval.
    pub run_id: Uuid,
    /// The step ID that requested approval.
    pub step_id: &'a str,
    /// Zero-based index of the step in the workflow.
    pub step_index: i32,
    /// Who may approve (user mention or role spec).
    pub approver_spec: &'a str,
    /// When this approval request expires.
    pub expires_at: DateTime<Utc>,
}

/// Insert a new approval request.
///
/// The `token` parameter is the raw (plaintext) token. It is hashed with
/// SHA-256 before storage so the DB never holds the raw value.
pub async fn create_approval(pool: &PgPool, params: CreateApprovalParams<'_>) -> Result<()> {
    let CreateApprovalParams {
        community_id,
        token,
        workflow_id,
        run_id,
        step_id,
        step_index,
        approver_spec,
        expires_at,
    } = params;
    let token_hash = hash_approval_token(token);

    sqlx::query(
        r#"
        INSERT INTO workflow_approvals
            (community_id, token, workflow_id, run_id, step_id, step_index, approver_spec, status, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(token_hash)
    .bind(workflow_id)
    .bind(run_id)
    .bind(step_id)
    .bind(step_index)
    .bind(approver_spec)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// Fetch an approval record by raw token.
///
/// The token is hashed before the DB lookup so plaintext tokens are never
/// sent to the database layer.
pub async fn get_approval(
    pool: &PgPool,
    community_id: CommunityId,
    token: &str,
) -> Result<ApprovalRecord> {
    let token_hash = hash_approval_token(token);
    get_approval_by_stored_hash(pool, community_id, &token_hash).await
}

/// Fetch an approval record by its already-hashed token value.
///
/// Use this when you already have the hash stored in the DB (e.g., from
/// `get_run_approvals`). The `token_hash` is used directly without re-hashing.
///
/// `workflow_approvals` is keyed `(community_id, token)`; the same token bytes
/// could in principle collide across communities, so the lookup binds the
/// server-resolved community alongside the token.
pub async fn get_approval_by_stored_hash(
    pool: &PgPool,
    community_id: CommunityId,
    token_hash: &[u8],
) -> Result<ApprovalRecord> {
    let row = sqlx::query(
        r#"
        SELECT token, workflow_id, run_id, step_id, step_index, approver_spec,
               status::text AS status, approver_pubkey, note, expires_at, created_at
        FROM workflow_approvals
        WHERE community_id = $1 AND token = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(token_hash)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DbError::NotFound("approval token (hashed)".to_string()))?;

    row_to_approval_record(row)
}

/// Fetch all approval records for a given workflow run.
pub async fn get_run_approvals(
    pool: &PgPool,
    community_id: CommunityId,
    workflow_id: Uuid,
    run_id: Uuid,
) -> Result<Vec<ApprovalRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT token, workflow_id, run_id, step_id, step_index, approver_spec,
               status::text AS status, approver_pubkey, note, expires_at, created_at
        FROM workflow_approvals
        WHERE community_id = $1 AND run_id = $2 AND workflow_id = $3
        ORDER BY step_index, created_at
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(run_id)
    .bind(workflow_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_approval_record).collect()
}

/// Update an approval's status, approver pubkey, and optional note.
/// Also stamps `granted_at` or `denied_at` based on the new status.
///
/// The `token` parameter is the raw (plaintext) token; it is hashed before
/// the WHERE lookup.
///
/// # TOCTOU safety (N5)
/// The WHERE clause includes `AND status = 'pending'` so that two concurrent
/// grant/deny requests cannot both succeed. If the approval was already acted
/// on (status != 'pending'), the UPDATE touches 0 rows and this function
/// returns `Ok(false)`. Callers should treat `false` as a conflict (HTTP 409).
pub async fn update_approval(
    pool: &PgPool,
    community_id: CommunityId,
    token: &str,
    status: ApprovalStatus,
    approver_pubkey: Option<&[u8]>,
    note: Option<&str>,
) -> Result<bool> {
    let token_hash = hash_approval_token(token);
    update_approval_by_stored_hash(
        pool,
        community_id,
        &token_hash,
        status,
        approver_pubkey,
        note,
    )
    .await
}

/// Update an approval by its already-hashed token value.
///
/// Use this when you already have the hash stored in the DB (e.g., from
/// `get_run_approvals`). The `token_hash` is used directly without re-hashing.
///
/// See [`update_approval`] for TOCTOU safety notes. The predicate binds the
/// server-resolved community alongside the token so an approval action for A/X
/// can never act on B/X.
pub async fn update_approval_by_stored_hash(
    pool: &PgPool,
    community_id: CommunityId,
    token_hash: &[u8],
    status: ApprovalStatus,
    approver_pubkey: Option<&[u8]>,
    note: Option<&str>,
) -> Result<bool> {
    let status_str = status.to_string();
    let affected = sqlx::query(
        r#"
        UPDATE workflow_approvals
        SET status          = $1::approval_status,
            approver_pubkey = $2,
            note            = $3,
            granted_at      = CASE WHEN $4 = 'granted' THEN NOW() ELSE granted_at END,
            denied_at       = CASE WHEN $5 = 'denied'  THEN NOW() ELSE denied_at  END
        WHERE community_id = $6 AND token = $7 AND status = 'pending'
        "#,
    )
    .bind(&status_str)
    .bind(approver_pubkey)
    .bind(note)
    .bind(&status_str) // for granted_at CASE
    .bind(&status_str) // for denied_at CASE
    .bind(community_id.as_uuid())
    .bind(token_hash)
    .execute(pool)
    .await?
    .rows_affected();

    Ok(affected > 0)
}

// -- Row mappers --------------------------------------------------------------

fn row_to_workflow_record(row: sqlx::postgres::PgRow) -> Result<WorkflowRecord> {
    let id: Uuid = row.try_get("id")?;
    let channel_id: Option<Uuid> = row.try_get("channel_id")?;

    let status_str: String = row.try_get("status")?;
    let status = status_str.parse::<WorkflowStatus>()?;

    let enabled: bool = row.try_get("enabled")?;

    let community_id: Uuid = row.try_get("community_id")?;

    Ok(WorkflowRecord {
        id,
        community_id: CommunityId::from_uuid(community_id),
        name: row.try_get("name")?,
        owner_pubkey: row.try_get("owner_pubkey")?,
        channel_id,
        definition: row.try_get("definition")?,
        definition_hash: row.try_get("definition_hash")?,
        definition_event_id: row.try_get("definition_event_id")?,
        status,
        enabled,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn row_to_run_record(row: sqlx::postgres::PgRow) -> Result<WorkflowRunRecord> {
    let id: Uuid = row.try_get("id")?;
    let community_id: Uuid = row.try_get("community_id")?;
    let workflow_id: Uuid = row.try_get("workflow_id")?;

    let status_str: String = row.try_get("status")?;
    let status = status_str.parse::<RunStatus>()?;

    Ok(WorkflowRunRecord {
        id,
        community_id: CommunityId::from_uuid(community_id),
        workflow_id,
        definition_event_id: row.try_get("definition_event_id")?,
        status,
        trigger_event_id: row.try_get("trigger_event_id")?,
        current_step: row.try_get("current_step")?,
        execution_trace: row.try_get("execution_trace")?,
        trigger_context: row.try_get("trigger_context")?,
        started_at: row.try_get("started_at")?,
        completed_at: row.try_get("completed_at")?,
        error_message: row.try_get("error_message")?,
        error_code: row.try_get("error_code")?,
        created_at: row.try_get("created_at")?,
    })
}

fn row_to_approval_record(row: sqlx::postgres::PgRow) -> Result<ApprovalRecord> {
    let workflow_id: Uuid = row.try_get("workflow_id")?;
    let run_id: Uuid = row.try_get("run_id")?;

    let status_str: String = row.try_get("status")?;
    let status = status_str.parse::<ApprovalStatus>()?;

    Ok(ApprovalRecord {
        token: row.try_get("token")?,
        workflow_id,
        run_id,
        step_id: row.try_get("step_id")?,
        step_index: row.try_get("step_index")?,
        approver_spec: row.try_get("approver_spec")?,
        status,
        approver_pubkey: row.try_get("approver_pubkey")?,
        note: row.try_get("note")?,
        expires_at: row.try_get("expires_at")?,
        created_at: row.try_get("created_at")?,
    })
}

/// Find a workflow by owner pubkey and name within a community. Returns the
/// first match (active or not).
pub async fn find_by_owner_and_name(
    pool: &PgPool,
    community_id: CommunityId,
    owner_pubkey: &[u8],
    name: &str,
) -> Result<Option<WorkflowRecord>> {
    let row = sqlx::query(
        r#"
        SELECT id, community_id, name, owner_pubkey, channel_id, definition, definition_hash, definition_event_id,
               status::text AS status, enabled, created_at, updated_at
        FROM workflows
        WHERE community_id = $1 AND owner_pubkey = $2 AND name = $3
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(owner_pubkey)
    .bind(name)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(row_to_workflow_record(r)?)),
        None => Ok(None),
    }
}

// -- Workflow agent deliveries ------------------------------------------------
//
// Durable, target-scoped delivery inbox and the complete transition state
// machine for workflow messages addressed to managed agents. This is the
// DB-layer complement of the zero-I/O `buzz_core::workflow_delivery` protocol
// vocabulary: it persists exactly B's canonical `WorkflowDeliveryBinding`
// (never a duplicate tuple spelling) and owns only the lifecycle around it.
//
//     pending --claim--> claimed --finish--> finished | failed
//        ^                   |
//        +------- reap -------+   (expired lease reclaimed; prior holder fenced)
//
// Leases are fenced by a monotonic `lease_generation`: every claim and every
// reap bumps it, and renew/finish only advance a row whose generation still
// matches the caller's token, so a reaped or superseded holder always fails
// closed. The reaper is a fleet-wide scan filtered through
// `community_write_allowed`, exactly like the scheduler prune scan, so a
// quiescing/fenced/deleted tenant is skipped before its write-fence trigger
// can abort healthy tenants in the same statement.
//
// Producer, runtime, API, and ACP reachability are intentionally absent: this
// node is dormant by contract.

/// Terminal outcome recorded on a workflow agent delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowDeliveryOutcome {
    /// The managed agent completed the delivery successfully.
    Finished,
    /// The managed agent failed the delivery permanently.
    Failed,
}

impl WorkflowDeliveryOutcome {
    fn as_status(self) -> &'static str {
        match self {
            WorkflowDeliveryOutcome::Finished => "finished",
            WorkflowDeliveryOutcome::Failed => "failed",
        }
    }
}

/// Lifecycle state of a durable workflow agent delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowDeliveryStatus {
    /// Created and awaiting a claim.
    Pending,
    /// Claimed under a live, fenced lease.
    Claimed,
    /// Terminally finished (success).
    Finished,
    /// Terminally failed (permanent).
    Failed,
}

impl fmt::Display for WorkflowDeliveryStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            WorkflowDeliveryStatus::Pending => "pending",
            WorkflowDeliveryStatus::Claimed => "claimed",
            WorkflowDeliveryStatus::Finished => "finished",
            WorkflowDeliveryStatus::Failed => "failed",
        })
    }
}

impl FromStr for WorkflowDeliveryStatus {
    type Err = DbError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(WorkflowDeliveryStatus::Pending),
            "claimed" => Ok(WorkflowDeliveryStatus::Claimed),
            "finished" => Ok(WorkflowDeliveryStatus::Finished),
            "failed" => Ok(WorkflowDeliveryStatus::Failed),
            other => Err(DbError::InvalidData(format!(
                "unknown workflow delivery status: {other}"
            ))),
        }
    }
}

/// The fenced lease a caller holds after winning a claim.
///
/// `lease_generation` is the fence token: any renew or finish that does not
/// present the row's current generation matches zero rows and fails closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowDeliveryLease {
    /// Owning community (server provenance).
    pub community_id: CommunityId,
    /// Durable delivery identifier.
    pub delivery_id: WorkflowDeliveryId,
    /// Managed-agent recipient that holds the lease.
    pub target_pubkey: PublicKey,
    /// Fence token: the generation this lease was granted under.
    pub lease_generation: i64,
    /// When the current lease expires and becomes reclaimable.
    pub lease_until: DateTime<Utc>,
}

/// A durable delivery row, decoded back into B's canonical binding plus its
/// lifecycle state.
#[derive(Debug, Clone)]
pub struct WorkflowAgentDeliveryRecord {
    /// Durable delivery identifier.
    pub id: WorkflowDeliveryId,
    /// Canonical protocol binding persisted verbatim from B.
    pub binding: WorkflowDeliveryBinding,
    /// Current lifecycle state.
    pub status: WorkflowDeliveryStatus,
    /// Current fence generation.
    pub lease_generation: i64,
    /// Expiry of the current claim, if claimed.
    pub lease_until: Option<DateTime<Utc>>,
    /// When the current claim was taken, if claimed.
    pub claimed_at: Option<DateTime<Utc>>,
    /// When the delivery reached a terminal state, if terminal.
    pub finished_at: Option<DateTime<Utc>>,
    /// Creation time for ordered polling.
    pub created_at: DateTime<Utc>,
}

/// Result of a terminal `finish` transition, giving callers a deterministic
/// convergence point for uncertain completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowDeliveryFinishOutcome {
    /// This call performed the once-only terminal transition.
    Settled(WorkflowDeliveryOutcome),
    /// The delivery was already terminal; the recorded status is returned so a
    /// retry after an uncertain crash converges idempotently to one terminal.
    AlreadyTerminal(WorkflowDeliveryStatus),
    /// The caller's lease was stale (reaped or superseded): fail closed.
    LeaseLost,
}

/// Result of a `renew` transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowDeliveryRenewOutcome {
    /// The lease was extended; the new expiry is returned.
    Renewed(DateTime<Utc>),
    /// The caller's lease was stale (reaped, superseded, or already terminal):
    /// fail closed.
    LeaseLost,
}

/// One canonical delivery to persist for a target, decomposed from B's binding.
///
/// The delivery identifier is caller-supplied (it must equal the identifier the
/// wake hint and claim request will carry), and the binding is B's canonical,
/// pre-validated tuple. `message_event_created_at` is the persistence key that
/// completes the events foreign key; it is not part of the protocol identity.
#[derive(Debug, Clone)]
pub struct WorkflowAgentDelivery {
    /// Stable durable identifier for this delivery.
    pub id: WorkflowDeliveryId,
    /// Canonical protocol binding (community/run/step/target/definition/message/cause).
    pub binding: WorkflowDeliveryBinding,
}

/// The persisted column decomposition of a `WorkflowDeliveryCause`:
/// `(cause_kind, cause_event_id, cause_scheduled_for, cause_webhook_invocation_id)`.
type CauseColumns = (
    &'static str,
    Option<Vec<u8>>,
    Option<DateTime<Utc>>,
    Option<Uuid>,
);

/// Decompose a `WorkflowDeliveryCause` into its persisted column triple.
fn cause_columns(cause: &WorkflowDeliveryCause) -> CauseColumns {
    match cause {
        WorkflowDeliveryCause::Event(event_id) => {
            ("event", Some(event_id.as_bytes().to_vec()), None, None)
        }
        WorkflowDeliveryCause::Schedule {
            scheduled_for_unix_seconds,
        } => (
            "schedule",
            None,
            Some(
                DateTime::<Utc>::from_timestamp(*scheduled_for_unix_seconds, 0)
                    .unwrap_or_else(|| DateTime::<Utc>::from_timestamp_nanos(0)),
            ),
            None,
        ),
        WorkflowDeliveryCause::Webhook { invocation_id } => {
            ("webhook", None, None, Some(*invocation_id))
        }
    }
}

/// Reconstruct a `WorkflowDeliveryCause` from its persisted columns.
fn cause_from_columns(
    kind: &str,
    event_id: Option<Vec<u8>>,
    scheduled_for: Option<DateTime<Utc>>,
    webhook_invocation_id: Option<Uuid>,
) -> Result<WorkflowDeliveryCause> {
    match kind {
        "event" => {
            let bytes =
                event_id.ok_or_else(|| DbError::InvalidData("event cause missing id".into()))?;
            let id = EventId::from_slice(&bytes)
                .map_err(|_| DbError::InvalidData("event cause id malformed".into()))?;
            Ok(WorkflowDeliveryCause::Event(id))
        }
        "schedule" => {
            let at = scheduled_for
                .ok_or_else(|| DbError::InvalidData("schedule cause missing instant".into()))?;
            Ok(WorkflowDeliveryCause::Schedule {
                scheduled_for_unix_seconds: at.timestamp(),
            })
        }
        "webhook" => {
            let invocation_id = webhook_invocation_id
                .ok_or_else(|| DbError::InvalidData("webhook cause missing invocation".into()))?;
            Ok(WorkflowDeliveryCause::Webhook { invocation_id })
        }
        other => Err(DbError::InvalidData(format!(
            "unknown delivery cause kind: {other}"
        ))),
    }
}

/// Decode one delivery row back into a record with its canonical binding.
///
/// Read queries must project `status` via `status::text AS status`: `status`
/// is the native `workflow_agent_delivery_status` enum, which the sqlx runtime
/// cannot decode into a Rust `String` — matching how every other native enum
/// column is read in this module.
fn row_to_delivery_record(row: &sqlx::postgres::PgRow) -> Result<WorkflowAgentDeliveryRecord> {
    let community_id = CommunityId::from_uuid(row.try_get("community_id")?);
    let id: Uuid = row.try_get("id")?;
    let workflow_id: Uuid = row.try_get("workflow_id")?;
    let run_id: Uuid = row.try_get("run_id")?;
    let step_id: String = row.try_get("step_id")?;
    let target_bytes: Vec<u8> = row.try_get("target_pubkey")?;
    let definition_bytes: Vec<u8> = row.try_get("definition_event_id")?;
    let message_bytes: Vec<u8> = row.try_get("message_event_id")?;
    let cause_kind: String = row.try_get("cause_kind")?;
    let cause_event_id: Option<Vec<u8>> = row.try_get("cause_event_id")?;
    let cause_scheduled_for: Option<DateTime<Utc>> = row.try_get("cause_scheduled_for")?;
    let cause_webhook_invocation_id: Option<Uuid> = row.try_get("cause_webhook_invocation_id")?;
    let status: String = row.try_get("status")?;

    let target_pubkey = PublicKey::from_slice(&target_bytes)
        .map_err(|_| DbError::InvalidData("delivery target pubkey malformed".into()))?;
    let definition_event_id = EventId::from_slice(&definition_bytes)
        .map_err(|_| DbError::InvalidData("delivery definition event id malformed".into()))?;
    let message_event_id = EventId::from_slice(&message_bytes)
        .map_err(|_| DbError::InvalidData("delivery message event id malformed".into()))?;
    let cause = cause_from_columns(
        &cause_kind,
        cause_event_id,
        cause_scheduled_for,
        cause_webhook_invocation_id,
    )?;

    let binding = WorkflowDeliveryBinding::new(
        community_id,
        workflow_id,
        run_id,
        step_id,
        target_pubkey,
        definition_event_id,
        message_event_id,
        cause,
    )
    .map_err(|error| DbError::InvalidData(format!("stored delivery binding invalid: {error}")))?;

    Ok(WorkflowAgentDeliveryRecord {
        id: WorkflowDeliveryId::from_uuid(id),
        binding,
        status: status.parse()?,
        lease_generation: row.try_get("lease_generation")?,
        lease_until: row.try_get("lease_until")?,
        claimed_at: row.try_get("claimed_at")?,
        finished_at: row.try_get("finished_at")?,
        created_at: row.try_get("created_at")?,
    })
}

/// Serialize one `(community, run, step)` delivery identity.
///
/// The returned transaction holds an advisory lock until every target row for
/// this step commits together, preventing duplicate producer retries from
/// racing two canonical deliveries for the same step. Returns whether any
/// delivery already exists for this identity so the caller can reuse the
/// already-signed visible message instead of signing a second one.
pub async fn lock_workflow_agent_delivery_identity(
    pool: &PgPool,
    community_id: CommunityId,
    run_id: Uuid,
    step_id: &str,
) -> Result<(Transaction<'static, Postgres>, bool)> {
    let mut transaction = pool.begin().await?;
    let identity = format!("{}:{run_id}:{step_id}", community_id.as_uuid());
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(identity)
        .execute(&mut *transaction)
        .await?;
    let existing = sqlx::query(
        "SELECT 1 FROM workflow_agent_deliveries \
         WHERE community_id = $1 AND run_id = $2 AND step_id = $3 LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(run_id)
    .bind(step_id)
    .fetch_optional(&mut *transaction)
    .await?
    .is_some();
    Ok((transaction, existing))
}

/// Atomically persist all canonical deliveries for one workflow step.
///
/// This is the ONLY insert path into `workflow_agent_deliveries`. Callers hold
/// the identity lock from [`lock_workflow_agent_delivery_identity`] and pass the
/// same transaction so the visible message insert (owned by the producer node)
/// and every delivery row commit or roll back together. Duplicate producer
/// retries collapse via the `(community, run, step, target)` uniqueness with
/// `ON CONFLICT DO NOTHING`; the returned vector lists only rows this call
/// actually created.
pub async fn commit_workflow_agent_deliveries(
    mut transaction: Transaction<'static, Postgres>,
    community_id: CommunityId,
    message_event_created_at: DateTime<Utc>,
    deliveries: &[WorkflowAgentDelivery],
) -> Result<Vec<WorkflowDeliveryId>> {
    let mut created = Vec::new();
    for delivery in deliveries {
        let binding = &delivery.binding;
        if binding.community_id() != community_id {
            return Err(DbError::InvalidData(
                "delivery binding community does not match committing community".into(),
            ));
        }
        let (cause_kind, cause_event_id, cause_scheduled_for, cause_webhook_invocation_id) =
            cause_columns(binding.cause());
        let affected = sqlx::query(
            "INSERT INTO workflow_agent_deliveries \
             (community_id, id, workflow_id, run_id, step_id, target_pubkey, \
              definition_event_id, message_event_id, message_event_created_at, \
              cause_kind, cause_event_id, cause_scheduled_for, cause_webhook_invocation_id) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) \
             ON CONFLICT (community_id, run_id, step_id, target_pubkey) DO NOTHING",
        )
        .bind(community_id.as_uuid())
        .bind(delivery.id.as_uuid())
        .bind(binding.workflow_id())
        .bind(binding.run_id())
        .bind(binding.step_id())
        .bind(binding.target_pubkey().to_bytes().to_vec())
        .bind(binding.definition_event_id().as_bytes().to_vec())
        .bind(binding.message_event_id().as_bytes().to_vec())
        .bind(message_event_created_at)
        .bind(cause_kind)
        .bind(cause_event_id)
        .bind(cause_scheduled_for)
        .bind(cause_webhook_invocation_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if affected == 1 {
            created.push(delivery.id);
        }
    }
    transaction.commit().await?;
    Ok(created)
}

/// Atomically claim one specific pending delivery, or the oldest pending
/// delivery for a target, under a fresh fenced lease.
///
/// Selection and update are scoped to the authenticated target and community.
/// An optional expected binding turns a forged or stale wake hint into a miss
/// rather than an alternate authority path: the candidate must match every
/// supplied binding field — workflow, run, step, definition, message, and the
/// full decomposed cause identity — so a claim can never settle against a row
/// whose binding disagrees with the caller's. The winning row's
/// `lease_generation` is bumped and returned as the fence token in
/// [`WorkflowDeliveryLease`]; `lease_seconds` sets the initial lease window.
pub async fn claim_workflow_agent_delivery(
    pool: &PgPool,
    community_id: CommunityId,
    target_pubkey: &PublicKey,
    delivery_id: Option<WorkflowDeliveryId>,
    expected: Option<&WorkflowDeliveryBinding>,
    lease_seconds: i64,
) -> Result<Option<(WorkflowDeliveryLease, WorkflowAgentDeliveryRecord)>> {
    if lease_seconds <= 0 {
        return Err(DbError::InvalidData(
            "delivery lease_seconds must be positive".into(),
        ));
    }
    // The expected binding is an authority claim about which row may be
    // consumed; the (community, target) request scope is where the row is
    // looked up. If they disagree, a foreign binding could authorize a row
    // selected under a different scope, so reject the mismatch before SQL
    // rather than letting the scope arguments silently override the binding.
    if let Some(expected) = expected {
        if expected.community_id() != community_id || expected.target_pubkey() != *target_pubkey {
            return Ok(None);
        }
    }
    let target_bytes = target_pubkey.to_bytes().to_vec();
    // Decompose the expected cause so a mismatch on any cause identity — not
    // only the shared columns — makes the claim a miss.
    let expected_cause = expected.map(|b| cause_columns(b.cause()));
    let row = sqlx::query(
        r#"
        WITH candidate AS (
            SELECT community_id, id
            FROM workflow_agent_deliveries
            WHERE community_id = $1 AND target_pubkey = $2
              AND ($3::uuid IS NULL OR id = $3)
              AND ($4::uuid IS NULL OR workflow_id = $4)
              AND ($5::uuid IS NULL OR run_id = $5)
              AND ($6::text IS NULL OR step_id = $6)
              AND ($7::bytea IS NULL OR definition_event_id = $7)
              AND ($8::bytea IS NULL OR message_event_id = $8)
              AND ($9::text IS NULL OR cause_kind = $9)
              AND ($10::bytea IS NULL OR cause_event_id = $10)
              AND ($11::timestamptz IS NULL OR cause_scheduled_for = $11)
              AND ($12::uuid IS NULL OR cause_webhook_invocation_id = $12)
              AND status = 'pending'
            ORDER BY created_at, id
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        )
        UPDATE workflow_agent_deliveries delivery
        SET status = 'claimed',
            lease_generation = delivery.lease_generation + 1,
            lease_until = NOW() + make_interval(secs => $13),
            claimed_at = NOW()
        FROM candidate
        WHERE delivery.community_id = candidate.community_id
          AND delivery.id = candidate.id
        RETURNING delivery.community_id, delivery.id, delivery.workflow_id,
            delivery.run_id, delivery.step_id, delivery.target_pubkey,
            delivery.definition_event_id, delivery.message_event_id,
            delivery.message_event_created_at, delivery.cause_kind,
            delivery.cause_event_id, delivery.cause_scheduled_for,
            delivery.cause_webhook_invocation_id, delivery.status::text AS status,
            delivery.lease_generation, delivery.lease_until, delivery.claimed_at,
            delivery.finished_at, delivery.created_at
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(&target_bytes)
    .bind(delivery_id.map(WorkflowDeliveryId::as_uuid))
    .bind(expected.map(WorkflowDeliveryBinding::workflow_id))
    .bind(expected.map(WorkflowDeliveryBinding::run_id))
    .bind(expected.map(|b| b.step_id().to_owned()))
    .bind(expected.map(|b| b.definition_event_id().as_bytes().to_vec()))
    .bind(expected.map(|b| b.message_event_id().as_bytes().to_vec()))
    .bind(expected_cause.as_ref().map(|(kind, ..)| *kind))
    .bind(expected_cause.as_ref().and_then(|(_, id, ..)| id.clone()))
    .bind(expected_cause.as_ref().and_then(|(_, _, at, _)| *at))
    .bind(expected_cause.as_ref().and_then(|(.., webhook)| *webhook))
    .bind(lease_seconds as f64)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };
    let record = row_to_delivery_record(&row)?;
    let lease = WorkflowDeliveryLease {
        community_id: record.binding.community_id(),
        delivery_id: record.id,
        target_pubkey: record.binding.target_pubkey(),
        lease_generation: record.lease_generation,
        lease_until: record
            .lease_until
            .ok_or_else(|| DbError::InvalidData("claimed delivery missing lease_until".into()))?,
    };
    Ok(Some((lease, record)))
}

/// Extend a live lease, fenced by the caller's generation and target.
///
/// Advances `lease_until` only for a still-claimed row whose current generation
/// matches the lease token, whose target matches the lease's held recipient,
/// and whose lease has not yet expired on the DB clock. A reaped, superseded,
/// already-terminal, expired, or wrong-target holder matches zero rows and
/// receives [`WorkflowDeliveryRenewOutcome::LeaseLost`]; a lease that has
/// passed its deadline cannot resurrect itself before the reaper runs.
pub async fn renew_workflow_agent_delivery(
    pool: &PgPool,
    lease: &WorkflowDeliveryLease,
    lease_seconds: i64,
) -> Result<WorkflowDeliveryRenewOutcome> {
    if lease_seconds <= 0 {
        return Err(DbError::InvalidData(
            "delivery lease_seconds must be positive".into(),
        ));
    }
    let row = sqlx::query(
        r#"
        UPDATE workflow_agent_deliveries
        SET lease_until = NOW() + make_interval(secs => $5)
        WHERE community_id = $1
          AND id = $2
          AND target_pubkey = $4
          AND status = 'claimed'
          AND lease_generation = $3
          AND lease_until >= NOW()
        RETURNING lease_until
        "#,
    )
    .bind(lease.community_id.as_uuid())
    .bind(lease.delivery_id.as_uuid())
    .bind(lease.lease_generation)
    .bind(lease.target_pubkey.to_bytes().to_vec())
    .bind(lease_seconds as f64)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(row) => Ok(WorkflowDeliveryRenewOutcome::Renewed(
            row.try_get("lease_until")?,
        )),
        None => Ok(WorkflowDeliveryRenewOutcome::LeaseLost),
    }
}

/// Perform the once-only terminal transition, fenced by the caller's lease, and
/// reconcile uncertain completion.
///
/// If the caller's full capability (target + generation) still holds a claimed,
/// unexpired row, it is settled to the requested terminal outcome exactly once.
/// If the row is already terminal under the caller's own target and generation,
/// status is returned so a retry after a crash between the agent's work and the
/// durable finish converges idempotently to the same terminal rather than
/// reopening the delivery. Any other state — reaped, superseded, expired, a
/// lost race, a wrong-target lease, or a terminal written by a newer holder at
/// a different generation — fails closed with
/// [`WorkflowDeliveryFinishOutcome::LeaseLost`].
pub async fn finish_workflow_agent_delivery(
    pool: &PgPool,
    lease: &WorkflowDeliveryLease,
    outcome: WorkflowDeliveryOutcome,
) -> Result<WorkflowDeliveryFinishOutcome> {
    let mut transaction = pool.begin().await?;

    // Fence + terminal-once guard in one statement: only a still-claimed row
    // under the caller's generation transitions, and it can only do so from a
    // non-terminal state.
    let settled = sqlx::query(
        r#"
        UPDATE workflow_agent_deliveries
        SET status = $4::workflow_agent_delivery_status,
            lease_until = NULL,
            claimed_at = NULL,
            finished_at = NOW()
        WHERE community_id = $1
          AND id = $2
          AND target_pubkey = $5
          AND status = 'claimed'
          AND lease_generation = $3
          AND lease_until >= NOW()
        RETURNING status::text AS status
        "#,
    )
    .bind(lease.community_id.as_uuid())
    .bind(lease.delivery_id.as_uuid())
    .bind(lease.lease_generation)
    .bind(outcome.as_status())
    .bind(lease.target_pubkey.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await?;

    if settled.is_some() {
        transaction.commit().await?;
        return Ok(WorkflowDeliveryFinishOutcome::Settled(outcome));
    }

    // No transition happened. Distinguish "already terminal under THIS lease"
    // (idempotent convergence for an uncertain-completion retry) from "lease
    // lost" (fail closed) by reading current state inside the same transaction.
    // The terminal read is fenced by the caller's FULL lease capability
    // (target + generation): finish never bumps the generation, so a genuine
    // retry by the true holder still matches, but a terminal written by a NEWER
    // holder (after this lease was reaped and re-claimed at a higher
    // generation) or a lease reconstituted with the wrong target must not be
    // laundered back as its own success. Any target or generation mismatch is
    // therefore LeaseLost.
    let current = sqlx::query(
        "SELECT status::text AS status FROM workflow_agent_deliveries \
         WHERE community_id = $1 AND id = $2 AND target_pubkey = $4 AND lease_generation = $3",
    )
    .bind(lease.community_id.as_uuid())
    .bind(lease.delivery_id.as_uuid())
    .bind(lease.lease_generation)
    .bind(lease.target_pubkey.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await?;
    transaction.commit().await?;

    match current {
        Some(row) => {
            let status: WorkflowDeliveryStatus = row.try_get::<String, _>("status")?.parse()?;
            match status {
                WorkflowDeliveryStatus::Finished | WorkflowDeliveryStatus::Failed => {
                    Ok(WorkflowDeliveryFinishOutcome::AlreadyTerminal(status))
                }
                _ => Ok(WorkflowDeliveryFinishOutcome::LeaseLost),
            }
        }
        None => Ok(WorkflowDeliveryFinishOutcome::LeaseLost),
    }
}

/// Reclaim expired delivery leases across the fleet, fencing prior holders out.
///
/// Every candidate row is filtered through `community_write_allowed`, so a
/// quiescing, fenced, or tombstoned tenant is skipped inside the mutating
/// statement — identical to the scheduler prune scan — before its per-row write
/// fence could abort healthy tenants. Each reclaimed row returns to `pending`
/// with `lease_generation` bumped, so the previous holder's later renew or
/// finish (which still presents the old generation) fails closed. Returns the
/// number of rows reclaimed.
pub async fn reap_expired_workflow_agent_deliveries(pool: &PgPool) -> Result<u64> {
    let result = sqlx::query(
        r#"
        UPDATE workflow_agent_deliveries
        SET status = 'pending',
            lease_generation = lease_generation + 1,
            lease_until = NULL,
            claimed_at = NULL
        WHERE status = 'claimed'
          AND lease_until < NOW()
          AND community_write_allowed(community_id)
        "#,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Fetch one delivery record by identifier, scoped to its community.
pub async fn get_workflow_agent_delivery(
    pool: &PgPool,
    community_id: CommunityId,
    delivery_id: WorkflowDeliveryId,
) -> Result<Option<WorkflowAgentDeliveryRecord>> {
    let row = sqlx::query(
        "SELECT community_id, id, workflow_id, run_id, step_id, target_pubkey, \
         definition_event_id, message_event_id, message_event_created_at, cause_kind, \
         cause_event_id, cause_scheduled_for, cause_webhook_invocation_id, \
         status::text AS status, lease_generation, lease_until, claimed_at, finished_at, \
         created_at FROM workflow_agent_deliveries WHERE community_id = $1 AND id = $2",
    )
    .bind(community_id.as_uuid())
    .bind(delivery_id.as_uuid())
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_delivery_record).transpose()
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // -- WorkflowStatus enum --------------------------------------------------

    #[test]
    fn workflow_status_display_is_lowercase() {
        assert_eq!(WorkflowStatus::Active.to_string(), "active");
        assert_eq!(WorkflowStatus::Disabled.to_string(), "disabled");
        assert_eq!(WorkflowStatus::Archived.to_string(), "archived");
    }

    #[test]
    fn workflow_status_from_str_round_trips() {
        for s in &["active", "disabled", "archived"] {
            let status: WorkflowStatus = s.parse().expect("parse");
            assert_eq!(status.to_string(), *s);
        }
    }

    #[test]
    fn workflow_status_from_str_rejects_unknown() {
        let err = "pending".parse::<WorkflowStatus>().unwrap_err();
        assert!(matches!(err, DbError::InvalidData(_)));
    }

    #[test]
    fn workflow_status_equality() {
        assert_eq!(WorkflowStatus::Active, WorkflowStatus::Active);
        assert_ne!(WorkflowStatus::Active, WorkflowStatus::Disabled);
    }

    // -- RunStatus enum -------------------------------------------------------

    #[test]
    fn run_status_display_is_lowercase() {
        assert_eq!(RunStatus::Pending.to_string(), "pending");
        assert_eq!(RunStatus::Running.to_string(), "running");
        assert_eq!(RunStatus::WaitingApproval.to_string(), "waiting_approval");
        assert_eq!(RunStatus::Completed.to_string(), "completed");
        assert_eq!(RunStatus::Failed.to_string(), "failed");
        assert_eq!(RunStatus::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn run_status_from_str_round_trips() {
        for s in &[
            "pending",
            "running",
            "waiting_approval",
            "completed",
            "failed",
            "cancelled",
        ] {
            let status: RunStatus = s.parse().expect("parse");
            assert_eq!(status.to_string(), *s);
        }
    }

    #[test]
    fn run_status_from_str_rejects_unknown() {
        let err = "active".parse::<RunStatus>().unwrap_err();
        assert!(matches!(err, DbError::InvalidData(_)));
    }

    // -- ApprovalStatus enum --------------------------------------------------

    #[test]
    fn approval_status_display_is_lowercase() {
        assert_eq!(ApprovalStatus::Pending.to_string(), "pending");
        assert_eq!(ApprovalStatus::Granted.to_string(), "granted");
        assert_eq!(ApprovalStatus::Denied.to_string(), "denied");
        assert_eq!(ApprovalStatus::Expired.to_string(), "expired");
    }

    #[test]
    fn approval_status_from_str_round_trips() {
        for s in &["pending", "granted", "denied", "expired"] {
            let status: ApprovalStatus = s.parse().expect("parse");
            assert_eq!(status.to_string(), *s);
        }
    }

    #[test]
    fn approval_status_from_str_rejects_unknown() {
        let err = "approved".parse::<ApprovalStatus>().unwrap_err();
        assert!(matches!(err, DbError::InvalidData(_)));
    }

    // -- WorkflowRecord -------------------------------------------------------

    #[test]
    fn workflow_record_fields_are_accessible() {
        let id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let now = Utc::now();
        let def = serde_json::json!({
            "name": "My Workflow",
            "trigger": { "on": "message_posted" },
            "steps": [{ "id": "s1", "action": "send_message", "text": "hi" }]
        });

        let community_id = CommunityId::from_uuid(Uuid::new_v4());

        let record = WorkflowRecord {
            id,
            community_id,
            name: "My Workflow".to_owned(),
            owner_pubkey: vec![0xab; 32],
            channel_id: Some(channel_id),
            definition: def.clone(),
            definition_hash: vec![0x01, 0x02, 0x03, 0x04],
            definition_event_id: None,
            status: WorkflowStatus::Active,
            enabled: true,
            created_at: now,
            updated_at: now,
        };

        assert_eq!(record.id, id);
        assert_eq!(record.community_id, community_id);
        assert_eq!(record.name, "My Workflow");
        assert_eq!(record.owner_pubkey, vec![0xab; 32]);
        assert_eq!(record.channel_id, Some(channel_id));
        assert_eq!(record.definition, def);
        assert_eq!(record.definition_hash, vec![0x01, 0x02, 0x03, 0x04]);
        assert_eq!(record.status, WorkflowStatus::Active);
        assert!(record.enabled);
    }

    #[test]
    fn workflow_record_channel_id_can_be_none() {
        let id = Uuid::new_v4();
        let now = Utc::now();

        let record = WorkflowRecord {
            id,
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            name: "Global Workflow".to_owned(),
            owner_pubkey: vec![0x00; 32],
            channel_id: None,
            definition: serde_json::json!({}),
            definition_hash: vec![],
            definition_event_id: None,
            status: WorkflowStatus::Active,
            enabled: true,
            created_at: now,
            updated_at: now,
        };

        assert!(record.channel_id.is_none());
    }

    #[test]
    fn workflow_record_clone_is_independent() {
        let id = Uuid::new_v4();
        let now = Utc::now();

        let record = WorkflowRecord {
            id,
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            name: "Original".to_owned(),
            owner_pubkey: vec![0x01; 32],
            channel_id: None,
            definition: serde_json::json!({}),
            definition_hash: vec![0xAA],
            definition_event_id: None,
            status: WorkflowStatus::Active,
            enabled: true,
            created_at: now,
            updated_at: now,
        };

        let mut cloned = record.clone();
        cloned.name = "Cloned".to_owned();

        assert_eq!(record.name, "Original");
        assert_eq!(cloned.name, "Cloned");
    }

    #[test]
    fn workflow_record_status_variants() {
        let now = Utc::now();
        for status in &[
            WorkflowStatus::Active,
            WorkflowStatus::Disabled,
            WorkflowStatus::Archived,
        ] {
            let record = WorkflowRecord {
                id: Uuid::new_v4(),
                community_id: CommunityId::from_uuid(Uuid::new_v4()),
                name: "Test".to_owned(),
                owner_pubkey: vec![],
                channel_id: None,
                definition: serde_json::json!({}),
                definition_hash: vec![],
                definition_event_id: None,
                status: status.clone(),
                enabled: true,
                created_at: now,
                updated_at: now,
            };
            assert_eq!(&record.status, status);
        }
    }

    #[test]
    fn workflow_record_disabled_has_enabled_false() {
        let now = Utc::now();
        let record = WorkflowRecord {
            id: Uuid::new_v4(),
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            name: "Paused".to_owned(),
            owner_pubkey: vec![],
            channel_id: None,
            definition: serde_json::json!({}),
            definition_hash: vec![],
            definition_event_id: None,
            status: WorkflowStatus::Active,
            enabled: false,
            created_at: now,
            updated_at: now,
        };
        assert!(!record.enabled);
        assert_eq!(record.status, WorkflowStatus::Active);
    }

    // -- WorkflowRunRecord ----------------------------------------------------

    #[test]
    fn workflow_run_record_fields_are_accessible() {
        let id = Uuid::new_v4();
        let workflow_id = Uuid::new_v4();
        let now = Utc::now();
        let trigger_event_id = vec![0xde, 0xad, 0xbe, 0xef];

        let record = WorkflowRunRecord {
            id,
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            workflow_id,
            definition_event_id: Some(vec![0x42; 32]),
            status: RunStatus::Running,
            trigger_event_id: Some(trigger_event_id.clone()),
            current_step: 2,
            execution_trace: serde_json::json!([
                { "step": "s1", "status": "completed" }
            ]),
            trigger_context: None,
            started_at: Some(now),
            completed_at: None,
            error_message: None,
            error_code: None,
            created_at: now,
        };

        assert_eq!(record.id, id);
        assert_eq!(record.workflow_id, workflow_id);
        assert_eq!(record.status, RunStatus::Running);
        assert_eq!(record.trigger_event_id, Some(trigger_event_id));
        assert_eq!(record.current_step, 2);
        assert!(record.started_at.is_some());
        assert!(record.completed_at.is_none());
        assert!(record.error_message.is_none());
    }

    #[test]
    fn workflow_run_record_no_trigger_event() {
        let now = Utc::now();
        let record = WorkflowRunRecord {
            id: Uuid::new_v4(),
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            workflow_id: Uuid::new_v4(),
            definition_event_id: None,
            status: RunStatus::Pending,
            trigger_event_id: None,
            current_step: 0,
            execution_trace: serde_json::json!([]),
            trigger_context: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            error_code: None,
            created_at: now,
        };

        assert!(record.trigger_event_id.is_none());
        assert_eq!(record.current_step, 0);
        assert!(record.started_at.is_none());
    }

    #[test]
    fn workflow_run_record_failed_with_error_message() {
        let now = Utc::now();
        let record = WorkflowRunRecord {
            id: Uuid::new_v4(),
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            workflow_id: Uuid::new_v4(),
            definition_event_id: None,
            status: RunStatus::Failed,
            trigger_event_id: None,
            current_step: 1,
            execution_trace: serde_json::json!([]),
            trigger_context: None,
            started_at: Some(now),
            completed_at: Some(now),
            error_message: Some("step timeout exceeded".to_owned()),
            error_code: Some("step_timeout".to_owned()),
            created_at: now,
        };

        assert_eq!(record.status, RunStatus::Failed);
        assert!(record.completed_at.is_some());
        assert_eq!(
            record.error_message.as_deref(),
            Some("step timeout exceeded")
        );
    }

    #[test]
    fn workflow_run_record_execution_trace_is_json_array() {
        let now = Utc::now();
        let trace = serde_json::json!([
            { "step_id": "notify", "status": "completed", "output": { "sent": true } },
            { "step_id": "log", "status": "skipped" }
        ]);

        let record = WorkflowRunRecord {
            id: Uuid::new_v4(),
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            workflow_id: Uuid::new_v4(),
            definition_event_id: None,
            status: RunStatus::Completed,
            trigger_event_id: None,
            current_step: 2,
            execution_trace: trace.clone(),
            trigger_context: None,
            started_at: Some(now),
            completed_at: Some(now),
            error_message: None,
            error_code: None,
            created_at: now,
        };

        assert!(record.execution_trace.is_array());
        assert_eq!(record.execution_trace.as_array().unwrap().len(), 2);
    }

    #[test]
    fn workflow_run_record_clone_is_independent() {
        let now = Utc::now();
        let record = WorkflowRunRecord {
            id: Uuid::new_v4(),
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            workflow_id: Uuid::new_v4(),
            definition_event_id: None,
            status: RunStatus::Pending,
            trigger_event_id: None,
            current_step: 0,
            execution_trace: serde_json::json!([]),
            trigger_context: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            error_code: None,
            created_at: now,
        };

        let mut cloned = record.clone();
        cloned.status = RunStatus::Running;

        assert_eq!(record.status, RunStatus::Pending);
        assert_eq!(cloned.status, RunStatus::Running);
    }

    // -- ApprovalRecord -------------------------------------------------------

    #[test]
    fn approval_record_fields_are_accessible() {
        let workflow_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let expires_at = Utc.with_ymd_and_hms(2026, 12, 31, 23, 59, 59).unwrap();
        let now = Utc::now();

        let record = ApprovalRecord {
            token: b"abc123def456abc123def456abc123de".to_vec(),
            workflow_id,
            run_id,
            step_id: "request_approval".to_owned(),
            step_index: 1,
            approver_spec: "@engineering-lead".to_owned(),
            status: ApprovalStatus::Pending,
            approver_pubkey: None,
            note: None,
            expires_at,
            created_at: now,
        };

        assert_eq!(record.token, b"abc123def456abc123def456abc123de");
        assert_eq!(record.workflow_id, workflow_id);
        assert_eq!(record.run_id, run_id);
        assert_eq!(record.step_id, "request_approval");
        assert_eq!(record.step_index, 1);
        assert_eq!(record.approver_spec, "@engineering-lead");
        assert_eq!(record.status, ApprovalStatus::Pending);
        assert!(record.approver_pubkey.is_none());
        assert!(record.note.is_none());
    }

    #[test]
    fn approval_record_granted_with_pubkey_and_note() {
        let now = Utc::now();
        let approver_pubkey = vec![0xca; 32];

        let record = ApprovalRecord {
            token: b"token-granted".to_vec(),
            workflow_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            step_id: "gate".to_owned(),
            step_index: 0,
            approver_spec: "@manager".to_owned(),
            status: ApprovalStatus::Granted,
            approver_pubkey: Some(approver_pubkey.clone()),
            note: Some("Looks good, approved.".to_owned()),
            expires_at: now,
            created_at: now,
        };

        assert_eq!(record.status, ApprovalStatus::Granted);
        assert_eq!(record.approver_pubkey, Some(approver_pubkey));
        assert_eq!(record.note.as_deref(), Some("Looks good, approved."));
    }

    #[test]
    fn approval_record_denied_with_note() {
        let now = Utc::now();

        let record = ApprovalRecord {
            token: b"token-denied".to_vec(),
            workflow_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            step_id: "gate".to_owned(),
            step_index: 0,
            approver_spec: "@manager".to_owned(),
            status: ApprovalStatus::Denied,
            approver_pubkey: Some(vec![0xbb; 32]),
            note: Some("Not ready for production.".to_owned()),
            expires_at: now,
            created_at: now,
        };

        assert_eq!(record.status, ApprovalStatus::Denied);
        assert!(record.note.is_some());
    }

    #[test]
    fn approval_record_clone_is_independent() {
        let now = Utc::now();
        let record = ApprovalRecord {
            token: b"original-token".to_vec(),
            workflow_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            step_id: "gate".to_owned(),
            step_index: 0,
            approver_spec: "@lead".to_owned(),
            status: ApprovalStatus::Pending,
            approver_pubkey: None,
            note: None,
            expires_at: now,
            created_at: now,
        };

        let mut cloned = record.clone();
        cloned.status = ApprovalStatus::Granted;

        assert_eq!(record.status, ApprovalStatus::Pending);
        assert_eq!(cloned.status, ApprovalStatus::Granted);
    }

    // -- Scheduled workflow claim confinement ---------------------------------
    //
    // RECONCILED spec (supersedes the earlier S1 lock; Eva/Max 2026-06-27).
    //
    // The earlier S1 lock asserted "`workflow_id` is globally unique, so the
    // claim resolves community server-side from `workflow_id` alone and the
    // caller never names it." The final schema does NOT have that property:
    // `workflows` PK is `(community_id, id)` and `scheduled_workflow_fires` is
    // keyed/FK'd by `(community_id, workflow_id, scheduled_for)`. Duplicate
    // workflow UUIDs across communities are explicitly allowed (and pinned by
    // the Issue-4 confinement tests below). So resolve-from-id-alone is both
    // unimplementable and unsafe: `WHERE w.id = $1` matches every community
    // holding that UUID and fans one claim across all of them.
    //
    // The invariant that survives is NOT "the claim never receives community";
    // it is "the community used for the claim is server provenance, never
    // client-controlled." For the global scheduler scan that provenance is the
    // `workflow.community_id` returned by `list_all_enabled_workflows()`. The
    // claim therefore takes `community_id` and binds
    // `WHERE w.community_id = $1 AND w.id = $2`, confining the claim row to the
    // intended tenant.
    //
    //   1. `workflows.community_id` is row-owned, NOT NULL, immutable.
    //   2. The claim binds `(community_id, workflow_id)` of the workflow row.
    //   3. Claim uniqueness is `(community_id, workflow_id, scheduled_for)`.
    //   4. `latest_scheduled_workflow_fire` / `attach_scheduled_workflow_run`
    //      are already community-scoped; `claim` now matches.
    //
    // `claim_confined_to_its_community` is the confinement lock: a dup workflow
    // UUID in A and B must claim independently (claiming A/id leaves B/id
    // claimable). The other two tests are characterization guards: same-window
    // race must yield exactly one winner, and pruning below the largest
    // interval breaks `latest_*` (the §5c retention rule Sami flagged).

    use crate::user::ensure_user;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());

        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    /// Insert a community with a unique host. Returns its `CommunityId`.
    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        let host = format!("test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&host)
            .execute(pool)
            .await
            .expect("insert community");
        CommunityId::from_uuid(id)
    }

    /// Insert a channel under a community. Returns the channel id.
    async fn make_channel(pool: &PgPool, community: CommunityId, owner: &[u8]) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO channels (id, community_id, name, created_by)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(id)
        .bind(community.as_uuid())
        .bind(format!("ch-{}", id.simple()))
        .bind(owner)
        .execute(pool)
        .await
        .expect("insert channel");
        id
    }

    /// Insert a workflow whose tenant is `community`'s channel. Returns the
    /// workflow id and the owning community for callers that want to assert
    /// the resolved tenant.
    async fn make_workflow_in(pool: &PgPool, community: CommunityId) -> (Uuid, CommunityId) {
        let owner = vec![0xa1; 32];
        ensure_user(pool, community, &owner)
            .await
            .expect("ensure owner");
        let channel_id = make_channel(pool, community, &owner).await;
        let workflow_id = create_workflow(
            pool,
            community,
            Some(channel_id),
            &owner,
            "f1-attack-workflow",
            r#"{"trigger":{"on":"schedule"},"steps":[]}"#,
            &[0u8; 32],
        )
        .await
        .expect("create workflow");
        (workflow_id, community)
    }

    /// Confinement: a duplicate workflow UUID existing in both community A and
    /// community B must claim independently. Claiming `(A, id, t)` must NOT
    /// consume `(B, id, t)` — B's identical instant stays claimable, and the
    /// A-claim's resolved community is A (server provenance), never B.
    ///
    /// This is the reconciliation of the old S1 lock with the real
    /// `(community_id, id)` schema: because `id` is not globally unique, the
    /// claim binds `WHERE w.community_id = $1 AND w.id = $2`. With the old
    /// bare-`id` SQL (`WHERE w.id = $1`), a single `INSERT ... SELECT` matched
    /// BOTH workflow rows and fanned the claim across A and B — this test goes
    /// RED on that regression (B/id is no longer independently claimable).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_confined_to_its_community() {
        let pool = setup_pool().await;

        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;

        // Same workflow UUID + same channel UUID in both communities — the PK
        // is `(community_id, id)`, so the collision is structurally allowed.
        let workflow_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        insert_workflow_with_ids(&pool, community_a, workflow_id, channel_id, "sched-a").await;
        insert_workflow_with_ids(&pool, community_b, workflow_id, channel_id, "sched-b").await;

        let scheduled_for = Utc.with_ymd_and_hms(2026, 6, 27, 0, 0, 0).unwrap();

        // Claim A/id/t.
        let claim_a = claim_scheduled_workflow_fire(&pool, community_a, workflow_id, scheduled_for)
            .await
            .expect("claim A should not error")
            .expect("claim A should win");
        assert_eq!(
            claim_a.community_id, community_a,
            "A-claim must resolve to community A (server provenance)"
        );
        assert_eq!(claim_a.workflow_id, workflow_id);
        assert_eq!(claim_a.scheduled_for, scheduled_for);

        // B/id/t must still be claimable — A's claim did not touch B's row.
        let claim_b = claim_scheduled_workflow_fire(&pool, community_b, workflow_id, scheduled_for)
            .await
            .expect("claim B should not error")
            .expect("claim B must still win — A's claim must not have consumed B's instant");
        assert_eq!(
            claim_b.community_id, community_b,
            "B-claim must resolve to community B"
        );

        // And a second A-claim for the same instant must now lose (dedup holds
        // within the community).
        let claim_a_again =
            claim_scheduled_workflow_fire(&pool, community_a, workflow_id, scheduled_for)
                .await
                .expect("second A-claim should not error");
        assert!(
            claim_a_again.is_none(),
            "the same (A, id, t) instant must not be claimable twice"
        );
    }

    /// Same `(community_id, workflow_id, scheduled_for)` claimed concurrently by
    /// N tasks must yield exactly one `Some` winner. Post-reconciliation the
    /// claim key is `(community_id, workflow_id, scheduled_for)`; `community_id`
    /// is server provenance, not a client-named label. Characterization guard:
    /// protects the dedup boundary against regressions in the claim SQL.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_same_window_claims_exactly_one_wins() {
        let pool = setup_pool().await;

        let community = make_community(&pool).await;
        let (workflow_id, _) = make_workflow_in(&pool, community).await;
        let scheduled_for = Utc.with_ymd_and_hms(2026, 6, 27, 0, 1, 0).unwrap();

        const N: usize = 8;
        let mut handles = Vec::with_capacity(N);
        for _ in 0..N {
            let pool = pool.clone();
            handles.push(tokio::spawn(async move {
                claim_scheduled_workflow_fire(&pool, community, workflow_id, scheduled_for).await
            }));
        }

        let mut winners = 0usize;
        for h in handles {
            let result = h.await.expect("task did not panic").expect("claim ok");
            if result.is_some() {
                winners += 1;
            }
        }
        assert_eq!(
            winners, 1,
            "exactly one task must win the claim race for (workflow_id, scheduled_for)"
        );
    }

    /// `attach_scheduled_workflow_run` links a won claim to the run it created.
    /// This is the regression for the missing `scheduled_workflow_fires.
    /// workflow_run_id` column: before the schema added it, the UPDATE failed at
    /// runtime with `column "workflow_run_id" does not exist`, so the audit link
    /// silently never populated and the scheduler warned on every fire. This test
    /// proves the column is present, the attach writes it, and the
    /// `workflow_run_id IS NULL` guard makes a second attach a no-op. It is RED
    /// without the migration column.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn attach_links_run_to_claim_and_is_idempotent() {
        let pool = setup_pool().await;

        let community = make_community(&pool).await;
        let (workflow_id, _) = make_workflow_in(&pool, community).await;
        let scheduled_for = Utc.with_ymd_and_hms(2026, 6, 27, 0, 2, 0).unwrap();

        // Win the claim for this instant.
        claim_scheduled_workflow_fire(&pool, community, workflow_id, scheduled_for)
            .await
            .expect("claim ok")
            .expect("claim wins");

        // Create the run the won claim is responsible for, then attach it.
        let run_id = create_workflow_run(&pool, community, workflow_id, &[0x42; 32], None, None)
            .await
            .expect("create run ok");

        let attached =
            attach_scheduled_workflow_run(&pool, community, workflow_id, scheduled_for, run_id)
                .await
                .expect("attach ok");
        assert!(attached, "first attach must update the claim row");

        // The column is populated with the run id.
        let linked: Option<Uuid> = sqlx::query_scalar(
            "SELECT workflow_run_id FROM scheduled_workflow_fires \
             WHERE community_id = $1 AND workflow_id = $2 AND scheduled_for = $3",
        )
        .bind(community.as_uuid())
        .bind(workflow_id)
        .bind(scheduled_for)
        .fetch_one(&pool)
        .await
        .expect("row exists");
        assert_eq!(
            linked,
            Some(run_id),
            "the claim row must now point at the run it created"
        );

        // A second attach is a no-op: the `workflow_run_id IS NULL` guard means
        // an already-linked claim is never re-pointed to a different run.
        let other_run = create_workflow_run(&pool, community, workflow_id, &[0x42; 32], None, None)
            .await
            .expect("create second run ok");
        let reattached =
            attach_scheduled_workflow_run(&pool, community, workflow_id, scheduled_for, other_run)
                .await
                .expect("second attach ok");
        assert!(
            !reattached,
            "attach must not overwrite an already-linked claim row"
        );
    }

    /// Documents the retention-vs-interval coupling Sami flagged for §5c:
    /// pruning every claim below the workflow's interval makes
    /// `latest_scheduled_workflow_fire` return `None`, which re-introduces the
    /// per-pod-clock anchor bug F5 was meant to fix. Test is GREEN today and
    /// MUST stay green — it pins the deployment-config rule that the janitor
    /// cutoff must exceed `MAX(interval_secs) + safety margin`. If a future
    /// change makes `latest_*` resilient to pruning (e.g. by reading the most
    /// recent workflow_run instead, or by retaining a sentinel row), this
    /// test's assertion encodes the contract that must be updated alongside.
    ///
    /// Test isolation: the prune primitive is global (filters only on
    /// `claimed_at`), so to avoid colliding with parallel claim tests we
    /// back-date this workflow's `claimed_at` into the deep past and use a
    /// past cutoff that cannot match any other test's `claimed_at = NOW()`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn latest_after_prune_below_interval_breaks_anchor() {
        let pool = setup_pool().await;

        let community = make_community(&pool).await;
        let (workflow_id, _) = make_workflow_in(&pool, community).await;
        let scheduled_for = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();

        claim_scheduled_workflow_fire(&pool, community, workflow_id, scheduled_for)
            .await
            .expect("claim ok")
            .expect("first claim wins");

        // Backdate this row's `claimed_at` so the global prune below targets
        // only this workflow's row and cannot race-delete other tests' rows.
        let backdated_claimed_at = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        sqlx::query(
            "UPDATE scheduled_workflow_fires SET claimed_at = $1 \
             WHERE community_id = $2 AND workflow_id = $3 AND scheduled_for = $4",
        )
        .bind(backdated_claimed_at)
        .bind(community.as_uuid())
        .bind(workflow_id)
        .bind(scheduled_for)
        .execute(&pool)
        .await
        .expect("backdate ok");

        let latest_before = latest_scheduled_workflow_fire(&pool, community, workflow_id)
            .await
            .expect("latest ok");
        assert_eq!(
            latest_before,
            Some(scheduled_for),
            "latest must reflect the claim before pruning",
        );

        // Janitor cutoff above only the back-dated row: prunes the anchor row
        // without touching anything claimed at wall-clock NOW.
        let cutoff = backdated_claimed_at + chrono::Duration::seconds(1);
        let pruned = prune_scheduled_workflow_fires_before(&pool, cutoff)
            .await
            .expect("prune ok");
        assert!(
            pruned >= 1,
            "expected at least one row pruned, got {pruned}"
        );

        let latest_after = latest_scheduled_workflow_fire(&pool, community, workflow_id)
            .await
            .expect("latest ok");
        assert_eq!(
            latest_after, None,
            "pruning below the largest interval breaks the DB anchor; \
             retention cutoff MUST exceed MAX(interval_secs) + safety margin (§5c)",
        );
    }

    // -- Issue 4: workflow / approval community confinement -------------------

    /// Insert a workflow under `community` with a caller-chosen `id` and
    /// `channel_id`, so two communities can be given the *same* workflow UUID
    /// and channel UUID (the PK is `(community_id, id)`, which structurally
    /// allows the collision). Returns nothing; callers already hold the ids.
    async fn insert_workflow_with_ids(
        pool: &PgPool,
        community: CommunityId,
        id: Uuid,
        channel_id: Uuid,
        name: &str,
    ) {
        let owner = vec![0xb2; 32];
        ensure_user(pool, community, &owner)
            .await
            .expect("ensure owner");
        // The channel must exist first: `workflows.channel_id` is a composite FK
        // to `(community_id, channel_id)`.
        sqlx::query(
            r#"
            INSERT INTO channels (id, community_id, name, created_by)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(channel_id)
        .bind(community.as_uuid())
        .bind(format!("ch-{}", channel_id.simple()))
        .bind(&owner)
        .execute(pool)
        .await
        .expect("insert channel");
        sqlx::query(
            r#"
            INSERT INTO workflows
                (id, community_id, name, owner_pubkey, channel_id, definition, definition_hash, status, enabled)
            VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, 'active', TRUE)
            "#,
        )
        .bind(id)
        .bind(community.as_uuid())
        .bind(name)
        .bind(&owner)
        .bind(channel_id)
        .bind(r#"{"trigger":{"on":"webhook"},"steps":[]}"#)
        .bind(&[0u8; 32][..])
        .execute(pool)
        .await
        .expect("insert workflow");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_revision_after_replacement_creates_no_run() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let workflow_id = Uuid::new_v4();
        insert_workflow_with_ids(
            &pool,
            community,
            workflow_id,
            Uuid::new_v4(),
            "revision-race",
        )
        .await;
        let revision_a = vec![0xa1u8; 32];
        let revision_b = vec![0xb2u8; 32];
        sqlx::query(
            "UPDATE workflows SET definition_event_id = $3 \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(workflow_id)
        .bind(revision_a.as_slice())
        .execute(&pool)
        .await
        .expect("install revision A");

        // B wins before stale trigger A enters its commit transaction.
        sqlx::query(
            "UPDATE workflows SET definition_event_id = $3 \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(workflow_id)
        .bind(revision_b.as_slice())
        .execute(&pool)
        .await
        .expect("replace with revision B");

        let mut trigger = pool.begin().await.expect("begin stale trigger");
        let current = get_workflow_for_share_in_transaction(&mut trigger, community, workflow_id)
            .await
            .expect("lock current workflow");
        assert_ne!(
            current.definition_event_id.as_deref(),
            Some(revision_a.as_slice())
        );
        trigger.rollback().await.expect("reject stale trigger");

        let runs: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_runs WHERE community_id = $1 AND workflow_id = $2",
        )
        .bind(community.as_uuid())
        .bind(workflow_id)
        .fetch_one(&pool)
        .await
        .expect("count runs after stale rejection");
        assert_eq!(runs, 0, "stale revision must not create a workflow run");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_run_transaction_rolls_back_and_retry_creates_exactly_one_run() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let workflow_id = Uuid::new_v4();
        insert_workflow_with_ids(
            &pool,
            community,
            workflow_id,
            Uuid::new_v4(),
            "atomic-trigger",
        )
        .await;
        let trigger_event_id = vec![0x7au8; 32];
        let trigger_pubkey = vec![0x7bu8; 32];
        let trigger_sig = vec![0x7cu8; 64];

        let mut aborted = pool.begin().await.expect("begin aborted transaction");
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at) \
             VALUES ($1, $2, $3, NOW(), 46020, '[]'::jsonb, '', $4, NOW())",
        )
        .bind(community.as_uuid())
        .bind(trigger_event_id.as_slice())
        .bind(trigger_pubkey.as_slice())
        .bind(trigger_sig.as_slice())
        .execute(&mut *aborted)
        .await
        .expect("insert trigger event before abort");
        create_workflow_run_in_transaction(
            &mut aborted,
            community,
            workflow_id,
            &[0x42; 32],
            Some(&trigger_event_id),
            None,
        )
        .await
        .expect("insert run before abort");
        aborted.rollback().await.expect("roll back run");

        let after_abort_event: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE community_id = $1 AND id = $2")
                .bind(community.as_uuid())
                .bind(&trigger_event_id)
                .fetch_one(&pool)
                .await
                .expect("count rolled-back trigger events");
        assert_eq!(
            after_abort_event, 0,
            "aborted trigger must leave no event row"
        );
        let after_abort: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_runs WHERE community_id = $1 AND trigger_event_id = $2",
        )
        .bind(community.as_uuid())
        .bind(&trigger_event_id)
        .fetch_one(&pool)
        .await
        .expect("count rolled-back runs");
        assert_eq!(after_abort, 0, "aborted trigger must leave no run row");

        let mut retry = pool.begin().await.expect("begin retry transaction");
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at) \
             VALUES ($1, $2, $3, NOW(), 46020, '[]'::jsonb, '', $4, NOW())",
        )
        .bind(community.as_uuid())
        .bind(trigger_event_id.as_slice())
        .bind(trigger_pubkey.as_slice())
        .bind(trigger_sig.as_slice())
        .execute(&mut *retry)
        .await
        .expect("insert trigger event on retry");
        create_workflow_run_in_transaction(
            &mut retry,
            community,
            workflow_id,
            &[0x42; 32],
            Some(&trigger_event_id),
            None,
        )
        .await
        .expect("insert retry run");
        retry.commit().await.expect("commit retry run");

        let after_retry_event: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE community_id = $1 AND id = $2")
                .bind(community.as_uuid())
                .bind(&trigger_event_id)
                .fetch_one(&pool)
                .await
                .expect("count committed trigger events");
        assert_eq!(
            after_retry_event, 1,
            "retry must create exactly one event row"
        );
        let after_retry: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_runs WHERE community_id = $1 AND trigger_event_id = $2",
        )
        .bind(community.as_uuid())
        .bind(&trigger_event_id)
        .fetch_one(&pool)
        .await
        .expect("count committed retry runs");
        assert_eq!(after_retry, 1, "retry must create exactly one run row");
    }

    /// Issue 4 (workflow identity): the same workflow UUID and channel UUID can
    /// exist in communities A and B (PK `(community_id, id)`). A request-scoped
    /// `get_workflow` / `list_enabled_channel_workflows` MUST return only the
    /// row owned by the bound community — never B's colliding row for an
    /// A-scoped lookup. Pre-fix these bound only `id` / `channel_id`, so a
    /// B-host request (or a webhook/manual trigger satisfying membership against
    /// B's colliding channel) could load and drive A's workflow.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_lookup_is_confined_to_its_community() {
        let pool = setup_pool().await;

        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;

        // Same workflow UUID and channel UUID in both communities.
        let shared_workflow_id = Uuid::new_v4();
        let shared_channel_id = Uuid::new_v4();
        insert_workflow_with_ids(
            &pool,
            community_a,
            shared_workflow_id,
            shared_channel_id,
            "wf-A",
        )
        .await;
        insert_workflow_with_ids(
            &pool,
            community_b,
            shared_workflow_id,
            shared_channel_id,
            "wf-B",
        )
        .await;

        // Scoped get returns each community's own row, never the other's.
        let from_a = get_workflow(&pool, community_a, shared_workflow_id)
            .await
            .expect("A's workflow exists");
        let from_b = get_workflow(&pool, community_b, shared_workflow_id)
            .await
            .expect("B's workflow exists");
        assert_eq!(
            from_a.community_id, community_a,
            "A lookup must resolve A's row"
        );
        assert_eq!(from_a.name, "wf-A");
        assert_eq!(
            from_b.community_id, community_b,
            "B lookup must resolve B's row"
        );
        assert_eq!(from_b.name, "wf-B");

        // A workflow that exists ONLY in B must be NotFound under A.
        let b_only_id = Uuid::new_v4();
        let b_only_channel = Uuid::new_v4();
        insert_workflow_with_ids(&pool, community_b, b_only_id, b_only_channel, "wf-B-only").await;
        let cross = get_workflow(&pool, community_a, b_only_id).await;
        assert!(
            matches!(cross, Err(DbError::NotFound(_))),
            "A must not see B's workflow by id: {cross:?}"
        );

        // The channel listing is confined too: A's channel listing yields only
        // A's workflow even though B has the same channel UUID.
        let listed_a = list_enabled_channel_workflows(&pool, community_a, shared_channel_id)
            .await
            .expect("list A");
        assert_eq!(
            listed_a.len(),
            1,
            "A's channel listing must contain exactly A's workflow"
        );
        assert_eq!(listed_a[0].community_id, community_a);
        assert_eq!(listed_a[0].name, "wf-A");
    }

    /// Issue 4 (workflow lifecycle): deleting `A/id` must not delete `B/id`
    /// when both communities hold the same workflow UUID. Pre-fix
    /// `delete_workflow` predicated only on `id`, so a NIP-09 a-tag deletion in
    /// one community would erase the colliding workflow in every community.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_delete_is_confined_to_its_community() {
        let pool = setup_pool().await;

        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;
        let shared_workflow_id = Uuid::new_v4();
        insert_workflow_with_ids(
            &pool,
            community_a,
            shared_workflow_id,
            Uuid::new_v4(),
            "wf-A",
        )
        .await;
        insert_workflow_with_ids(
            &pool,
            community_b,
            shared_workflow_id,
            Uuid::new_v4(),
            "wf-B",
        )
        .await;

        delete_workflow(&pool, community_a, shared_workflow_id)
            .await
            .expect("delete A's workflow");

        // A's row is gone; B's identical-UUID row survives untouched.
        assert!(
            matches!(
                get_workflow(&pool, community_a, shared_workflow_id).await,
                Err(DbError::NotFound(_))
            ),
            "A's workflow must be deleted"
        );
        let surviving_b = get_workflow(&pool, community_b, shared_workflow_id)
            .await
            .expect("B's workflow must survive A's delete");
        assert_eq!(surviving_b.community_id, community_b);
        assert_eq!(surviving_b.name, "wf-B");
    }

    /// Issue 4 (approval path): the same approval token can hash to the same
    /// bytes in A and B (PK `(community_id, token)`). A scoped grant/deny acting
    /// on `A/token` MUST NOT touch `B/token`. Pre-fix the approval helpers
    /// predicated only on `token`, so granting one community's approval would
    /// silently resolve another's colliding gate.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn approval_is_confined_to_its_community() {
        let pool = setup_pool().await;

        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;

        // Same workflow + run + token in both communities.
        let workflow_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        insert_workflow_with_ids(&pool, community_a, workflow_id, channel_id, "wf-A").await;
        insert_workflow_with_ids(&pool, community_b, workflow_id, Uuid::new_v4(), "wf-B").await;

        let run_a = create_workflow_run(&pool, community_a, workflow_id, &[0x42; 32], None, None)
            .await
            .expect("run A");
        let run_b = create_workflow_run(&pool, community_b, workflow_id, &[0x42; 32], None, None)
            .await
            .expect("run B");

        let token = "shared-approval-token";
        let expires = Utc::now() + chrono::Duration::hours(1);
        create_approval(
            &pool,
            CreateApprovalParams {
                community_id: community_a,
                token,
                workflow_id,
                run_id: run_a,
                step_id: "gate",
                step_index: 0,
                approver_spec: "@anyone",
                expires_at: expires,
            },
        )
        .await
        .expect("create approval A");
        create_approval(
            &pool,
            CreateApprovalParams {
                community_id: community_b,
                token,
                workflow_id,
                run_id: run_b,
                step_id: "gate",
                step_index: 0,
                approver_spec: "@anyone",
                expires_at: expires,
            },
        )
        .await
        .expect("create approval B");

        // Scoped read returns each community's own approval (its own run id).
        let read_a = get_approval(&pool, community_a, token)
            .await
            .expect("read A");
        let read_b = get_approval(&pool, community_b, token)
            .await
            .expect("read B");
        assert_eq!(read_a.run_id, run_a, "A read must resolve A's approval");
        assert_eq!(read_b.run_id, run_b, "B read must resolve B's approval");

        // Granting A/token must NOT act on B/token.
        let approver = vec![0xc3; 32];
        let granted = update_approval(
            &pool,
            community_a,
            token,
            ApprovalStatus::Granted,
            Some(&approver),
            None,
        )
        .await
        .expect("grant A");
        assert!(granted, "A's approval must be granted");

        let after_a = get_approval(&pool, community_a, token)
            .await
            .expect("re-read A");
        let after_b = get_approval(&pool, community_b, token)
            .await
            .expect("re-read B");
        assert_eq!(after_a.status, ApprovalStatus::Granted, "A is now granted");
        assert_eq!(
            after_b.status,
            ApprovalStatus::Pending,
            "B's approval must remain pending after A is granted"
        );
    }

    // -- SEC-006: disable-on-membership-loss primitive -------------------------

    /// `disable_workflows_for_owner_in_channel` must disable exactly the
    /// departing owner's enabled workflows in that channel — not other owners'
    /// workflows, not the same owner's workflows in other channels — and be
    /// idempotent. Disabled workflows must drop out of the trigger-path list.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn disable_for_owner_scopes_to_owner_and_channel() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;

        let departing = vec![0xd1; 32];
        let staying = vec![0xd2; 32];
        ensure_user(&pool, community, &departing)
            .await
            .expect("ensure departing");
        ensure_user(&pool, community, &staying)
            .await
            .expect("ensure staying");

        let channel_a = make_channel(&pool, community, &departing).await;
        let channel_b = make_channel(&pool, community, &departing).await;

        let def = r#"{"trigger":{"on":"message_posted"},"steps":[]}"#;
        let wf_departing_a = create_workflow(
            &pool,
            community,
            Some(channel_a),
            &departing,
            "departing-a",
            def,
            &[0u8; 32],
        )
        .await
        .expect("wf departing a");
        let wf_departing_b = create_workflow(
            &pool,
            community,
            Some(channel_b),
            &departing,
            "departing-b",
            def,
            &[0u8; 32],
        )
        .await
        .expect("wf departing b");
        let wf_staying_a = create_workflow(
            &pool,
            community,
            Some(channel_a),
            &staying,
            "staying-a",
            def,
            &[0u8; 32],
        )
        .await
        .expect("wf staying a");

        let disabled =
            disable_workflows_for_owner_in_channel(&pool, community, channel_a, &departing)
                .await
                .expect("disable");
        assert_eq!(
            disabled, 1,
            "exactly the departing owner's channel-A workflow"
        );

        // Idempotent: second call finds nothing enabled.
        let again = disable_workflows_for_owner_in_channel(&pool, community, channel_a, &departing)
            .await
            .expect("disable again");
        assert_eq!(again, 0, "second disable must be a no-op");

        let enabled_a = list_enabled_channel_workflows(&pool, community, channel_a)
            .await
            .expect("list channel a");
        let enabled_a_ids: Vec<Uuid> = enabled_a.iter().map(|w| w.id).collect();
        assert!(
            !enabled_a_ids.contains(&wf_departing_a),
            "departing owner's workflow must no longer be trigger-eligible"
        );
        assert!(
            enabled_a_ids.contains(&wf_staying_a),
            "other owners' workflows in the channel must be untouched"
        );

        let enabled_b = list_enabled_channel_workflows(&pool, community, channel_b)
            .await
            .expect("list channel b");
        assert!(
            enabled_b.iter().any(|w| w.id == wf_departing_b),
            "same owner's workflow in a different channel must be untouched"
        );
    }

    // -- Workflow agent delivery state machine --------------------------------
    //
    // These fresh-Postgres proofs exercise the complete transition system:
    // create-idempotency, community/target claim isolation, fenced leases,
    // reap, terminal-once finish, uncertain-completion convergence, the
    // fleet-wide lifecycle fence, deletion cascade, and commit rollback. Each
    // requires a disposable Postgres and is `#[ignore]` by default.

    use nostr::Keys;

    /// Scaffold under one community: workflow + run + a real message event, and
    /// return a canonical binding for `target` plus the identifiers needed to
    /// commit and claim it.
    async fn make_delivery_scaffold(
        pool: &PgPool,
        community: CommunityId,
        target: &PublicKey,
    ) -> (WorkflowAgentDelivery, DateTime<Utc>) {
        let owner = vec![0xc3; 32];
        ensure_user(pool, community, &owner)
            .await
            .expect("ensure owner");
        let channel_id = make_channel(pool, community, &owner).await;
        let definition_event_id = EventId::from_byte_array([0x11; 32]);
        let workflow_id = create_workflow(
            pool,
            community,
            Some(channel_id),
            &owner,
            "delivery-wf",
            r#"{"trigger":{"on":"webhook"},"steps":[]}"#,
            &[0u8; 32],
        )
        .await
        .expect("create workflow");
        let run_id = create_workflow_run(
            pool,
            community,
            workflow_id,
            definition_event_id.as_bytes(),
            None,
            None,
        )
        .await
        .expect("create run");

        // A real message event row so the delivery's events FK is satisfiable.
        let message = EventBuilderKeys::signed_kind9(&owner, channel_id);
        let message_event_created_at = message.created_at;
        insert_test_event(pool, community, channel_id, &message).await;

        let binding = WorkflowDeliveryBinding::new(
            community,
            workflow_id,
            run_id,
            "notify",
            *target,
            definition_event_id,
            message.id,
            WorkflowDeliveryCause::Event(EventId::from_byte_array([0x33; 32])),
        )
        .expect("valid binding");

        (
            WorkflowAgentDelivery {
                id: WorkflowDeliveryId::from_uuid(Uuid::new_v4()),
                binding,
            },
            message_event_created_at,
        )
    }

    /// Minimal event fixture: id, pubkey, created_at, kind.
    struct TestEvent {
        id: EventId,
        pubkey: Vec<u8>,
        created_at: DateTime<Utc>,
    }

    struct EventBuilderKeys;
    impl EventBuilderKeys {
        fn signed_kind9(pubkey: &[u8], _channel_id: Uuid) -> TestEvent {
            // A unique 32-byte id: the fresh UUID's 16 bytes written twice.
            let uuid = Uuid::new_v4().into_bytes();
            let mut id = [0u8; 32];
            id[..16].copy_from_slice(&uuid);
            id[16..].copy_from_slice(&uuid);
            TestEvent {
                id: EventId::from_byte_array(id),
                pubkey: pubkey.to_vec(),
                created_at: Utc::now(),
            }
        }
    }

    async fn insert_test_event(
        pool: &PgPool,
        community: CommunityId,
        channel_id: Uuid,
        event: &TestEvent,
    ) {
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id) \
             VALUES ($1,$2,$3,$4,$5,$6::jsonb,$7,$8,$9)",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().to_vec())
        .bind(&event.pubkey)
        .bind(event.created_at)
        .bind(9i32)
        .bind("[]")
        .bind("")
        .bind(vec![0u8; 64])
        .bind(channel_id)
        .execute(pool)
        .await
        .expect("insert message event");
    }

    async fn commit_one(
        pool: &PgPool,
        community: CommunityId,
        delivery: &WorkflowAgentDelivery,
        message_created_at: DateTime<Utc>,
    ) -> Vec<WorkflowDeliveryId> {
        let run_id = delivery.binding.run_id();
        let step = delivery.binding.step_id().to_owned();
        let (tx, _existing) = lock_workflow_agent_delivery_identity(pool, community, run_id, &step)
            .await
            .expect("lock identity");
        commit_workflow_agent_deliveries(
            tx,
            community,
            message_created_at,
            std::slice::from_ref(delivery),
        )
        .await
        .expect("commit deliveries")
    }

    /// Fence a community so its `deletion_state` is no longer `active`, exactly
    /// as the deletion control plane would.
    ///
    /// The `enforce_community_tombstone` trigger only admits a lifecycle
    /// transition when the transaction-local executor GUCs name this community
    /// and its new fence generation, so mirror `set_executor_gucs`: set both
    /// GUCs and perform the bump in a single transaction.
    async fn fence_community(pool: &PgPool, community: CommunityId) {
        let mut tx = pool.begin().await.expect("begin fence tx");
        sqlx::query(
            "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                    set_config('buzz.deletion_fence_generation', $2, true)",
        )
        .bind(community.as_uuid().to_string())
        .bind("1")
        .execute(&mut *tx)
        .await
        .expect("set executor gucs");
        sqlx::query(
            "UPDATE communities \
             SET deletion_state = 'quiescing', deletion_fence_generation = 1 \
             WHERE id = $1",
        )
        .bind(community.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("fence community");
        tx.commit().await.expect("commit fence tx");
    }

    /// A duplicate producer retry for the same (community, run, step, target)
    /// must collapse to exactly one row.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn create_is_idempotent_across_producer_retries() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let target = Keys::generate().public_key();
        let (delivery, created_at) = make_delivery_scaffold(&pool, community, &target).await;

        let first = commit_one(&pool, community, &delivery, created_at).await;
        assert_eq!(first, vec![delivery.id], "first commit creates the row");

        // Retry with the same identity but a different delivery id: the unique
        // (community, run, step, target) collapses it to a no-op.
        let retry = WorkflowAgentDelivery {
            id: WorkflowDeliveryId::from_uuid(Uuid::new_v4()),
            binding: delivery.binding.clone(),
        };
        let second = commit_one(&pool, community, &retry, created_at).await;
        assert!(second.is_empty(), "duplicate producer retry must collapse");

        let stored = get_workflow_agent_delivery(&pool, community, delivery.id)
            .await
            .expect("get")
            .expect("original row survives");
        assert_eq!(stored.status, WorkflowDeliveryStatus::Pending);
        assert_eq!(
            stored.binding, delivery.binding,
            "binding persisted verbatim"
        );
    }

    /// Same delivery UUID in two communities must claim independently, and one
    /// target cannot consume another target's row.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_is_community_and_target_isolated() {
        let pool = setup_pool().await;
        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;
        let target = Keys::generate().public_key();
        let other_target = Keys::generate().public_key();

        let (mut da, ca) = make_delivery_scaffold(&pool, community_a, &target).await;
        let (mut db, cb) = make_delivery_scaffold(&pool, community_b, &target).await;
        // Force the same delivery UUID across communities: PK is (community, id).
        let shared = WorkflowDeliveryId::from_uuid(Uuid::new_v4());
        da.id = shared;
        db.id = shared;
        commit_one(&pool, community_a, &da, ca).await;
        commit_one(&pool, community_b, &db, cb).await;

        // A wrong target cannot claim A's row.
        let wrong = claim_workflow_agent_delivery(
            &pool,
            community_a,
            &other_target,
            Some(shared),
            None,
            30,
        )
        .await
        .expect("claim");
        assert!(wrong.is_none(), "another target must not claim the row");

        // Claiming A does not consume B's identical UUID.
        let claim_a =
            claim_workflow_agent_delivery(&pool, community_a, &target, Some(shared), None, 30)
                .await
                .expect("claim a")
                .expect("A claimable");
        assert_eq!(claim_a.0.community_id, community_a);
        let claim_b =
            claim_workflow_agent_delivery(&pool, community_b, &target, Some(shared), None, 30)
                .await
                .expect("claim b")
                .expect("B still claimable — A's claim must not consume it");
        assert_eq!(claim_b.0.community_id, community_b);

        // A second claim in A now loses (the row is claimed).
        let again =
            claim_workflow_agent_delivery(&pool, community_a, &target, Some(shared), None, 30)
                .await
                .expect("claim a again");
        assert!(again.is_none(), "claimed row must not be claimable twice");
    }

    /// A forged/stale wake binding that disagrees with the row makes the claim a
    /// miss rather than an alternate authority path.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_rejects_mismatched_expected_binding() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let target = Keys::generate().public_key();
        let (delivery, created_at) = make_delivery_scaffold(&pool, community, &target).await;
        commit_one(&pool, community, &delivery, created_at).await;

        // Build an expected binding with a different step_id.
        let bad = WorkflowDeliveryBinding::new(
            community,
            delivery.binding.workflow_id(),
            delivery.binding.run_id(),
            "different-step",
            target,
            delivery.binding.definition_event_id(),
            delivery.binding.message_event_id(),
            delivery.binding.cause().clone(),
        )
        .expect("binding");
        let miss = claim_workflow_agent_delivery(
            &pool,
            community,
            &target,
            Some(delivery.id),
            Some(&bad),
            30,
        )
        .await
        .expect("claim");
        assert!(miss.is_none(), "mismatched binding must not claim");

        // A binding that disagrees only on the cause identity is also a miss:
        // the full decomposed cause is load-bearing, not just the shared cols.
        let bad_cause = WorkflowDeliveryBinding::new(
            community,
            delivery.binding.workflow_id(),
            delivery.binding.run_id(),
            delivery.binding.step_id(),
            target,
            delivery.binding.definition_event_id(),
            delivery.binding.message_event_id(),
            WorkflowDeliveryCause::Webhook {
                invocation_id: Uuid::new_v4(),
            },
        )
        .expect("binding");
        let miss_cause = claim_workflow_agent_delivery(
            &pool,
            community,
            &target,
            Some(delivery.id),
            Some(&bad_cause),
            30,
        )
        .await
        .expect("claim");
        assert!(
            miss_cause.is_none(),
            "a binding disagreeing only on cause must not claim"
        );

        // A binding that disagrees only on workflow_id is likewise a miss.
        let bad_workflow = WorkflowDeliveryBinding::new(
            community,
            Uuid::new_v4(),
            delivery.binding.run_id(),
            delivery.binding.step_id(),
            target,
            delivery.binding.definition_event_id(),
            delivery.binding.message_event_id(),
            delivery.binding.cause().clone(),
        )
        .expect("binding");
        let miss_workflow = claim_workflow_agent_delivery(
            &pool,
            community,
            &target,
            Some(delivery.id),
            Some(&bad_workflow),
            30,
        )
        .await
        .expect("claim");
        assert!(
            miss_workflow.is_none(),
            "a binding disagreeing only on workflow_id must not claim"
        );

        // The matching binding claims.
        let hit = claim_workflow_agent_delivery(
            &pool,
            community,
            &target,
            Some(delivery.id),
            Some(&delivery.binding),
            30,
        )
        .await
        .expect("claim")
        .expect("matching binding claims");
        assert_eq!(hit.1.status, WorkflowDeliveryStatus::Claimed);
    }

    /// renew and finish under a stale (superseded) lease generation fail closed.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lease_is_fenced_by_generation() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let target = Keys::generate().public_key();
        let (delivery, created_at) = make_delivery_scaffold(&pool, community, &target).await;
        commit_one(&pool, community, &delivery, created_at).await;

        let (lease, _) =
            claim_workflow_agent_delivery(&pool, community, &target, Some(delivery.id), None, 30)
                .await
                .expect("claim")
                .expect("claimable");

        // A fabricated stale lease with the wrong generation must fail closed.
        let stale = WorkflowDeliveryLease {
            lease_generation: lease.lease_generation - 1,
            ..lease
        };
        assert_eq!(
            renew_workflow_agent_delivery(&pool, &stale, 30)
                .await
                .expect("renew"),
            WorkflowDeliveryRenewOutcome::LeaseLost
        );
        assert_eq!(
            finish_workflow_agent_delivery(&pool, &stale, WorkflowDeliveryOutcome::Finished)
                .await
                .expect("finish"),
            WorkflowDeliveryFinishOutcome::LeaseLost
        );

        // The current lease renews and finishes.
        assert!(matches!(
            renew_workflow_agent_delivery(&pool, &lease, 30)
                .await
                .expect("renew"),
            WorkflowDeliveryRenewOutcome::Renewed(_)
        ));
    }

    /// An expired lease is reclaimed by the reaper, its generation bumped, and
    /// the prior holder's finish/renew then fails closed.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reap_reclaims_expired_and_fences_prior_holder() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let target = Keys::generate().public_key();
        let (delivery, created_at) = make_delivery_scaffold(&pool, community, &target).await;
        commit_one(&pool, community, &delivery, created_at).await;

        // Claim with a lease so short it is already expiring; force expiry.
        let (lease, _) =
            claim_workflow_agent_delivery(&pool, community, &target, Some(delivery.id), None, 1)
                .await
                .expect("claim")
                .expect("claimable");
        sqlx::query(
            "UPDATE workflow_agent_deliveries SET lease_until = NOW() - INTERVAL '1 minute' \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(delivery.id.as_uuid())
        .execute(&pool)
        .await
        .expect("force expiry");

        let reclaimed = reap_expired_workflow_agent_deliveries(&pool)
            .await
            .expect("reap");
        assert_eq!(reclaimed, 1, "expired lease reclaimed");

        let after = get_workflow_agent_delivery(&pool, community, delivery.id)
            .await
            .expect("get")
            .expect("row")
            .clone();
        assert_eq!(after.status, WorkflowDeliveryStatus::Pending);
        assert!(
            after.lease_generation > lease.lease_generation,
            "generation bumped"
        );
        assert!(after.lease_until.is_none());

        // Prior holder's finish now fails closed (its generation is stale).
        assert_eq!(
            finish_workflow_agent_delivery(&pool, &lease, WorkflowDeliveryOutcome::Finished)
                .await
                .expect("finish"),
            WorkflowDeliveryFinishOutcome::LeaseLost
        );

        // The row is claimable again under a fresh lease.
        let reclaim =
            claim_workflow_agent_delivery(&pool, community, &target, Some(delivery.id), None, 30)
                .await
                .expect("claim")
                .expect("re-claimable after reap");
        assert!(reclaim.0.lease_generation > after.lease_generation - 1);
    }

    /// finish is once-only and an uncertain-completion retry converges to the
    /// same terminal.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn finish_is_terminal_once_and_reconciles_idempotently() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let target = Keys::generate().public_key();
        let (delivery, created_at) = make_delivery_scaffold(&pool, community, &target).await;
        commit_one(&pool, community, &delivery, created_at).await;

        let (lease, _) =
            claim_workflow_agent_delivery(&pool, community, &target, Some(delivery.id), None, 30)
                .await
                .expect("claim")
                .expect("claimable");

        assert_eq!(
            finish_workflow_agent_delivery(&pool, &lease, WorkflowDeliveryOutcome::Failed)
                .await
                .expect("finish"),
            WorkflowDeliveryFinishOutcome::Settled(WorkflowDeliveryOutcome::Failed)
        );

        // A retry (uncertain completion) under the same lease converges to the
        // recorded terminal rather than reopening or flipping it — even if the
        // retry requests a different outcome.
        assert_eq!(
            finish_workflow_agent_delivery(&pool, &lease, WorkflowDeliveryOutcome::Finished)
                .await
                .expect("finish retry"),
            WorkflowDeliveryFinishOutcome::AlreadyTerminal(WorkflowDeliveryStatus::Failed)
        );

        let stored = get_workflow_agent_delivery(&pool, community, delivery.id)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(stored.status, WorkflowDeliveryStatus::Failed);
        assert!(stored.finished_at.is_some());
        assert!(stored.lease_until.is_none());
    }

    /// The reaper is a fleet-wide scan that must skip a non-active tenant, and
    /// the write fence must block any direct mutation on a fenced tenant.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reaper_and_fence_respect_community_lifecycle() {
        let pool = setup_pool().await;
        let healthy = make_community(&pool).await;
        let fenced = make_community(&pool).await;
        let target = Keys::generate().public_key();

        let (dh, ch) = make_delivery_scaffold(&pool, healthy, &target).await;
        let (df, cf) = make_delivery_scaffold(&pool, fenced, &target).await;
        commit_one(&pool, healthy, &dh, ch).await;
        commit_one(&pool, fenced, &df, cf).await;

        // Both claimed and both expired.
        for (community, delivery) in [(healthy, &dh), (fenced, &df)] {
            claim_workflow_agent_delivery(&pool, community, &target, Some(delivery.id), None, 1)
                .await
                .expect("claim")
                .expect("claimable");
        }
        // Expire only this test's two tenants' leases: the reaper scan is
        // fleet-wide by contract, so expiring every claimed row would sweep in
        // sibling tests' rows and make the count non-deterministic.
        sqlx::query(
            "UPDATE workflow_agent_deliveries SET lease_until = NOW() - INTERVAL '1 minute' \
             WHERE status = 'claimed' AND community_id IN ($1, $2)",
        )
        .bind(healthy.as_uuid())
        .bind(fenced.as_uuid())
        .execute(&pool)
        .await
        .expect("force expiry");

        // Fence one tenant, then reap. Only the healthy tenant's row is reclaimed.
        fence_community(&pool, fenced).await;
        let reclaimed = reap_expired_workflow_agent_deliveries(&pool)
            .await
            .expect("reap");
        assert_eq!(reclaimed, 1, "reaper must skip the non-active tenant");

        let healthy_row = get_workflow_agent_delivery(&pool, healthy, dh.id)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(healthy_row.status, WorkflowDeliveryStatus::Pending);
        let fenced_row = get_workflow_agent_delivery(&pool, fenced, df.id)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(
            fenced_row.status,
            WorkflowDeliveryStatus::Claimed,
            "fenced tenant's delivery is untouched by the reaper"
        );

        // A direct mutation on the fenced tenant is rejected by the write fence.
        let direct = sqlx::query(
            "UPDATE workflow_agent_deliveries SET status = 'pending' WHERE community_id = $1",
        )
        .bind(fenced.as_uuid())
        .execute(&pool)
        .await;
        assert!(
            direct.is_err(),
            "write fence must block a fenced-tenant mutation"
        );
    }

    /// A terminal written by a NEWER holder must not be laundered back to a
    /// stale prior holder as its own successful reconciliation. Reproduce the
    /// exact interleaving Larry flagged: gen-1 claims and expires, the reaper
    /// bumps to a fresh generation, gen-2 claims and finishes, then gen-1 calls
    /// `finish`. The gen-1 UPDATE correctly misses, and the generation-fenced
    /// terminal read must return `LeaseLost` rather than `AlreadyTerminal`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_holder_finish_after_newer_holder_settles_fails_closed() {
        // Exercise both the same-outcome and conflicting-outcome interleavings:
        // neither may attribute the newer holder's terminal to the stale one.
        for (gen2_outcome, gen1_retry) in [
            (
                WorkflowDeliveryOutcome::Finished,
                WorkflowDeliveryOutcome::Finished,
            ),
            (
                WorkflowDeliveryOutcome::Finished,
                WorkflowDeliveryOutcome::Failed,
            ),
        ] {
            let pool = setup_pool().await;
            let community = make_community(&pool).await;
            let target = Keys::generate().public_key();
            let (delivery, created_at) = make_delivery_scaffold(&pool, community, &target).await;
            commit_one(&pool, community, &delivery, created_at).await;

            // gen-1 claims, then its lease is expired and reaped.
            let (gen1, _) = claim_workflow_agent_delivery(
                &pool,
                community,
                &target,
                Some(delivery.id),
                None,
                1,
            )
            .await
            .expect("claim")
            .expect("claimable");
            sqlx::query(
                "UPDATE workflow_agent_deliveries SET lease_until = NOW() - INTERVAL '1 minute' \
                 WHERE community_id = $1 AND id = $2",
            )
            .bind(community.as_uuid())
            .bind(delivery.id.as_uuid())
            .execute(&pool)
            .await
            .expect("force expiry");
            assert_eq!(
                reap_expired_workflow_agent_deliveries(&pool)
                    .await
                    .expect("reap"),
                1
            );

            // gen-2 claims the reaped row (higher generation) and finishes it.
            let (gen2, _) = claim_workflow_agent_delivery(
                &pool,
                community,
                &target,
                Some(delivery.id),
                None,
                30,
            )
            .await
            .expect("claim")
            .expect("re-claimable");
            assert!(gen2.lease_generation > gen1.lease_generation, "reap bumped");
            assert_eq!(
                finish_workflow_agent_delivery(&pool, &gen2, gen2_outcome)
                    .await
                    .expect("gen2 finish"),
                WorkflowDeliveryFinishOutcome::Settled(gen2_outcome)
            );

            // gen-1's finish must fail closed, NOT converge to gen-2's terminal.
            assert_eq!(
                finish_workflow_agent_delivery(&pool, &gen1, gen1_retry)
                    .await
                    .expect("gen1 finish"),
                WorkflowDeliveryFinishOutcome::LeaseLost,
                "a stale holder must not be credited with a newer holder's terminal"
            );
        }
    }

    /// A lease cannot depend on the reaper running first to become invalid: once
    /// the deadline passes, neither renew nor finish may resurrect authority or
    /// settle out-of-lease work, even before the reaper wins.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expired_lease_cannot_renew_or_finish_before_reap() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let target = Keys::generate().public_key();
        let (delivery, created_at) = make_delivery_scaffold(&pool, community, &target).await;
        commit_one(&pool, community, &delivery, created_at).await;

        let (lease, _) =
            claim_workflow_agent_delivery(&pool, community, &target, Some(delivery.id), None, 30)
                .await
                .expect("claim")
                .expect("claimable");

        // Expire the lease on the DB clock WITHOUT reaping: status is still
        // 'claimed' and the generation is unchanged, so only the deadline gate
        // can reject the expired holder.
        sqlx::query(
            "UPDATE workflow_agent_deliveries SET lease_until = NOW() - INTERVAL '1 minute' \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(delivery.id.as_uuid())
        .execute(&pool)
        .await
        .expect("force expiry");

        assert_eq!(
            renew_workflow_agent_delivery(&pool, &lease, 30)
                .await
                .expect("renew"),
            WorkflowDeliveryRenewOutcome::LeaseLost,
            "an expired lease must not resurrect itself via renew"
        );
        assert_eq!(
            finish_workflow_agent_delivery(&pool, &lease, WorkflowDeliveryOutcome::Finished)
                .await
                .expect("finish"),
            WorkflowDeliveryFinishOutcome::LeaseLost,
            "an expired lease must not settle out-of-lease work via finish"
        );

        // The row is untouched (still claimed at the same generation) and thus
        // still reclaimable by the reaper — the deadline gate rejected the
        // holder without mutating state.
        let after = get_workflow_agent_delivery(&pool, community, delivery.id)
            .await
            .expect("get")
            .expect("row");
        assert_eq!(after.status, WorkflowDeliveryStatus::Claimed);
        assert_eq!(after.lease_generation, lease.lease_generation);

        // Reap this test's own expired row before returning: it proves the row
        // was genuinely still reclaimable, and it clears the fleet-wide reaper's
        // view so a sibling test's fleet-wide reap count stays deterministic.
        assert_eq!(
            reap_expired_workflow_agent_deliveries(&pool)
                .await
                .expect("reap"),
            1,
            "the expired-but-unmutated row is reclaimed by the reaper"
        );
    }

    /// The lease is a target-scoped capability: renew, finish, and the terminal
    /// reconciliation SELECT must all bind the held `target_pubkey`, or a lease
    /// reconstituted with the wrong target but the right delivery id and
    /// generation could renew, settle, or be credited with a terminal it never
    /// earned. The DB owner boundary makes wrong-target authority unrepresentable.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lease_is_fenced_by_target() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let target = Keys::generate().public_key();
        let (delivery, created_at) = make_delivery_scaffold(&pool, community, &target).await;
        commit_one(&pool, community, &delivery, created_at).await;

        let (lease, _) =
            claim_workflow_agent_delivery(&pool, community, &target, Some(delivery.id), None, 30)
                .await
                .expect("claim")
                .expect("claimable");

        // A lease with the correct id + generation but a foreign target must
        // fail closed on renew and finish — the id/generation alone are not the
        // capability.
        let wrong_target = WorkflowDeliveryLease {
            target_pubkey: Keys::generate().public_key(),
            ..lease
        };
        assert_ne!(wrong_target.target_pubkey, lease.target_pubkey);
        assert_eq!(
            renew_workflow_agent_delivery(&pool, &wrong_target, 30)
                .await
                .expect("renew"),
            WorkflowDeliveryRenewOutcome::LeaseLost,
            "a wrong-target lease must not renew"
        );
        assert_eq!(
            finish_workflow_agent_delivery(&pool, &wrong_target, WorkflowDeliveryOutcome::Finished)
                .await
                .expect("finish"),
            WorkflowDeliveryFinishOutcome::LeaseLost,
            "a wrong-target lease must not settle"
        );

        // The real target still holds full authority.
        assert!(matches!(
            renew_workflow_agent_delivery(&pool, &lease, 30)
                .await
                .expect("renew"),
            WorkflowDeliveryRenewOutcome::Renewed(_)
        ));
        assert_eq!(
            finish_workflow_agent_delivery(&pool, &lease, WorkflowDeliveryOutcome::Finished)
                .await
                .expect("finish"),
            WorkflowDeliveryFinishOutcome::Settled(WorkflowDeliveryOutcome::Finished)
        );

        // Once the true holder has settled, a wrong-target retry must fail
        // closed via the terminal reconciliation SELECT — it must NOT be
        // laundered the real holder's terminal as `AlreadyTerminal`.
        assert_eq!(
            finish_workflow_agent_delivery(&pool, &wrong_target, WorkflowDeliveryOutcome::Finished)
                .await
                .expect("finish retry"),
            WorkflowDeliveryFinishOutcome::LeaseLost,
            "a wrong-target retry must not be credited the real holder's terminal"
        );
        // The true holder's own idempotent retry still converges to its terminal.
        assert_eq!(
            finish_workflow_agent_delivery(&pool, &lease, WorkflowDeliveryOutcome::Failed)
                .await
                .expect("finish retry"),
            WorkflowDeliveryFinishOutcome::AlreadyTerminal(WorkflowDeliveryStatus::Finished)
        );
    }

    /// The claim's expected binding is an authority claim, not a scope override:
    /// its own community and target must equal the request scope, or a foreign
    /// binding could authorize a row selected under a different scope.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_rejects_expected_binding_scope_mismatch() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let other_community = make_community(&pool).await;
        let target = Keys::generate().public_key();
        let other_target = Keys::generate().public_key();
        let (delivery, created_at) = make_delivery_scaffold(&pool, community, &target).await;
        commit_one(&pool, community, &delivery, created_at).await;

        // An expected binding whose community disagrees with the request scope
        // must be rejected before SQL — never authorize the request-scope row.
        let foreign_community = WorkflowDeliveryBinding::new(
            other_community,
            delivery.binding.workflow_id(),
            delivery.binding.run_id(),
            delivery.binding.step_id(),
            target,
            delivery.binding.definition_event_id(),
            delivery.binding.message_event_id(),
            delivery.binding.cause().clone(),
        )
        .expect("binding");
        assert!(
            claim_workflow_agent_delivery(
                &pool,
                community,
                &target,
                Some(delivery.id),
                Some(&foreign_community),
                30,
            )
            .await
            .expect("claim")
            .is_none(),
            "an expected binding for another community must not claim this row"
        );

        // Likewise a binding whose target disagrees with the request scope.
        let foreign_target = WorkflowDeliveryBinding::new(
            community,
            delivery.binding.workflow_id(),
            delivery.binding.run_id(),
            delivery.binding.step_id(),
            other_target,
            delivery.binding.definition_event_id(),
            delivery.binding.message_event_id(),
            delivery.binding.cause().clone(),
        )
        .expect("binding");
        assert!(
            claim_workflow_agent_delivery(
                &pool,
                community,
                &target,
                Some(delivery.id),
                Some(&foreign_target),
                30,
            )
            .await
            .expect("claim")
            .is_none(),
            "an expected binding for another target must not claim this row"
        );

        // The row is untouched and still claimable under a scope-consistent
        // binding.
        assert!(
            claim_workflow_agent_delivery(
                &pool,
                community,
                &target,
                Some(delivery.id),
                Some(&delivery.binding),
                30,
            )
            .await
            .expect("claim")
            .is_some(),
            "a scope-consistent binding still claims"
        );
    }

    /// Deleting the owning run cascades the delivery rows away.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn run_deletion_cascades_deliveries() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let target = Keys::generate().public_key();
        let (delivery, created_at) = make_delivery_scaffold(&pool, community, &target).await;
        commit_one(&pool, community, &delivery, created_at).await;

        sqlx::query("DELETE FROM workflow_runs WHERE community_id = $1 AND id = $2")
            .bind(community.as_uuid())
            .bind(delivery.binding.run_id())
            .execute(&pool)
            .await
            .expect("delete run");

        let gone = get_workflow_agent_delivery(&pool, community, delivery.id)
            .await
            .expect("get");
        assert!(gone.is_none(), "run deletion must cascade the delivery row");
    }

    /// A failure after a partial insert inside the commit transaction rolls back
    /// every delivery row for the step together (all-or-nothing).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn commit_rolls_back_all_targets_on_failure() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let target_a = Keys::generate().public_key();
        let target_b = Keys::generate().public_key();
        let (base, created_at) = make_delivery_scaffold(&pool, community, &target_a).await;

        // Second delivery: same run/step, target B, but a bogus workflow_id so
        // its FK insert fails — the whole transaction must roll back.
        let good = WorkflowAgentDelivery {
            id: WorkflowDeliveryId::from_uuid(Uuid::new_v4()),
            binding: base.binding.clone(),
        };
        let bad_binding = WorkflowDeliveryBinding::new(
            community,
            Uuid::new_v4(), // nonexistent workflow_id -> FK violation
            base.binding.run_id(),
            base.binding.step_id(),
            target_b,
            base.binding.definition_event_id(),
            base.binding.message_event_id(),
            base.binding.cause().clone(),
        )
        .expect("binding");
        let bad = WorkflowAgentDelivery {
            id: WorkflowDeliveryId::from_uuid(Uuid::new_v4()),
            binding: bad_binding,
        };

        let (tx, _existing) = lock_workflow_agent_delivery_identity(
            &pool,
            community,
            base.binding.run_id(),
            base.binding.step_id(),
        )
        .await
        .expect("lock");
        let result =
            commit_workflow_agent_deliveries(tx, community, created_at, &[good.clone(), bad]).await;
        assert!(result.is_err(), "FK violation must fail the commit");

        // The good target's row must NOT be visible — the transaction rolled back.
        let a = get_workflow_agent_delivery(&pool, community, good.id)
            .await
            .expect("get");
        assert!(a.is_none(), "partial insert must not survive rollback");
    }
}
