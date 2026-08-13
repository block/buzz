//! Rebuildable Buzz Tasks projection.
//!
//! A task event and its projection transition commit in one transaction. The
//! signed event stream remains the source of truth, while this table provides a
//! small owner-scoped read model for list and detail APIs.

use buzz_core::task::{TaskEventPayloadV1, TaskEventV1, TaskPriority, TaskResolution, TaskType};
use buzz_core::{CommunityId, StoredEvent};
use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{PgPool, Postgres, QueryBuilder, Row, Transaction};
use uuid::Uuid;

use crate::error::{DbError, Result};

/// Materialized task status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// Waiting for the owner to act in Buzz.
    Open,
    /// The source action completed in Buzz.
    Resolved,
    /// The source agent withdrew the request.
    Withdrawn,
}

impl TaskStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::Withdrawn => "withdrawn",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "resolved" => Ok(Self::Resolved),
            "withdrawn" => Ok(Self::Withdrawn),
            other => Err(DbError::InvalidData(format!(
                "unknown buzz_tasks status {other}"
            ))),
        }
    }
}

/// Due-date slice applied by the task list query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskDueBucket {
    /// No due-date filter.
    All,
    /// Overdue tasks and tasks due before the supplied local-day end.
    Today {
        /// Exclusive UTC boundary for the caller's next local midnight.
        end: DateTime<Utc>,
    },
    /// Tasks due on/after the supplied local-day end, plus tasks without a due date.
    Later {
        /// Inclusive UTC boundary for the caller's next local midnight.
        start: DateTime<Utc>,
    },
}

/// Stable inputs for an owner-scoped task page.
#[derive(Debug, Clone, Copy)]
pub struct TaskListQuery {
    /// Optional status filter. The public API defaults to `open`.
    pub status: Option<TaskStatus>,
    /// Due-date bucket.
    pub bucket: TaskDueBucket,
    /// Maximum rows, clamped to 101 by the database boundary so an API can
    /// request one look-ahead row for a 100-item page.
    pub limit: i64,
    /// Opaque-cursor offset.
    pub offset: i64,
    /// Snapshot time used by overdue sorting across every page in one cursor chain.
    pub as_of: DateTime<Utc>,
}

impl TaskListQuery {
    /// Construct the default open/all query.
    pub fn open(limit: i64, offset: i64, as_of: DateTime<Utc>) -> Self {
        Self {
            status: Some(TaskStatus::Open),
            bucket: TaskDueBucket::All,
            limit,
            offset,
            as_of,
        }
    }
}

/// One row from the owner-private task projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    /// Server-resolved community identity.
    pub community_id: CommunityId,
    /// Stable task UUID.
    pub id: Uuid,
    /// Owner pubkey that must act.
    pub assignee_pubkey: Vec<u8>,
    /// Source channel UUID.
    pub channel_id: Uuid,
    /// Exact source Nostr event ID bytes.
    pub source_event_id: Vec<u8>,
    /// Agent pubkey that authored the task.
    pub agent_pubkey: Vec<u8>,
    /// Display name snapshot.
    pub agent_name: String,
    /// Owner action type.
    pub task_type: String,
    /// Short title.
    pub title: String,
    /// Optional short context.
    pub context: Option<String>,
    /// Priority string.
    pub priority: String,
    /// Optional due time.
    pub due_at: Option<DateTime<Utc>>,
    /// Current task status.
    pub status: TaskStatus,
    /// Source message creation time.
    pub source_created_at: DateTime<Utc>,
    /// Monotonic source version.
    pub source_version: i64,
    /// Source-side update time.
    pub source_updated_at: DateTime<Utc>,
    /// Terminal time for resolved/withdrawn tasks.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Result of an atomic task-event/projection write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskProjectionOutcome {
    /// Requested event inserted a new task.
    Inserted,
    /// Updated event changed an open task.
    Updated,
    /// Resolved event made the task terminal.
    Resolved,
    /// A lower or equal source version was stored but did not change the projection.
    Stale,
    /// The exact signed event already existed; neither event nor projection changed.
    DuplicateEvent,
}

