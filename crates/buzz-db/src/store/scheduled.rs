//! Durable, owner-scoped scheduled delivery. A dispatch holds its row lock until
//! the normal message ingest has returned, serializing cancellation with sending.

#[cfg(test)]
#[path = "scheduled/postgres_tests.rs"]
mod postgres_tests;

use buzz_core::{CommunityId, StoredEvent};
use serde::Serialize;
use sqlx::Row;
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    runtime::{begin_community_write_transaction, observability::WriterOperation},
    AdmittedTx, Db, DbError, Result,
};

/// A scheduled message; the signed event is never returned to another owner.
#[derive(Debug, Clone, Serialize)]
pub struct ScheduledMessage {
    /// Caller-generated idempotency key.
    pub id: Uuid,
    /// Compare-and-swap revision.
    pub revision: i64,
    /// Exact user-signed event to publish.
    pub event: serde_json::Value,
    /// Unix seconds chosen for delivery.
    pub send_at: i64,
    /// Scheduled, sent, failed or cancelled.
    pub status: String,
    /// Sanitized delivery failure, if any.
    pub error: Option<String>,
}

fn row_message(row: &sqlx::postgres::PgRow) -> Result<ScheduledMessage> {
    Ok(ScheduledMessage {
        id: row.try_get("id")?,
        revision: row.try_get("revision")?,
        event: row.try_get("event")?,
        send_at: row.try_get("send_at")?,
        status: row.try_get("status")?,
        error: row.try_get("error")?,
    })
}

const MAX_DELIVERY_ATTEMPTS: i32 = 20;
const RETRY_EXHAUSTED: &str = "Delivery failed after repeated attempts. Review and reschedule it.";

fn conflict() -> DbError {
    DbError::InvalidData("scheduled-message-conflict".into())
}

/// A due item as last observed by the fleet scan.
pub struct ScheduledCandidate {
    /// The community that owns the queue row.
    pub community: CommunityId,
    /// The relay host for that community.
    pub host: String,
    /// The author's public key.
    pub owner: Vec<u8>,
    /// The owner's queue ID.
    pub id: Uuid,
    revision: i64,
}

/// Worker-owned admitted transaction. Dropping it rolls back the claim.
pub struct ScheduledDispatch {
    tx: Option<AdmittedTx>,
    pool: sqlx::PgPool,
    community: CommunityId,
    owner: Vec<u8>,
    /// Exact queued message.
    pub message: ScheduledMessage,
}

impl ScheduledDispatch {
    /// Community admitted by the held transaction.
    pub fn community(&self) -> CommunityId {
        self.community
    }
    /// Commit delivery, a permanent refusal, or a bounded transient backoff.
    pub async fn finish(mut self, status: &str, error: Option<&str>, now: i64) -> Result<()> {
        let mut tx = match self.tx.take() {
            Some(tx) => tx,
            None if status == "scheduled" => {
                // An ingest error may have dropped the claim. Record a retry
                // only if no other worker or owner mutation has won since.
                begin_community_write_transaction(
                    &self.pool,
                    self.community,
                    WriterOperation::EventWrite,
                )
                .await?
            }
            None => return Ok(()),
        };
        match status {
            "sent" => mark_sent(&mut tx, &self.owner, self.message.id).await?,
            "failed" => {
                sqlx::query("UPDATE scheduled_messages SET status='failed', error=$4, updated_at=clock_timestamp() WHERE community_id=$1 AND owner=$2 AND id=$3")
                    .bind(self.community.as_uuid()).bind(&self.owner).bind(self.message.id).bind(error)
                    .execute(tx.conn()).await?;
            }
            "scheduled" => {
                record_retry(
                    &mut tx,
                    &self.owner,
                    self.message.id,
                    self.message.revision,
                    now,
                    error,
                )
                .await?;
            }
            _ => return Err(conflict()),
        }
        tx.commit().await
    }

    /// Store the approved message, mentions, thread counters, push work and sent
    /// state on the claim transaction. A lost claim can never publish afterward.
    /// Pending queue timestamps reserve the replica fence until this commit.
    pub async fn persist(
        &mut self,
        db: &Db,
        event: &nostr::Event,
        channel: Option<Uuid>,
        thread: Option<crate::event::ThreadMetadataParams<'_>>,
    ) -> Result<(StoredEvent, bool)> {
        let (stored, inserted, ()) = self
            .persist_with(db, event, async |tx| {
                let (stored, inserted) =
                    crate::event::insert_event_with_thread_metadata_in_transaction(
                        tx, event, channel, thread,
                    )
                    .await?;
                Ok::<_, DbError>((stored, inserted, ()))
            })
            .await?;
        Ok((stored, inserted))
    }

