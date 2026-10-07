//! NIP-AR atomic artifact acceptance. Payloads remain opaque signed events.
//!
//! Channel write permission is checked by the relay's ordinary ingest gates
//! before this transaction; here the relay only serializes each identity and
//! compares the expected head.
use crate::{Db, DbError, Result};
use buzz_core::artifact::{ArtifactEnvelope, ArtifactOp};
use buzz_core::{CommunityId, StoredEvent};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::Row;
use uuid::Uuid;

/// Acceptance outcome. Only a stale-`prev` conflict carries the current head,
/// which the caller discloses after authorizing its channel.
#[derive(Debug)]
pub enum ArtifactOutcome {
    /// Accepted once; stored events to publish (the revision, plus the source
    /// removal on a move).
    Accepted(Vec<StoredEvent>),
    /// Identical previously accepted event; no repeated side effects.
    Duplicate,
    /// `prev` or identity no longer matches; the client should reconcile.
    Conflict(&'static str),
    /// Protocol violation with a public-safe reason.
    Rejected(&'static str),
}

fn invalid(message: &str) -> DbError {
    DbError::InvalidData(message.into())
}

/// The replaced revision (`prev`) was readable in the source, so it
/// distinguishes repeated moves without revealing other activity.
fn removal_marker(keys: &Keys, artifact: Uuid, source: Uuid, prev: &[u8]) -> Result<Event> {
    let tags = [
        ["ar", "1"].map(str::to_owned),
        ["d".into(), artifact.to_string()],
        ["h".into(), source.to_string()],
        ["reason".into(), "moved".into()],
        ["prev".into(), hex::encode(prev)],
    ]
    .into_iter()
    .map(Tag::parse)
    .collect::<std::result::Result<Vec<_>, _>>()
    .map_err(|e| invalid(&e.to_string()))?;
    EventBuilder::new(Kind::Custom(45011), "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| invalid(&e.to_string()))
}

impl Db {
    /// Check the durable acceptance ledger, which survives redaction and retention.
    pub async fn artifact_accepted(&self, community: CommunityId, id: &[u8]) -> Result<bool> {
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM artifact_revisions WHERE community_id=$1 AND event_id=$2)",
        )
        .bind(community.as_uuid())
        .bind(id)
        .fetch_one(&mut *conn)
        .await?)
    }

    /// Current home channel, read before authorizing a move's source.
    pub async fn artifact_home(
        &self,
        community: CommunityId,
        artifact: Uuid,
    ) -> Result<Option<Uuid>> {
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        Ok(sqlx::query_scalar(
            "SELECT channel_id FROM artifact_heads WHERE community_id=$1 AND artifact_id=$2",
        )
        .bind(community.as_uuid())
        .bind(artifact)
        .fetch_optional(&mut *conn)
        .await?)
    }