/// Atomically store a signed task event and apply its projection transition.
///
/// The source message must already exist in the same tenant and channel and be
/// signed by the task agent. Invalid identity or status transitions roll back
/// the event insert as well as the projection mutation.
pub async fn insert_task_event_with_projection(
    pool: &PgPool,
    community_id: CommunityId,
    event: &Event,
    task: &TaskEventV1,
) -> Result<TaskProjectionOutcome> {
    let reparsed = TaskEventV1::parse(event)
        .map_err(|error| DbError::InvalidData(format!("task event contract: {error}")))?;
    if &reparsed != task {
        return Err(DbError::InvalidData(
            "parsed task does not match signed event".into(),
        ));
    }

    let mut tx = pool.begin().await?;
    let registered_owner: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT agent_owner_pubkey FROM users \
         WHERE community_id=$1 AND pubkey=$2 AND deactivated_at IS NULL FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(task.agent_pubkey.as_bytes().as_slice())
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    if registered_owner.as_deref() != Some(task.owner_pubkey.as_bytes().as_slice()) {
        return Err(DbError::AccessDenied(
            "task p tag is not the registered owner of the task agent".into(),
        ));
    }

    let channel: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM channels \
         WHERE community_id=$1 AND id=$2 AND deleted_at IS NULL FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(task.channel_id)
    .fetch_optional(&mut *tx)
    .await?;
    if channel.is_none() {
        return Err(DbError::InvalidData(
            "task channel does not exist in the resolved community".into(),
        ));
    }

    for (role, pubkey) in [
        ("agent", task.agent_pubkey.as_bytes().as_slice()),
        ("owner", task.owner_pubkey.as_bytes().as_slice()),
    ] {
        let member: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT pubkey FROM channel_members \
             WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3 \
               AND removed_at IS NULL FOR SHARE",
        )
        .bind(community_id.as_uuid())
        .bind(task.channel_id)
        .bind(pubkey)
        .fetch_optional(&mut *tx)
        .await?;
        if member.is_none() {
            return Err(DbError::AccessDenied(format!(
                "task {role} must be an active channel member"
            )));
        }
    }

    let source = sqlx::query(
        "SELECT created_at, pubkey, kind FROM events \
         WHERE community_id=$1 AND id=$2 AND channel_id=$3 AND deleted_at IS NULL \
         ORDER BY created_at DESC LIMIT 1 FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(task.source_event_id.as_bytes().as_slice())
    .bind(task.channel_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(source) = source else {
        return Err(DbError::InvalidData(
            "task source event not found in tenant channel".into(),
        ));
    };
    let source_pubkey: Vec<u8> = source.try_get("pubkey")?;
    if source_pubkey != task.agent_pubkey.to_bytes() {
        return Err(DbError::AccessDenied(
            "task source event must be signed by the task agent".into(),
        ));
    }
    let source_created_at: DateTime<Utc> = source.try_get("created_at")?;
    let source_kind: i32 = source.try_get("kind")?;
    if ![
        buzz_core::kind::KIND_STREAM_MESSAGE,
        buzz_core::kind::KIND_STREAM_MESSAGE_V2,
    ]
    .contains(&(source_kind as u32))
    {
        return Err(DbError::InvalidData(
            "task source must be a Buzz stream message".into(),
        ));
    }
    if task.payload.source_updated_at() < source_created_at {
        return Err(DbError::InvalidData(
            "task sourceUpdatedAt predates the source message".into(),
        ));
    }
    if event_created_at(event)? < source_created_at {
        return Err(DbError::InvalidData(
            "task event predates the source message".into(),
        ));
    }

    let (stored_event, was_inserted) = crate::event::insert_event_with_thread_metadata_tx(
        &mut tx,
        community_id,
        event,
        Some(task.channel_id),
        None,
    )
    .await?;
    if !was_inserted {
        tx.rollback().await?;
        return Ok(TaskProjectionOutcome::DuplicateEvent);
    }

    let outcome = apply_projection(
        &mut tx,
        community_id,
        task,
        &stored_event,
        source_created_at,
    )
    .await?;
    tx.commit().await?;
    Ok(outcome)
}

async fn apply_projection(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    task: &TaskEventV1,
    stored_event: &StoredEvent,
    source_created_at: DateTime<Utc>,
) -> Result<TaskProjectionOutcome> {
    match &task.payload {
        TaskEventPayloadV1::Requested(payload) => {
            let result = sqlx::query(
                "INSERT INTO buzz_tasks (\
                    community_id, id, assignee_pubkey, channel_id, source_event_id, \
                    agent_pubkey, agent_name, task_type, title, context, priority, due_at, \
                    status, source_created_at, source_version, source_updated_at, \
                    task_event_id, task_event_created_at\
                 ) VALUES (\
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,\
                    'open',$13,$14,$15,$16,$17\
                 ) ON CONFLICT DO NOTHING",
            )
            .bind(community_id.as_uuid())
            .bind(task.task_id)
            .bind(task.owner_pubkey.as_bytes().as_slice())
            .bind(task.channel_id)
            .bind(task.source_event_id.as_bytes().as_slice())
            .bind(task.agent_pubkey.as_bytes().as_slice())
            .bind(&payload.agent_name)
            .bind(task_type_str(payload.task_type))
            .bind(&payload.title)
            .bind(&payload.context)
            .bind(priority_str(payload.priority))
            .bind(payload.due_at)
            .bind(source_created_at)
            .bind(payload.source_version)
            .bind(payload.source_updated_at)
            .bind(stored_event.event.id.as_bytes().as_slice())
            .bind(event_created_at(&stored_event.event)?)
            .execute(&mut **tx)
            .await?;
            if result.rows_affected() == 1 {
                return Ok(TaskProjectionOutcome::Inserted);
            }

            let existing = select_projection_identity(tx, community_id, task.task_id).await?;
            if let Some(existing) = existing {
                validate_identity(&existing, task)?;
                if existing.source_version >= payload.source_version {
                    return Ok(TaskProjectionOutcome::Stale);
                }
            }
            Err(DbError::InvalidData(
                "task request conflicts with an existing task source identity".into(),
            ))
        }
        TaskEventPayloadV1::Updated(payload) => {
            let existing = require_open_newer(
                tx,
                community_id,
                task,
                payload.source_version,
                payload.source_updated_at,
            )
            .await?;
            if existing.is_none() {
                return Ok(TaskProjectionOutcome::Stale);
            }
            sqlx::query(
                "UPDATE buzz_tasks SET \
                    agent_name=$3, task_type=$4, title=$5, context=$6, priority=$7, due_at=$8, \
                    source_version=$9, source_updated_at=$10, task_event_id=$11, \
                    task_event_created_at=$12, updated_at=now() \
                 WHERE community_id=$1 AND id=$2",
            )
            .bind(community_id.as_uuid())
            .bind(task.task_id)
            .bind(&payload.agent_name)
            .bind(task_type_str(payload.task_type))
            .bind(&payload.title)
            .bind(&payload.context)
            .bind(priority_str(payload.priority))
            .bind(payload.due_at)
            .bind(payload.source_version)
            .bind(payload.source_updated_at)
            .bind(stored_event.event.id.as_bytes().as_slice())
            .bind(event_created_at(&stored_event.event)?)
            .execute(&mut **tx)
            .await?;
            Ok(TaskProjectionOutcome::Updated)
        }
        TaskEventPayloadV1::Resolved(payload) => {
            let existing = require_open_newer(
                tx,
                community_id,
                task,
                payload.source_version,
                payload.source_updated_at,
            )
            .await?;
            if existing.is_none() {
                return Ok(TaskProjectionOutcome::Stale);
            }
            let status = match payload.resolution {
                TaskResolution::Resolved => TaskStatus::Resolved,
                TaskResolution::Withdrawn => TaskStatus::Withdrawn,
            };
            sqlx::query(
                "UPDATE buzz_tasks SET status=$3, source_version=$4, source_updated_at=$5, \
                    resolved_at=$5, task_event_id=$6, task_event_created_at=$7, updated_at=now() \
                 WHERE community_id=$1 AND id=$2",
            )
            .bind(community_id.as_uuid())
            .bind(task.task_id)
            .bind(status.as_str())
            .bind(payload.source_version)
            .bind(payload.source_updated_at)
            .bind(stored_event.event.id.as_bytes().as_slice())
            .bind(event_created_at(&stored_event.event)?)
            .execute(&mut **tx)
            .await?;
            Ok(TaskProjectionOutcome::Resolved)
        }
    }
}

struct ProjectionIdentity {
    owner: Vec<u8>,
    channel_id: Uuid,
    source_event_id: Vec<u8>,
    agent: Vec<u8>,
    status: TaskStatus,
    source_version: i64,
    source_updated_at: DateTime<Utc>,
}

async fn select_projection_identity(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    task_id: Uuid,
) -> Result<Option<ProjectionIdentity>> {
    let row = sqlx::query(
        "SELECT assignee_pubkey, channel_id, source_event_id, agent_pubkey, status, \
                source_version, source_updated_at \
         FROM buzz_tasks WHERE community_id=$1 AND id=$2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(task_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        Ok(ProjectionIdentity {
            owner: row.try_get("assignee_pubkey")?,
            channel_id: row.try_get("channel_id")?,
            source_event_id: row.try_get("source_event_id")?,
            agent: row.try_get("agent_pubkey")?,
            status: TaskStatus::parse(row.try_get("status")?)?,
            source_version: row.try_get("source_version")?,
            source_updated_at: row.try_get("source_updated_at")?,
        })
    })
    .transpose()
}

