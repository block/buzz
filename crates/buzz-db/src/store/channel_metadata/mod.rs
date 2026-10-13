//! Application-owned channel metadata serialization.
//!
//! One admitted transaction owns fresh channel state, current membership,
//! application provenance, and replacement publication. Signing remains with the
//! relay/operator caller. No connection or raw SQL escapes the guard.

mod activation;
mod command;
mod snapshot;

#[cfg(test)]
mod postgres_tests;

use buzz_core::{channel_labels::LabelSet, CommunityId, StoredEvent};
use nostr::PublicKey;
use sqlx::Row;
use uuid::Uuid;

use crate::{channel::ChannelRecord, AdmittedTx, Db, DbError, Result};

pub use command::{ChannelCreate, CommandApplication};

/// A channel's fresh metadata held behind cross-replica transaction locks.
#[must_use = "dropping a metadata write rolls back all effects"]
pub struct ChannelMetadataWrite {
    tx: AdmittedTx,
    channel_id: Uuid,
    relay: PublicKey,
    channel: Option<ChannelRecord>,
    labels: LabelSet,
    previous: Option<StoredEvent>,
    published: Option<StoredEvent>,
    command_applied: bool,
    must_rollback: bool,
}

impl Db {
    /// Begin a serialized channel metadata write, including creation of a missing UUID.
    ///
    /// Lock order: admitted community -> legacy addressable coordinate -> NIP-33
    /// coordinate -> membership -> shared TTL -> channel row. Both coordinate
    /// keys are necessary: existing APIs encode UUID bytes versus d-tag bytes.
    pub async fn begin_channel_metadata_write(
        &self,
        community: CommunityId,
        channel_id: Uuid,
        relay: PublicKey,
    ) -> Result<ChannelMetadataWrite> {
        if channel_id.is_nil() {
            return Err(DbError::InvalidData("nil channel UUID".into()));
        }
        let mut tx = self.begin_event_write_transaction(community).await?;
        for coordinate in [
            channel_id.as_bytes().as_slice(),
            channel_id.to_string().as_bytes(),
        ] {
            let key = crate::replaceable::event_replacement_lock_key(
                community,
                39000,
                relay.as_bytes(),
                Some(coordinate),
            );
            crate::observability::observe_advisory_lock(
                crate::observability::LockType::Replacement,
                sqlx::query("SELECT pg_advisory_xact_lock($1)")
                    .bind(key)
                    .execute(tx.conn()),
            )
            .await?;
        }
        crate::channel_members::acquire_channel_membership_lock_in_transaction(&mut tx, channel_id)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
            .bind(crate::channel::channel_ttl_lock_key(community, channel_id))
            .execute(tx.conn())
            .await?;
        let (channel, labels) = read_channel(&mut tx, channel_id).await?;
        let previous = sqlx::query(
            "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
             FROM events WHERE community_id = $1 AND kind = 39000 AND pubkey = $2 \
             AND channel_id = $3 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community.as_uuid())
        .bind(relay.as_bytes().as_slice())
        .bind(channel_id)
        .fetch_optional(tx.conn())
        .await?
        .map(crate::event::row_to_stored_event)
        .transpose()?
        .flatten();
        Ok(ChannelMetadataWrite {
            tx,
            channel_id,
            relay,
            channel,
            labels,
            previous,
            published: None,
            command_applied: false,
            must_rollback: false,
        })
    }
}

async fn read_channel(tx: &mut AdmittedTx, id: Uuid) -> Result<(Option<ChannelRecord>, LabelSet)> {
    // Include deleted rows: a tombstone is not an available creation UUID.
    let row = sqlx::query(
        "SELECT *, channel_type::text AS channel_type, visibility::text AS visibility \
         FROM channels WHERE community_id = $1 AND id = $2 FOR NO KEY UPDATE",
    )
    .bind(tx.community().as_uuid())
    .bind(id)
    .fetch_optional(tx.conn())
    .await?;
    match row {
        Some(row) => {
            let values: Vec<String> = row.try_get("labels")?;
            let labels =
                LabelSet::new(values.clone()).map_err(|e| DbError::InvalidData(e.to_string()))?;
            if labels.values() != values {
                return Err(DbError::InvalidData("noncanonical stored labels".into()));
            }
            Ok((Some(crate::channel::row_to_channel_record(row)?), labels))
        }
        None => Ok((None, LabelSet::default())),
    }
}

impl ChannelMetadataWrite {
    /// Current labels captured under the channel lock.
    pub fn labels(&self) -> &LabelSet {
        &self.labels
    }

    /// Current live channel, or a generic not-found error for missing/deleted rows.
    pub fn channel(&self) -> Result<&ChannelRecord> {
        self.channel
            .as_ref()
            .filter(|ch| ch.deleted_at.is_none())
            .ok_or(DbError::ChannelNotFound(self.channel_id))
    }

    /// Roll back all command, channel and snapshot changes.
    pub async fn rollback(self) -> Result<()> {
        self.tx.rollback().await
    }

    /// Commit only after the current projection has been preserved or repaired.
    ///
    /// A commit error is an uncertain outcome to the caller, not proof of rejection.
    /// The returned new snapshot may be dispatched only after this method succeeds.
    pub async fn commit(mut self) -> Result<Option<StoredEvent>> {
        if self.must_rollback {
            return Err(DbError::InvalidData(
                "metadata transaction must roll back".into(),
            ));
        }
        if self.command_applied && self.published.is_none() && self.needs_snapshot().await? {
            return Err(DbError::InvalidData(
                "command has no authoritative metadata snapshot".into(),
            ));
        }
        self.tx.commit().await?;
        Ok(self.published)
    }
}