    /// Atomically compare the expected head, store the full revision, advance
    /// the head, and on a move store the source removal. `authorized_source` is
    /// the move source the caller authorized; a head that has since moved elsewhere
    /// conflicts. Relay keys only sign removals.
    pub async fn accept_artifact(
        &self,
        community: CommunityId,
        event: &Event,
        env: &ArtifactEnvelope,
        authorized_source: Option<Uuid>,
        relay_keys: &Keys,
    ) -> Result<ArtifactOutcome> {
        let mut tx = self.begin_event_write_transaction(community).await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(tx.conn())
            .await?;
        // A coordinate lock handles the missing-row create race as well as edits.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("artifact:{}:{}", community.as_uuid(), env.id))
            .execute(tx.conn())
            .await?;
        let duplicate: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM artifact_revisions WHERE community_id=$1 AND event_id=$2)",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(tx.conn())
        .await?;
        if duplicate {
            return Ok(ArtifactOutcome::Duplicate);
        }
        let head = sqlx::query(
            "SELECT event_id,channel_id,artifact_type,root,deleted FROM artifact_heads WHERE community_id=$1 AND artifact_id=$2 FOR UPDATE",
        )
        .bind(community.as_uuid())
        .bind(env.id)
        .fetch_optional(tx.conn())
        .await?;
        let source = head.as_ref().map(|h| h.get::<Uuid, _>("channel_id"));
        let old_root = head
            .as_ref()
            .and_then(|h| h.get::<Option<Vec<u8>>, _>("root"));
        match (&head, env.op) {
            (Some(_), ArtifactOp::Create) => {
                return Ok(ArtifactOutcome::Conflict("artifact identity is taken"))
            }
            (None, ArtifactOp::Create) => {}
            (None, _) => return Ok(ArtifactOutcome::Conflict("artifact head unavailable")),
            (Some(head), op) => {
                let current: Vec<u8> = head.get("event_id");
                if env.prev.as_deref() != Some(current.as_slice()) {
                    return Ok(ArtifactOutcome::Conflict("artifact head changed"));
                }
                if env.artifact_type != head.get::<String, _>("artifact_type") {
                    return Ok(ArtifactOutcome::Rejected("artifact type is immutable"));
                }
                if head.get::<bool, _>("deleted") != (op == ArtifactOp::Restore) {
                    return Ok(ArtifactOutcome::Rejected(
                        "deleted artifacts require restore; live artifacts cannot restore",
                    ));
                }
                if (op == ArtifactOp::Move) != (source != Some(env.home)) {
                    return Ok(ArtifactOutcome::Rejected(
                        "only move changes home and move must change home",
                    ));
                }
                if op == ArtifactOp::Move && authorized_source != source {
                    return Ok(ArtifactOutcome::Conflict("artifact home changed"));
                }
                if op == ArtifactOp::Delete && env.root != old_root {
                    return Ok(ArtifactOutcome::Rejected("delete preserves root"));
                }
            }
        }
        if head.is_none() || env.root != old_root || source != Some(env.home) {
            if let Some(root) = &env.root {
                let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2 AND channel_id=$3 AND deleted_at IS NULL AND kind IN (9,40002,45001,45003))")
                    .bind(community.as_uuid()).bind(root).bind(env.home).fetch_one(tx.conn()).await?;
                if !exists {
                    return Ok(ArtifactOutcome::Rejected(
                        "root must be an existing conversation anchor in home",
                    ));
                }
            }
        }
        sqlx::query(
            "INSERT INTO artifact_revisions (community_id,event_id,artifact_id) VALUES ($1,$2,$3)",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .bind(env.id)
        .execute(tx.conn())
        .await?;
        sqlx::query("INSERT INTO artifact_heads (community_id,artifact_id,event_id,channel_id,artifact_type,root,deleted) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (community_id,artifact_id) DO UPDATE SET event_id=EXCLUDED.event_id,channel_id=EXCLUDED.channel_id,root=EXCLUDED.root,deleted=EXCLUDED.deleted")
            .bind(community.as_uuid()).bind(env.id).bind(event.id.as_bytes().as_slice()).bind(env.home).bind(&env.artifact_type).bind(&env.root).bind(env.op == ArtifactOp::Delete).execute(tx.conn()).await?;
        let (stored, _) =
            crate::event::insert_event_in_transaction(&mut tx, event, Some(env.home)).await?;
        crate::insert_mentions_in_transaction(&mut tx, event, Some(env.home)).await?;
        let mut accepted = vec![stored];
        if let (ArtifactOp::Move, Some(source), Some(prev)) = (env.op, source, &env.prev) {
            let removal = removal_marker(relay_keys, env.id, source, prev)?;
            let (stored, _) =
                crate::event::insert_event_in_transaction(&mut tx, &removal, Some(source)).await?;
            accepted.push(stored);
        }
        tx.commit().await?;
        Ok(ArtifactOutcome::Accepted(accepted))
    }
}