fn validate_identity(existing: &ProjectionIdentity, task: &TaskEventV1) -> Result<()> {
    if existing.owner != task.owner_pubkey.to_bytes()
        || existing.channel_id != task.channel_id
        || existing.source_event_id != task.source_event_id.as_bytes()
        || existing.agent != task.agent_pubkey.to_bytes()
    {
        return Err(DbError::AccessDenied(
            "task transition changed signed source identity".into(),
        ));
    }
    Ok(())
}

async fn require_open_newer(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    task: &TaskEventV1,
    source_version: i64,
    source_updated_at: DateTime<Utc>,
) -> Result<Option<ProjectionIdentity>> {
    let existing = select_projection_identity(tx, community_id, task.task_id)
        .await?
        .ok_or_else(|| DbError::NotFound("Buzz Task transition target".into()))?;
    validate_identity(&existing, task)?;
    if source_version <= existing.source_version {
        return Ok(None);
    }
    if existing.status != TaskStatus::Open {
        return Err(DbError::InvalidData(
            "terminal Buzz Task cannot transition again".into(),
        ));
    }
    if source_updated_at < existing.source_updated_at {
        return Err(DbError::InvalidData(
            "newer Buzz Task version has an older sourceUpdatedAt".into(),
        ));
    }
    Ok(Some(existing))
}

