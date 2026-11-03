//! Deletion requests (kind 5 and kind 9005) that store the request and apply
//! it in one transaction.
//!
//! The request's write gates come first (see [`crate::lifecycle`]), so a
//! request into an archived channel stores nothing and deletes nothing. A
//! refusal decided under the locks rolls back the whole request.

use std::collections::BTreeSet;

use buzz_core::{CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use nostr::Event;
use uuid::Uuid;

use crate::event::ThreadMetadataParams;
use crate::{AdmittedTx, ArchiveGate, Db, DbError, Result};

/// The key of a reaction row: its target event, the target's `created_at`,
/// the reacting pubkey and the emoji.
#[derive(Clone, Copy, Debug)]
pub struct ReactionRowKey<'a> {
    /// The reacted-to event id.
    pub event_id: &'a [u8],
    /// The reacted-to event's `created_at`.
    pub event_created_at: DateTime<Utc>,
    /// The reacting pubkey.
    pub pubkey: &'a [u8],
    /// The reaction emoji.
    pub emoji: &'a str,
}

/// The event a deletion request removes.
#[derive(Clone, Copy, Debug)]
pub struct DeletionTarget<'a> {
    /// The target event id.
    pub event_id: &'a [u8],
    /// Delete the target only if it lives in this channel. A target in any
    /// other channel leaves the request stored with no effect (kind 9005).
    pub required_channel: Option<Uuid>,
    /// Remove a kind:7 target's reaction row too (kind 5): the row that
    /// carries the target's id, else the row with this key.
    pub reaction: Option<Option<ReactionRowKey<'a>>>,
}

/// The result of [`Db::insert_event_deletion`].
#[derive(Debug)]
pub struct EventDeletionWrite {
    /// The stored deletion request.
    pub stored: StoredEvent,
    /// `false` when the request was already stored. Nothing changed then.
    pub inserted: bool,
    /// Whether this request deleted the target.
    pub deleted: bool,
    /// The target's channel.
    pub target_channel: Option<Uuid>,
    /// The target's thread root, whose counters the delete changed.
    pub root_event_id: Option<Vec<u8>>,
}

/// The result of [`Db::insert_coordinate_deletion`].
#[derive(Debug)]
pub struct CoordinateDeletionWrite {
    /// The stored deletion request.
    pub stored: StoredEvent,
    /// `false` when the request was already stored. Nothing changed then.
    pub inserted: bool,
    /// Whether this request deleted a version of the coordinate.
    pub deleted: bool,
}

