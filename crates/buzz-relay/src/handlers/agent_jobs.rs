//! Atomic admission and canonical projection for public agent-job events.
//!
//! This module deliberately validates lifecycle state inside the same writer
//! transaction that stores the signed event. Event-history queries are not an
//! admission primitive: they race under concurrent publishers.

use chrono::{DateTime, Utc};
use nostr::Event;
use serde::Serialize;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use buzz_core::agent_job::{
    parse_agent_job_event, AgentJobErrorState, AgentJobPayload, AgentJobProgressState,
    ParsedAgentJobEvent,
};
use buzz_core::kind::{KIND_JOB_CANCEL, KIND_JOB_REQUEST};
use buzz_core::{CommunityId, StoredEvent};

/// Result of atomically admitting one public job event.
pub(crate) enum AgentJobPersistOutcome {
    /// The signed event and its projection transition committed.
    Inserted(StoredEvent),
    /// This exact signed event ID was already committed.
    Replay,
}

/// Relay admission failure for a public job event.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentJobAdmissionError {
    /// Malformed envelope/content or an invalid lifecycle transition.
    #[error("{0}")]
    Rejected(String),
    /// Persistence failed before commit.
    #[error("{0}")]
    Internal(String),
}

/// Canonical relay projection returned by indexed lookup/list operations.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AgentJobProjection {
    pub job_id: Uuid,
    pub request_event_id: String,
    pub channel_id: Uuid,
    pub requester_pubkey: String,
    pub target_pubkey: String,
    pub state: String,
    pub attempt: u32,
    pub progress_seq: Option<u64>,
    pub summary: String,
    pub cancel_requested: bool,
    pub terminal_event_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// One admitted event in the canonical chain.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AgentJobChainEntry {
    pub event_id: String,
    pub kind: u32,
    pub author_pubkey: String,
    pub attempt: Option<u32>,
    pub progress_seq: Option<u64>,
    pub created_at: DateTime<Utc>,
    /// Original signed Nostr event admitted for this transition.
    pub event: Event,
}

/// Canonical status and ordered signed-event chain for one job.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AgentJobLookup {
    pub status: AgentJobProjection,
    pub chain: Vec<AgentJobChainEntry>,
}

#[derive(Debug)]
struct LockedJob {
    request_event_id: Vec<u8>,
    channel_id: Uuid,
    requester_pubkey: Vec<u8>,
    target_pubkey: Vec<u8>,
    state: String,
    attempt: i64,
    progress_seq: Option<u64>,
    cancel_requested: bool,
}

fn reject(message: impl Into<String>) -> AgentJobAdmissionError {
    AgentJobAdmissionError::Rejected(message.into())
}

fn internal(error: impl std::fmt::Display) -> AgentJobAdmissionError {
    AgentJobAdmissionError::Internal(format!("agent job persistence failed: {error}"))
}

fn is_terminal_state(state: &str) -> bool {
    matches!(state, "succeeded" | "failed" | "cancelled" | "lost")
}

fn event_time(event: &Event) -> Result<DateTime<Utc>, AgentJobAdmissionError> {
    DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
        .ok_or_else(|| reject("invalid agent job event timestamp"))
}

async fn event_replayed(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &Event,
) -> Result<bool, AgentJobAdmissionError> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM agent_job_events WHERE community_id = $1 AND event_id = $2)",
    )
    .bind(community_id.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .fetch_one(&mut **tx)
    .await
    .map_err(internal)
}