    /// Run the ordinary event writer on the held claim, including its related
    /// writes. The writer must return the exact stored message. Any error
    /// drops the transaction so neither the message nor sent state commits.
    pub async fn persist_with<T, E: From<DbError>>(
        &mut self,
        db: &Db,
        event: &nostr::Event,
        write: impl AsyncFnOnce(&mut AdmittedTx) -> std::result::Result<(StoredEvent, bool, T), E>,
    ) -> std::result::Result<(StoredEvent, bool, T), E> {
        if event.kind.as_u16() != 9
            || serde_json::to_value(event).map_err(DbError::from)? != self.message.event
        {
            return Err(conflict().into());
        }
        let mut tx = self.tx.take().ok_or_else(conflict)?;
        tx.authorize_scheduled_event(event.id);
        tx.set_push_enqueue(db.push_enqueue.clone());
        let result = write(&mut tx).await?;
        if result.0.event.id != event.id {
            return Err(conflict().into());
        }
        mark_sent(&mut tx, &self.owner, self.message.id).await?;
        tx.commit().await?;
        Ok(result)
    }
}

impl Db {
    /// Return all active items (capped at creation) and the last 100 sent items.
    pub async fn scheduled_list(
        &self,
        community: CommunityId,
        owner: &[u8],
    ) -> Result<Vec<ScheduledMessage>> {
        let rows = sqlx::query("(SELECT * FROM scheduled_messages WHERE community_id=$1 AND owner=$2 AND status IN ('scheduled','failed') ORDER BY send_at LIMIT 256) UNION ALL (SELECT * FROM scheduled_messages WHERE community_id=$1 AND owner=$2 AND status='sent' ORDER BY send_at DESC LIMIT 100)")
            .bind(community.as_uuid()).bind(owner).fetch_all(&self.pool).await?;
        rows.iter().map(row_message).collect()
    }

