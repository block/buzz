//! Reaction persistence.
//!
//! One reaction per user per emoji per event. Soft-delete via removed_at.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{
    error::Result,
    event::{insert_event_with_thread_metadata_tx, ThreadMetadataParams},
    Db,
};
use buzz_core::{CommunityId, StoredEvent};

// -- Public structs -----------------------------------------------------------

/// Result of atomically inserting a kind:7 reaction event and its reaction row.
#[derive(Debug)]
pub enum ReactionEventInsertOutcome {
    /// Target event was absent in this community, or was soft-deleted. No writes committed.
    TargetMissing,
    /// The active `(target, actor, emoji)` reaction already exists. No event was stored.
    Duplicate,
    /// Reaction row and event transaction committed.
    Inserted {
        /// Stored reaction event.
        stored_event: Box<StoredEvent>,
        /// Whether the event row itself was newly inserted.
        was_inserted: bool,
    },
}

/// A grouped set of reactions for a single emoji on an event.
#[derive(Debug, Clone)]
pub struct ReactionGroup {
    /// The emoji character or shortcode used in this reaction group.
    pub emoji: String,
    /// Total number of active reactions with this emoji.
    pub count: i64,
    /// Individual users who reacted with this emoji.
    pub users: Vec<ReactionUser>,
}

/// A single user who reacted with a given emoji.
#[derive(Debug, Clone)]
pub struct ReactionUser {
    /// Compressed 33-byte public key of the reacting user.
    pub pubkey: Vec<u8>,
    /// Optional display name resolved from the users table.
    pub display_name: Option<String>,
    /// Nostr event ID of the kind:7 reaction event (raw bytes), if present.
    /// Clients use this to build signed kind:5 deletion events for reaction removal.
    pub reaction_event_id: Option<Vec<u8>>,
}

/// Bulk reaction entry for embedding in message lists.
#[derive(Debug, Clone)]
pub struct BulkReactionEntry {
    /// The event this reaction entry belongs to.
    pub event_id: Vec<u8>,
    /// Partition key timestamp for the event.
    pub event_created_at: DateTime<Utc>,
    /// Emoji + count summaries for this event.
    pub reactions: Vec<ReactionSummary>,
}

/// Emoji + count summary (no user list) for bulk fetches.
#[derive(Debug, Clone)]
pub struct ReactionSummary {
    /// The emoji character or shortcode.
    pub emoji: String,
    /// Number of active reactions with this emoji.
    pub count: i64,
}

/// Active reaction row metadata for a specific actor + emoji + target tuple.
#[derive(Debug, Clone)]
pub struct ActiveReactionRecord {
    /// Nostr event ID of the reaction event, if this row came from a real kind:7 event.
    pub reaction_event_id: Option<Vec<u8>>,
}

// -- Write operations ---------------------------------------------------------

const ADD_REACTION_SQL: &str = r#"
        INSERT INTO reactions (community_id, event_created_at, event_id, pubkey, emoji, reaction_event_id)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (community_id, event_created_at, event_id, pubkey, emoji) DO UPDATE SET
            created_at = NOW(),
            removed_at = NULL,
            reaction_event_id = COALESCE(EXCLUDED.reaction_event_id, reactions.reaction_event_id)
        WHERE reactions.removed_at IS NOT NULL
        "#;