fn task_type_str(value: TaskType) -> &'static str {
    match value {
        TaskType::Reply => "reply",
        TaskType::Approval => "approval",
        TaskType::Choice => "choice",
        TaskType::Review => "review",
    }
}

fn priority_str(value: TaskPriority) -> &'static str {
    match value {
        TaskPriority::Low => "low",
        TaskPriority::Medium => "medium",
        TaskPriority::High => "high",
    }
}

fn event_created_at(event: &Event) -> Result<DateTime<Utc>> {
    let seconds = event.created_at.as_secs() as i64;
    DateTime::from_timestamp(seconds, 0).ok_or(DbError::InvalidTimestamp(seconds))
}

/// Fetch one task for its owner inside one server-resolved community.
///
/// The HTTP boundary must additionally re-check current access to the returned
/// channel before serializing this row or its navigation URL.
pub async fn get_task_for_owner(
    pool: &PgPool,
    community_id: CommunityId,
    owner_pubkey: &[u8],
    task_id: Uuid,
) -> Result<Option<TaskRecord>> {
    let row = sqlx::query(
        "SELECT t.community_id, t.id, t.assignee_pubkey, t.channel_id, t.source_event_id, \
                t.agent_pubkey, t.agent_name, t.task_type, t.title, t.context, t.priority, \
                t.due_at, t.status, t.source_created_at, t.source_version, \
                t.source_updated_at, t.resolved_at \
         FROM buzz_tasks t WHERE t.community_id=$1 AND t.assignee_pubkey=$2 AND t.id=$3 \
           AND EXISTS (SELECT 1 FROM channel_members cm \
                       WHERE cm.community_id=t.community_id AND cm.channel_id=t.channel_id \
                         AND cm.pubkey=t.assignee_pubkey AND cm.removed_at IS NULL) \
           AND EXISTS (SELECT 1 FROM events e \
                       WHERE e.community_id=t.community_id AND e.id=t.source_event_id \
                         AND e.channel_id=t.channel_id AND e.pubkey=t.agent_pubkey \
                         AND e.deleted_at IS NULL)",
    )
    .bind(community_id.as_uuid())
    .bind(owner_pubkey)
    .bind(task_id)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_task).transpose()
}

