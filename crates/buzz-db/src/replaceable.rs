//! Replaceable-event persistence and coordinate locking.

use buzz_core::{CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{Acquire, Postgres, Transaction};
use uuid::Uuid;

use crate::observability::{self, LockType, TransactionOperation};
use crate::{Db, DbError, Result};

/// Result category for a parameterized-replaceable event write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterizedReplaceStatus {
    /// The incoming event was inserted as the coordinate's live head.
    Inserted,
    /// The exact event was already accepted.
    Duplicate,
    /// A newer event, or lower-ID same-second event, already dominates it.
    Superseded,
    /// A requested current revision has no live coordinate head.
    RevisionMissing,
    /// The live coordinate head differs from the requested revision.
    RevisionMismatch,
    /// An exact replay was required, but the event is not the live head.
    ReplayOnlyMiss,
}

/// Structural precondition for a parameterized-replaceable write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterizedReplacePrecondition<'a> {
    /// Apply normal NIP-33 ordering without a revision precondition.
    Unconditional,
    /// Require the live head to match this validated event ID.
    ExpectedRevision(&'a [u8]),
    /// Accept only an exact live-head replay and perform no mutation otherwise.
    ExactReplayOnly,
}

/// Result of a transaction-bound parameterized-replaceable event write.
#[derive(Clone, Debug)]
pub struct ParameterizedReplaceResult {
    /// Stored representation of the submitted event.
    pub event: StoredEvent,
    /// Whether and why the coordinate accepted the event.
    pub status: ParameterizedReplaceStatus,
}

impl ParameterizedReplaceResult {
    fn new(
        event: &nostr::Event,
        received_at: DateTime<Utc>,
        channel_id: Option<Uuid>,
        status: ParameterizedReplaceStatus,
    ) -> Self {
        Self {
            event: StoredEvent::with_received_at(
                event.clone(),
                received_at,
                channel_id,
                status == ParameterizedReplaceStatus::Inserted,
            ),
            status,
        }
    }
}