/// Add (or re-activate) a reaction.
///
/// Returns `Ok(true)` if the reaction was added or re-activated, `Ok(false)` if
/// the reaction is already active (duplicate, no change made).
///
/// Uses `INSERT ... ON CONFLICT DO UPDATE` to eliminate the TOCTOU race where
/// two concurrent adds both see no existing row and then race to INSERT.
pub async fn add_reaction(
    pool: &PgPool,
    community: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    pubkey: &[u8],
    emoji: &str,
    reaction_event_id: Option<&[u8]>,
) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let result = sqlx::query(ADD_REACTION_SQL)
        .bind(community.as_uuid())
        .bind(event_created_at)
        .bind(event_id)
        .bind(pubkey)
        .bind(emoji)
        .bind(reaction_event_id)
        .execute(&mut *connection)
        .await?;

    // Three cases:
    // (a) New reaction (no existing row): INSERT succeeds → rows_affected = 1 → true.
    // (b) Reactivating (row exists, removed_at IS NOT NULL): WHERE matches → UPDATE fires
    //     → rows_affected = 1 → true.
    // (c) Active duplicate (row exists, removed_at IS NULL): WHERE fails → no UPDATE
    //     → rows_affected = 0 → false. Caller should short-circuit and not store the event.
    Ok(result.rows_affected() != 0)
}

/// Add (or re-activate) a reaction inside an existing transaction.
///
/// Uses the same `INSERT ... ON CONFLICT DO UPDATE ... WHERE removed_at IS NOT NULL`
/// statement as [`add_reaction`], preserving the new / re-activate / active-duplicate
/// semantics while letting callers atomically couple the reaction row to other writes.
pub(crate) async fn add_reaction_tx(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    pubkey: &[u8],
    emoji: &str,
    reaction_event_id: Option<&[u8]>,
) -> Result<bool> {
    let result = sqlx::query(ADD_REACTION_SQL)
        .bind(community.as_uuid())
        .bind(event_created_at)
        .bind(event_id)
        .bind(pubkey)
        .bind(emoji)
        .bind(reaction_event_id)
        .execute(&mut **tx)
        .await?;

    Ok(result.rows_affected() != 0)
}

/// Atomically insert a kind:7 reaction event and its reaction row.
///
/// Ordering is load-bearing: resolve target, upsert/reactivate the reaction row,
/// check `rows_affected`, then insert the kind:7 event. Active duplicates return
/// before event insertion so duplicate reactions never store a duplicate kind:7.
#[allow(clippy::too_many_arguments)]
pub async fn insert_reaction_event_with_thread_metadata(
    pool: &PgPool,
    community_id: CommunityId,
    reaction_event: &Event,
    channel_id: Option<Uuid>,
    thread_meta: Option<ThreadMetadataParams<'_>>,
    target_event_id: &[u8],
    actor_pubkey: &[u8],
    emoji: &str,
) -> Result<ReactionEventInsertOutcome> {
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;

    let target_row = sqlx::query(
        "SELECT created_at FROM events \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(target_event_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(target_row) = target_row else {
        tx.rollback().await?;
        return Ok(ReactionEventInsertOutcome::TargetMissing);
    };
    let target_created_at: DateTime<Utc> = target_row.get("created_at");

    // Preserve add_reaction's exact new / re-activate / active-duplicate semantics.
    let reaction_inserted = add_reaction_tx(
        &mut tx,
        community_id,
        target_event_id,
        target_created_at,
        actor_pubkey,
        emoji,
        Some(reaction_event.id.as_bytes()),
    )
    .await?;

    if !reaction_inserted {
        tx.rollback().await?;
        return Ok(ReactionEventInsertOutcome::Duplicate);
    }

    let (stored_event, was_inserted) = insert_event_with_thread_metadata_tx(
        &mut tx,
        community_id,
        reaction_event,
        channel_id,
        thread_meta,
    )
    .await?;

    tx.commit().await?;

    Ok(ReactionEventInsertOutcome::Inserted {
        stored_event: Box::new(stored_event),
        was_inserted,
    })
}

/// Soft-delete a reaction by setting `removed_at`.
///
/// Returns `true` if a row was updated, `false` if not found or already removed.
pub async fn remove_reaction(
    pool: &PgPool,
    community: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    pubkey: &[u8],
    emoji: &str,
) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let result = sqlx::query(
        r#"
        UPDATE reactions
        SET removed_at = NOW()
        WHERE community_id = $1
          AND event_created_at = $2
          AND event_id = $3
          AND pubkey = $4
          AND emoji = $5
          AND removed_at IS NULL
        "#,
    )
    .bind(community.as_uuid())
    .bind(event_created_at)
    .bind(event_id)
    .bind(pubkey)
    .bind(emoji)
    .execute(&mut *connection)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Soft-delete a reaction by the reaction event's own ID.