impl Db {
    /// Store the deletion request `event` in `channel_id` and soft-delete its
    /// target `target` in one transaction: the thread counters, the artifact
    /// head and (for kind 5) the reaction row change with it. With no
    /// `target` only the request is stored.
    ///
    /// `gate` decides whether an archived channel refuses the request (see
    /// [`ArchiveGate`]): `channel_id` or the channel of a target it deletes.
    /// The target's channel never changes, so it is read before the gates. A
    /// target that is missing, already deleted, or outside
    /// `required_channel` leaves the request stored with no effect.
    #[datastore_span(name = "insert_event_deletion", system = "postgresql")]
    pub async fn insert_event_deletion(
        &self,
        community_id: CommunityId,
        event: &Event,
        channel_id: Option<Uuid>,
        thread_meta: Option<ThreadMetadataParams<'_>>,
        target: Option<DeletionTarget<'_>>,
        gate: ArchiveGate,
    ) -> Result<EventDeletionWrite> {
        let mut tx = self.begin_event_write_transaction(community_id).await?;
        let target_row = match target {
            Some(target) => target_row(&mut tx, target.event_id).await?,
            None => None,
        };
        // A target outside `required_channel` is not deleted, so its channel
        // is not gated: a refusal there would reveal where the target lives.
        let target_channel = target_row.and_then(|(_, channel)| channel);
        let deletes_target = target.is_some_and(|target| {
            target
                .required_channel
                .is_none_or(|required| target_channel == Some(required))
        });
        let mut gated: BTreeSet<Uuid> = channel_id.into_iter().collect();
        if deletes_target {
            gated.extend(target_channel);
        }
        let gated: Vec<Uuid> = gated.into_iter().collect();
        crate::lifecycle::lock_write_gates(&mut tx, gate, &gated).await?;

        let (stored, inserted) = insert_request(&mut tx, event, channel_id, thread_meta).await?;
        let mut write = EventDeletionWrite {
            stored,
            inserted,
            deleted: false,
            target_channel,
            root_event_id: None,
        };
        if !inserted {
            tx.rollback().await?;
            return Ok(write);
        }
        let (Some(target), Some((kind, _)), true) = (target, target_row, deletes_target) else {
            tx.commit().await?;
            return Ok(write);
        };

        let (parent, root) = thread_links(&mut tx, target.event_id).await?;
        write.deleted = crate::event::soft_delete_event_and_update_thread_in_tx(
            &mut tx,
            target.event_id,
            parent.as_deref(),
            root.as_deref(),
        )
        .await?;
        if write.deleted {
            write.root_event_id = root;
            match target.reaction {
                Some(fallback) if kind == buzz_core::kind::KIND_REACTION as i32 => {
                    remove_reaction_row(&mut tx, target.event_id, fallback).await?;
                }
                _ => {}
            }
        }
        tx.commit().await?;
        Ok(write)
    }

    /// Store the NIP-09 deletion request `event` for the parameterized
    /// coordinate `(kind, pubkey, d_tag)` and soft-delete every version of it
    /// created at or before the request, in one transaction.
    ///
    /// `gate` decides whether an archived channel refuses the request (see
    /// [`ArchiveGate`]): `channel_id` or the channel of any version it would
    /// delete. The versions are read before the gates and again under the
    /// coordinate lock; if one moved to an ungated channel the request is
    /// retried once, then fails with [`DbError::LifecycleConflict`]. A
    /// version is never skipped.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "insert_coordinate_deletion", system = "postgresql")]
    pub async fn insert_coordinate_deletion(
        &self,
        community_id: CommunityId,
        event: &Event,
        channel_id: Option<Uuid>,
        thread_meta: Option<ThreadMetadataParams<'_>>,
        kind: i32,
        pubkey: &[u8],
        d_tag: &str,
        gate: ArchiveGate,
    ) -> Result<CoordinateDeletionWrite> {
        let coordinate = Coordinate {
            kind,
            pubkey,
            d_tag,
        };
        match self
            .insert_coordinate_deletion_once(
                community_id,
                event,
                channel_id,
                thread_meta,
                coordinate,
                gate,
            )
            .await
        {
            Err(DbError::LifecycleConflict) => {
                self.insert_coordinate_deletion_once(
                    community_id,
                    event,
                    channel_id,
                    thread_meta,
                    coordinate,
                    gate,
                )
                .await
            }
            result => result,
        }
    }

