//! Per-message coverage: whether the sidebar counts a message as unread, by
//! the same rules, positions and floors. The client never infers arrival order.

use super::{model::*, projection::read_account, writes};
use buzz_core::CommunityId;
use chrono::DateTime;
use sqlx::{Acquire, Row};
use std::collections::HashMap;
use uuid::Uuid;

use crate::{observability, Db, DbError, Result};

/// At most this many message IDs per coverage request.
pub const MAX_COVERAGE_IDS: usize = 100;

/// One message's coverage answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    /// Counted: the sidebar includes it in an unread row.
    Unread,
    /// Not counted: covered by a position or floor, the actor's own, or a
    /// reply in a thread the actor does not live-follow.
    Read,
    /// Not answerable: missing, deleted, ineligible, outside a joined channel,
    /// on no arrival index, or with unresolved ancestry.
    Unknown,
}

/// The answers plus each answered message's channel, so callers can recheck
/// access before release.
#[derive(Debug, Default)]
pub struct CoverageAnswer {
    /// Answer per requested lowercase hex ID.
    pub messages: HashMap<String, Coverage>,
    /// Channels of every answer other than `Unknown`.
    pub channels: Vec<Uuid>,
    /// The actor's applied-intent count in this answer's snapshot.
    pub revision: i64,
}

impl Db {
    /// Classify up to [`MAX_COVERAGE_IDS`] messages in one read-only snapshot.
    /// Callers must recheck admission/resource access before releasing this data.
    pub async fn personal_read_coverage(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        retention_seconds: u32,
        ids: &[String],
    ) -> Result<CoverageAnswer> {
        if !(1..=MAX_COVERAGE_IDS).contains(&ids.len()) {
            return Err(DbError::InvalidData("invalid coverage ids".into()));
        }
        let mut answer = CoverageAnswer::default();
        let mut bytes = Vec::with_capacity(ids.len());
        for id in ids {
            let id = id.to_ascii_lowercase();
            match writes::event_id(&id) {
                Some(b) => bytes.push(b),
                None => return Err(DbError::InvalidData("invalid coverage id".into())),
            }
            answer.messages.insert(id, Coverage::Unknown);
        }
        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut tx = conn.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        writes::deadlines(&mut tx).await?;
        let actor_bytes = actor.to_bytes();
        let account = read_account(&mut tx, community, &actor_bytes, retention_seconds).await?;
        let cutoff = DateTime::from_timestamp_millis(account.cutoff_ms)
            .ok_or_else(|| DbError::InvalidData("invalid dormancy cutoff".into()))?;
        // Scope: a timeline message (top-level or broadcast reply) answers by
        // the timeline position, any other resolved reply by its thread's
        // live follow. Positions are floored exactly as in the sidebar.
        let rows = sqlx::query(
            r#"SELECT encode(e.id,'hex') AS id, e.channel_id,
                CASE
                    WHEN e.pubkey=$2 THEN 'read'
                    WHEN e.read_timeline THEN
                        CASE WHEN e.received_at > GREATEST(cf.through_timestamp,
                            COALESCE($3::timestamptz,'infinity'), $4) THEN 'unread' ELSE 'read' END
                    WHEN tm.root_event_id IS NOT NULL AND tm.root_event_id<>e.id
                        AND tm.received_at IS NOT NULL THEN
                        CASE WHEN tf.root_id IS NULL OR tf.unfollowed
                                OR NOT (tf.explicit OR tf.active_at >= $4) THEN 'read'
                            WHEN e.received_at > GREATEST(tf.through_timestamp,
                                COALESCE($3::timestamptz,'infinity'), $4) THEN 'unread'
                            ELSE 'read' END
                    ELSE 'unknown' END AS coverage
             FROM events e
             JOIN channel_members cm ON cm.community_id=$1 AND cm.channel_id=e.channel_id
                 AND cm.pubkey=$2 AND cm.removed_at IS NULL
             JOIN channels c ON c.community_id=$1 AND c.id=e.channel_id AND c.deleted_at IS NULL
             LEFT JOIN thread_metadata tm ON tm.community_id=$1 AND tm.channel_id=e.channel_id
                 AND tm.event_created_at=e.created_at AND tm.event_id=e.id
             LEFT JOIN personal_read_frontiers cf ON cf.community_id=$1 AND cf.actor=$2
                 AND cf.channel_id=e.channel_id AND cf.root_id=''::bytea
             LEFT JOIN personal_read_frontiers tf ON tf.community_id=$1 AND tf.actor=$2
                 AND tf.channel_id=e.channel_id AND tf.root_id=tm.root_event_id
             WHERE e.community_id=$1 AND e.id=ANY($5) AND e.kind=ANY($6)
                 AND e.deleted_at IS NULL"#,
        )
        .bind(community.as_uuid())
        .bind(actor_bytes.as_slice())
        .bind(account.started_at)
        .bind(cutoff)
        .bind(&bytes)
        .bind(ELIGIBLE_KINDS.as_slice())
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut channels = std::collections::BTreeSet::new();
        for row in rows {
            let id: String = row.try_get("id")?;
            let coverage = match row.try_get::<String, _>("coverage")?.as_str() {
                "read" => Coverage::Read,
                "unread" => Coverage::Unread,
                _ => Coverage::Unknown,
            };
            if coverage != Coverage::Unknown {
                if let Some(channel) = row.try_get::<Option<Uuid>, _>("channel_id")? {
                    channels.insert(channel);
                }
            }
            answer.messages.insert(id, coverage);
        }
        answer.channels = channels.into_iter().collect();
        answer.revision = account.revision;
        Ok(answer)
    }
}
