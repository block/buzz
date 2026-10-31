//! Explicit, short-lived operator-listener subscriptions to one community thread.

use chrono::{TimeDelta, Utc};
use nostr::Event;
use uuid::Uuid;

use crate::{error::Result, AdmittedTx, CommunityId, Db};

/// A watch must be renewed with fresh target consent within 30 days.
pub const THREAD_WATCH_RETENTION: TimeDelta = TimeDelta::days(30);
/// Includes expired rows until the next registration prunes them.
pub const MAX_THREAD_WATCHES: i64 = 128;

impl Db {
    /// Register or renew exactly one target-signed community/channel/root binding.
    /// Invalid membership, root, missing registration or capacity fails closed.
    pub async fn register_operator_listener_thread(
        &self,
        community: CommunityId,
        listener: &[u8],
        target: &[u8],
        channel: Uuid,
        root: &[u8],
    ) -> Result<bool> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        // Serialize capacity and renewals for this listener/target pair. No new
        // watch may outlive an expired or explicitly removed target registration.
        let registered = sqlx::query(
            "SELECT 1 FROM operator_listener_pubkeys WHERE listener_pubkey = $1 \
             AND target_pubkey = $2 AND created_at >= $3 FOR UPDATE",
        )
        .bind(listener)
        .bind(target)
        .bind(Utc::now() - super::operator_listener::PUBKEY_RETENTION)
        .fetch_optional(tx.conn())
        .await?
        .is_some();
        if !registered {
            return Ok(false);
        }
        let authorized = sqlx::query(
            "SELECT 1 FROM channels c \
             JOIN users u ON u.community_id = c.community_id AND u.pubkey = $3 AND u.deactivated_at IS NULL \
             JOIN channel_members m ON m.community_id = c.community_id AND m.channel_id = c.id \
             JOIN events e ON e.community_id = c.community_id AND e.channel_id = c.id AND e.id = $4 \
             WHERE c.community_id = $1 AND c.id = $2 AND c.deleted_at IS NULL AND c.archived_at IS NULL \
               AND m.pubkey = $3 AND m.removed_at IS NULL AND e.deleted_at IS NULL \
               AND e.kind IN (9, 40002, 45001, 45003) \
               AND NOT EXISTS (SELECT 1 FROM thread_metadata tm \
                 WHERE tm.community_id = $1 AND tm.event_id = e.id AND tm.parent_event_id IS NOT NULL) \
             FOR SHARE OF c, m, e",
        ).bind(community.as_uuid()).bind(channel).bind(target).bind(root)
            .fetch_optional(tx.conn()).await?.is_some();
        if !authorized {
            return Ok(false);
        }
        sqlx::query(
            "DELETE FROM operator_listener_threads WHERE community_id = $1 \
            AND listener_pubkey = $2 AND target_pubkey = $3 AND expires_at <= now()",
        )
        .bind(community.as_uuid())
        .bind(listener)
        .bind(target)
        .execute(tx.conn())
        .await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM operator_listener_threads \
            WHERE community_id = $1 AND listener_pubkey = $2 AND target_pubkey = $3",
        )
        .bind(community.as_uuid())
        .bind(listener)
        .bind(target)
        .fetch_one(tx.conn())
        .await?;
        let existing = sqlx::query(
            "SELECT 1 FROM operator_listener_threads \
            WHERE community_id = $1 AND listener_pubkey = $2 AND target_pubkey = $3 \
              AND channel_id = $4 AND root_event_id = $5",
        )
        .bind(community.as_uuid())
        .bind(listener)
        .bind(target)
        .bind(channel)
        .bind(root)
        .fetch_optional(tx.conn())
        .await?
        .is_some();
        if count >= MAX_THREAD_WATCHES && !existing {
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO operator_listener_threads \
            (community_id, listener_pubkey, target_pubkey, channel_id, root_event_id, expires_at) \
            VALUES ($1, $2, $3, $4, $5, $6) \
            ON CONFLICT (community_id, listener_pubkey, target_pubkey, channel_id, root_event_id) \
            DO UPDATE SET expires_at = EXCLUDED.expires_at",
        )
        .bind(community.as_uuid())
        .bind(listener)
        .bind(target)
        .bind(channel)
        .bind(root)
        .bind(Utc::now() + THREAD_WATCH_RETENTION)
        .execute(tx.conn())
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Remove one binding for the authenticated listener; never affects another target.
    pub async fn remove_operator_listener_thread(
        &self,
        community: CommunityId,
        listener: &[u8],
        target: &[u8],
        channel: Uuid,
        root: &[u8],
    ) -> Result<()> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        sqlx::query("DELETE FROM operator_listener_threads WHERE community_id = $1 \
            AND listener_pubkey = $2 AND target_pubkey = $3 AND channel_id = $4 AND root_event_id = $5")
            .bind(community.as_uuid()).bind(listener).bind(target).bind(channel).bind(root)
            .execute(tx.conn()).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Reuse the existing outbox-reap cadence to remove bounded batches of expired
    /// watches through community admission. Deleting communities own their purge.
    pub async fn reap_operator_listener_threads(&self) -> Result<u64> {
        let communities: Vec<Uuid> = sqlx::query_scalar(
            "SELECT w.community_id FROM operator_listener_threads w \
             JOIN communities c ON c.id = w.community_id AND c.deletion_state = 'active' AND c.deleted_at IS NULL AND c.archived_at IS NULL \
             WHERE w.expires_at <= now() ORDER BY w.expires_at LIMIT 100",
        ).fetch_all(&self.pool).await?;
        let mut seen = std::collections::HashSet::new();
        let mut removed = 0;
        for id in communities {
            if !seen.insert(id) {
                continue;
            }
            let mut tx = crate::begin_community_write_transaction(
                &self.pool,
                CommunityId::from_uuid(id),
                crate::observability::WriterOperation::Maintenance,
            )
            .await?;
            removed += sqlx::query(
                "DELETE FROM operator_listener_threads WHERE community_id = $1 \
                 AND (listener_pubkey,target_pubkey,channel_id,root_event_id) IN ( \
                   SELECT listener_pubkey,target_pubkey,channel_id,root_event_id FROM operator_listener_threads \
                   WHERE community_id = $1 AND expires_at <= now() ORDER BY expires_at LIMIT 1024 FOR UPDATE SKIP LOCKED)",
            ).bind(id).execute(tx.conn()).await?.rows_affected();
            tx.commit().await?;
        }
        Ok(removed)
    }
}

