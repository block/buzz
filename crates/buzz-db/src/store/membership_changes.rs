//! Memberships that ended since a time: removals and channel deletions.
use crate::{Db, Result};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

/// Most changes one read returns. A larger result is reported as
/// [`MembershipChanges::Overflow`] so the caller falls back to a full read.
pub const MAX_ROWS: usize = 2048;

/// Why a membership ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MembershipChangeReason {
    /// The reader's membership row was removed (kick or leave).
    Removed,
    /// The channel was deleted while the reader's membership was still active.
    Deleted,
}

impl MembershipChangeReason {
    /// Wire name of the reason.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::Deleted => "deleted",
        }
    }
}

/// One ended membership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipChange {
    /// Channel the reader's membership ended in.
    pub channel_id: Uuid,
    /// Database time of the removal or deletion.
    pub at: DateTime<Utc>,
    /// Why the membership ended.
    pub reason: MembershipChangeReason,
}

/// Result of [`Db::membership_changes_since`].
#[derive(Debug)]
pub enum MembershipChanges {
    /// Every change at or after `since`, oldest first.
    Changes {
        /// Database clock when the read ran.
        as_of: DateTime<Utc>,
        /// Ended memberships.
        changes: Vec<MembershipChange>,
    },
    /// More than [`MAX_ROWS`] changes matched; none are returned.
    Overflow,
}

impl Db {
    /// List the reader's memberships that ended at or after `since`.
    ///
    /// Removed rows are reported whatever the channel's current visibility,
    /// because a client's channel list is membership-based. Rejoining clears
    /// `removed_at`, so a rejoined channel is not reported.
    pub async fn membership_changes_since(
        &self,
        community: CommunityId,
        reader: &[u8],
        since: DateTime<Utc>,
    ) -> Result<MembershipChanges> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        // Read the clock first: a transaction that stamped before this instant
        // but commits later is covered by the client's overlap, not by us.
        let as_of: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        let rows = sqlx::query(CHANGES)
            .bind(community.as_uuid())
            .bind(reader)
            .bind(since)
            .bind((MAX_ROWS + 1) as i64)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        if rows.len() > MAX_ROWS {
            return Ok(MembershipChanges::Overflow);
        }
        let changes = rows
            .into_iter()
            .map(|row| {
                let removed: bool = row.try_get("removed")?;
                Ok(MembershipChange {
                    channel_id: row.try_get("channel_id")?,
                    at: row.try_get("at")?,
                    reason: if removed {
                        MembershipChangeReason::Removed
                    } else {
                        MembershipChangeReason::Deleted
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(MembershipChanges::Changes { as_of, changes })
    }
}

// Uses channel_members_sync (community_id, pubkey, sync_modified_at, channel_id)
// for removals and the reader's active memberships to reach deleted channels.
const CHANGES: &str = r#"
SELECT true AS removed, m.channel_id, m.sync_modified_at AS at
FROM channel_members m
WHERE m.community_id=$1 AND m.pubkey=$2
  AND m.removed_at IS NOT NULL AND m.sync_modified_at >= $3
UNION ALL
SELECT false, c.id, c.deleted_at
FROM channel_members m
JOIN channels c ON c.community_id=m.community_id AND c.id=m.channel_id
WHERE m.community_id=$1 AND m.pubkey=$2 AND m.removed_at IS NULL
  AND c.deleted_at IS NOT NULL AND c.deleted_at >= $3
ORDER BY at, channel_id
LIMIT $4
"#;
