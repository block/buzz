//! Atomic event deletion, retention-free purging, and thread accounting.

use buzz_core::{kind::KIND_CANVAS, CommunityId};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{DbError, Result},
    AdmittedTx,
};

/// SQL predicate over an `events` row for coordinates that carry no
/// historical value: NIP-RS read state and Buzz mesh heartbeats. Their
/// deletion is physical, including the mention index, so soft-deleted payloads
/// never accumulate. This is exactly the classification used by the migration
/// 0011 / 0019 purge triggers this path replaces.
macro_rules! retention_free_event_predicate {
    () => {
        "((kind = 30078 \
           AND d_tag ~ '^read-state:[0-9a-f]{32}$' \
           AND (SELECT count(*) \
                FROM jsonb_array_elements(CASE WHEN jsonb_typeof(tags) = 'array' THEN tags ELSE '[]'::jsonb END) tag \
                WHERE jsonb_typeof(tag) = 'array' AND tag->0 = '\"d\"'::jsonb) = 1 \
           AND EXISTS (SELECT 1 \
                FROM jsonb_array_elements(CASE WHEN jsonb_typeof(tags) = 'array' THEN tags ELSE '[]'::jsonb END) tag \
                WHERE jsonb_typeof(tag) = 'array' AND jsonb_array_length(tag) >= 2 \
                  AND jsonb_typeof(tag->1) = 'string' AND tag->>0 = 'd' AND tag->>1 = d_tag) \
           AND (SELECT count(*) \
                FROM jsonb_array_elements(CASE WHEN jsonb_typeof(tags) = 'array' THEN tags ELSE '[]'::jsonb END) tag \
                WHERE tag = '[\"t\", \"read-state\"]'::jsonb) = 1) \
          OR (kind = 30003 \
              AND d_tag LIKE 'buzz-mesh-member-status:%' \
              AND tags @> '[[\"k\", \"buzz-mesh-status\"]]'::jsonb))"
    };
}

/// Kinds that can hold a retention-free coordinate. Cheap Rust gate so
/// ordinary deletions skip the purge statement entirely.
fn may_be_retention_free(kind: i32) -> bool {
    kind == 30078 || kind == 30003
}

/// Soft-delete the live row for an addressable coordinate
/// `(kind, pubkey, d_tag)` — the NIP-33 replacement key — provided it is not
/// newer than the deletion request.
///
/// Used by `handle_a_tag_deletion` to honour NIP-09 a-tag deletions for any
/// parameterized-replaceable kind. The WHERE clause mirrors
/// `replace_parameterized_event` so the coordinate semantics stay consistent:
/// `channel_id` is intentionally NOT in the key (NIP-33 replacement is global
/// per the spec — `channel_id` is stored for query scoping, not identity).
///
/// `deletion_created_at_secs` is the deletion event's own `created_at`. NIP-09
/// scopes an `a`-tag deletion to versions at or before that instant, so a
/// delayed or replayed tombstone signed between two versions must not erase the
/// newer replacement. `events.created_at` is immutable per row, so the predicate
/// guarantees a tombstone can never erase a version newer than itself — the UPDATE
/// re-evaluates its WHERE clause after any lock wait, so a replacement that races
/// the deletion and lands with a later `created_at` is always spared.
///
/// This does NOT guarantee deletion completeness when a same-coordinate
/// replacement races the deletion: the deletion may evaluate its predicate before
/// the replacement arrives, miss the incoming head, and return `Ok(false)`. That
/// outcome is state-identical to the deletion having arrived first (old head
/// gone, new head present), which is a valid Nostr ordering — Nostr never fixes
/// the order of concurrent writes from different signers, and even same-signer
/// ordering is advisory. The return value feeds only a debug log, not a
/// correctness gate.
///
/// Returns `Ok(true)` if a row was deleted, `Ok(false)` if no live row matched
/// (already deleted, never existed, or strictly newer than the deletion).
pub async fn soft_delete_by_coordinate(
    pool: &PgPool,
    community_id: CommunityId,
    kind: i32,
    pubkey: &[u8],
    d_tag: &str,
    deletion_created_at_secs: i64,
) -> Result<bool> {
    let deletion_created_at = DateTime::from_timestamp(deletion_created_at_secs, 0)
        .ok_or(DbError::InvalidTimestamp(deletion_created_at_secs))?;
    let mut tx = crate::begin_community_event_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let purged = if may_be_retention_free(kind) {
        purge_retention_free_events(
            &mut tx,
            RetentionFreeTarget::Coordinate {
                kind,
                pubkey,
                d_tag,
                created_at_or_before: deletion_created_at,
            },
        )
        .await?
    } else {
        0
    };
    // `AND NOT COALESCE(<predicate>, false)` holds the invariant in SQL rather than through
    // statement order: under READ COMMITTED this UPDATE takes a fresh snapshot,
    // so a retention-free head committed by a racing replacement after the
    // purge ran is spared instead of soft-deleted (the "deletion arrived
    // first" outcome documented above). COALESCE keeps a NULL predicate
    // (e.g. NULL `d_tag`) on the ordinary soft-delete path, as the triggers do.
    let result = sqlx::query(concat!(
        "UPDATE events SET deleted_at = NOW() \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL \
         AND created_at <= $5 AND NOT COALESCE(",
        retention_free_event_predicate!(),
        ", false)"
    ))
    .bind(community_id.as_uuid())
    .bind(kind)
    .bind(pubkey)
    .bind(d_tag)
    .bind(deletion_created_at)
    .execute(tx.conn())
    .await?;

    tx.commit().await?;

    Ok(purged > 0 || result.rows_affected() > 0)
}

