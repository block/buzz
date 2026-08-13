//! Rebuildable Buzz Tasks projection.
//!
//! A task event and its projection transition commit in one transaction. The
//! signed event stream remains the source of truth, while this table provides a
//! small internal read model for validation and efficient replay. Public reads
//! stay on the relay's existing Nostr/bridge query surface.

use buzz_core::task::{TaskEventPayloadV1, TaskEventV1, TaskPriority, TaskResolution, TaskType};
use buzz_core::{CommunityId, StoredEvent};
use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{PgPool, Postgres, Row, Transaction};
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

    // Task reads use the relay's ordinary `#p` filter, whose SQL path joins
    // `event_mentions`. Keep that index row in the same transaction as the
    // signed event and projection so an accepted task is immediately readable
    // through both WS delivery and the authenticated bridge.
    sqlx::query(
        "INSERT INTO event_mentions \
         (community_id, pubkey_hex, event_id, event_created_at, channel_id, event_kind) \
         VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(task.owner_pubkey.to_hex())
    .bind(event.id.as_bytes().as_slice())
    .bind(event_created_at(event)?)
    .bind(task.channel_id)
    .bind(event.kind.as_u16() as i32)
    .execute(&mut *tx)
    .await?;

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
    source_created_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

async fn select_projection_identity(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    task_id: Uuid,
) -> Result<Option<ProjectionIdentity>> {
    let row = sqlx::query(
        "SELECT assignee_pubkey, channel_id, source_event_id, agent_pubkey, status, \
                source_version, source_updated_at, source_created_at, created_at \
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
            source_created_at: row.try_get("source_created_at")?,
            created_at: row.try_get("created_at")?,
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

#[derive(Debug)]
struct FoldedTaskProjection {
    agent_name: String,
    task_type: TaskType,
    title: String,
    context: Option<String>,
    priority: TaskPriority,
    due_at: Option<DateTime<Utc>>,
    status: TaskStatus,
    source_version: i64,
    source_updated_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    task_event_id: Vec<u8>,
    task_event_created_at: DateTime<Utc>,
}

/// Soft-delete one signed task event and rebuild its projection from the
/// remaining live event stream in the same transaction.
///
/// The projection is derived state: deleting the current update reverts to the
/// preceding live version, deleting a terminal event reopens the last live
/// open version, and deleting the only request removes the projection. This
/// function holds the task projection row lock across both the event tombstone
/// and rebuild, so concurrent task transitions cannot observe an intermediate
/// state.
pub async fn soft_delete_task_event_and_rebuild_projection(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
         FROM events WHERE community_id=$1 AND id=$2 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(event_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(false);
    };
    let stored = crate::event::row_to_stored_event(row)?.ok_or_else(|| {
        DbError::InvalidData("could not reconstruct stored Buzz Task event".into())
    })?;
    let deleted_task = TaskEventV1::parse(&stored.event)
        .map_err(|error| DbError::InvalidData(format!("stored task event contract: {error}")))?;

    let existing = select_projection_identity(&mut tx, community_id, deleted_task.task_id).await?;
    let result = sqlx::query(
        "UPDATE events SET deleted_at=now() \
         WHERE community_id=$1 AND id=$2 AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(event_id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    if let Some(existing) = existing {
        let rows = sqlx::query(
            "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
             FROM events \
             WHERE community_id=$1 AND tags @> $2 \
               AND kind IN ($3,$4,$5) AND deleted_at IS NULL \
             ORDER BY received_at ASC, id ASC",
        )
        .bind(community_id.as_uuid())
        .bind(serde_json::json!([["d", deleted_task.task_id.to_string()]]))
        .bind(buzz_core::kind::KIND_TASK_REQUESTED as i32)
        .bind(buzz_core::kind::KIND_TASK_UPDATED as i32)
        .bind(buzz_core::kind::KIND_TASK_RESOLVED as i32)
        .fetch_all(&mut *tx)
        .await?;

        let mut live = Vec::with_capacity(rows.len());
        for row in rows {
            let stored = crate::event::row_to_stored_event(row)?.ok_or_else(|| {
                DbError::InvalidData("could not reconstruct live Buzz Task event".into())
            })?;
            let parsed = TaskEventV1::parse(&stored.event).map_err(|error| {
                DbError::InvalidData(format!("live task event contract: {error}"))
            })?;
            validate_identity(&existing, &parsed)?;
            live.push((parsed, stored));
        }
        live.sort_by(|(left_task, left_event), (right_task, right_event)| {
            left_task
                .payload
                .source_version()
                .cmp(&right_task.payload.source_version())
                .then_with(|| left_event.received_at.cmp(&right_event.received_at))
                .then_with(|| left_event.event.id.cmp(&right_event.event.id))
        });

        let folded = fold_live_projection(live)?;
        sqlx::query("DELETE FROM buzz_tasks WHERE community_id=$1 AND id=$2")
            .bind(community_id.as_uuid())
            .bind(deleted_task.task_id)
            .execute(&mut *tx)
            .await?;
        if let Some(folded) = folded {
            sqlx::query(
                "INSERT INTO buzz_tasks (\
                    community_id, id, assignee_pubkey, channel_id, source_event_id, \
                    agent_pubkey, agent_name, task_type, title, context, priority, due_at, \
                    status, source_created_at, source_version, source_updated_at, resolved_at, \
                    task_event_id, task_event_created_at, created_at, updated_at\
                 ) VALUES (\
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,now()\
                 )",
            )
            .bind(community_id.as_uuid())
            .bind(deleted_task.task_id)
            .bind(&existing.owner)
            .bind(existing.channel_id)
            .bind(&existing.source_event_id)
            .bind(&existing.agent)
            .bind(&folded.agent_name)
            .bind(task_type_str(folded.task_type))
            .bind(&folded.title)
            .bind(&folded.context)
            .bind(priority_str(folded.priority))
            .bind(folded.due_at)
            .bind(folded.status.as_str())
            .bind(existing.source_created_at)
            .bind(folded.source_version)
            .bind(folded.source_updated_at)
            .bind(folded.resolved_at)
            .bind(&folded.task_event_id)
            .bind(folded.task_event_created_at)
            .bind(existing.created_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;
    Ok(true)
}

fn fold_live_projection(
    live: Vec<(TaskEventV1, StoredEvent)>,
) -> Result<Option<FoldedTaskProjection>> {
    let mut folded: Option<FoldedTaskProjection> = None;
    for (task, stored) in live {
        let task_event_id = stored.event.id.as_bytes().to_vec();
        let task_event_created_at = event_created_at(&stored.event)?;
        match task.payload {
            TaskEventPayloadV1::Requested(payload) if folded.is_none() => {
                folded = Some(FoldedTaskProjection {
                    agent_name: payload.agent_name,
                    task_type: payload.task_type,
                    title: payload.title,
                    context: payload.context,
                    priority: payload.priority,
                    due_at: payload.due_at,
                    status: TaskStatus::Open,
                    source_version: payload.source_version,
                    source_updated_at: payload.source_updated_at,
                    resolved_at: None,
                    task_event_id,
                    task_event_created_at,
                });
            }
            TaskEventPayloadV1::Requested(_) => {}
            TaskEventPayloadV1::Updated(payload) => {
                let Some(current) = folded.as_mut() else {
                    continue;
                };
                if payload.source_version <= current.source_version {
                    continue;
                }
                if current.status != TaskStatus::Open {
                    return Err(DbError::InvalidData(
                        "live Buzz Task stream transitions after terminal state".into(),
                    ));
                }
                if payload.source_updated_at < current.source_updated_at {
                    return Err(DbError::InvalidData(
                        "live Buzz Task stream moves sourceUpdatedAt backward".into(),
                    ));
                }
                current.agent_name = payload.agent_name;
                current.task_type = payload.task_type;
                current.title = payload.title;
                current.context = payload.context;
                current.priority = payload.priority;
                current.due_at = payload.due_at;
                current.source_version = payload.source_version;
                current.source_updated_at = payload.source_updated_at;
                current.task_event_id = task_event_id;
                current.task_event_created_at = task_event_created_at;
            }
            TaskEventPayloadV1::Resolved(payload) => {
                let Some(current) = folded.as_mut() else {
                    continue;
                };
                if payload.source_version <= current.source_version {
                    continue;
                }
                if current.status != TaskStatus::Open {
                    return Err(DbError::InvalidData(
                        "live Buzz Task stream transitions after terminal state".into(),
                    ));
                }
                if payload.source_updated_at < current.source_updated_at {
                    return Err(DbError::InvalidData(
                        "live Buzz Task stream moves sourceUpdatedAt backward".into(),
                    ));
                }
                current.status = match payload.resolution {
                    TaskResolution::Resolved => TaskStatus::Resolved,
                    TaskResolution::Withdrawn => TaskStatus::Withdrawn,
                };
                current.source_version = payload.source_version;
                current.source_updated_at = payload.source_updated_at;
                current.resolved_at = Some(payload.source_updated_at);
                current.task_event_id = task_event_id;
                current.task_event_created_at = task_event_created_at;
            }
        }
    }
    Ok(folded)
}