/// Derive the transaction-scoped advisory-lock key for an event coordinate.
///
/// Hash collisions only add serialization; the SQL predicates still determine
/// which rows are read or changed.
pub(crate) fn event_replacement_lock_key(
    community_id: CommunityId,
    kind: i32,
    pubkey: &[u8],
    coordinate: Option<&[u8]>,
) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    let kind_bytes = kind.to_le_bytes();
    for bytes in [
        community_id.as_uuid().as_bytes().as_slice(),
        kind_bytes.as_slice(),
        pubkey,
    ] {
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    if let Some(coordinate) = coordinate {
        for byte in coordinate {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash as i64
}

/// Replace a parameterized event in a caller-owned transaction.
///
/// This function acquires a transaction-scoped advisory lock but never commits
/// or rolls back the outer transaction. The typed precondition can require the
/// current live head to have an exact event ID or restrict the operation to an
/// idempotent replay.
async fn replace_parameterized_event_in_transaction_impl(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &nostr::Event,
    d_tag: &str,
    channel_id: Option<Uuid>,
    precondition: ParameterizedReplacePrecondition<'_>,
) -> Result<ParameterizedReplaceResult> {
    let kind_i32 = buzz_core::kind::event_kind_i32(event);
    let pubkey_bytes = event.pubkey.to_bytes();
    let created_at_secs = event.created_at.as_secs() as i64;
    let created_at = DateTime::from_timestamp(created_at_secs, 0)
        .ok_or(DbError::InvalidTimestamp(created_at_secs))?;
    let received_at = Utc::now();

    let lock_key = event_replacement_lock_key(
        community_id,
        kind_i32,
        pubkey_bytes.as_slice(),
        Some(d_tag.as_bytes()),
    );
    observability::observe_advisory_lock(
        LockType::Replacement,
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut **tx),
    )
    .await?;

    let d_tag_count = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|part| part == "d"))
        .count();
    let has_exact_d_tag = event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.len() >= 2 && parts[0] == "d" && parts[1] == d_tag
    });
    let read_state_t_tag_count = event
        .tags
        .iter()
        .filter(|tag| {
            let parts = tag.as_slice();
            parts.len() == 2 && parts[0] == "t" && parts[1] == "read-state"
        })
        .count();
    let is_nip_rs = kind_i32 == buzz_core::kind::KIND_READ_STATE as i32
        && d_tag_count == 1
        && has_exact_d_tag
        && d_tag.strip_prefix("read-state:").is_some_and(|slot| {
            slot.len() == 32
                && slot
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        && read_state_t_tag_count == 1;
    let is_buzz_mesh_status = kind_i32 == buzz_core::kind::KIND_BOOKMARK_SET as i32
        && d_tag.starts_with("buzz-mesh-member-status:")
        && event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.len() == 2 && parts[0] == "k" && parts[1] == "buzz-mesh-status"
        });
    let hard_delete_superseded = is_nip_rs || is_buzz_mesh_status;

    let existing: Option<(DateTime<Utc>, Vec<u8>)> = sqlx::query_as(
        "SELECT created_at, id FROM events \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL \
         ORDER BY created_at DESC, id ASC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(kind_i32)
    .bind(pubkey_bytes.as_slice())
    .bind(d_tag)
    .fetch_optional(&mut **tx)
    .await?;
    let watermark: Option<(DateTime<Utc>, Vec<u8>)> = if is_nip_rs {
        sqlx::query_as(
            "SELECT created_at, event_id FROM parameterized_event_watermarks \
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4",
        )
        .bind(community_id.as_uuid())
        .bind(kind_i32)
        .bind(pubkey_bytes.as_slice())
        .bind(d_tag)
        .fetch_optional(&mut **tx)
        .await?
    } else {
        None
    };

    let incoming_id = event.id.as_bytes().as_slice();
    if existing
        .as_ref()
        .is_some_and(|(_, existing_id)| existing_id.as_slice() == incoming_id)
        || watermark
            .as_ref()
            .is_some_and(|(_, event_id)| event_id.as_slice() == incoming_id)
    {
        return Ok(ParameterizedReplaceResult::new(
            event,
            received_at,
            channel_id,
            ParameterizedReplaceStatus::Duplicate,
        ));
    }

    if precondition == ParameterizedReplacePrecondition::ExactReplayOnly {
        return Ok(ParameterizedReplaceResult::new(
            event,
            received_at,
            channel_id,
            ParameterizedReplaceStatus::ReplayOnlyMiss,
        ));
    }

    if let ParameterizedReplacePrecondition::ExpectedRevision(expected_revision) = precondition {
        let status = match existing.as_ref() {
            None => Some(ParameterizedReplaceStatus::RevisionMissing),
            Some((_, existing_id)) if existing_id.as_slice() != expected_revision => {
                Some(ParameterizedReplaceStatus::RevisionMismatch)
            }
            Some(_) => None,
        };
        if let Some(status) = status {
            return Ok(ParameterizedReplaceResult::new(
                event,
                received_at,
                channel_id,
                status,
            ));
        }
    }

    let dominated = existing
        .iter()
        .chain(watermark.iter())
        .any(|(accepted_ts, accepted_id)| {
            created_at < *accepted_ts
                || (created_at == *accepted_ts && incoming_id >= accepted_id.as_slice())
        });
    if dominated {
        return Ok(ParameterizedReplaceResult::new(
            event,
            received_at,
            channel_id,
            ParameterizedReplaceStatus::Superseded,
        ));
    }

    let mut savepoint = tx.begin().await?;
    if existing.is_some() {
        let previous_nip_rs_hard_delete: Option<String> = if is_nip_rs {
            sqlx::query_scalar(
                "SELECT NULLIF(current_setting('buzz.nip_rs_hard_delete', true), '')",
            )
            .fetch_one(&mut *savepoint)
            .await?
        } else {
            None
        };
        if is_nip_rs {
            sqlx::query("SELECT set_config('buzz.nip_rs_hard_delete', 'on', true)")
                .execute(&mut *savepoint)
                .await?;
        }
        let statement = if hard_delete_superseded {
            "DELETE FROM events \
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL"
        } else {
            "UPDATE events SET deleted_at = NOW() \
             WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL"
        };
        sqlx::query(statement)
            .bind(community_id.as_uuid())
            .bind(kind_i32)
            .bind(pubkey_bytes.as_slice())
            .bind(d_tag)
            .execute(&mut *savepoint)
            .await?;

        if is_nip_rs {
            let previous_value = previous_nip_rs_hard_delete.as_deref().unwrap_or_default();
            sqlx::query("SELECT set_config('buzz.nip_rs_hard_delete', $1, true)")
                .bind(previous_value)
                .execute(&mut *savepoint)
                .await?;
        }

        if hard_delete_superseded {
            if let Some((_, existing_id)) = &existing {
                sqlx::query("DELETE FROM event_mentions WHERE community_id = $1 AND event_id = $2")
                    .bind(community_id.as_uuid())
                    .bind(existing_id)
                    .execute(&mut *savepoint)
                    .await?;
            }
        }
    }

    let sig_bytes = event.sig.serialize();
    let tags_json = serde_json::to_value(&event.tags)?;
    let insert_result = sqlx::query(
        "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag, not_before) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
         ON CONFLICT DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(incoming_id)
    .bind(pubkey_bytes.as_slice())
    .bind(created_at)
    .bind(kind_i32)
    .bind(&tags_json)
    .bind(&event.content)
    .bind(sig_bytes.as_slice())
    .bind(received_at)
    .bind(channel_id)
    .bind(d_tag)
    .bind(crate::event::extract_not_before(event))
    .execute(&mut *savepoint)
    .await?;

    if insert_result.rows_affected() == 0 {
        savepoint.rollback().await?;
        return Ok(ParameterizedReplaceResult::new(
            event,
            received_at,
            channel_id,
            ParameterizedReplaceStatus::Duplicate,
        ));
    }

    if is_nip_rs {
        sqlx::query(
            "INSERT INTO parameterized_event_watermarks \
                 (community_id, kind, pubkey, d_tag, created_at, event_id) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (community_id, kind, pubkey, d_tag) DO UPDATE SET \
                 created_at = EXCLUDED.created_at, event_id = EXCLUDED.event_id",
        )
        .bind(community_id.as_uuid())
        .bind(kind_i32)
        .bind(pubkey_bytes.as_slice())
        .bind(d_tag)
        .bind(created_at)
        .bind(incoming_id)
        .execute(&mut *savepoint)
        .await?;
    }

    crate::insert_mentions_in_transaction(&mut savepoint, community_id, event, channel_id).await?;
    savepoint.commit().await?;

    Ok(ParameterizedReplaceResult::new(
        event,
        received_at,
        channel_id,
        ParameterizedReplaceStatus::Inserted,
    ))
}

impl Db {
    /// Replace a NIP-33 event inside a caller-owned transaction.
    ///
    /// The caller owns commit or rollback. Requiring [`Transaction`] here and
    /// in the internal state machine makes the advisory-lock contract explicit.
    pub async fn replace_parameterized_event_in_transaction(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community_id: CommunityId,
        event: &nostr::Event,
        d_tag: &str,
        channel_id: Option<Uuid>,
        precondition: ParameterizedReplacePrecondition<'_>,
    ) -> Result<ParameterizedReplaceResult> {
        replace_parameterized_event_in_transaction_impl(
            tx,
            community_id,
            event,
            d_tag,
            channel_id,
            precondition,
        )
        .await
    }

    /// Atomically replace a NIP-33 parameterized replaceable event.
    ///
    /// Replacement keys on `(kind, pubkey, d_tag)` across channels. The
    /// highest timestamp wins; same-second ties use the lowest event ID.
    #[datastore_span(name = "replace_parameterized_event", system = "postgresql")]
    pub async fn replace_parameterized_event(
        &self,
        community_id: CommunityId,
        event: &nostr::Event,
        d_tag: &str,
        channel_id: Option<Uuid>,
    ) -> Result<(StoredEvent, bool)> {
        let (mut tx, transaction_timer) = observability::begin_transaction(
            &self.pool,
            TransactionOperation::ReplaceParameterizedEvent,
        )
        .await?;
        transaction_timer
            .observe(async {
                let result = self
                    .replace_parameterized_event_in_transaction(
                        &mut tx,
                        community_id,
                        event,
                        d_tag,
                        channel_id,
                        ParameterizedReplacePrecondition::Unconditional,
                    )
                    .await?;
                let was_inserted = result.status == ParameterizedReplaceStatus::Inserted;
                if was_inserted {
                    tx.commit().await?;
                } else {
                    tx.rollback().await?;
                }
                Ok((result.event, was_inserted))
            })
            .await
    }
}