/// Enqueue only committed canonical replies, never root events or channel chatter.
/// Called after thread metadata is persisted, in the event's admitted transaction.
pub(crate) async fn enqueue_thread_replies(tx: &mut AdmittedTx, event: &Event) -> Result<u64> {
    let community = tx.community();
    let created = chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).ok_or(
        crate::error::DbError::InvalidTimestamp(event.created_at.as_secs() as i64),
    )?;
    let rows = sqlx::query(
        "INSERT INTO operator_listener_outbox \
         (listener_pubkey, target_pubkey, community_id, event_id, event_kind, event_created_at, thread_root_event_id) \
         SELECT w.listener_pubkey, w.target_pubkey, w.community_id, $2, $3, $4, w.root_event_id \
         FROM thread_metadata tm \
         JOIN operator_listener_threads w ON w.community_id = tm.community_id AND w.channel_id = tm.channel_id \
           AND w.root_event_id = COALESCE(tm.root_event_id, tm.parent_event_id) \
         JOIN operator_listener_pubkeys r ON r.listener_pubkey = w.listener_pubkey AND r.target_pubkey = w.target_pubkey \
           AND r.created_at >= $5 \
         JOIN users u ON u.community_id = w.community_id AND u.pubkey = w.target_pubkey AND u.deactivated_at IS NULL \
         JOIN channel_members m ON m.community_id = w.community_id AND m.channel_id = w.channel_id \
           AND m.pubkey = w.target_pubkey AND m.removed_at IS NULL \
         JOIN channels c ON c.community_id = w.community_id AND c.id = w.channel_id AND c.deleted_at IS NULL AND c.archived_at IS NULL \
         JOIN events root ON root.community_id = w.community_id AND root.channel_id = w.channel_id \
           AND root.id = w.root_event_id AND root.deleted_at IS NULL \
         WHERE tm.community_id = $1 AND tm.event_id = $2 AND tm.event_created_at = $4 \
           AND tm.parent_event_id IS NOT NULL AND tm.event_id <> w.root_event_id AND w.expires_at > now() AND w.target_pubkey <> $6 \
         ON CONFLICT DO NOTHING",
    ).bind(community.as_uuid()).bind(event.id.as_bytes().as_slice()).bind(i32::from(event.kind.as_u16()))
        .bind(created).bind(Utc::now() - super::operator_listener::PUBKEY_RETENTION)
        .bind(event.pubkey.as_bytes().as_slice())
        .execute(tx.conn()).await?;
    Ok(rows.rows_affected())
}

#[cfg(test)]
#[path = "operator_listener_threads_postgres_tests.rs"]
mod postgres_tests;