/// Purge live retention-free rows matching `$target`, plus their mention rows,
/// and return the number of events removed.
macro_rules! purge_retention_free_sql {
    ($target:literal) => {
        concat!(
            "WITH purged AS ( \
                 DELETE FROM events \
                 WHERE community_id = $1 AND ",
            $target,
            " AND deleted_at IS NULL AND ",
            retention_free_event_predicate!(),
            " RETURNING id \
             ), purged_mentions AS ( \
                 DELETE FROM event_mentions mention USING purged \
                 WHERE mention.community_id = $1 AND mention.event_id = purged.id \
             ) \
             SELECT count(*) FROM purged"
        )
    };
}

// The known-target statements below are executed in production and referenced
// directly by the pruning tests. They run with `.persistent(false)`: a cached
// prepared statement may switch to a generic plan after five executions, and on
// PostgreSQL 16 a generic plan takes `AccessShareLock` on every `events`
// partition (and its indexes) during plan validation, before runtime pruning.
// An unnamed statement is always planned with its bound parameters, so only the
// partition selected by `created_at` is locked.
const SELECT_KNOWN_EVENT_SQL: &str = "SELECT kind, channel_id, created_at FROM events \
     WHERE community_id = $1 AND id = $2 AND created_at = $3";
const SOFT_DELETE_KNOWN_EVENT_SQL: &str = concat!(
    "UPDATE events SET deleted_at = NOW() \
     WHERE community_id = $1 AND id = $2 AND created_at = $3 \
     AND deleted_at IS NULL AND NOT COALESCE(",
    retention_free_event_predicate!(),
    ", false)"
);
const PURGE_KNOWN_EVENT_SQL: &str = purge_retention_free_sql!("id = $2 AND created_at = $3");

/// Which live rows a deletion targets.
enum RetentionFreeTarget<'a> {
    Event {
        event_id: &'a [u8],
        created_at: DateTime<Utc>,
    },
    Coordinate {
        kind: i32,
        pubkey: &'a [u8],
        d_tag: &'a str,
        created_at_or_before: DateTime<Utc>,
    },
}

/// Physically delete live retention-free rows matched by `target`, plus their
/// mention rows, in the caller's transaction. The generic soft delete that
/// follows excludes the same predicate, so a retention-free row is either
/// purged here or left live, never soft-deleted. Returns the number of events
/// removed.
async fn purge_retention_free_events(
    tx: &mut AdmittedTx,
    target: RetentionFreeTarget<'_>,
) -> Result<u64> {
    let community_id = tx.community();
    // Migration 0011 fences NIP-RS hard deletes behind a transaction-local
    // opt-in. The fence and this opt-in are removed together once the
    // migration-only triggers are dropped.
    sqlx::query("SELECT set_config('buzz.nip_rs_hard_delete', 'on', true)")
        .execute(tx.conn())
        .await?;

    let purged: i64 = match target {
        RetentionFreeTarget::Event {
            event_id,
            created_at,
        } => {
            sqlx::query_scalar(PURGE_KNOWN_EVENT_SQL)
                .persistent(false)
                .bind(community_id.as_uuid())
                .bind(event_id)
                .bind(created_at)
                .fetch_one(tx.conn())
                .await?
        }
        RetentionFreeTarget::Coordinate {
            kind,
            pubkey,
            d_tag,
            created_at_or_before,
        } => {
            sqlx::query_scalar(purge_retention_free_sql!(
                "kind = $2 AND pubkey = $3 AND d_tag = $4 AND created_at <= $5"
            ))
            .bind(community_id.as_uuid())
            .bind(kind)
            .bind(pubkey)
            .bind(d_tag)
            .bind(created_at_or_before)
            .fetch_one(tx.conn())
            .await?
        }
    };

    Ok(purged as u64)
}

/// An event a NIP-09 (kind 5) or NIP-29 (kind 9005) deletion soft-deletes in
/// the deletion's own insert transaction, with the thread it belongs to.
#[derive(Clone, Copy, Debug)]
pub struct DeletionTarget<'a> {
    /// The event to soft-delete.
    pub event_id: &'a [u8],
    /// The target's exact signed timestamp, not the deletion's timestamp.
    pub created_at: DateTime<Utc>,
    /// Its direct parent, whose `reply_count` drops when it is deleted.
    pub parent_event_id: Option<&'a [u8]>,
    /// Its thread root, whose `descendant_count` drops when it is deleted.
    pub root_event_id: Option<&'a [u8]>,
}