async fn insert_signed_event(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &Event,
    channel_id: Uuid,
    received_at: DateTime<Utc>,
) -> Result<(), AgentJobAdmissionError> {
    let created_at = event_time(event)?;
    let tags = serde_json::to_value(&event.tags).map_err(internal)?;
    let pubkey = event.pubkey.to_bytes();
    let signature = event.sig.serialize();
    let inserted = sqlx::query(
        r#"
        INSERT INTO events
            (community_id, id, pubkey, created_at, kind, tags, content, sig,
             received_at, channel_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .bind(pubkey.as_slice())
    .bind(created_at)
    .bind(event.kind.as_u16() as i32)
    .bind(tags)
    .bind(&event.content)
    .bind(signature.as_slice())
    .bind(received_at)
    .bind(channel_id)
    .execute(&mut **tx)
    .await
    .map_err(internal)?;

    if inserted.rows_affected() != 1 {
        return Err(reject("conflicting agent job event id already exists"));
    }
    Ok(())
}

async fn insert_chain_entry(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &Event,
    parsed: &ParsedAgentJobEvent,
) -> Result<(), AgentJobAdmissionError> {
    let author = event.pubkey.to_bytes();
    let attempt = parsed.payload.attempt().map(i64::from);
    let seq = parsed.seq.map(|value| value.to_string());
    sqlx::query(
        r#"
        INSERT INTO agent_job_events
            (community_id, event_id, event_created_at, job_id, chain_seq, kind,
             author_pubkey, attempt, progress_seq)
        SELECT $1, $2, $3, $4,
               COALESCE((
                   SELECT MAX(chain_seq) FROM agent_job_events
                   WHERE community_id = $1 AND job_id = $4
               ), 0) + 1,
               $5, $6, $7, $8::numeric
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .bind(event_time(event)?)
    .bind(parsed.job)
    .bind(parsed.kind as i32)
    .bind(author.as_slice())
    .bind(attempt)
    .bind(seq)
    .execute(&mut **tx)
    .await
    .map_err(internal)?;
    Ok(())
}

async fn authorize_request(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    channel_id: Uuid,
    requester: &[u8],
    target: &[u8],
) -> Result<(), AgentJobAdmissionError> {
    let registered_owner: Option<Vec<u8>> = sqlx::query_scalar(
        r#"
        SELECT agent_owner_pubkey
        FROM users
        WHERE community_id = $1 AND pubkey = $2 AND agent_owner_pubkey IS NOT NULL
        FOR SHARE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(target)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?;
    if registered_owner.is_none() {
        return Err(reject("target is not a registered managed agent runtime"));
    }

    let target_is_member = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT 1
        FROM channel_members
        WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3
          AND removed_at IS NULL
        FOR SHARE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(target)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?
    .is_some();
    if !target_is_member {
        return Err(reject(
            "target managed agent is not an active channel member",
        ));
    }

    let requester_is_member = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT 1
        FROM channel_members
        WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3
          AND removed_at IS NULL
        FOR SHARE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(requester)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?
    .is_some();
    if !requester_is_member {
        return Err(reject(
            "requester is not authorized for the target agent channel",
        ));
    }
    Ok(())
}

async fn lock_job(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    job_id: Uuid,
) -> Result<LockedJob, AgentJobAdmissionError> {
    let row = sqlx::query(
        r#"
        SELECT request_event_id, channel_id, requester_pubkey, target_pubkey,
               state, attempt, progress_seq::text AS progress_seq, cancel_requested
        FROM agent_jobs
        WHERE community_id = $1 AND job_id = $2
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(job_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| reject("agent job request does not exist"))?;

    let progress_text: Option<String> = row.try_get("progress_seq").map_err(internal)?;
    let progress_seq = progress_text
        .map(|value| value.parse::<u64>().map_err(internal))
        .transpose()?;
    Ok(LockedJob {
        request_event_id: row.try_get("request_event_id").map_err(internal)?,
        channel_id: row.try_get("channel_id").map_err(internal)?,
        requester_pubkey: row.try_get("requester_pubkey").map_err(internal)?,
        target_pubkey: row.try_get("target_pubkey").map_err(internal)?,
        state: row.try_get("state").map_err(internal)?,
        attempt: row.try_get("attempt").map_err(internal)?,
        progress_seq,
        cancel_requested: row.try_get("cancel_requested").map_err(internal)?,
    })
}

async fn validate_lifecycle_link(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &Event,
    parsed: &ParsedAgentJobEvent,
    job: &LockedJob,
) -> Result<(), AgentJobAdmissionError> {
    if parsed.channel_id != job.channel_id {
        return Err(reject("agent job lifecycle channel does not match request"));
    }
    let linked = parsed
        .linked_event_id
        .as_ref()
        .ok_or_else(|| reject("agent job lifecycle event must link its request"))?;
    if linked.as_bytes() != job.request_event_id.as_slice() {
        return Err(reject("agent job lifecycle e tag does not match request"));
    }

    let author = event.pubkey.to_bytes();
    if parsed.kind == KIND_JOB_CANCEL {
        let owner: Option<Vec<u8>> = sqlx::query_scalar(
            r#"
            SELECT agent_owner_pubkey
            FROM users
            WHERE community_id = $1 AND pubkey = $2 AND agent_owner_pubkey IS NOT NULL
            FOR SHARE
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(job.target_pubkey.as_slice())
        .fetch_optional(&mut **tx)
        .await
        .map_err(internal)?;
        if author.as_slice() != job.requester_pubkey.as_slice()
            && author.as_slice() != job.target_pubkey.as_slice()
            && owner.as_deref() != Some(author.as_slice())
        {
            return Err(reject(
                "only the original requester, target agent, or target agent owner may cancel an agent job",
            ));
        }
        if parsed.peer.to_bytes().as_slice() != job.target_pubkey.as_slice() {
            return Err(reject("agent job cancel target does not match request"));
        }
    } else {
        if author.as_slice() != job.target_pubkey.as_slice() {
            return Err(reject(
                "only the target agent may publish job lifecycle events",
            ));
        }
        if parsed.peer.to_bytes().as_slice() != job.requester_pubkey.as_slice() {
            return Err(reject(
                "agent job lifecycle requester does not match request",
            ));
        }
    }
    if is_terminal_state(&job.state) {
        return Err(reject("agent job terminal state is immutable"));
    }
    Ok(())
}

async fn persist_request(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &Event,
    parsed: &ParsedAgentJobEvent,
) -> Result<bool, AgentJobAdmissionError> {
    let request = match &parsed.payload {
        AgentJobPayload::Request(value) => value,
        _ => return Err(reject("agent job request kind/payload mismatch")),
    };
    let requester = event.pubkey.to_bytes();
    let target = parsed.peer.to_bytes();
    authorize_request(
        tx,
        community_id,
        parsed.channel_id,
        requester.as_slice(),
        target.as_slice(),
    )
    .await?;

    let inserted = sqlx::query(
        r#"
        INSERT INTO agent_jobs
            (community_id, job_id, request_event_id, request_created_at,
             channel_id, requester_pubkey, target_pubkey, state, summary)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'requested', $8)
        ON CONFLICT (community_id, job_id) DO NOTHING
        RETURNING job_id
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(parsed.job)
    .bind(event.id.as_bytes().as_slice())
    .bind(event_time(event)?)
    .bind(parsed.channel_id)
    .bind(requester.as_slice())
    .bind(target.as_slice())
    .bind(&request.summary)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?;

    if inserted.is_none() {
        if event_replayed(tx, community_id, event).await? {
            return Ok(false);
        }
        return Err(reject("agent job UUID is already bound to another request"));
    }
    Ok(true)
}

async fn persist_lifecycle(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &Event,
    parsed: &ParsedAgentJobEvent,
) -> Result<bool, AgentJobAdmissionError> {
    let job = lock_job(tx, community_id, parsed.job).await?;
    // The row lock is also the concurrency serialization point. Re-check replay
    // after acquiring it because another transaction may have committed while
    // this transaction waited.
    if event_replayed(tx, community_id, event).await? {
        return Ok(false);
    }
    validate_lifecycle_link(tx, community_id, event, parsed, &job).await?;

    match &parsed.payload {
        AgentJobPayload::Accepted(payload) => {
            if job.state != "requested" || job.attempt != 0 {
                return Err(reject("agent job may be accepted exactly once"));
            }
            if payload.attempt != 1 {
                return Err(reject("first agent job attempt must be 1"));
            }
            let state = if job.cancel_requested {
                "cancelling"
            } else {
                "accepted"
            };
            sqlx::query(
                "UPDATE agent_jobs SET state = $3, attempt = $4, updated_at = NOW() WHERE community_id = $1 AND job_id = $2",
            )
            .bind(community_id.as_uuid())
            .bind(parsed.job)
            .bind(state)
            .bind(i64::from(payload.attempt))
            .execute(&mut **tx)
            .await
            .map_err(internal)?;
        }
        AgentJobPayload::Progress(payload) => {
            if !matches!(job.state.as_str(), "accepted" | "running" | "cancelling") {
                return Err(reject("agent job progress requires a prior acceptance"));
            }
            if i64::from(payload.attempt) != job.attempt {
                return Err(reject(
                    "agent job progress attempt does not match active attempt",
                ));
            }
            if job
                .progress_seq
                .is_some_and(|previous| payload.seq <= previous)
            {
                return Err(reject("agent job progress seq must be strictly monotonic"));
            }
            let state = match payload.state {
                AgentJobProgressState::Running => "running",
                AgentJobProgressState::Cancelling => "cancelling",
            };
            sqlx::query(
                r#"
                UPDATE agent_jobs
                SET state = $3, progress_seq = $4::numeric, summary = $5, updated_at = NOW()
                WHERE community_id = $1 AND job_id = $2
                "#,
            )
            .bind(community_id.as_uuid())
            .bind(parsed.job)
            .bind(state)
            .bind(payload.seq.to_string())
            .bind(&payload.summary)
            .execute(&mut **tx)
            .await
            .map_err(internal)?;
        }
        AgentJobPayload::Result(payload) => {
            if !matches!(job.state.as_str(), "accepted" | "running" | "cancelling") {
                return Err(reject("agent job result requires a prior acceptance"));
            }
            if i64::from(payload.attempt) != job.attempt {
                return Err(reject(
                    "agent job result attempt does not match active attempt",
                ));
            }
            sqlx::query(
                r#"
                UPDATE agent_jobs
                SET state = 'succeeded', summary = $3, terminal_event_id = $4,
                    terminal_created_at = $5, updated_at = NOW()
                WHERE community_id = $1 AND job_id = $2
                "#,
            )
            .bind(community_id.as_uuid())
            .bind(parsed.job)
            .bind(&payload.summary)
            .bind(event.id.as_bytes().as_slice())
            .bind(payload.finished_at)
            .execute(&mut **tx)
            .await
            .map_err(internal)?;
        }
        AgentJobPayload::Error(payload) => {
            if !matches!(job.state.as_str(), "accepted" | "running" | "cancelling") {
                return Err(reject("agent job error requires a prior acceptance"));
            }
            if i64::from(payload.attempt) != job.attempt {
                return Err(reject(
                    "agent job error attempt does not match active attempt",
                ));
            }
            let state = match payload.state {
                AgentJobErrorState::Failed => "failed",
                AgentJobErrorState::Cancelled => "cancelled",
                AgentJobErrorState::Lost => "lost",
            };
            sqlx::query(
                r#"
                UPDATE agent_jobs
                SET state = $3, summary = $4, terminal_event_id = $5,
                    terminal_created_at = $6, updated_at = NOW()
                WHERE community_id = $1 AND job_id = $2
                "#,
            )
            .bind(community_id.as_uuid())
            .bind(parsed.job)
            .bind(state)
            .bind(&payload.summary)
            .bind(event.id.as_bytes().as_slice())
            .bind(payload.finished_at)
            .execute(&mut **tx)
            .await
            .map_err(internal)?;
        }
        AgentJobPayload::Cancel(_) => {
            if job.cancel_requested {
                return Err(reject("agent job cancellation was already requested"));
            }
            let state = if job.state == "requested" {
                "requested"
            } else {
                "cancelling"
            };
            sqlx::query(
                r#"
                UPDATE agent_jobs
                SET state = $3, cancel_requested = TRUE, cancel_event_id = $4,
                    updated_at = NOW()
                WHERE community_id = $1 AND job_id = $2
                "#,
            )
            .bind(community_id.as_uuid())
            .bind(parsed.job)
            .bind(state)
            .bind(event.id.as_bytes().as_slice())
            .execute(&mut **tx)
            .await
            .map_err(internal)?;
        }
        AgentJobPayload::Request(_) => {
            return Err(reject("agent job lifecycle kind/payload mismatch"));
        }
    }
    Ok(true)
}

/// Parse, authorize, project, and store one signed job event atomically.
pub(crate) async fn persist_agent_job_event(
    db: &buzz_db::Db,
    community_id: CommunityId,
    event: &Event,
) -> Result<AgentJobPersistOutcome, AgentJobAdmissionError> {
    let parsed = parse_agent_job_event(event).map_err(|error| reject(error.to_string()))?;
    let received_at = Utc::now();
    let mut tx = db.begin_transaction().await.map_err(internal)?;

    if event_replayed(&mut tx, community_id, event).await? {
        tx.rollback().await.map_err(internal)?;
        return Ok(AgentJobPersistOutcome::Replay);
    }

    let should_insert = if parsed.kind == KIND_JOB_REQUEST {
        persist_request(&mut tx, community_id, event, &parsed).await?
    } else {
        persist_lifecycle(&mut tx, community_id, event, &parsed).await?
    };
    if !should_insert {
        tx.rollback().await.map_err(internal)?;
        return Ok(AgentJobPersistOutcome::Replay);
    }

    insert_signed_event(&mut tx, community_id, event, parsed.channel_id, received_at).await?;
    insert_chain_entry(&mut tx, community_id, event, &parsed).await?;
    tx.commit().await.map_err(internal)?;

    Ok(AgentJobPersistOutcome::Inserted(
        StoredEvent::with_received_at(event.clone(), received_at, Some(parsed.channel_id), true),
    ))
}

fn parse_hex(bytes: Vec<u8>, field: &'static str) -> Result<String, AgentJobAdmissionError> {
    if bytes.len() != 32 {
        return Err(internal(format!("invalid {field} length in projection")));
    }
    Ok(hex::encode(bytes))
}

fn parse_u64_text(value: Option<String>) -> Result<Option<u64>, AgentJobAdmissionError> {
    value
        .map(|text| text.parse::<u64>().map_err(internal))
        .transpose()
}
fn signed_event_from_row(row: &sqlx::postgres::PgRow) -> Result<Event, AgentJobAdmissionError> {
    let created_at: DateTime<Utc> = row.try_get("signed_created_at").map_err(internal)?;
    let kind: i32 = row.try_get("signed_kind").map_err(internal)?;
    let kind = u16::try_from(kind).map_err(internal)?;
    let signature: Vec<u8> = row.try_get("signed_sig").map_err(internal)?;
    let event_json = serde_json::json!({
        "id": parse_hex(row.try_get("signed_id").map_err(internal)?, "signed event id")?,
        "pubkey": parse_hex(
            row.try_get("signed_pubkey").map_err(internal)?,
            "signed event pubkey",
        )?,
        "created_at": created_at.timestamp(),
        "kind": kind,
        "tags": row.try_get::<serde_json::Value, _>("signed_tags").map_err(internal)?,
        "content": row.try_get::<String, _>("signed_content").map_err(internal)?,
        "sig": hex::encode(signature),
    });
    serde_json::from_value(event_json).map_err(internal)
}

fn projection_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<AgentJobProjection, AgentJobAdmissionError> {
    let attempt: i64 = row.try_get("attempt").map_err(internal)?;
    let attempt = u32::try_from(attempt).map_err(internal)?;
    Ok(AgentJobProjection {
        job_id: row.try_get("job_id").map_err(internal)?,
        request_event_id: parse_hex(
            row.try_get("request_event_id").map_err(internal)?,
            "request event id",
        )?,
        channel_id: row.try_get("channel_id").map_err(internal)?,
        requester_pubkey: parse_hex(
            row.try_get("requester_pubkey").map_err(internal)?,
            "requester pubkey",
        )?,
        target_pubkey: parse_hex(
            row.try_get("target_pubkey").map_err(internal)?,
            "target pubkey",
        )?,
        state: row.try_get("state").map_err(internal)?,
        attempt,
        progress_seq: parse_u64_text(row.try_get("progress_seq").map_err(internal)?)?,
        summary: row.try_get("summary").map_err(internal)?,
        cancel_requested: row.try_get("cancel_requested").map_err(internal)?,
        terminal_event_id: row
            .try_get::<Option<Vec<u8>>, _>("terminal_event_id")
            .map_err(internal)?
            .map(|value| parse_hex(value, "terminal event id"))
            .transpose()?,
        updated_at: row.try_get("updated_at").map_err(internal)?,
    })
}

/// Indexed, writer-consistent lookup of canonical status and event chain.
pub(crate) async fn lookup_agent_job(
    db: &buzz_db::Db,
    community_id: CommunityId,
    job_id: Uuid,
) -> Result<Option<AgentJobLookup>, AgentJobAdmissionError> {
    let mut tx = db.begin_transaction().await.map_err(internal)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    let row = sqlx::query(
        r#"
        SELECT job_id, request_event_id, channel_id, requester_pubkey, target_pubkey,
               state, attempt, progress_seq::text AS progress_seq, summary,
               cancel_requested, terminal_event_id, updated_at
        FROM agent_jobs
        WHERE community_id = $1 AND job_id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(job_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    let Some(row) = row else {
        tx.rollback().await.map_err(internal)?;
        return Ok(None);
    };
    let status = projection_from_row(&row)?;
    let rows = sqlx::query(
        r#"
        SELECT aje.event_id, aje.kind, aje.author_pubkey, aje.attempt,
               aje.progress_seq::text AS progress_seq, aje.event_created_at,
               e.id AS signed_id, e.pubkey AS signed_pubkey,
               e.created_at AS signed_created_at, e.kind AS signed_kind,
               e.tags AS signed_tags, e.content AS signed_content, e.sig AS signed_sig
        FROM agent_job_events aje
        JOIN events e
          ON e.community_id = aje.community_id AND e.id = aje.event_id
        WHERE aje.community_id = $1 AND aje.job_id = $2
        ORDER BY aje.chain_seq
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(job_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    let mut chain = Vec::with_capacity(rows.len());
    for row in rows {
        let attempt = row
            .try_get::<Option<i64>, _>("attempt")
            .map_err(internal)?
            .map(|value| u32::try_from(value).map_err(internal))
            .transpose()?;
        chain.push(AgentJobChainEntry {
            event_id: parse_hex(row.try_get("event_id").map_err(internal)?, "chain event id")?,
            kind: u32::try_from(row.try_get::<i32, _>("kind").map_err(internal)?)
                .map_err(internal)?,
            author_pubkey: parse_hex(
                row.try_get("author_pubkey").map_err(internal)?,
                "chain author pubkey",
            )?,
            attempt,
            progress_seq: parse_u64_text(row.try_get("progress_seq").map_err(internal)?)?,
            created_at: row.try_get("event_created_at").map_err(internal)?,
            event: signed_event_from_row(&row)?,
        });
    }
    tx.commit().await.map_err(internal)?;
    Ok(Some(AgentJobLookup { status, chain }))
}

/// Indexed canonical list for an authorized participant, constrained to
/// currently accessible channels and optional target/channel/state filters.
pub(crate) async fn list_agent_jobs(
    db: &buzz_db::Db,
    community_id: CommunityId,
    participant: &[u8],
    accessible_channels: &[Uuid],
    target_pubkey: Option<&[u8]>,
    channel_id: Option<Uuid>,
    state: Option<&str>,
    limit: u16,
) -> Result<Vec<AgentJobProjection>, AgentJobAdmissionError> {
    let limit = i64::from(limit.clamp(1, 500));
    let mut tx = db.begin_transaction().await.map_err(internal)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    let rows = sqlx::query(
        r#"
        SELECT job_id, request_event_id, channel_id, requester_pubkey, target_pubkey,
               state, attempt, progress_seq::text AS progress_seq, summary,
               cancel_requested, terminal_event_id, updated_at
        FROM agent_jobs
        WHERE community_id = $1
          AND (requester_pubkey = $2 OR target_pubkey = $2)
          AND channel_id = ANY($3::uuid[])
          AND ($4::bytea IS NULL OR target_pubkey = $4)
          AND ($5::uuid IS NULL OR channel_id = $5)
          AND ($6::text IS NULL OR state = $6)
        ORDER BY updated_at DESC, job_id
        LIMIT $7
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(participant)
    .bind(accessible_channels)
    .bind(target_pubkey)
    .bind(channel_id)
    .bind(state)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    let projections = rows
        .iter()
        .map(projection_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    tx.commit().await.map_err(internal)?;
    Ok(projections)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::kind::{KIND_JOB_ACCEPTED, KIND_JOB_PROGRESS, KIND_JOB_RESULT};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sqlx::PgPool;

    fn signed_event(
        keys: &Keys,
        kind: u32,
        content: serde_json::Value,
        tags: Vec<Vec<String>>,
    ) -> Event {
        let tags = tags
            .into_iter()
            .map(|parts| Tag::parse(parts).expect("valid test tag"))
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::Custom(kind as u16), content.to_string())
            .tags(tags)
            .allow_self_tagging()
            .sign_with_keys(keys)
            .expect("sign test event")
    }

    fn request_event(
        requester: &Keys,
        target: &Keys,
        channel: Uuid,
        job: Uuid,
        summary: &str,
    ) -> Event {
        signed_event(
            requester,
            KIND_JOB_REQUEST,
            serde_json::json!({
                "schema": 1, "driver": "lh", "argv": ["lockdown", "run"],
                "cwd": "/tmp", "summary": summary
            }),
            vec![
                vec!["h".into(), channel.to_string()],
                vec!["p".into(), target.public_key().to_hex()],
                vec!["job".into(), job.to_string()],
            ],
        )
    }

    fn lifecycle_tags(peer: &Keys, channel: Uuid, job: Uuid, request: &Event) -> Vec<Vec<String>> {
        vec![
            vec!["h".into(), channel.to_string()],
            vec!["p".into(), peer.public_key().to_hex()],
            vec!["job".into(), job.to_string()],
            vec!["e".into(), request.id.to_hex()],
        ]
    }

    async fn seed_job_test() -> (PgPool, buzz_db::Db, CommunityId, Uuid, Keys, Keys) {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect test DB");
        buzz_db::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        let channel = Uuid::new_v4();
        let requester = Keys::generate();
        let target = Keys::generate();
        let requester_bytes = requester.public_key().to_bytes();
        let target_bytes = target.public_key().to_bytes();

        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(format!("agent-jobs-{}.test", community_uuid.simple()))
            .execute(&pool)
            .await
            .expect("seed community");
        sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(requester_bytes.as_slice())
            .execute(&pool)
            .await
            .expect("seed requester");
        sqlx::query(
            "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3)",
        )
        .bind(community_uuid)
        .bind(target_bytes.as_slice())
        .bind(requester_bytes.as_slice())
        .execute(&pool)
        .await
        .expect("seed managed agent");
        sqlx::query(
            "INSERT INTO channels (community_id, id, name, channel_type, visibility, created_by) VALUES ($1, $2, 'jobs', 'stream', 'private', $3)",
        )
        .bind(community_uuid)
        .bind(channel)
        .bind(requester_bytes.as_slice())
        .execute(&pool)
        .await
        .expect("seed channel");
        for (pubkey, role) in [
            (requester_bytes.as_slice(), "owner"),
            (target_bytes.as_slice(), "bot"),
        ] {
            sqlx::query(
                "INSERT INTO channel_members (community_id, channel_id, pubkey, role) VALUES ($1, $2, $3, $4::member_role)",
            )
            .bind(community_uuid)
            .bind(channel)
            .bind(pubkey)
            .bind(role)
            .execute(&pool)
            .await
            .expect("seed channel member");
        }
        (
            pool.clone(),
            buzz_db::Db::from_pool(pool),
            community,
            channel,
            requester,
            target,
        )
    }

    #[tokio::test]
    async fn malformed_envelope_is_rejected_before_database_access() {
        let requester = Keys::generate();
        let target = Keys::generate();
        let malformed = signed_event(
            &requester,
            KIND_JOB_REQUEST,
            serde_json::json!({
                "schema": 1, "driver": "lh", "argv": [], "cwd": "/tmp",
                "summary": "bad", "unknown": true
            }),
            vec![
                vec!["h".into(), Uuid::new_v4().to_string()],
                vec!["p".into(), target.public_key().to_hex()],
                vec!["job".into(), Uuid::new_v4().to_string()],
            ],
        );
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://invalid:invalid@127.0.0.1:1/invalid")
            .expect("lazy pool");
        let db = buzz_db::Db::from_pool(pool);
        assert!(matches!(
            persist_agent_job_event(&db, CommunityId::from_uuid(Uuid::new_v4()), &malformed).await,
            Err(AgentJobAdmissionError::Rejected(_))
        ));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn nonmember_owner_cannot_submit_agent_job_request() {
        let (pool, db, community, channel, owner, target) = seed_job_test().await;
        let owner_bytes = owner.public_key().to_bytes();
        sqlx::query(
            "DELETE FROM channel_members WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(owner_bytes.as_slice())
        .execute(&pool)
        .await
        .expect("remove owner channel membership");

        let job = Uuid::new_v4();
        let request = request_event(&owner, &target, channel, job, "unauthorized owner request");
        let error = match persist_agent_job_event(&db, community, &request).await {
            Err(error) => error,
            Ok(_) => panic!("nonmember owner request must fail closed"),
        };
        assert!(matches!(
            error,
            AgentJobAdmissionError::Rejected(message)
                if message.contains("requester is not authorized")
        ));
        let persisted: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_jobs WHERE community_id = $1 AND job_id = $2",
        )
        .bind(community.as_uuid())
        .bind(job)
        .fetch_one(&pool)
        .await
        .expect("count persisted jobs");
        assert_eq!(persisted, 0);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_admission_serializes_job_uuid_and_lifecycle() {
        let (_pool, db, community, channel, requester, target) = seed_job_test().await;
        let job = Uuid::new_v4();
        let request_a = request_event(&requester, &target, channel, job, "request A");
        let request_b = request_event(&requester, &target, channel, job, "request B");
        let (a, b) = tokio::join!(
            persist_agent_job_event(&db, community, &request_a),
            persist_agent_job_event(&db, community, &request_b),
        );
        assert_eq!(
            [&a, &b]
                .into_iter()
                .filter(|r| matches!(r, Ok(AgentJobPersistOutcome::Inserted(_))))
                .count(),
            1
        );
        assert_eq!(
            [&a, &b]
                .into_iter()
                .filter(|r| matches!(r, Err(AgentJobAdmissionError::Rejected(_))))
                .count(),
            1
        );
        let request = if matches!(a, Ok(AgentJobPersistOutcome::Inserted(_))) {
            request_a
        } else {
            request_b
        };
        assert!(matches!(
            persist_agent_job_event(&db, community, &request).await,
            Ok(AgentJobPersistOutcome::Replay)
        ));

        let accepted_a = signed_event(
            &target,
            KIND_JOB_ACCEPTED,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 1,
                "state": "accepted", "accepted_at": Utc::now()
            }),
            lifecycle_tags(&requester, channel, job, &request),
        );
        let accepted_b = signed_event(
            &target,
            KIND_JOB_ACCEPTED,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 1,
                "state": "accepted",
                "accepted_at": Utc::now() + chrono::Duration::seconds(1)
            }),
            lifecycle_tags(&requester, channel, job, &request),
        );
        let (a, b) = tokio::join!(
            persist_agent_job_event(&db, community, &accepted_a),
            persist_agent_job_event(&db, community, &accepted_b),
        );
        assert_eq!(
            [&a, &b]
                .into_iter()
                .filter(|r| matches!(r, Ok(AgentJobPersistOutcome::Inserted(_))))
                .count(),
            1
        );
        assert_eq!(
            [&a, &b]
                .into_iter()
                .filter(|r| matches!(r, Err(AgentJobAdmissionError::Rejected(_))))
                .count(),
            1
        );
        let lookup = lookup_agent_job(&db, community, job)
            .await
            .expect("lookup")
            .expect("job");
        assert_eq!(lookup.status.state, "accepted");
        assert_eq!(lookup.chain.len(), 2);
        assert_eq!(lookup.chain[0].kind, KIND_JOB_REQUEST);
        assert_eq!(lookup.chain[1].kind, KIND_JOB_ACCEPTED);
        assert_eq!(lookup.chain[0].event.id.to_hex(), lookup.chain[0].event_id);
        lookup.chain[0]
            .event
            .verify()
            .expect("signed request event");
        let listed = list_agent_jobs(
            &db,
            community,
            requester.public_key().as_bytes(),
            &[channel],
            None,
            Some(channel),
            Some("accepted"),
            10,
        )
        .await
        .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].job_id, job);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lifecycle_rejects_cross_scope_non_monotonic_and_post_terminal_events() {
        let (_pool, db, community, channel, requester, target) = seed_job_test().await;
        let job = Uuid::new_v4();
        let request = request_event(&requester, &target, channel, job, "run");
        persist_agent_job_event(&db, community, &request)
            .await
            .expect("request");

        let wrong_author = signed_event(
            &Keys::generate(),
            KIND_JOB_ACCEPTED,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 1,
                "state": "accepted", "accepted_at": Utc::now()
            }),
            lifecycle_tags(&requester, channel, job, &request),
        );
        assert!(matches!(
            persist_agent_job_event(&db, community, &wrong_author).await,
            Err(AgentJobAdmissionError::Rejected(_))
        ));
        let wrong_peer = Keys::generate();
        let wrong_target = signed_event(
            &target,
            KIND_JOB_ACCEPTED,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 1,
                "state": "accepted", "accepted_at": Utc::now()
            }),
            lifecycle_tags(&wrong_peer, channel, job, &request),
        );
        assert!(matches!(
            persist_agent_job_event(&db, community, &wrong_target).await,
            Err(AgentJobAdmissionError::Rejected(_))
        ));
        let wrong_channel = signed_event(
            &target,
            KIND_JOB_ACCEPTED,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 1,
                "state": "accepted", "accepted_at": Utc::now()
            }),
            lifecycle_tags(&requester, Uuid::new_v4(), job, &request),
        );
        assert!(matches!(
            persist_agent_job_event(&db, community, &wrong_channel).await,
            Err(AgentJobAdmissionError::Rejected(_))
        ));

        let accepted = signed_event(
            &target,
            KIND_JOB_ACCEPTED,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 1,
                "state": "accepted", "accepted_at": Utc::now()
            }),
            lifecycle_tags(&requester, channel, job, &request),
        );
        persist_agent_job_event(&db, community, &accepted)
            .await
            .expect("accepted");

        let mut wrong_attempt_tags = lifecycle_tags(&requester, channel, job, &request);
        wrong_attempt_tags.push(vec!["seq".into(), "1".into()]);
        let wrong_attempt = signed_event(
            &target,
            KIND_JOB_PROGRESS,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 2, "seq": 1,
                "state": "running", "summary": "wrong attempt", "artifacts": []
            }),
            wrong_attempt_tags,
        );
        assert!(matches!(
            persist_agent_job_event(&db, community, &wrong_attempt).await,
            Err(AgentJobAdmissionError::Rejected(_))
        ));

        let mut progress_tags = lifecycle_tags(&requester, channel, job, &request);
        progress_tags.push(vec!["seq".into(), "1".into()]);
        let progress = signed_event(
            &target,
            KIND_JOB_PROGRESS,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 1, "seq": 1,
                "state": "running", "summary": "running", "artifacts": []
            }),
            progress_tags.clone(),
        );
        persist_agent_job_event(&db, community, &progress)
            .await
            .expect("progress");
        let stale = signed_event(
            &target,
            KIND_JOB_PROGRESS,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 1, "seq": 1,
                "state": "running", "summary": "stale", "artifacts": []
            }),
            progress_tags,
        );
        assert!(matches!(
            persist_agent_job_event(&db, community, &stale).await,
            Err(AgentJobAdmissionError::Rejected(_))
        ));

        let result = signed_event(
            &target,
            KIND_JOB_RESULT,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 1, "state": "succeeded",
                "exit_code": 0, "summary": "done", "artifacts": [],
                "finished_at": Utc::now()
            }),
            lifecycle_tags(&requester, channel, job, &request),
        );
        persist_agent_job_event(&db, community, &result)
            .await
            .expect("terminal");
        let mut late_tags = lifecycle_tags(&requester, channel, job, &request);
        late_tags.push(vec!["seq".into(), "2".into()]);
        let late = signed_event(
            &target,
            KIND_JOB_PROGRESS,
            serde_json::json!({
                "schema": 1, "job": job, "attempt": 1, "seq": 2,
                "state": "running", "summary": "late", "artifacts": []
            }),
            late_tags,
        );
        assert!(matches!(
            persist_agent_job_event(&db, community, &late).await,
            Err(AgentJobAdmissionError::Rejected(_))
        ));
        let lookup = lookup_agent_job(&db, community, job)
            .await
            .expect("lookup")
            .expect("job");
        assert_eq!(lookup.status.state, "succeeded");
        assert_eq!(lookup.status.progress_seq, Some(1));
        assert_eq!(lookup.chain.len(), 4);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn requester_target_and_registered_owner_can_cancel_but_unrelated_signer_cannot() {
        let (pool, db, community, channel, requester, target) = seed_job_test().await;
        let owner = Keys::generate();
        let owner_bytes = owner.public_key().to_bytes();
        let target_bytes = target.public_key().to_bytes();
        sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2)")
            .bind(community.as_uuid())
            .bind(owner_bytes.as_slice())
            .execute(&pool)
            .await
            .expect("seed distinct managed agent owner");
        sqlx::query(
            "UPDATE users SET agent_owner_pubkey = $3 WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(target_bytes.as_slice())
        .bind(owner_bytes.as_slice())
        .execute(&pool)
        .await
        .expect("assign distinct managed agent owner");

        for (signer, principal) in [
            (&requester, "requester"),
            (&target, "target"),
            (&owner, "owner"),
        ] {
            let job = Uuid::new_v4();
            let request = request_event(&requester, &target, channel, job, principal);
            persist_agent_job_event(&db, community, &request)
                .await
                .expect("request");
            let accepted = signed_event(
                &target,
                KIND_JOB_ACCEPTED,
                serde_json::json!({
                    "schema": 1, "job": job, "attempt": 1,
                    "state": "accepted", "accepted_at": Utc::now()
                }),
                lifecycle_tags(&requester, channel, job, &request),
            );
            persist_agent_job_event(&db, community, &accepted)
                .await
                .expect("accepted");
            let cancel = signed_event(
                signer,
                KIND_JOB_CANCEL,
                serde_json::json!({
                    "schema": 1, "job": job, "reason": format!("{principal} stop")
                }),
                lifecycle_tags(&target, channel, job, &request),
            );
            persist_agent_job_event(&db, community, &cancel)
                .await
                .unwrap_or_else(|error| panic!("{principal} cancellation rejected: {error}"));

            let lookup = lookup_agent_job(&db, community, job)
                .await
                .expect("lookup")
                .expect("job");
            assert_eq!(lookup.status.state, "cancelling");
            assert!(lookup.status.cancel_requested);
            assert_eq!(lookup.chain.len(), 3);
        }

        let unrelated = Keys::generate();
        let job = Uuid::new_v4();
        let request = request_event(&requester, &target, channel, job, "reject unrelated");
        persist_agent_job_event(&db, community, &request)
            .await
            .expect("request");
        let cancel = signed_event(
            &unrelated,
            KIND_JOB_CANCEL,
            serde_json::json!({"schema": 1, "job": job, "reason": "unauthorized"}),
            lifecycle_tags(&target, channel, job, &request),
        );
        assert!(matches!(
            persist_agent_job_event(&db, community, &cancel).await,
            Err(AgentJobAdmissionError::Rejected(_))
        ));
        let lookup = lookup_agent_job(&db, community, job)
            .await
            .expect("lookup")
            .expect("job");
        assert_eq!(lookup.status.state, "requested");
        assert!(!lookup.status.cancel_requested);
        assert_eq!(lookup.chain.len(), 1);
    }
}