///
/// Returns `true` if a row was updated, `false` if not found or already removed.
pub async fn remove_reaction_by_source_event_id(
    pool: &PgPool,
    community: CommunityId,
    reaction_event_id: &[u8],
) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let result = sqlx::query(
        r#"
        UPDATE reactions
        SET removed_at = NOW()
        WHERE community_id = $1
          AND reaction_event_id = $2
          AND removed_at IS NULL
        "#,
    )
    .bind(community.as_uuid())
    .bind(reaction_event_id)
    .execute(&mut *connection)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Look up the active reaction row for one actor + emoji + target tuple.
pub async fn get_active_reaction_record(
    pool: &PgPool,
    community: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    pubkey: &[u8],
    emoji: &str,
) -> Result<Option<ActiveReactionRecord>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let row = sqlx::query(
        r#"
        SELECT reaction_event_id
        FROM reactions
        WHERE community_id = $1
          AND event_id = $2
          AND event_created_at = $3
          AND pubkey = $4
          AND emoji = $5
          AND removed_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(community.as_uuid())
    .bind(event_id)
    .bind(event_created_at)
    .bind(pubkey)
    .bind(emoji)
    .fetch_optional(&mut *connection)
    .await?;

    row.map(|row| -> Result<ActiveReactionRecord> {
        Ok(ActiveReactionRecord {
            reaction_event_id: row.try_get("reaction_event_id")?,
        })
    })
    .transpose()
}

/// Backfill the source event ID on an active reaction row.
///
/// Called after the kind:7 event is created and stored, to link the
/// reaction row to its source event. Returns `true` if the row was updated.
pub async fn set_reaction_event_id(
    pool: &PgPool,
    community: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    pubkey: &[u8],
    emoji: &str,
    reaction_event_id: &[u8],
) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let result = sqlx::query(
        r#"
        UPDATE reactions
        SET reaction_event_id = $1
        WHERE community_id = $2
          AND event_created_at = $3
          AND event_id = $4
          AND pubkey = $5
          AND emoji = $6
          AND removed_at IS NULL
        "#,
    )
    .bind(reaction_event_id)
    .bind(community.as_uuid())
    .bind(event_created_at)
    .bind(event_id)
    .bind(pubkey)
    .bind(emoji)
    .execute(&mut *connection)
    .await?;

    Ok(result.rows_affected() > 0)
}

// -- Read operations ----------------------------------------------------------

