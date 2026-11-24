//! The search outbox: the durable queue between event admission and the
//! search index.
//!
//! When [`enable`] has been called (the relay does so when it is configured
//! with an OpenSearch address), every admitted insert of a
//! [`SEARCHABLE_KINDS`] event writes a `search_outbox` row in the same
//! transaction, so a stored message always has its index change queued and a
//! rolled-back write never does. The write makes no call to the search
//! engine.
//!
//! The indexer re-reads `events` for every row it claims, so duplicate or
//! out-of-order rows for one event are harmless.

use std::sync::atomic::{AtomicBool, Ordering};

use buzz_core::{media::EDIT_GOVERNED_KINDS, CommunityId};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{AdmittedTx, Result};

/// Event kinds the search index holds; no other kind is ever queued. They
/// are the message kinds an edit can replace, so the list has one owner.
pub const SEARCHABLE_KINDS: [i32; EDIT_GOVERNED_KINDS.len()] = {
    let mut kinds = [0; EDIT_GOVERNED_KINDS.len()];
    let mut i = 0;
    while i < kinds.len() {
        kinds[i] = EDIT_GOVERNED_KINDS[i] as i32;
        i += 1;
    }
    kinds
};

/// Communities a claim may pass over because their deletion fenced them
/// between candidate selection and admission, or another worker holds all
/// their rows.
const CLAIM_COMMUNITY_CANDIDATES: usize = 8;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Start writing outbox rows on admission, for the life of the process.
///
/// Process-wide because the outbox follows every event insert path. Call it
/// once at startup, before serving writes, when a search index is configured.
/// Fails, leaving the outbox off, if migrations have not created the table:
/// enabled without it, every searchable insert would fail.
pub async fn enable(pool: &PgPool) -> Result<()> {
    let exists: bool = sqlx::query_scalar("SELECT to_regclass('public.search_outbox') IS NOT NULL")
        .fetch_one(pool)
        .await?;
    if !exists {
        return Err(crate::DbError::InvalidData(
            "the search_outbox table is missing; run migrations before enabling search indexing"
                .into(),
        ));
    }
    ENABLED.store(true, Ordering::Relaxed);
    Ok(())
}

/// Queue `event_id` for indexing in the admitted transaction that inserted it.
pub(crate) async fn enqueue_in_transaction(
    tx: &mut AdmittedTx,
    event_id: &[u8],
    created_at: DateTime<Utc>,
    kind: i32,
) -> Result<()> {
    if !ENABLED.load(Ordering::Relaxed) || !SEARCHABLE_KINDS.contains(&kind) {
        return Ok(());
    }
    let community = tx.community();
    sqlx::query(
        "INSERT INTO search_outbox (community_id, event_id, event_created_at) VALUES ($1, $2, $3)",
    )
    .bind(community.as_uuid())
    .bind(event_id)
    .bind(created_at)
    .execute(tx.conn())
    .await?;
    Ok(())
}

/// The current state of a queued event, as the index should hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchDocument {
    /// Event id.
    pub event_id: [u8; 32],
    /// Event kind.
    pub kind: i32,
    /// Author public key.
    pub pubkey: [u8; 32],
    /// Channel, for channel-scoped events.
    pub channel_id: Option<Uuid>,
    /// Signed creation time.
    pub created_at: DateTime<Utc>,
    /// Message text.
    pub content: String,
    /// The channel's type (`stream`, `forum`, `dm`, ...), when it has one.
    pub channel_type: Option<String>,
    /// Whether the author is an agent with an owner.
    pub author_is_agent: bool,
    /// Root of the thread the event replies in, if it is a reply.
    pub thread_root: Option<[u8; 32]>,
    /// Hex public keys the event mentions.
    pub mentions: Vec<String>,
}

/// Rows claimed for one community, with the current state of their events.
///
/// Holds an admitted transaction, so the community cannot be fenced for
/// deletion while the batch is being indexed. Dropping the batch without
/// [`SearchOutboxBatch::complete`] rolls back and leaves the rows queued.
#[must_use = "dropping a batch leaves its rows queued; call complete()"]
pub struct SearchOutboxBatch {
    tx: AdmittedTx,
    row_ids: Vec<Uuid>,
    /// Events to write to the index.
    pub documents: Vec<SearchDocument>,
    /// Queued events that are gone, deleted or not searchable: their
    /// documents must be removed from the index.
    pub removed: Vec<[u8; 32]>,
}

impl SearchOutboxBatch {
    /// The community every row in the batch belongs to.
    pub fn community(&self) -> CommunityId {
        self.tx.community()
    }

    /// Delete the claimed rows and commit. Call once the index holds the
    /// batch.
    pub async fn complete(mut self) -> Result<()> {
        let community = self.tx.community();
        sqlx::query("DELETE FROM search_outbox WHERE community_id = $1 AND id = ANY($2)")
            .bind(community.as_uuid())
            .bind(&self.row_ids)
            .execute(self.tx.conn())
            .await?;
        self.tx.commit().await
    }
}

