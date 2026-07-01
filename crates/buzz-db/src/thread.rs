//! Thread metadata persistence.
//!
//! Tracks parent/root relationships, depth, and reply counts for infinitely
//! nested threads. The `thread_metadata` table is populated when events are
//! ingested and updated as replies arrive or are deleted.

use buzz_core::StoredEvent;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use buzz_core::CommunityId;

use crate::{error::Result, event::row_to_stored_event};

// -- Structs ------------------------------------------------------------------

/// A single reply within a thread, joined with event content.
#[derive(Debug, Clone)]
pub struct ThreadReply {
    /// The Nostr event ID of this reply.
    pub event_id: Vec<u8>,
    /// The event ID of the direct parent (one level up), if any.
    pub parent_event_id: Option<Vec<u8>>,
    /// The event ID of the thread root (top-level message), if any.
    pub root_event_id: Option<Vec<u8>>,
    /// The channel this reply belongs to.
    pub channel_id: Uuid,
    /// Compressed public key of the reply author.
    pub pubkey: Vec<u8>,
    /// Nostr event tags (JSON array), used to extract effective author.
    pub tags: serde_json::Value,
    /// Text content of the reply.
    pub content: String,
    /// Fully reconstructed event row for this reply.
    pub stored_event: StoredEvent,
    /// Nesting depth within the thread (root = 0, direct reply = 1, etc.).
    pub depth: i32,
    /// When the reply was created.
    pub created_at: DateTime<Utc>,
    /// Whether this reply is also broadcast to the channel timeline.
    pub broadcast: bool,
}

/// Aggregated thread statistics for a root message.
#[derive(Debug, Clone)]
pub struct ThreadSummary {
    /// Number of direct replies to the root message.
    pub reply_count: i32,
    /// Total number of replies at all nesting levels.
    pub descendant_count: i32,
    /// Timestamp of the most recent reply in the thread.
    pub last_reply_at: Option<DateTime<Utc>>,
    /// Compressed public keys of all participants who have replied.
    pub participants: Vec<Vec<u8>>,
}

/// A top-level channel message with optional thread summary.
#[derive(Debug, Clone)]
pub struct TopLevelMessage {
    /// The Nostr event ID of this message.
    pub event_id: Vec<u8>,
    /// Compressed public key of the message author.
    pub pubkey: Vec<u8>,
    /// Nostr event tags (JSON array), used to extract effective author.
    pub tags: serde_json::Value,
    /// Text content of the message.
    pub content: String,
    /// Nostr event kind number.
    pub kind: i32,
    /// When the message was created.
    pub created_at: DateTime<Utc>,
    /// The channel this message belongs to.
    pub channel_id: Uuid,
    /// Thread statistics for this message, if it has replies.
    pub thread_summary: Option<ThreadSummary>,
}

/// Raw thread_metadata row -- used when processing deletes or computing ancestry.
#[derive(Debug, Clone)]
pub struct ThreadMetadataRecord {
    /// The Nostr event ID this metadata row tracks.
    pub event_id: Vec<u8>,
    /// Partition key timestamp for the event.
    pub event_created_at: DateTime<Utc>,
    /// The channel this event belongs to.
    pub channel_id: Uuid,
    /// Event ID of the direct parent, if this is a reply.
    pub parent_event_id: Option<Vec<u8>>,
    /// Event ID of the thread root, if this is a nested reply.
    pub root_event_id: Option<Vec<u8>>,
    /// Nesting depth (root = 0).
    pub depth: i32,
    /// Number of direct replies to this event.
    pub reply_count: i32,
    /// Total number of descendants at all nesting levels.
    pub descendant_count: i32,
    /// Whether this event is broadcast to the channel timeline.
    pub broadcast: bool,
}

// -- Write operations ---------------------------------------------------------