/// Get all active reactions for an event, grouped by emoji.
///
/// Returns one [`ReactionGroup`] per emoji, each containing the list of reacting
/// user pubkeys. Display names are NOT resolved here -- callers should enrich via
/// scoped user lookups if needed.
///
/// `cursor` is reserved for future keyset pagination (currently unused).
pub async fn get_reactions(
    pool: &PgPool,
    community: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    limit: u32,
    _cursor: Option<&str>,
) -> Result<Vec<ReactionGroup>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await?;
    // Two-step query: first get the limited set of distinct emoji groups,
    // then fetch all rows for those groups. This ensures `limit` applies to
    // emoji groups (the API contract), not raw rows — so one busy emoji
    // cannot consume the entire page and hide other groups.
    let rows = sqlx::query(
        r#"
        SELECT r.emoji, r.pubkey, r.reaction_event_id
        FROM reactions r
        INNER JOIN (
            SELECT DISTINCT emoji
            FROM reactions
            WHERE community_id = $1
              AND event_id = $2
              AND event_created_at = $3
              AND removed_at IS NULL
            ORDER BY emoji
            LIMIT $4
        ) g ON g.emoji = r.emoji
        WHERE r.community_id = $1
          AND r.event_id = $2
          AND r.event_created_at = $3
          AND r.removed_at IS NULL
        ORDER BY r.emoji, r.created_at
        "#,
    )
    .bind(community.as_uuid())
    .bind(event_id)
    .bind(event_created_at)
    .bind(limit as i64)
    .fetch_all(&mut *connection)
    .await?;

    // Group individual rows by emoji in Rust.
    let mut groups: Vec<ReactionGroup> = Vec::new();
    let mut current_emoji: Option<String> = None;
    let mut current_users: Vec<ReactionUser> = Vec::new();

    for row in &rows {
        let emoji: String = row.try_get("emoji")?;
        let pubkey: Vec<u8> = row.try_get("pubkey")?;
        let reaction_event_id: Option<Vec<u8>> = row.try_get("reaction_event_id")?;

        if current_emoji.as_ref() != Some(&emoji) {
            if let Some(prev_emoji) = current_emoji.take() {
                let count = current_users.len() as i64;
                groups.push(ReactionGroup {
                    emoji: prev_emoji,
                    count,
                    users: std::mem::take(&mut current_users),
                });
            }
            current_emoji = Some(emoji);
        }

        current_users.push(ReactionUser {
            pubkey,
            display_name: None,
            reaction_event_id,
        });
    }

    // Flush the final group.
    if let Some(emoji) = current_emoji {
        let count = current_users.len() as i64;
        groups.push(ReactionGroup {
            emoji,
            count,
            users: current_users,
        });
    }

    Ok(groups)
}

/// Batch-fetch emoji counts for a set of (event_id, event_created_at) pairs.
///
/// Returns one [`BulkReactionEntry`] per input pair that has at least one
/// active reaction. Pairs with no reactions are omitted.
pub async fn get_reactions_bulk(
    pool: &PgPool,
    community: CommunityId,
    event_ids: &[(&[u8], DateTime<Utc>)],
) -> Result<Vec<BulkReactionEntry>> {
    if event_ids.is_empty() {
        return Ok(Vec::new());
    }

    // Run one query per event. For typical message-list sizes (<=100 events)
    // this is acceptable; a single-query approach with dynamic IN clauses over
    // composite keys can be added later if needed.
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await?;
    let mut entries = Vec::new();

    for (event_id, event_created_at) in event_ids {
        let rows = sqlx::query(
            r#"
            SELECT emoji, COUNT(*) AS count
            FROM reactions
            WHERE community_id = $1
              AND event_id = $2
              AND event_created_at = $3
              AND removed_at IS NULL
            GROUP BY emoji
            ORDER BY emoji
            "#,
        )
        .bind(community.as_uuid())
        .bind(*event_id)
        .bind(event_created_at)
        .fetch_all(&mut *connection)
        .await?;

        if rows.is_empty() {
            continue;
        }

        let mut reactions = Vec::with_capacity(rows.len());
        for row in rows {
            let emoji: String = row.try_get("emoji")?;
            let count: i64 = row.try_get("count")?;
            reactions.push(ReactionSummary { emoji, count });
        }

        entries.push(BulkReactionEntry {
            event_id: event_id.to_vec(),
            event_created_at: *event_created_at,
            reactions,
        });
    }

    Ok(entries)
}

// -- Db API -------------------------------------------------------------------