    async fn insert_coordinate_deletion_once(
        &self,
        community_id: CommunityId,
        event: &Event,
        channel_id: Option<Uuid>,
        thread_meta: Option<ThreadMetadataParams<'_>>,
        coordinate: Coordinate<'_>,
        gate: ArchiveGate,
    ) -> Result<CoordinateDeletionWrite> {
        let created_at_secs = event.created_at.as_secs() as i64;
        let cutoff = DateTime::from_timestamp(created_at_secs, 0)
            .ok_or(DbError::InvalidTimestamp(created_at_secs))?;
        let Coordinate {
            kind,
            pubkey,
            d_tag,
        } = coordinate;
        let mut tx = self.begin_event_write_transaction(community_id).await?;
        let gates = crate::replaceable::lock_coordinate_write_gates(
            &mut tx,
            gate,
            kind,
            pubkey,
            d_tag,
            Some(cutoff),
            channel_id.as_slice(),
        )
        .await?;

        let lock_key = crate::replaceable::event_replacement_lock_key(
            community_id,
            kind,
            pubkey,
            Some(d_tag.as_bytes()),
        );
        crate::observability::observe_advisory_lock(
            crate::observability::LockType::Replacement,
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(lock_key)
                .execute(tx.conn()),
        )
        .await?;
        if !gates.is_exempt() {
            let channels = crate::replaceable::live_version_channels(
                &mut tx,
                kind,
                pubkey,
                d_tag,
                Some(cutoff),
            )
            .await?;
            if !channels.into_iter().all(|channel| gates.covers(channel)) {
                return Err(DbError::LifecycleConflict);
            }
        }

        let (stored, inserted) = insert_request(&mut tx, event, channel_id, thread_meta).await?;
        if !inserted {
            tx.rollback().await?;
            return Ok(CoordinateDeletionWrite {
                stored,
                inserted,
                deleted: false,
            });
        }
        let deleted =
            crate::event::soft_delete_by_coordinate_in_tx(&mut tx, kind, pubkey, d_tag, cutoff)
                .await?;
        tx.commit().await?;
        Ok(CoordinateDeletionWrite {
            stored,
            inserted,
            deleted,
        })
    }
}

/// A parameterized-replaceable coordinate.
#[derive(Clone, Copy)]
struct Coordinate<'a> {
    kind: i32,
    pubkey: &'a [u8],
    d_tag: &'a str,
}

/// Store the request as `Db::insert_event_with_thread_metadata` does.
async fn insert_request(
    tx: &mut AdmittedTx,
    event: &Event,
    channel_id: Option<Uuid>,
    thread_meta: Option<ThreadMetadataParams<'_>>,
) -> Result<(StoredEvent, bool)> {
    let (stored, inserted) =
        crate::event::insert_event_with_thread_metadata_tx(tx, event, channel_id, thread_meta)
            .await?;
    if inserted {
        crate::insert_mentions_in_transaction(tx, event, channel_id).await?;
    }
    Ok((stored, inserted))
}

/// The target's kind and channel, deleted or not.
async fn target_row(tx: &mut AdmittedTx, event_id: &[u8]) -> Result<Option<(i32, Option<Uuid>)>> {
    Ok(sqlx::query_as(
        "SELECT kind, channel_id FROM events WHERE community_id = $1 AND id = $2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(tx.community().as_uuid())
    .bind(event_id)
    .fetch_optional(tx.conn())
    .await?)
}

/// A thread parent and root event id.
type ThreadLinks = (Option<Vec<u8>>, Option<Vec<u8>>);

/// The target's thread parent and root, whose counters a delete lowers.
async fn thread_links(tx: &mut AdmittedTx, event_id: &[u8]) -> Result<ThreadLinks> {
    let links: Option<ThreadLinks> = sqlx::query_as(
        "SELECT parent_event_id, root_event_id FROM thread_metadata \
         WHERE community_id = $1 AND event_id = $2 LIMIT 1",
    )
    .bind(tx.community().as_uuid())
    .bind(event_id)
    .fetch_optional(tx.conn())
    .await?;
    Ok(links.unwrap_or_default())
}

/// Remove the reaction row of the deleted kind:7 `reaction_event_id`: the row
/// that carries its id, else the row with `fallback`'s key.
async fn remove_reaction_row(
    tx: &mut AdmittedTx,
    reaction_event_id: &[u8],
    fallback: Option<ReactionRowKey<'_>>,
) -> Result<()> {
    if crate::reaction::remove_reaction_by_source_event_id_tx(tx, reaction_event_id).await? {
        return Ok(());
    }
    if let Some(key) = fallback {
        crate::reaction::remove_reaction_tx(
            tx,
            key.event_id,
            key.event_created_at,
            key.pubkey,
            key.emoji,
        )
        .await?;
    }
    Ok(())
}