/// Insert a row into `thread_metadata`.
///
/// If `parent_event_id` is `Some`, also increments the parent's reply count
/// and the root's descendant count (always, including when root == parent).
///
/// The INSERT and all counter UPDATEs are wrapped in a single transaction so a
/// crash between them cannot leave reply_count / descendant_count inconsistent
/// with the actual number of reply rows (F9).
#[allow(clippy::too_many_arguments)]
pub async fn insert_thread_metadata(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    channel_id: Uuid,
    parent_event_id: Option<&[u8]>,
    parent_event_created_at: Option<DateTime<Utc>>,
    root_event_id: Option<&[u8]>,
    root_event_created_at: Option<DateTime<Utc>>,
    depth: i32,
    broadcast: bool,
) -> Result<()> {
    let mut tx = pool.begin().await?;

    let result = sqlx::query(
        r#"
        INSERT INTO thread_metadata
            (community_id, event_created_at, event_id, channel_id,
             parent_event_id, parent_event_created_at,
             root_event_id, root_event_created_at,
             depth, broadcast)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(event_created_at)
    .bind(event_id)
    .bind(channel_id)
    .bind(parent_event_id)
    .bind(parent_event_created_at)
    .bind(root_event_id)
    .bind(root_event_created_at)
    .bind(depth)
    .bind(broadcast)
    .execute(&mut *tx)
    .await?;

    // Only bump reply counts if the row was actually inserted (not a duplicate).
    // ON CONFLICT DO NOTHING on a duplicate key returns rows_affected = 0.
    if result.rows_affected() > 0 {
        if let Some(pid) = parent_event_id {
            // Ensure the parent has a thread_metadata row so the UPDATE below
            // has something to hit. Root (depth=0) messages don't get a row on
            // first insert, so we create a stub here.
            let parent_ts = parent_event_created_at.unwrap_or(event_created_at);
            sqlx::query(
                r#"
                INSERT INTO thread_metadata
                    (community_id, event_created_at, event_id, channel_id,
                     parent_event_id, parent_event_created_at,
                     root_event_id, root_event_created_at,
                     depth, broadcast)
                VALUES ($1, $2, $3, $4, NULL, NULL, NULL, NULL, 0, false)
                ON CONFLICT DO NOTHING
                "#,
            )
            .bind(community_id.as_uuid())
            .bind(parent_ts)
            .bind(pid)
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;

            // Ensure the root also has a row (may differ from parent for nested replies).
            if let Some(root_id) = root_event_id {
                if root_id != pid {
                    let root_ts = root_event_created_at.unwrap_or(event_created_at);
                    sqlx::query(
                        r#"
                        INSERT INTO thread_metadata
                            (event_created_at, event_id, channel_id,
                             parent_event_id, parent_event_created_at,
                             root_event_id, root_event_created_at,
                             depth, broadcast)
                        VALUES ($1, $2, $3, NULL, NULL, NULL, NULL, 0, false)
                        ON CONFLICT DO NOTHING
                        "#,
                    )
                    .bind(community_id.as_uuid())
                    .bind(root_ts)
                    .bind(root_id)
                    .bind(channel_id)
                    .execute(&mut *tx)
                    .await?;
                }
            }

            // Increment parent's direct reply count and last_reply_at.
            sqlx::query(
                r#"
                UPDATE thread_metadata
                SET reply_count   = reply_count + 1,
                    last_reply_at = NOW()
                WHERE community_id = $1 AND event_id = $2
                "#,
            )
            .bind(community_id.as_uuid())
            .bind(pid)
            .execute(&mut *tx)
            .await?;

            // Increment root's total descendant count.
            if let Some(root_id) = root_event_id {
                sqlx::query(
                    r#"
                    UPDATE thread_metadata
                    SET descendant_count = descendant_count + 1
                    WHERE community_id = $1 AND event_id = $2
                    "#,
                )
                .bind(community_id.as_uuid())
                .bind(root_id)
                .execute(&mut *tx)
                .await?;
            }
        }
    }

    tx.commit().await?;

    Ok(())
}

/// Increment `reply_count` (and `last_reply_at`) on the parent event.
/// If `root_event_id` is provided, also increments `descendant_count` on the
/// root -- even when root == parent (direct reply to root). This is correct
/// because `reply_count` tracks direct children only, while `descendant_count`
/// tracks ALL descendants at every nesting level.
///
/// NOTE: The primary increment path is inlined inside [`insert_thread_metadata`]'s
/// transaction. This standalone version exists for future use cases where
/// incrementing outside of insert is needed (e.g., event re-parenting).
#[allow(dead_code)]
pub async fn increment_reply_count(
    pool: &PgPool,
    community_id: CommunityId,
    parent_event_id: &[u8],
    root_event_id: Option<&[u8]>,
) -> Result<()> {
    // Always bump the parent's direct reply count and last-reply timestamp.
    sqlx::query(
        r#"
        UPDATE thread_metadata
        SET reply_count  = reply_count + 1,
            last_reply_at = NOW()
        WHERE community_id = $1 AND event_id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(parent_event_id)
    .execute(pool)
    .await?;

    // Always bump root's descendant_count, regardless of whether root == parent.
    if let Some(root_id) = root_event_id {
        sqlx::query(
            r#"
            UPDATE thread_metadata
            SET descendant_count = descendant_count + 1
            WHERE community_id = $1 AND event_id = $2
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(root_id)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// Decrement `reply_count` on the parent event (floor at 0).
/// If `root_event_id` is provided, also decrements `descendant_count` on the
/// root -- even when root == parent. Mirrors the increment logic exactly.
pub async fn decrement_reply_count(
    pool: &PgPool,
    community_id: CommunityId,
    parent_event_id: &[u8],
    root_event_id: Option<&[u8]>,
) -> Result<()> {
    // Always decrement the parent's direct reply count (floor at 0).
    sqlx::query(
        r#"
        UPDATE thread_metadata
        SET reply_count = GREATEST(reply_count - 1, 0)
        WHERE community_id = $1 AND event_id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(parent_event_id)
    .execute(pool)
    .await?;

    // Always decrement root's descendant_count, regardless of whether root == parent.
    if let Some(root_id) = root_event_id {
        sqlx::query(
            r#"
            UPDATE thread_metadata
            SET descendant_count = GREATEST(descendant_count - 1, 0)
            WHERE community_id = $1 AND event_id = $2
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(root_id)
        .execute(pool)
        .await?;
    }

    Ok(())
}

// -- Read operations ----------------------------------------------------------

/// Fetch all replies under a root event, ordered chronologically.
///
/// - `depth_limit` -- if `Some(n)`, only returns replies at depth <= n.
/// - `cursor` -- keyset pagination cursor. The result order is
///   `(event_created_at ASC, event_id ASC)`; the cursor is the composite key of
///   the last row already seen and the query returns rows strictly after it. A
///   composite `(timestamp, event_id)` tiebreak is required because thread
///   replies routinely share a `created_at` second (bursty threads); a
///   timestamp-only cursor silently drops every tied reply past the page limit.
///   Wire encoding: `8-byte big-endian i64 seconds` followed by the raw
///   `event_id` bytes (32 for a standard Nostr id). A bare 8-byte cursor is
///   still accepted for back-compat and paginates on timestamp alone (unsafe
///   across same-second ties) -- prefer the composite form.
/// - `limit` -- maximum rows returned (caller should cap this).
pub async fn get_thread_replies(
    pool: &PgPool,
    community_id: CommunityId,
    root_event_id: &[u8],
    depth_limit: Option<u32>,
    limit: u32,
    cursor: Option<&[u8]>,
) -> Result<Vec<ThreadReply>> {
    // Decode cursor bytes -> keyset (timestamp, optional event_id) for the
    // WHERE condition. Layout: 8-byte BE i64 seconds, then the raw event_id.
    // An 8-byte-only cursor is legacy timestamp-only paging (no tiebreak).
    let cursor_key: Option<(DateTime<Utc>, Option<Vec<u8>>)> = match cursor {
        Some(bytes) if bytes.len() >= 8 => {
            let secs = i64::from_be_bytes(bytes[..8].try_into().expect("length checked"));
            DateTime::from_timestamp(secs, 0).map(|ts| {
                let id = if bytes.len() > 8 {
                    Some(bytes[8..].to_vec())
                } else {
                    None
                };
                (ts, id)
            })
        }
        _ => None,
    };

    // Build the query dynamically based on optional filters.
    // Track the next positional parameter index.
    let mut param_idx = 3u32; // $1 is community_id, $2 is root_event_id
    let mut sql = String::from(
        r#"
        SELECT
            tm.event_id,
            e.id,
            tm.parent_event_id,
            tm.root_event_id,
            tm.channel_id,
            e.pubkey,
            e.created_at AS created_at,
            e.tags,
            e.content,
            e.kind,
            e.sig,
            e.received_at,
            tm.depth,
            tm.event_created_at,
            tm.broadcast
        FROM thread_metadata tm
        JOIN events e
            ON e.community_id = tm.community_id
           AND e.created_at = tm.event_created_at
           AND e.id         = tm.event_id
        WHERE tm.community_id = $1
          AND tm.root_event_id = $2
          AND e.deleted_at IS NULL
        "#,
    );

    if depth_limit.is_some() {
        sql.push_str(&format!(" AND tm.depth <= ${param_idx}"));
        param_idx += 1;
    }
    match &cursor_key {
        Some((_, Some(_))) => {
            // Composite keyset: strict row comparison with an event_id tiebreak
            // so same-second replies paginate without gaps or duplicates.
            let ts_idx = param_idx;
            let id_idx = param_idx + 1;
            sql.push_str(&format!(
                " AND (tm.event_created_at, tm.event_id) > (${ts_idx}, ${id_idx})"
            ));
            param_idx += 2;
        }
        Some((_, None)) => {
            // Legacy timestamp-only cursor (no tiebreak).
            sql.push_str(&format!(" AND tm.event_created_at > ${param_idx}"));
            param_idx += 1;
        }
        None => {}
    }

    sql.push_str(&format!(
        " ORDER BY tm.event_created_at ASC, tm.event_id ASC LIMIT ${param_idx}"
    ));

    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(community_id.as_uuid())
        .bind(root_event_id);

    if let Some(dl) = depth_limit {
        q = q.bind(dl as i32);
    }
    match &cursor_key {
        Some((ts, Some(id))) => {
            q = q.bind(*ts).bind(id.clone());
        }
        Some((ts, None)) => {
            q = q.bind(*ts);
        }
        None => {}
    }
    q = q.bind(limit as i32);

    let rows = q.fetch_all(pool).await?;

    let mut replies = Vec::with_capacity(rows.len());
    for row in rows {
        let event_id: Vec<u8> = row.try_get("event_id")?;
        let parent_event_id: Option<Vec<u8>> = row.try_get("parent_event_id")?;
        let root_event_id_col: Option<Vec<u8>> = row.try_get("root_event_id")?;
        let channel_id: Uuid = row.try_get("channel_id")?;
        let pubkey: Vec<u8> = row.try_get("pubkey")?;
        let tags: serde_json::Value = row.try_get("tags")?;
        let depth: i32 = row.try_get("depth")?;
        let created_at: DateTime<Utc> = row.try_get("event_created_at")?;
        let broadcast_val: bool = row.try_get("broadcast")?;

        // Skip rows that fail event reconstruction (e.g. corrupt signature)
        // rather than failing the whole thread query, matching the
        // skip-and-continue semantics of the prior get_events_by_ids path.
        let stored_event = match row_to_stored_event(row)? {
            Some(se) => se,
            None => continue,
        };

        replies.push(ThreadReply {
            event_id,
            parent_event_id,
            root_event_id: root_event_id_col,
            channel_id,
            pubkey,
            tags,
            content: stored_event.event.content.clone(),
            stored_event,
            depth,
            created_at,
            broadcast: broadcast_val,
        });
    }

    Ok(replies)
}

/// Fetch aggregated thread stats for a single event, plus up to 10 participant pubkeys.
pub async fn get_thread_summary(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
) -> Result<Option<ThreadSummary>> {
    let row = sqlx::query(
        r#"
        SELECT reply_count, descendant_count, last_reply_at
        FROM thread_metadata
        WHERE community_id = $1 AND event_id = $2
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(event_id)
    .fetch_optional(pool)
    .await?;

    let row = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    let reply_count: i32 = row.try_get("reply_count")?;
    let descendant_count: i32 = row.try_get("descendant_count")?;
    let last_reply_at: Option<DateTime<Utc>> = row.try_get("last_reply_at")?;

    // Collect distinct participant pubkeys from the thread, most recent first.
    let participant_rows = sqlx::query(
        r#"
        SELECT pubkey FROM (
            SELECT DISTINCT e.pubkey, MAX(e.created_at) AS last_seen
            FROM thread_metadata tm
            JOIN events e
                ON e.community_id = tm.community_id
               AND e.created_at = tm.event_created_at
               AND e.id         = tm.event_id
            WHERE tm.community_id = $1
              AND tm.root_event_id = $2
              AND e.deleted_at IS NULL
            GROUP BY e.pubkey
        ) sub
        ORDER BY last_seen DESC
        LIMIT 10
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(event_id)
    .fetch_all(pool)
    .await?;

    let participants: Vec<Vec<u8>> = participant_rows
        .into_iter()
        .map(|r| r.try_get::<Vec<u8>, _>("pubkey"))
        .collect::<std::result::Result<_, _>>()?;

    Ok(Some(ThreadSummary {
        reply_count,
        descendant_count,
        last_reply_at,
        participants,
    }))
}

/// Fetch top-level messages for a channel (depth = 0, or broadcast replies).
///
/// Returns events that are either:
/// - Not in thread_metadata at all (no thread context set yet), OR
/// - At depth 0 (root messages), OR
/// - At depth 1 with `broadcast = true` (replies surfaced to the channel)
///
/// Default ordering is newest-first (DESC). When `since_cursor` is provided
/// without `before_cursor`, ordering flips to oldest-first (ASC) for
/// chronological polling.
///
/// `before_cursor` enables backward keyset pagination (pass the `created_at`
/// of the last item from the previous page). `since_cursor` enables forward
/// polling (returns only messages created after the given timestamp).
pub async fn get_channel_messages_top_level(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    limit: u32,
    before_cursor: Option<DateTime<Utc>>,
    since_cursor: Option<DateTime<Utc>>,
    kind_filter: Option<&[u32]>,
) -> Result<Vec<TopLevelMessage>> {
    let mut param_idx = 3u32; // $1 is community_id, $2 is channel_id
    let mut sql = String::from(
        r#"
        SELECT
            e.id          AS event_id,
            e.pubkey,
            e.tags,
            e.content,
            e.kind,
            e.created_at,
            e.channel_id
        FROM events e
        LEFT JOIN thread_metadata tm
            ON tm.community_id = e.community_id
           AND tm.event_created_at = e.created_at
           AND tm.event_id         = e.id
        WHERE e.community_id = $1
          AND e.channel_id = $2
          AND e.deleted_at IS NULL
          AND (
                tm.depth IS NULL
             OR tm.depth = 0
             OR (tm.depth = 1 AND tm.broadcast = true)
          )
        "#,
    );

    if before_cursor.is_some() {
        sql.push_str(&format!(" AND e.created_at < ${param_idx}"));
        param_idx += 1;
    }

    if since_cursor.is_some() {
        sql.push_str(&format!(" AND e.created_at > ${param_idx}"));
        param_idx += 1;
    }

    if let Some(kinds) = kind_filter {
        if !kinds.is_empty() {
            let list = kinds
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(",");
            sql.push_str(&format!(" AND e.kind IN ({list})"));
        }
    }

    let order = if since_cursor.is_some() && before_cursor.is_none() {
        "ASC"
    } else {
        "DESC"
    };
    sql.push_str(&format!(
        " ORDER BY e.created_at {order} LIMIT ${param_idx}"
    ));

    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(community_id.as_uuid())
        .bind(channel_id);

    if let Some(cursor) = before_cursor {
        q = q.bind(cursor);
    }
    if let Some(cursor) = since_cursor {
        q = q.bind(cursor);
    }
    q = q.bind(limit as i32);

    let rows = q.fetch_all(pool).await?;

    let mut messages = Vec::with_capacity(rows.len());
    for row in rows {
        let event_id: Vec<u8> = row.try_get("event_id")?;
        let pubkey: Vec<u8> = row.try_get("pubkey")?;
        let tags: serde_json::Value = row.try_get("tags")?;
        let content: String = row.try_get("content")?;
        let kind: i32 = row.try_get("kind")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let ch_id: Uuid = row.try_get("channel_id")?;

        messages.push(TopLevelMessage {
            event_id,
            pubkey,
            tags,
            content,
            kind,
            created_at,
            channel_id: ch_id,
            thread_summary: None, // Populated by caller if needed
        });
    }

    Ok(messages)
}

/// Look up a single thread_metadata row by event_id.
///
/// Used when processing soft-deletes to find the parent/root so reply counts
/// can be decremented.
pub async fn get_thread_metadata_by_event(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
) -> Result<Option<ThreadMetadataRecord>> {
    let row = sqlx::query(
        r#"
        SELECT
            event_id,
            event_created_at,
            channel_id,
            parent_event_id,
            root_event_id,
            depth,
            reply_count,
            descendant_count,
            broadcast
        FROM thread_metadata
        WHERE community_id = $1 AND event_id = $2
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(event_id)
    .fetch_optional(pool)
    .await?;

    let row = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    let event_id_col: Vec<u8> = row.try_get("event_id")?;
    let event_created_at: DateTime<Utc> = row.try_get("event_created_at")?;
    let channel_id: Uuid = row.try_get("channel_id")?;
    let parent_event_id: Option<Vec<u8>> = row.try_get("parent_event_id")?;
    let root_event_id: Option<Vec<u8>> = row.try_get("root_event_id")?;
    let depth: i32 = row.try_get("depth")?;
    let reply_count: i32 = row.try_get("reply_count")?;
    let descendant_count: i32 = row.try_get("descendant_count")?;
    let broadcast_val: bool = row.try_get("broadcast")?;

    Ok(Some(ThreadMetadataRecord {
        event_id: event_id_col,
        event_created_at,
        channel_id,
        parent_event_id,
        root_event_id,
        depth,
        reply_count,
        descendant_count,
        broadcast: broadcast_val,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        channel::{ChannelType, ChannelVisibility},
        event::{insert_event_with_thread_metadata, ThreadMetadataParams},
    };
    use nostr::{EventBuilder, Keys, Kind};

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());

        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    fn make_stream_event(keys: &Keys, content: &str) -> nostr::Event {
        EventBuilder::new(Kind::Custom(9), content)
            .sign_with_keys(keys)
            .expect("sign event")
    }

    fn event_created_at(event: &nostr::Event) -> DateTime<Utc> {
        DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
            .expect("event timestamp is valid")
    }

    async fn make_test_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("thread-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        id
    }

    async fn create_test_channel(
        pool: &PgPool,
        name: &str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
        description: Option<&str>,
        created_by: &[u8],
        ttl_seconds: Option<i32>,
    ) -> crate::error::Result<(crate::channel::ChannelRecord, buzz_core::CommunityId)> {
        let id = Uuid::new_v4();
        let community_id = make_test_community(pool).await;

        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, description, created_by, ttl_seconds, ttl_deadline)
            VALUES
                ($1, $2, $3, $4::channel_type, $5::channel_visibility, $6, $7, $8,
                 CASE WHEN $8 IS NOT NULL THEN NOW() + ($8 || ' seconds')::interval ELSE NULL END)
            "#,
        )
        .bind(id)
        .bind(community_id)
        .bind(name)
        .bind(channel_type.as_str())
        .bind(visibility.as_str())
        .bind(description)
        .bind(created_by)
        .bind(ttl_seconds)
        .execute(pool)
        .await
        .expect("insert test channel");

        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
            VALUES ($1, $2, $3, 'owner', $4)
            "#,
        )
        .bind(community_id)
        .bind(id)
        .bind(created_by)
        .bind(created_by)
        .execute(pool)
        .await
        .expect("insert owner membership");

        crate::channel::get_channel(pool, buzz_core::CommunityId::from_uuid(community_id), id)
            .await
            .map(|channel| (channel, buzz_core::CommunityId::from_uuid(community_id)))
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn get_thread_metadata_by_event_is_scoped_when_event_id_collides_across_communities() {
        let pool = setup_pool().await;
        let author = Keys::generate();
        let channel_id = Uuid::new_v4();
        let community_a = make_test_community(&pool).await;
        let community_b = make_test_community(&pool).await;
        let community_a = buzz_core::CommunityId::from_uuid(community_a);
        let community_b = buzz_core::CommunityId::from_uuid(community_b);

        crate::channel::create_channel_with_id(
            &pool,
            community_a,
            channel_id,
            &format!("thread-collision-a-{channel_id}"),
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            author.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create community A channel");
        crate::channel::create_channel_with_id(
            &pool,
            community_b,
            channel_id,
            &format!("thread-collision-b-{channel_id}"),
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            author.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create community B channel");

        let event = make_stream_event(&author, "same id in both communities");
        let created_at = event_created_at(&event);
        insert_event_with_thread_metadata(
            &pool,
            community_a,
            &event,
            Some(channel_id),
            Some(ThreadMetadataParams {
                event_id: event.id.as_bytes(),
                event_created_at: created_at,
                channel_id,
                parent_event_id: None,
                parent_event_created_at: None,
                root_event_id: None,
                root_event_created_at: None,
                depth: 0,
                broadcast: true,
            }),
        )
        .await
        .expect("insert community A metadata");
        insert_event_with_thread_metadata(
            &pool,
            community_b,
            &event,
            Some(channel_id),
            Some(ThreadMetadataParams {
                event_id: event.id.as_bytes(),
                event_created_at: created_at,
                channel_id,
                parent_event_id: None,
                parent_event_created_at: None,
                root_event_id: None,
                root_event_created_at: None,
                depth: 3,
                broadcast: false,
            }),
        )
        .await
        .expect("insert community B metadata");

        let a = get_thread_metadata_by_event(&pool, community_a, event.id.as_bytes())
            .await
            .expect("lookup community A metadata")
            .expect("community A metadata exists");
        let b = get_thread_metadata_by_event(&pool, community_b, event.id.as_bytes())
            .await
            .expect("lookup community B metadata")
            .expect("community B metadata exists");

        assert_eq!(a.depth, 0);
        assert!(a.broadcast);
        assert_eq!(b.depth, 3);
        assert!(!b.broadcast);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn get_thread_replies_reconstructs_stored_events() {
        let pool = setup_pool().await;
        let author = Keys::generate();
        let (channel, community) = create_test_channel(
            &pool,
            &format!("thread-replies-{}", Uuid::new_v4()),
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            author.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create channel");

        let root = make_stream_event(&author, "root");
        let root_created_at = event_created_at(&root);
        insert_event_with_thread_metadata(&pool, community, &root, Some(channel.id), None)
            .await
            .expect("insert root event");

        let reply = make_stream_event(&author, "reply");
        let reply_created_at = event_created_at(&reply);
        let reply_id = reply.id.to_hex();
        insert_event_with_thread_metadata(
            &pool,
            community,
            &reply,
            Some(channel.id),
            Some(ThreadMetadataParams {
                event_id: reply.id.as_bytes(),
                event_created_at: reply_created_at,
                channel_id: channel.id,
                parent_event_id: Some(root.id.as_bytes()),
                parent_event_created_at: Some(root_created_at),
                root_event_id: Some(root.id.as_bytes()),
                root_event_created_at: Some(root_created_at),
                depth: 1,
                broadcast: false,
            }),
        )
        .await
        .expect("insert reply event and metadata");

        let replies = get_thread_replies(&pool, community, root.id.as_bytes(), Some(10), 10, None)
            .await
            .expect("fetch thread replies");

        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].stored_event.event.id.to_hex(), reply_id);
        assert_eq!(replies[0].stored_event.event.content, "reply");
        assert_eq!(replies[0].stored_event.channel_id, Some(channel.id));
        assert_eq!(replies[0].depth, 1);
    }

    /// Replies that share the same `created_at` second (bursty threads are the
    /// common case) must paginate without gaps or duplicates. Before the
    /// composite `(event_created_at, event_id)` keyset, a timestamp-only cursor
    /// advanced past the whole tied second after one page, silently dropping
    /// every tied reply beyond the first page's limit — the "missed messages"
    /// bug this read-path work exists to fix. This pins the tiebreak.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn get_thread_replies_pages_same_second_ties_without_loss() {
        use nostr::Timestamp;

        let pool = setup_pool().await;
        let author = Keys::generate();
        let (channel, community) = create_test_channel(
            &pool,
            &format!("thread-ties-{}", Uuid::new_v4()),
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            author.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create channel");

        let root = make_stream_event(&author, "root");
        let root_created_at = event_created_at(&root);
        insert_event_with_thread_metadata(&pool, community, &root, Some(channel.id), None)
            .await
            .expect("insert root event");

        // Pin every reply to the SAME second so pagination must lean entirely on
        // the event_id tiebreak. Distinct content keeps the ids distinct.
        let tie_secs: u64 = root.created_at.as_secs() + 1;
        let tie_ts = DateTime::from_timestamp(tie_secs as i64, 0).expect("valid timestamp");
        let reply_count = 5usize;
        let mut expected_ids = Vec::with_capacity(reply_count);
        for i in 0..reply_count {
            let reply = EventBuilder::new(Kind::Custom(9), format!("tie-{i}"))
                .custom_created_at(Timestamp::from(tie_secs))
                .sign_with_keys(&author)
                .expect("sign tied reply");
            expected_ids.push(reply.id.as_bytes().to_vec());
            insert_event_with_thread_metadata(
                &pool,
                community,
                &reply,
                Some(channel.id),
                Some(ThreadMetadataParams {
                    event_id: reply.id.as_bytes(),
                    event_created_at: tie_ts,
                    channel_id: channel.id,
                    parent_event_id: Some(root.id.as_bytes()),
                    parent_event_created_at: Some(root_created_at),
                    root_event_id: Some(root.id.as_bytes()),
                    root_event_created_at: Some(root_created_at),
                    depth: 1,
                    broadcast: false,
                }),
            )
            .await
            .expect("insert tied reply");
        }

        // Page with limit=2 across 5 same-second replies. Build the next cursor
        // from the last row's (created_at seconds, event_id) — the exact keyset
        // the bridge derives transparently for the client.
        let page_limit = 2u32;
        let mut collected: Vec<Vec<u8>> = Vec::new();
        let mut cursor: Option<Vec<u8>> = None;
        loop {
            let page = get_thread_replies(
                &pool,
                community,
                root.id.as_bytes(),
                Some(10),
                page_limit,
                cursor.as_deref(),
            )
            .await
            .expect("fetch page");
            if page.is_empty() {
                break;
            }
            let last = page.last().expect("non-empty page");
            let mut next = last.created_at.timestamp().to_be_bytes().to_vec();
            next.extend_from_slice(&last.event_id);
            cursor = Some(next);
            let full = page.len() as u32 == page_limit;
            for reply in page {
                collected.push(reply.event_id);
            }
            if !full {
                break;
            }
        }

        // No gaps, no duplicates: the paged union equals the full tied set.
        assert_eq!(
            collected.len(),
            reply_count,
            "paged {} replies, expected all {}",
            collected.len(),
            reply_count
        );
        let mut unique = collected.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), reply_count, "paging produced duplicates");
        let mut expected_sorted = expected_ids.clone();
        expected_sorted.sort();
        assert_eq!(unique, expected_sorted, "paged set != inserted tied set");
    }

    /// A reply whose stored row can no longer be reconstructed into a
    /// `nostr::Event` (e.g. corrupt signature from out-of-band storage damage)
    /// must be skipped, with the rest of the thread still returned — not
    /// surfaced as a query error that takes down the whole thread read.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn get_thread_replies_skips_unreconstructable_row() {
        let pool = setup_pool().await;
        let author = Keys::generate();
        let (channel, community) = create_test_channel(
            &pool,
            &format!("thread-replies-corrupt-{}", Uuid::new_v4()),
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            author.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create channel");

        let root = make_stream_event(&author, "root");
        let root_created_at = event_created_at(&root);
        insert_event_with_thread_metadata(&pool, community, &root, Some(channel.id), None)
            .await
            .expect("insert root event");

        // Two replies under the same root: one stays valid, one we corrupt.
        let good = make_stream_event(&author, "good");
        let good_id = good.id.to_hex();
        let good_created_at = event_created_at(&good);
        insert_event_with_thread_metadata(
            &pool,
            community,
            &good,
            Some(channel.id),
            Some(ThreadMetadataParams {
                event_id: good.id.as_bytes(),
                event_created_at: good_created_at,
                channel_id: channel.id,
                parent_event_id: Some(root.id.as_bytes()),
                parent_event_created_at: Some(root_created_at),
                root_event_id: Some(root.id.as_bytes()),
                root_event_created_at: Some(root_created_at),
                depth: 1,
                broadcast: false,
            }),
        )
        .await
        .expect("insert good reply");

        let bad = make_stream_event(&author, "bad");
        let bad_created_at = event_created_at(&bad);
        insert_event_with_thread_metadata(
            &pool,
            community,
            &bad,
            Some(channel.id),
            Some(ThreadMetadataParams {
                event_id: bad.id.as_bytes(),
                event_created_at: bad_created_at,
                channel_id: channel.id,
                parent_event_id: Some(root.id.as_bytes()),
                parent_event_created_at: Some(root_created_at),
                root_event_id: Some(root.id.as_bytes()),
                root_event_created_at: Some(root_created_at),
                depth: 1,
                broadcast: false,
            }),
        )
        .await
        .expect("insert bad reply");

        // Corrupt the bad reply's signature in place: truncating the 64-byte
        // sig makes `row_to_stored_event` fail to deserialize the event and
        // return `Ok(None)` — the case the skip-and-continue must handle.
        let rows_changed = sqlx::query("UPDATE events SET sig = $1 WHERE id = $2")
            .bind(vec![0u8; 32])
            .bind(bad.id.as_bytes())
            .execute(&pool)
            .await
            .expect("corrupt bad reply sig")
            .rows_affected();
        assert_eq!(rows_changed, 1, "expected to corrupt exactly one row");

        let replies = get_thread_replies(&pool, community, root.id.as_bytes(), Some(10), 10, None)
            .await
            .expect("fetch thread replies must succeed despite a corrupt row");

        // The corrupt reply is skipped; the valid one survives. The whole
        // query does NOT 500 on a single unreconstructable row.
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].stored_event.event.id.to_hex(), good_id);
        assert_eq!(replies[0].stored_event.event.content, "good");
    }
}