    /// Recognize an exact committed operation before retry-time freshness checks.
    /// No write is performed, and cancelled or superseded operations never replay.
    pub async fn scheduled_replayed(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: Uuid,
        expected: Option<i64>,
        event: &serde_json::Value,
    ) -> Result<bool> {
        let revision = match expected {
            Some(value) => match value.checked_add(1) {
                Some(value) => value,
                None => return Ok(false),
            },
            None => 1,
        };
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM scheduled_messages WHERE community_id=$1 AND owner=$2 AND id=$3 AND revision=$4 AND event=$5 AND status IN ('scheduled','sent','failed'))")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(revision).bind(event)
            .fetch_one(&self.pool).await?)
    }

    /// Insert/update/cancel under the owner's lock, using an explicit revision.
    /// Identical retries succeed after an acknowledgement was lost.
    #[allow(clippy::too_many_arguments)]
    pub async fn scheduled_mutate(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: Uuid,
        expected: Option<i64>,
        event: Option<&serde_json::Value>,
        send_at: i64,
    ) -> Result<()> {
        let mut tx =
            begin_community_write_transaction(&self.pool, community, WriterOperation::EventWrite)
                .await?;
        let key = format!("scheduled:{community}:{}", hex::encode(owner));
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(key)
            .execute(tx.conn())
            .await?;
        let existing = sqlx::query("SELECT * FROM scheduled_messages WHERE community_id=$1 AND owner=$2 AND id=$3 FOR UPDATE")
            .bind(community.as_uuid()).bind(owner).bind(id).fetch_optional(tx.conn()).await?;
        let event_id = event
            .map(|v| {
                v["id"]
                    .as_str()
                    .and_then(|s| hex::decode(s).ok())
                    .ok_or_else(conflict)
            })
            .transpose()?;
        if let Some(row) = existing {
            let current = row_message(&row)?;
            let stored: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)",
            )
            .bind(community.as_uuid())
            .bind(row.try_get::<Vec<u8>, _>("event_id")?)
            .fetch_one(tx.conn())
            .await?;
            if stored && current.status != "sent" {
                sqlx::query("UPDATE scheduled_messages SET status='sent',error=NULL,updated_at=now() WHERE community_id=$1 AND owner=$2 AND id=$3")
                    .bind(community.as_uuid()).bind(owner).bind(id).execute(tx.conn()).await?;
                tx.commit().await?;
                return Err(conflict());
            }
            if current.status != "cancelled"
                && event.is_some_and(|v| *v == current.event)
                && current.send_at == send_at
            {
                return tx.commit().await;
            }
            if event.is_none()
                && current.status == "cancelled"
                && expected == Some(current.revision - 1)
            {
                return tx.commit().await;
            }
            if expected != Some(current.revision)
                || !matches!(current.status.as_str(), "scheduled" | "failed")
            {
                return Err(conflict());
            }
            validate_reservation(&mut tx, event, send_at).await?;
            sqlx::query("UPDATE scheduled_messages SET revision=revision+1, event=COALESCE($4,event), event_id=COALESCE($5,event_id), send_at=CASE WHEN $4 IS NULL THEN send_at ELSE $6 END, next_attempt_at=$6, status=$7, error=NULL, attempts=0,
                updated_at=clock_timestamp(),
                prune_at=CASE WHEN $7='cancelled' THEN GREATEST(clock_timestamp(), to_timestamp(send_at)) + interval '30 days' END
                WHERE community_id=$1 AND owner=$2 AND id=$3")
                .bind(community.as_uuid()).bind(owner).bind(id).bind(event).bind(event_id).bind(send_at)
                .bind(if event.is_some() { "scheduled" } else { "cancelled" }).execute(tx.conn()).await?;
        } else {
            if expected.is_some() || event.is_none() {
                return Err(conflict());
            }
            validate_reservation(&mut tx, event, send_at).await?;
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM scheduled_messages WHERE community_id=$1 AND owner=$2 AND status IN ('scheduled','failed')")
                .bind(community.as_uuid()).bind(owner).fetch_one(tx.conn()).await?;
            if count >= 256 {
                return Err(DbError::InvalidData("scheduled-message-limit".into()));
            }
            let inserted = sqlx::query("INSERT INTO scheduled_messages(community_id,owner,id,event,event_id,send_at,next_attempt_at) VALUES($1,$2,$3,$4,$5,$6,$6) ON CONFLICT (community_id,owner,event_id) DO NOTHING")
                .bind(community.as_uuid()).bind(owner).bind(id).bind(event).bind(event_id).bind(send_at).execute(tx.conn()).await?;
            if inserted.rows_affected() != 1 {
                return Err(conflict());
            }
        }
        tx.commit().await
    }

    /// Bounded fleet scan; each candidate is rechecked under admission and lock.
    pub async fn scheduled_due(&self, now: i64) -> Result<Vec<ScheduledCandidate>> {
        let rows = sqlx::query("SELECT s.community_id,c.host,s.owner,s.id,s.revision FROM scheduled_messages s JOIN communities c ON c.id=s.community_id WHERE s.status='scheduled' AND s.next_attempt_at<=$1 AND c.deletion_state='active' AND c.deleted_at IS NULL ORDER BY s.next_attempt_at LIMIT 100")
            .bind(now).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|r| {
                Ok(ScheduledCandidate {
                    community: CommunityId::from_uuid(r.try_get("community_id")?),
                    host: r.try_get("host")?,
                    owner: r.try_get("owner")?,
                    id: r.try_get("id")?,
                    revision: r.try_get("revision")?,
                })
            })
            .collect()
    }

    /// Back off a candidate whose dispatch failed outside a usable claim.
    pub async fn scheduled_defer(&self, candidate: &ScheduledCandidate, now: i64) -> Result<()> {
        let mut tx = begin_community_write_transaction(
            &self.pool,
            candidate.community,
            WriterOperation::EventWrite,
        )
        .await?;
        record_retry(
            &mut tx,
            &candidate.owner,
            candidate.id,
            candidate.revision,
            now,
            Some("Delivery delayed; the relay will retry."),
        )
        .await?;
        tx.commit().await
    }

    /// Prune a bounded batch of expired terminal rows. Every delete rechecks
    /// expiry under community admission, including when workers overlap.
    pub async fn scheduled_prune_completed(&self) -> Result<()> {
        let rows: Vec<(Uuid, Vec<u8>, Uuid)> = sqlx::query_as(
            "SELECT s.community_id,s.owner,s.id FROM scheduled_messages s
             JOIN communities c ON c.id=s.community_id
             WHERE s.prune_at <= now() AND c.deletion_state='active' AND c.deleted_at IS NULL
             ORDER BY s.prune_at LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut communities = BTreeMap::<Uuid, (Vec<Vec<u8>>, Vec<Uuid>)>::new();
        for (community, owner, id) in rows {
            let (owners, ids) = communities.entry(community).or_default();
            owners.push(owner);
            ids.push(id);
        }
        for (community, (owners, ids)) in communities {
            let result = async {
                let mut tx = begin_community_write_transaction(
                    &self.pool, CommunityId::from_uuid(community), WriterOperation::EventWrite,
                ).await?;
                sqlx::query("DELETE FROM scheduled_messages s USING unnest($2::bytea[], $3::uuid[]) AS expired(owner,id)
                    WHERE s.community_id=$1 AND s.owner=expired.owner AND s.id=expired.id
                      AND s.prune_at <= now()")
                    .bind(community).bind(&owners).bind(&ids).execute(tx.conn()).await?;
                tx.commit().await
            }.await;
            if let Err(error) = result {
                tracing::warn!(%community, error_kind = %error.kind(), "Scheduled delivery retention batch failed");
            }
        }
        Ok(())
    }

    /// Claim a due item on one worker. Other workers skip the held row.
    pub async fn scheduled_claim(
        &self,
        community: CommunityId,
        owner: &[u8],
        id: Uuid,
        now: i64,
    ) -> Result<Option<ScheduledDispatch>> {
        let mut tx =
            begin_community_write_transaction(&self.pool, community, WriterOperation::EventWrite)
                .await?;
        let row = sqlx::query("SELECT * FROM scheduled_messages WHERE community_id=$1 AND owner=$2 AND id=$3 AND status='scheduled' AND next_attempt_at<=$4 FOR UPDATE SKIP LOCKED")
            .bind(community.as_uuid()).bind(owner).bind(id).bind(now).fetch_optional(tx.conn()).await?;
        row.map(|r| {
            Ok(ScheduledDispatch {
                message: row_message(&r)?,
                tx: Some(tx),
                pool: self.pool.clone(),
                community,
                owner: owner.to_vec(),
            })
        })
        .transpose()
    }
}

async fn mark_sent(tx: &mut AdmittedTx, owner: &[u8], id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE scheduled_messages
        SET status='sent', error=NULL, updated_at=clock_timestamp(),
            prune_at=GREATEST(clock_timestamp(), to_timestamp(send_at)) + interval '30 days'
        WHERE community_id=$1 AND owner=$2 AND id=$3",
    )
    .bind(tx.community().as_uuid())
    .bind(owner)
    .bind(id)
    .execute(tx.conn())
    .await?;
    Ok(())
}