/// List tasks visible to one owner in their currently accessible channels.
pub async fn list_tasks_for_owner(
    pool: &PgPool,
    community_id: CommunityId,
    owner_pubkey: &[u8],
    accessible_channel_ids: &[Uuid],
    query: &TaskListQuery,
) -> Result<Vec<TaskRecord>> {
    if accessible_channel_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<Postgres>::new(
        "SELECT t.community_id, t.id, t.assignee_pubkey, t.channel_id, t.source_event_id, \
                t.agent_pubkey, t.agent_name, t.task_type, t.title, t.context, t.priority, \
                t.due_at, t.status, t.source_created_at, t.source_version, \
                t.source_updated_at, t.resolved_at \
         FROM buzz_tasks t WHERE t.community_id=",
    );
    builder
        .push_bind(community_id.as_uuid())
        .push(" AND t.assignee_pubkey=")
        .push_bind(owner_pubkey)
        .push(" AND t.channel_id = ANY(")
        .push_bind(accessible_channel_ids)
        .push(
            ") AND EXISTS (SELECT 1 FROM channel_members cm \
               WHERE cm.community_id=t.community_id AND cm.channel_id=t.channel_id \
                 AND cm.pubkey=t.assignee_pubkey AND cm.removed_at IS NULL) \
               AND EXISTS (SELECT 1 FROM events e \
               WHERE e.community_id=t.community_id AND e.id=t.source_event_id \
                 AND e.channel_id=t.channel_id AND e.pubkey=t.agent_pubkey \
                 AND e.deleted_at IS NULL)",
        );
    if let Some(status) = query.status {
        builder.push(" AND t.status=").push_bind(status.as_str());
    }
    match query.bucket {
        TaskDueBucket::All => {}
        TaskDueBucket::Today { end } => {
            builder.push(" AND t.due_at < ").push_bind(end);
        }
        TaskDueBucket::Later { start } => {
            builder
                .push(" AND (t.due_at IS NULL OR t.due_at >= ")
                .push_bind(start)
                .push(")");
        }
    }
    builder
        .push(" ORDER BY (t.due_at IS NOT NULL AND t.due_at < ")
        .push_bind(query.as_of)
        .push(
            ") DESC, CASE priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END ASC, \
               t.due_at ASC NULLS LAST, t.source_created_at DESC, t.id ASC LIMIT ",
        )
        .push_bind(query.limit.clamp(1, 101))
        .push(" OFFSET ")
        .push_bind(query.offset.max(0));

    builder
        .build()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(row_to_task)
        .collect()
}

fn row_to_task(row: sqlx::postgres::PgRow) -> Result<TaskRecord> {
    let community_id: Uuid = row.try_get("community_id")?;
    let status: &str = row.try_get("status")?;
    Ok(TaskRecord {
        community_id: CommunityId::from_uuid(community_id),
        id: row.try_get("id")?,
        assignee_pubkey: row.try_get("assignee_pubkey")?,
        channel_id: row.try_get("channel_id")?,
        source_event_id: row.try_get("source_event_id")?,
        agent_pubkey: row.try_get("agent_pubkey")?,
        agent_name: row.try_get("agent_name")?,
        task_type: row.try_get("task_type")?,
        title: row.try_get("title")?,
        context: row.try_get("context")?,
        priority: row.try_get("priority")?,
        due_at: row.try_get("due_at")?,
        status: TaskStatus::parse(status)?,
        source_created_at: row.try_get("source_created_at")?,
        source_version: row.try_get("source_version")?,
        source_updated_at: row.try_get("source_updated_at")?,
        resolved_at: row.try_get("resolved_at")?,
    })
}