impl Db {
    /// Atomically insert a kind:7 reaction event and its reaction row.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(
        name = "insert_reaction_event_with_thread_metadata",
        system = "postgresql"
    )]
    pub async fn insert_reaction_event_with_thread_metadata(
        &self,
        community_id: CommunityId,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
        thread_meta: Option<ThreadMetadataParams<'_>>,
        target_event_id: &[u8],
        actor_pubkey: &[u8],
        emoji: &str,
    ) -> Result<ReactionEventInsertOutcome> {
        let outcome = crate::reaction::insert_reaction_event_with_thread_metadata(
            &self.pool,
            community_id,
            event,
            channel_id,
            thread_meta,
            target_event_id,
            actor_pubkey,
            emoji,
        )
        .await?;
        if let ReactionEventInsertOutcome::Inserted {
            was_inserted: true, ..
        } = &outcome
        {
            if let Err(e) =
                crate::insert_mentions(&self.pool, community_id, event, channel_id).await
            {
                tracing::warn!(event_id = %event.id, "Failed to insert mentions: {e}");
            }
        }
        Ok(outcome)
    }

    /// Add (or re-activate) a reaction.
    #[datastore_span(name = "add_reaction", system = "postgresql")]
    pub async fn add_reaction(
        &self,
        community: CommunityId,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
        reaction_event_id: Option<&[u8]>,
    ) -> Result<bool> {
        crate::reaction::add_reaction(
            &self.pool,
            community,
            event_id,
            event_created_at,
            pubkey,
            emoji,
            reaction_event_id,
        )
        .await
    }

    /// Soft-delete a reaction.
    #[datastore_span(name = "remove_reaction", system = "postgresql")]
    pub async fn remove_reaction(
        &self,
        community: CommunityId,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
    ) -> Result<bool> {
        crate::reaction::remove_reaction(
            &self.pool,
            community,
            event_id,
            event_created_at,
            pubkey,
            emoji,
        )
        .await
    }

    /// Soft-delete a reaction by its source event ID.
    #[datastore_span(name = "remove_reaction_by_source_event_id", system = "postgresql")]
    pub async fn remove_reaction_by_source_event_id(
        &self,
        community: CommunityId,
        reaction_event_id: &[u8],
    ) -> Result<bool> {
        crate::reaction::remove_reaction_by_source_event_id(
            &self.pool,
            community,
            reaction_event_id,
        )
        .await
    }

    /// Look up the active reaction row for one actor + emoji + target tuple.
    #[datastore_span(name = "get_active_reaction_record", system = "postgresql")]
    pub async fn get_active_reaction_record(
        &self,
        community: CommunityId,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
    ) -> Result<Option<crate::reaction::ActiveReactionRecord>> {
        crate::reaction::get_active_reaction_record(
            &self.pool,
            community,
            event_id,
            event_created_at,
            pubkey,
            emoji,
        )
        .await
    }

    /// Backfill the source event ID on an active reaction row.
    #[datastore_span(name = "set_reaction_event_id", system = "postgresql")]
    pub async fn set_reaction_event_id(
        &self,
        community: CommunityId,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        pubkey: &[u8],
        emoji: &str,
        reaction_event_id: &[u8],
    ) -> Result<bool> {
        crate::reaction::set_reaction_event_id(
            &self.pool,
            community,
            event_id,
            event_created_at,
            pubkey,
            emoji,
            reaction_event_id,
        )
        .await
    }

    /// Get all active reactions for an event, grouped by emoji.
    #[datastore_span(name = "get_reactions", system = "postgresql")]
    pub async fn get_reactions(
        &self,
        community: CommunityId,
        event_id: &[u8],
        event_created_at: DateTime<Utc>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<Vec<crate::reaction::ReactionGroup>> {
        crate::reaction::get_reactions(
            &self.pool,
            community,
            event_id,
            event_created_at,
            limit,
            cursor,
        )
        .await
    }

    /// Batch-fetch emoji counts for a set of (event_id, event_created_at) pairs.
    #[datastore_span(name = "get_reactions_bulk", system = "postgresql")]
    pub async fn get_reactions_bulk(
        &self,
        community: CommunityId,
        event_ids: &[(&[u8], DateTime<Utc>)],
    ) -> Result<Vec<crate::reaction::BulkReactionEntry>> {
        crate::reaction::get_reactions_bulk(&self.pool, community, event_ids).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::{
        error::DbError,
        event::{get_event_by_id, insert_event},
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());

        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    async fn make_test_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("reaction-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        id
    }

    fn make_text_event(content: &str) -> nostr::Event {
        EventBuilder::new(Kind::Custom(9), content)
            .sign_with_keys(&Keys::generate())
            .expect("sign text event")
    }

    fn make_reaction_event(keys: &Keys, target_id_hex: &str, emoji: &str) -> nostr::Event {
        let nonce = Uuid::new_v4().to_string();
        EventBuilder::new(Kind::Custom(7), emoji)
            .tags(vec![
                Tag::parse(["e", target_id_hex]).expect("reaction e tag"),
                Tag::parse(["nonce", nonce.as_str()]).expect("nonce tag"),
            ])
            .sign_with_keys(keys)
            .expect("sign reaction event")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reaction_single_tx_stores_wrapped_max_shortcode() {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let target = make_text_event("long custom emoji target");
        insert_event(&pool, community, &target, None)
            .await
            .expect("insert target");

        let actor = Keys::generate();
        let emoji = format!(":{}:", "a".repeat(64));
        let reaction = make_reaction_event(&actor, &target.id.to_hex(), &emoji);
        let outcome = insert_reaction_event_with_thread_metadata(
            &pool,
            community,
            &reaction,
            None,
            None,
            target.id.as_bytes(),
            &actor.public_key().to_bytes(),
            &emoji,
        )
        .await
        .expect("store wrapped 64-character shortcode");

        assert!(matches!(
            outcome,
            ReactionEventInsertOutcome::Inserted {
                was_inserted: true,
                ..
            }
        ));
        assert_eq!(emoji.chars().count(), 66);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reaction_single_tx_duplicate_short_circuit_stores_no_event() {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let target = make_text_event("reaction target");
        insert_event(&pool, community, &target, None)
            .await
            .expect("insert target");

        let actor = Keys::generate();
        let actor_pubkey = actor.public_key().to_bytes();
        let target_hex = target.id.to_hex();
        let first = make_reaction_event(&actor, &target_hex, "👍");
        let second = make_reaction_event(&actor, &target_hex, "👍");

        let first_outcome = insert_reaction_event_with_thread_metadata(
            &pool,
            community,
            &first,
            None,
            None,
            target.id.as_bytes(),
            &actor_pubkey,
            "👍",
        )
        .await
        .expect("first reaction insert");
        assert!(matches!(
            first_outcome,
            ReactionEventInsertOutcome::Inserted {
                was_inserted: true,
                ..
            }
        ));

        let duplicate = insert_reaction_event_with_thread_metadata(
            &pool,
            community,
            &second,
            None,
            None,
            target.id.as_bytes(),
            &actor_pubkey,
            "👍",
        )
        .await
        .expect("duplicate reaction insert");
        assert!(matches!(duplicate, ReactionEventInsertOutcome::Duplicate));

        let duplicate_event = get_event_by_id(&pool, community, second.id.as_bytes())
            .await
            .expect("lookup duplicate reaction event");
        assert!(
            duplicate_event.is_none(),
            "active duplicate reaction must short-circuit before storing kind:7 event"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reaction_single_tx_cross_community_target_rejected() {
        let pool = setup_pool().await;
        let community_a = CommunityId::from_uuid(make_test_community(&pool).await);
        let community_b = CommunityId::from_uuid(make_test_community(&pool).await);
        let target = make_text_event("community A target only");
        insert_event(&pool, community_a, &target, None)
            .await
            .expect("insert target in A");

        let actor = Keys::generate();
        let actor_pubkey = actor.public_key().to_bytes();
        let reaction = make_reaction_event(&actor, &target.id.to_hex(), "👍");

        let outcome = insert_reaction_event_with_thread_metadata(
            &pool,
            community_b,
            &reaction,
            None,
            None,
            target.id.as_bytes(),
            &actor_pubkey,
            "👍",
        )
        .await
        .expect("cross-community reaction attempt");
        assert!(matches!(outcome, ReactionEventInsertOutcome::TargetMissing));

        assert!(
            get_event_by_id(&pool, community_b, reaction.id.as_bytes())
                .await
                .expect("lookup B reaction event")
                .is_none(),
            "reaction event must not store when target exists only in another community"
        );
        assert!(
            crate::reaction::get_active_reaction_record(
                &pool,
                community_b,
                target.id.as_bytes(),
                DateTime::from_timestamp(target.created_at.as_secs() as i64, 0).unwrap(),
                &actor_pubkey,
                "👍",
            )
            .await
            .expect("lookup B reaction row")
            .is_none(),
            "reaction row must not be inserted for cross-community target miss"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reaction_single_tx_event_insert_failure_rolls_back_reaction() {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let target = make_text_event("rollback target");
        insert_event(&pool, community, &target, None)
            .await
            .expect("insert target");

        let actor = Keys::generate();
        let actor_pubkey = actor.public_key().to_bytes();
        let target_hex = target.id.to_hex();
        let bad_reaction = EventBuilder::new(Kind::Custom(20000), "👍")
            .tags(vec![
                Tag::parse(["e", target_hex.as_str()]).expect("reaction e tag")
            ])
            .sign_with_keys(&actor)
            .expect("sign ephemeral reaction-shaped event");
        let target_created_at = DateTime::from_timestamp(target.created_at.as_secs() as i64, 0)
            .expect("target timestamp");

        let err = insert_reaction_event_with_thread_metadata(
            &pool,
            community,
            &bad_reaction,
            None,
            None,
            target.id.as_bytes(),
            &actor_pubkey,
            "👍",
        )
        .await
        .expect_err("ephemeral event insert must fail after reaction upsert attempt");
        assert!(matches!(err, DbError::EphemeralEventRejected(20000)));

        assert!(
            crate::reaction::get_active_reaction_record(
                &pool,
                community,
                target.id.as_bytes(),
                target_created_at,
                &actor_pubkey,
                "👍",
            )
            .await
            .expect("lookup reaction row after rollback")
            .is_none(),
            "transaction rollback must remove the reaction row when event insert fails"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reaction_single_tx_reactivates_soft_deleted_reaction() {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let target = make_text_event("reactivation target");
        insert_event(&pool, community, &target, None)
            .await
            .expect("insert target");

        let actor = Keys::generate();
        let actor_pubkey = actor.public_key().to_bytes();
        let target_hex = target.id.to_hex();
        let target_created_at = DateTime::from_timestamp(target.created_at.as_secs() as i64, 0)
            .expect("target timestamp");
        let first = make_reaction_event(&actor, &target_hex, "👍");
        let second = make_reaction_event(&actor, &target_hex, "👍");

        assert!(matches!(
            insert_reaction_event_with_thread_metadata(
                &pool,
                community,
                &first,
                None,
                None,
                target.id.as_bytes(),
                &actor_pubkey,
                "👍",
            )
            .await
            .expect("first reaction insert"),
            ReactionEventInsertOutcome::Inserted { .. }
        ));
        assert!(crate::reaction::remove_reaction(
            &pool,
            community,
            target.id.as_bytes(),
            target_created_at,
            &actor_pubkey,
            "👍",
        )
        .await
        .expect("soft delete reaction"));

        let outcome = insert_reaction_event_with_thread_metadata(
            &pool,
            community,
            &second,
            None,
            None,
            target.id.as_bytes(),
            &actor_pubkey,
            "👍",
        )
        .await
        .expect("reactivate reaction");
        assert!(matches!(
            outcome,
            ReactionEventInsertOutcome::Inserted {
                was_inserted: true,
                ..
            }
        ));

        let active = crate::reaction::get_active_reaction_record(
            &pool,
            community,
            target.id.as_bytes(),
            target_created_at,
            &actor_pubkey,
            "👍",
        )
        .await
        .expect("active record after reactivation")
        .expect("reaction active after reactivation");
        assert_eq!(
            active.reaction_event_id.as_deref(),
            Some(second.id.as_bytes().as_slice()),
            "reactivation through the tx path must preserve add_reaction's source-id update semantics"
        );
    }

    /// BUG-5 regression: the `reactions` table is community-scoped
    /// (`PK (community_id, event_created_at, event_id, pubkey, emoji)`), so a
    /// reaction added under community A must be invisible and unremovable from
    /// community B — even for the *identical* `(event_id, pubkey, emoji)` shape.
    /// Before the fix, `add_reaction` omitted `community_id` (NOT NULL → 500) and
    /// every read/remove filtered `event_id` only (latent cross-tenant bleed).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reactions_are_scoped_to_community() {
        let pool = setup_pool().await;
        let db = Db::from_pool(pool.clone());
        let community_a = CommunityId::from_uuid(make_test_community(&pool).await);
        let community_b = CommunityId::from_uuid(make_test_community(&pool).await);

        // Identical referenced-event shape across both tenants.
        let event_id = [0xABu8; 32];
        let event_created_at = Utc::now();
        let pubkey = [7u8; 32];
        let emoji = "👍";

        // (1) Add succeeds under A (this INSERT 500'd before the fix).
        assert!(
            db.add_reaction(
                community_a,
                &event_id,
                event_created_at,
                &pubkey,
                emoji,
                None
            )
            .await
            .expect("add reaction under A"),
            "first reaction under A must be inserted"
        );
        // Idempotent: re-adding the same active reaction is a no-op.
        assert!(
            !db.add_reaction(
                community_a,
                &event_id,
                event_created_at,
                &pubkey,
                emoji,
                None
            )
            .await
            .expect("duplicate reaction under A"),
            "active duplicate under A must not re-insert"
        );

        // (2) Visible on A, invisible on B (grouped read path).
        let groups_a = db
            .get_reactions(community_a, &event_id, event_created_at, 100, None)
            .await
            .expect("get reactions A");
        assert_eq!(groups_a.len(), 1, "A must see its own reaction group");
        assert_eq!(groups_a[0].emoji, emoji);
        assert_eq!(groups_a[0].count, 1);

        let groups_b = db
            .get_reactions(community_b, &event_id, event_created_at, 100, None)
            .await
            .expect("get reactions B");
        assert!(
            groups_b.is_empty(),
            "B must NOT see A's reaction for the same event shape, got {groups_b:?}"
        );

        // (3) Active-record lookup is scoped: present on A, absent on B.
        assert!(
            db.get_active_reaction_record(community_a, &event_id, event_created_at, &pubkey, emoji)
                .await
                .expect("active record A")
                .is_some(),
            "A's active reaction record must be present"
        );
        assert!(
            db.get_active_reaction_record(community_b, &event_id, event_created_at, &pubkey, emoji)
                .await
                .expect("active record B")
                .is_none(),
            "B must not find A's active reaction record"
        );

        // (4) B can add the identical shape independently (no PK collision).
        assert!(
            db.add_reaction(
                community_b,
                &event_id,
                event_created_at,
                &pubkey,
                emoji,
                None
            )
            .await
            .expect("add reaction under B"),
            "B must be able to add the same shape as its own scoped row"
        );

        // (5) Removing from B does not touch A's row.
        assert!(
            db.remove_reaction(community_b, &event_id, event_created_at, &pubkey, emoji)
                .await
                .expect("remove under B"),
            "B remove must affect B's own row"
        );
        assert!(
            db.get_active_reaction_record(community_a, &event_id, event_created_at, &pubkey, emoji)
                .await
                .expect("active record A after B remove")
                .is_some(),
            "A's reaction must survive a B-side removal"
        );

        // (6) A remove affects only A; A's read now empty.
        assert!(
            db.remove_reaction(community_a, &event_id, event_created_at, &pubkey, emoji)
                .await
                .expect("remove under A"),
            "A remove must affect A's row"
        );
        let groups_a_after = db
            .get_reactions(community_a, &event_id, event_created_at, 100, None)
            .await
            .expect("get reactions A after remove");
        assert!(
            groups_a_after.is_empty(),
            "A's reaction must be gone after A removes it"
        );
    }
}