async fn record_retry(
    tx: &mut AdmittedTx,
    owner: &[u8],
    id: Uuid,
    revision: i64,
    now: i64,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "WITH candidate AS (
        SELECT attempts FROM scheduled_messages
        WHERE community_id=$1 AND owner=$2 AND id=$3 AND revision=$4
          AND status='scheduled' AND next_attempt_at <= $5
        FOR UPDATE SKIP LOCKED
    )
    UPDATE scheduled_messages s
    SET status=CASE WHEN candidate.attempts >= $6-1 THEN 'failed' ELSE 'scheduled' END,
        error=CASE WHEN candidate.attempts >= $6-1 THEN $7 ELSE $8 END,
        attempts=candidate.attempts+1,
        next_attempt_at=$5+(1::bigint << LEAST(candidate.attempts, 8)),
        updated_at=clock_timestamp()
    FROM candidate WHERE s.community_id=$1 AND s.owner=$2 AND s.id=$3",
    )
    .bind(tx.community().as_uuid())
    .bind(owner)
    .bind(id)
    .bind(revision)
    .bind(now)
    .bind(MAX_DELIVERY_ATTEMPTS)
    .bind(RETRY_EXHAUSTED)
    .bind(error)
    .execute(tx.conn())
    .await?;
    Ok(())
}

// A new reservation must be newer than every previously proved fence wall.
// Use the writer's clock, including after lock waits; host clock skew must not
// admit an event behind an already published replica completeness proof.
async fn validate_reservation(
    tx: &mut AdmittedTx,
    event: Option<&serde_json::Value>,
    send_at: i64,
) -> Result<()> {
    let Some(event) = event else {
        return Ok(());
    };
    let created = event["created_at"].as_i64().ok_or_else(conflict)?;
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(tx.conn())
            .await?;
    if send_at < now - 300
        || send_at > now + 120 * 86_400
        || created < send_at - 300
        || created > send_at + 5
    {
        return Err(conflict());
    }
    Ok(())
}