/// Claim up to `limit` of the oldest queued rows of one community not in
/// `skip`, skipping rows another worker holds, and read their events' current
/// state.
///
/// Returns `None` when no other active community has unclaimed rows.
pub async fn claim_batch(
    pool: &PgPool,
    limit: i64,
    skip: &[CommunityId],
) -> Result<Option<SearchOutboxBatch>> {
    let mut refused: Vec<Uuid> = skip.iter().map(|community| *community.as_uuid()).collect();
    for _ in 0..CLAIM_COMMUNITY_CANDIDATES {
        // Exclude fenced communities so they cannot occupy the queue head.
        let candidate: Option<Uuid> = sqlx::query_scalar(
            "SELECT o.community_id FROM search_outbox o \
             JOIN communities c ON c.id = o.community_id \
             WHERE c.deletion_state = 'active' AND c.deleted_at IS NULL \
               AND o.community_id <> ALL($1) \
             ORDER BY o.created_at LIMIT 1",
        )
        .bind(&refused)
        .fetch_optional(pool)
        .await?;
        let Some(id) = candidate else {
            return Ok(None);
        };
        let community = CommunityId::from_uuid(id);
        let Some(mut tx) = crate::begin_community_sweep_transaction(
            pool,
            community,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?
        else {
            refused.push(id);
            continue;
        };
        let rows = sqlx::query(
            "SELECT id, event_id, event_created_at FROM search_outbox \
             WHERE community_id = $1 \
             ORDER BY created_at \
             FOR UPDATE SKIP LOCKED \
             LIMIT $2",
        )
        .bind(community.as_uuid())
        .bind(limit)
        .fetch_all(tx.conn())
        .await?;
        if rows.is_empty() {
            // Another worker holds every row of this community.
            refused.push(id);
            continue;
        }
        let mut row_ids = Vec::with_capacity(rows.len());
        let mut queued = std::collections::BTreeMap::<[u8; 32], DateTime<Utc>>::new();
        for row in &rows {
            row_ids.push(row.try_get("id")?);
            queued.insert(
                bytes32(row.try_get("event_id")?)?,
                row.try_get("event_created_at")?,
            );
        }
        let documents = read_documents(&mut tx, &queued).await?;
        for document in &documents {
            queued.remove(&document.event_id);
        }
        let removed = queued.into_keys().collect();
        return Ok(Some(SearchOutboxBatch {
            tx,
            row_ids,
            documents,
            removed,
        }));
    }
    Ok(None)
}

/// Read the current state of each live, searchable queued event.
async fn read_documents(
    tx: &mut AdmittedTx,
    queued: &std::collections::BTreeMap<[u8; 32], DateTime<Utc>>,
) -> Result<Vec<SearchDocument>> {
    let community = tx.community();
    let ids: Vec<&[u8]> = queued.keys().map(|id| id.as_slice()).collect();
    let created: Vec<DateTime<Utc>> = queued.values().copied().collect();
    // The (created_at, id) pair is the events key, so each lookup prunes to
    // one partition.
    let rows = sqlx::query(
        "SELECT e.id, e.kind, e.pubkey, e.channel_id, e.created_at, e.content, \
                c.channel_type::text AS channel_type, \
                (u.agent_owner_pubkey IS NOT NULL) AS author_is_agent, \
                t.root_event_id, \
                ARRAY(SELECT m.pubkey_hex::text FROM event_mentions m \
                      WHERE m.community_id = e.community_id AND m.event_id = e.id \
                      ORDER BY m.pubkey_hex) AS mentions \
         FROM unnest($2::bytea[], $3::timestamptz[]) AS k(id, created_at) \
         JOIN events e \
           ON e.community_id = $1 AND e.created_at = k.created_at AND e.id = k.id \
         LEFT JOIN channels c ON c.community_id = e.community_id AND c.id = e.channel_id \
         LEFT JOIN users u ON u.community_id = e.community_id AND u.pubkey = e.pubkey \
         LEFT JOIN thread_metadata t \
           ON t.community_id = e.community_id \
          AND t.event_created_at = e.created_at AND t.event_id = e.id \
         WHERE e.deleted_at IS NULL AND e.kind = ANY($4)",
    )
    .bind(community.as_uuid())
    .bind(&ids)
    .bind(&created)
    .bind(SEARCHABLE_KINDS.as_slice())
    .fetch_all(tx.conn())
    .await?;
    let mut documents = Vec::with_capacity(rows.len());
    for row in rows {
        documents.push(SearchDocument {
            event_id: bytes32(row.try_get("id")?)?,
            kind: row.try_get("kind")?,
            pubkey: bytes32(row.try_get("pubkey")?)?,
            channel_id: row.try_get("channel_id")?,
            created_at: row.try_get("created_at")?,
            content: row.try_get("content")?,
            channel_type: row.try_get("channel_type")?,
            author_is_agent: row.try_get("author_is_agent")?,
            thread_root: row
                .try_get::<Option<Vec<u8>>, _>("root_event_id")?
                .map(bytes32)
                .transpose()?,
            mentions: row.try_get("mentions")?,
        });
    }
    Ok(documents)
}

/// Queued rows, for lag checks and tests.
pub async fn depth(pool: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM search_outbox")
        .fetch_one(pool)
        .await?)
}

fn bytes32(bytes: Vec<u8>) -> Result<[u8; 32]> {
    <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| crate::DbError::InvalidData(format!("expected 32 bytes, got {}", bytes.len())))
}