/// Atomically soft-delete an event and decrement thread reply counters.
///
/// Wraps the delete + counter update in a single transaction so a crash between
/// them cannot leave counters permanently inflated. Returns `Ok(true)` if the
/// event was deleted this call.
///
/// When the target event is a kind-40100 (canvas) event, this function derives
/// the target's `kind` and `channel_id` from the database inside the same
/// transaction and acquires the same `(community, kind, channel)` advisory lock
/// used by [`super::event::insert_canvas_head_checked`] before the UPDATE. This prevents a
/// concurrent tagged write from observing a head that is simultaneously being
/// removed. The serialization invariant is owned entirely by this function;
/// callers do not classify the target kind.
pub async fn soft_delete_event_and_update_thread(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
    parent_event_id: Option<&[u8]>,
    root_event_id: Option<&[u8]>,
) -> Result<bool> {
    let mut tx = crate::begin_community_event_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let deleted = soft_delete_event_and_update_thread_in_tx(
        &mut tx,
        event_id,
        None,
        parent_event_id,
        root_event_id,
    )
    .await?;
    tx.commit().await?;
    Ok(deleted)
}

/// Transaction-scoped body of [`soft_delete_event_and_update_thread`].
///
/// Callers that must fence the delete with their own writes (e.g. the admin
/// action lease/marker) run this inside their transaction; the caller commits.
/// `None` discovers the target timestamp once; `Some` never falls back to ID-only.
pub(crate) async fn soft_delete_event_and_update_thread_in_tx(
    tx: &mut AdmittedTx,
    event_id: &[u8],
    created_at: Option<DateTime<Utc>>,
    parent_event_id: Option<&[u8]>,
    root_event_id: Option<&[u8]>,
) -> Result<bool> {
    let community_id = tx.community();
    use crate::store::replaceable::event_replacement_lock_key;

    // ID-only compatibility callers discover the immutable timestamp once.
    // Planned targets already have it; every subsequent events-table access
    // uses the exact partition key. Kind/channel remain canonical DB reads,
    // never caller-supplied classification (including for deleted rows).
    let target: Option<(i32, Option<Uuid>, DateTime<Utc>)> = match created_at {
        Some(created_at) => {
            sqlx::query_as(SELECT_KNOWN_EVENT_SQL)
                .persistent(false)
                .bind(community_id.as_uuid())
                .bind(event_id)
                .bind(created_at)
                .fetch_optional(tx.conn())
                .await?
        }
        None => {
            sqlx::query_as(
                "SELECT kind, channel_id, created_at FROM events \
             WHERE community_id = $1 AND id = $2",
            )
            .bind(community_id.as_uuid())
            .bind(event_id)
            .fetch_optional(tx.conn())
            .await?
        }
    };
    let Some((kind, channel_id, created_at)) = target else {
        return Ok(false);
    };

    // Relay-signed move removals are the source channel's only replay record.
    if kind == 45011 {
        return Err(DbError::InvalidData(
            "artifact removal markers cannot be deleted".into(),
        ));
    }

    if let Some(channel_id) = channel_id {
        if kind == KIND_CANVAS as i32 {
            let lock_key = event_replacement_lock_key(
                community_id,
                kind,
                &[],
                Some(channel_id.as_bytes().as_slice()),
            );
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(lock_key)
                .execute(tx.conn())
                .await?;
        }
    }

    let purged = if may_be_retention_free(kind) {
        purge_retention_free_events(
            tx,
            RetentionFreeTarget::Event {
                event_id,
                created_at,
            },
        )
        .await?
    } else {
        0
    };
    let result = sqlx::query(SOFT_DELETE_KNOWN_EVENT_SQL)
        .persistent(false)
        .bind(community_id.as_uuid())
        .bind(event_id)
        .bind(created_at)
        .execute(tx.conn())
        .await?;

    let deleted = purged > 0 || result.rows_affected() > 0;

    if deleted {
        sqlx::query("UPDATE artifact_heads SET modified_at=clock_timestamp() WHERE community_id=$1 AND artifact_id=(SELECT artifact_id FROM artifact_revisions WHERE community_id=$1 AND event_id=$2) AND event_id=$2")
            .bind(community_id.as_uuid()).bind(event_id).execute(tx.conn()).await?;
        if let Some(pid) = parent_event_id {
            sqlx::query(
                "UPDATE thread_metadata \
                 SET reply_count = GREATEST(reply_count - 1, 0) \
                 WHERE community_id = $1 AND event_id = $2",
            )
            .bind(community_id.as_uuid())
            .bind(pid)
            .execute(tx.conn())
            .await?;

            if let Some(root_id) = root_event_id {
                sqlx::query(
                    "UPDATE thread_metadata \
                     SET descendant_count = GREATEST(descendant_count - 1, 0) \
                     WHERE community_id = $1 AND event_id = $2",
                )
                .bind(community_id.as_uuid())
                .bind(root_id)
                .execute(tx.conn())
                .await?;
            }
        }
    }

    Ok(deleted)
}

#[cfg(test)]
mod postgres_tests;
