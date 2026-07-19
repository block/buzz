//! Event storage and retrieval.
//!
//! AUTH events (kind 22242) are never stored — they carry bearer tokens.
//! Ephemeral events (kinds 20000–29999) are never stored — Redis pub/sub only.
//! Deduplication is application-layer: ON CONFLICT DO NOTHING.

use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{PgPool, Postgres, QueryBuilder, Row, Transaction};
use uuid::Uuid;

use buzz_core::kind::{
    event_kind_i32, is_ephemeral, is_parameterized_replaceable, KIND_AUTH, KIND_EVENT_REMINDER,
    KIND_HUDDLE_STARTED,
};
use buzz_core::{CommunityId, StoredEvent};

use crate::error::{DbError, Result};

/// Optional filters for [`query_events`].
#[derive(Debug, Clone)]
pub struct EventQuery {
    /// Server-resolved community scope.
    pub community_id: CommunityId,
    /// Restrict results to this channel.
    pub channel_id: Option<Uuid>,
    /// Restrict results to these kind values (stored as `i32` in Postgres).
    pub kinds: Option<Vec<i32>>,
    /// Restrict results to events from this pubkey.
    pub pubkey: Option<Vec<u8>>,
    /// Return events created at or after this time.
    pub since: Option<DateTime<Utc>>,
    /// Return events created at or before this time.
    pub until: Option<DateTime<Utc>>,
    /// Maximum number of events to return.
    pub limit: Option<i64>,
    /// Number of events to skip (for pagination).
    pub offset: Option<i64>,
    /// Restrict to events with a `p` tag mentioning this hex pubkey.
    /// Joins against `event_mentions` table (indexed).
    pub p_tag_hex: Option<String>,
    /// Restrict to events with this exact `d_tag` value (NIP-33).
    /// Pushed into SQL via the `idx_events_parameterized` index.
    pub d_tag: Option<String>,
    /// Restrict to events with any of these `d_tag` values (multi-value NIP-33 pushdown).
    /// Used when a filter has multiple `#d` values and targets only NIP-33 kinds.
    pub d_tags: Option<Vec<String>>,
    /// Composite keyset cursor: exclude events at or "after" this (created_at, id) pair.
    /// Used with `until` for stable pagination: events where
    /// `created_at < until OR (created_at = until AND id > before_id)`.
    /// When set, `until` must also be set.
    pub before_id: Option<Vec<u8>>,
    /// When true, restricts results to global events (`channel_id IS NULL`).
    /// Use for endpoints that serve non-channel data (e.g. kind:1 notes) to
    /// defensively prevent leaking channel-scoped events if the ingest
    /// invariant (`is_global_only_kind`) ever changes.
    /// Mutually exclusive with `channel_id`.
    pub global_only: bool,
    /// Restrict results to events from any of these pubkeys (multi-author `IN` pushdown).
    pub authors: Option<Vec<Vec<u8>>>,
    /// Restrict results to events with any of these IDs (multi-id `IN` pushdown).
    pub ids: Option<Vec<Vec<u8>>>,
    /// Restrict results to events with an `e` tag referencing any of these event IDs (hex).
    /// Uses JSONB containment (`tags @> ...`) against the `tags` column.
    pub e_tags: Option<Vec<String>>,
    /// Restrict results to events in any of these channels (multi-channel `IN` pushdown).
    /// Used by NIP-45 COUNT to enforce channel access without fetching all rows.
    pub channel_ids: Option<Vec<uuid::Uuid>>,
    /// Override the default limit clamp (1000). Used by COUNT fallback path
    /// which needs to fetch all matching events for post-filter counting.
    /// When None, the default clamp of 1000 applies.
    pub max_limit: Option<i64>,
}

impl EventQuery {
    /// Construct an unconstrained query inside a server-resolved community.
    ///
    /// `community_id` has no safe default. This keeps call sites concise while
    /// making tenant provenance explicit at construction.
    #[must_use]
    pub const fn for_community(community_id: CommunityId) -> Self {
        Self {
            community_id,
            channel_id: None,
            kinds: None,
            pubkey: None,
            since: None,
            until: None,
            limit: None,
            offset: None,
            p_tag_hex: None,
            d_tag: None,
            d_tags: None,
            before_id: None,
            global_only: false,
            authors: None,
            ids: None,
            e_tags: None,
            channel_ids: None,
            max_limit: None,
        }
    }
}

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

/// Maximum length for a `d_tag` value (bytes). NIP-33 d-tags are short identifiers;
/// anything beyond this is either a bug or abuse.
pub const D_TAG_MAX_LEN: usize = 1024;

/// Maximum huddle-start content bytes considered by the parent-link lookup.
///
/// The canonical content is a small JSON object containing one UUID. Rejecting
/// oversized candidates keeps a malformed lifecycle event from making audio
/// admission pull large text rows into memory.
const HUDDLE_LINK_CONTENT_MAX_BYTES: i64 = 512;
/// Maximum candidate rows inspected after SQL prefiltering by parent, creator,
/// kind, and UUID substring.
const HUDDLE_LINK_CANDIDATE_LIMIT: i64 = 32;

/// Extract the `d_tag` value for storage.
///
/// For NIP-33 parameterized replaceable events (kind 30000–39999): returns the first
/// `d` tag's value, or `""` if no `d` tag is present (per NIP-33 spec).
/// For all other events: returns `None` (column stays NULL).
pub fn extract_d_tag(event: &Event) -> Option<String> {
    let kind_u32 = event.kind.as_u16() as u32;
    if !is_parameterized_replaceable(kind_u32) {
        return None;
    }
    let val = event
        .tags
        .iter()
        .find_map(|tag| {
            let parts = tag.as_slice();
            if parts.len() >= 2 && parts[0] == "d" {
                Some(parts[1].to_string())
            } else {
                None
            }
        })
        .unwrap_or_default(); // Missing d tag → empty string per NIP-33
    Some(val)
}

/// Extract the `not_before` timestamp for materialization in the `events` table.
///
/// Only applies to `kind:30300` (NIP-ER event reminders). Returns the first
/// valid `not_before` tag value as an `i64` Unix timestamp, or `None` if the
/// event is not a reminder or has no `not_before` tag.
pub fn extract_not_before(event: &Event) -> Option<i64> {
    let kind_u32 = event.kind.as_u16() as u32;
    if kind_u32 != KIND_EVENT_REMINDER {
        return None;
    }
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        if parts.len() >= 2 && parts[0] == "not_before" {
            parts[1].parse::<i64>().ok()
        } else {
            None
        }
    })
}

fn huddle_started_content_links(content: &str, ephemeral_channel_id: Uuid) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| {
            value
                .get("ephemeral_channel_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok())
        })
        .is_some_and(|id| id == ephemeral_channel_id)
}

/// Return whether `parent_channel_id` has a creator-signed huddle-start event
/// that links to `ephemeral_channel_id`.
///
/// The creator constraint matters: a member of some unrelated channel can post
/// their own kind:48100 event there, but they cannot sign as the creator of the
/// target ephemeral channel.
pub async fn huddle_started_link_exists(
    pool: &PgPool,
    community_id: CommunityId,
    parent_channel_id: Uuid,
    ephemeral_channel_id: Uuid,
    creator_pubkey: &[u8],
) -> Result<bool> {
    let uuid_needle = format!("%{}%", ephemeral_channel_id);
    let candidates: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT content
        FROM events
        WHERE deleted_at IS NULL
          AND community_id = $1
          AND channel_id = $2
          AND kind = $3
          AND pubkey = $4
          AND octet_length(content) <= $5
          AND content ILIKE $6
        ORDER BY created_at DESC, id ASC
        LIMIT $7
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(parent_channel_id)
    .bind(KIND_HUDDLE_STARTED as i32)
    .bind(creator_pubkey)
    .bind(HUDDLE_LINK_CONTENT_MAX_BYTES)
    .bind(uuid_needle)
    .bind(HUDDLE_LINK_CANDIDATE_LIMIT)
    .fetch_all(pool)
    .await?;

    Ok(candidates
        .iter()
        .any(|content| huddle_started_content_links(content, ephemeral_channel_id)))
}

/// Insert a Nostr event. Rejects AUTH and ephemeral kinds.
///
/// Returns `(StoredEvent, was_inserted)` — `was_inserted` is `false` on duplicate.
pub async fn insert_event(
    pool: &PgPool,
    community_id: CommunityId,
    event: &Event,
    channel_id: Option<Uuid>,
) -> Result<(StoredEvent, bool)> {
    let kind_u16 = event.kind.as_u16();
    let kind_u32 = u32::from(kind_u16);

    if kind_u32 == KIND_AUTH {
        return Err(DbError::AuthEventRejected);
    }
    if is_ephemeral(kind_u32) {
        return Err(DbError::EphemeralEventRejected(kind_u16));
    }

    let id_bytes = event.id.as_bytes();
    let pubkey_bytes = event.pubkey.to_bytes();
    let sig_bytes = event.sig.serialize();
    let tags_json = serde_json::to_value(&event.tags)?;
    // Cast chain: nostr Kind (u16) → i32 (Postgres INT column). Safe: all Buzz kinds fit in i32.
    let kind_i32 = event_kind_i32(event);
    let created_at_secs = event.created_at.as_secs() as i64;
    let created_at = DateTime::from_timestamp(created_at_secs, 0)
        .ok_or(DbError::InvalidTimestamp(created_at_secs))?;
    let received_at = Utc::now();
    let d_tag = extract_d_tag(event);
    let not_before = extract_not_before(event);
    let result = sqlx::query(
        r#"
        INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag, not_before)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id_bytes.as_slice())
    .bind(pubkey_bytes.as_slice())
    .bind(created_at)
    .bind(kind_i32)
    .bind(&tags_json)
    .bind(&event.content)
    .bind(sig_bytes.as_slice())
    .bind(received_at)
    .bind(channel_id)
    .bind(d_tag.as_deref())
    .bind(not_before)
    .execute(pool)
    .await?;

    let was_inserted = result.rows_affected() > 0;

    Ok((
        StoredEvent::with_received_at(event.clone(), received_at, channel_id, true),
        was_inserted,
    ))
}

/// Query events with optional filters. Results ordered by `created_at DESC`.
///
/// Uses `QueryBuilder` for dynamic filter composition — avoids string concatenation
/// while keeping all user values in bind parameters.
pub async fn query_events(pool: &PgPool, q: &EventQuery) -> Result<Vec<StoredEvent>> {
    // Composite cursor requires both halves.
    if q.before_id.is_some() && q.until.is_none() {
        return Err(DbError::InvalidData(
            "before_id requires until to be set".to_string(),
        ));
    }

    // global_only and channel_id are mutually exclusive.
    if q.global_only && q.channel_id.is_some() {
        return Err(DbError::InvalidData(
            "global_only and channel_id are mutually exclusive".to_string(),
        ));
    }

    // Empty list means "match nothing" — return empty immediately.
    if q.kinds.as_deref().is_some_and(|k| k.is_empty()) {
        return Ok(vec![]);
    }
    if q.authors.as_deref().is_some_and(|a| a.is_empty()) {
        return Ok(vec![]);
    }
    if q.ids.as_deref().is_some_and(|i| i.is_empty()) {
        return Ok(vec![]);
    }
    if q.e_tags.as_deref().is_some_and(|e| e.is_empty()) {
        return Ok(vec![]);
    }

    let clamp = q.max_limit.unwrap_or(1000);
    let limit_val = q.limit.unwrap_or(100).min(clamp);
    let offset_val = q.offset.unwrap_or(0);

    let mut qb: QueryBuilder<sqlx::Postgres> = if let Some(ref p_hex) = q.p_tag_hex {
        // Join against event_mentions for #p-filtered queries (indexed).
        let mut b = QueryBuilder::new(
            "SELECT e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, \
             e.sig, e.received_at, e.channel_id \
             FROM events e \
             INNER JOIN event_mentions m \
                ON e.community_id = m.community_id AND e.id = m.event_id \
             WHERE e.community_id = ",
        );
        b.push_bind(q.community_id.as_uuid());
        b.push(" AND m.community_id = ");
        b.push_bind(q.community_id.as_uuid());
        b.push(" AND e.deleted_at IS NULL AND m.pubkey_hex = ");
        b.push_bind(p_hex.to_ascii_lowercase());
        b
    } else {
        let mut b = QueryBuilder::new(
            "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
             FROM events WHERE community_id = ",
        );
        b.push_bind(q.community_id.as_uuid());
        b.push(" AND deleted_at IS NULL");
        b
    };

    // Use unqualified column names when no join, qualified when joined.
    let col_prefix = if q.p_tag_hex.is_some() { "e." } else { "" };

    if let Some(ch) = q.channel_id {
        qb.push(format!(" AND {col_prefix}channel_id = "))
            .push_bind(ch);
    } else if q.global_only {
        qb.push(format!(" AND {col_prefix}channel_id IS NULL"));
    }

    // Multi-channel IN pushdown: restrict to events in any of these channels
    // OR global events (channel_id IS NULL). Used by NIP-45 COUNT to enforce
    // channel access at the SQL level without fetching all rows.
    //
    // SECURITY: Some(empty vec) means "user has access to NO channels" —
    // only global events (channel_id IS NULL) should be returned.
    if let Some(ref ch_ids) = q.channel_ids {
        if ch_ids.is_empty() {
            // No channel access — only global (non-channel) events visible.
            qb.push(format!(" AND {col_prefix}channel_id IS NULL"));
        } else {
            qb.push(format!(
                " AND ({col_prefix}channel_id IS NULL OR {col_prefix}channel_id IN ("
            ));
            let mut sep = qb.separated(", ");
            for ch in ch_ids {
                sep.push_bind(*ch);
            }
            qb.push("))");
        }
    }

    if let Some(ks) = q.kinds.as_deref().filter(|k| !k.is_empty()) {
        qb.push(format!(" AND {col_prefix}kind IN ("));
        let mut sep = qb.separated(", ");
        for k in ks {
            sep.push_bind(*k);
        }
        qb.push(")");
    }

    if let Some(ref pk) = q.pubkey {
        qb.push(format!(" AND {col_prefix}pubkey = "))
            .push_bind(pk.clone());
    }

    // Multi-author IN pushdown (mutually exclusive with single pubkey in practice).
    if let Some(ref authors) = q.authors {
        if !authors.is_empty() {
            qb.push(format!(" AND {col_prefix}pubkey IN ("));
            let mut sep = qb.separated(", ");
            for a in authors {
                sep.push_bind(a.clone());
            }
            qb.push(")");
        }
    }

    // Multi-id IN pushdown.
    if let Some(ref ids) = q.ids {
        if !ids.is_empty() {
            qb.push(format!(" AND {col_prefix}id IN ("));
            let mut sep = qb.separated(", ");
            for id in ids {
                sep.push_bind(id.clone());
            }
            qb.push(")");
        }
    }

    // e-tag pushdown via JSONB containment: tags @> '[["e","<hex>"]]'.
    // Multiple e-tags use OR (any match). Served by idx_events_tags_gin
    // (GIN, jsonb_path_ops — migrations/0004): the channel-window aux closure
    // fans this out once per retained row, which made unindexed containment
    // the dominant scroll-back cost (~1.7s/page on staging).
    if let Some(ref e_tags) = q.e_tags {
        if !e_tags.is_empty() {
            qb.push(" AND (");
            for (i, hex_id) in e_tags.iter().enumerate() {
                if i > 0 {
                    qb.push(" OR ");
                }
                // Build the JSONB literal: [["e","<hex>"]]
                let containment = serde_json::json!([["e", hex_id]]);
                qb.push(format!("{col_prefix}tags @> "));
                qb.push_bind(containment);
            }
            qb.push(")");
        }
    }

    if let Some(s) = q.since {
        qb.push(format!(" AND {col_prefix}created_at >= "))
            .push_bind(s);
    }
    if let Some(u) = q.until {
        if let Some(ref bid) = q.before_id {
            // Composite keyset cursor for stable pagination.
            // With ORDER BY created_at DESC, id ASC, "next page" means:
            //   created_at < cursor_ts OR (created_at = cursor_ts AND id > cursor_id)
            qb.push(format!(" AND ({col_prefix}created_at < "));
            qb.push_bind(u);
            qb.push(format!(" OR ({col_prefix}created_at = "));
            qb.push_bind(u);
            qb.push(format!(" AND {col_prefix}id > "));
            qb.push_bind(bid.clone());
            qb.push("))");
        } else {
            qb.push(format!(" AND {col_prefix}created_at <= "))
                .push_bind(u);
        }
    }

    if let Some(ref d) = q.d_tag {
        qb.push(format!(" AND {col_prefix}d_tag = "))
            .push_bind(d.clone());
    } else if let Some(ref ds) = q.d_tags {
        if !ds.is_empty() {
            qb.push(format!(" AND {col_prefix}d_tag IN ("));
            let mut sep = qb.separated(", ");
            for d in ds {
                sep.push_bind(d.clone());
            }
            qb.push(")");
        }
    }

    // Composite ordering for deterministic pagination across ALL callers of
    // query_events (WebSocket REQ, REST endpoints, canvas, notes, etc.).
    // The `id ASC` tiebreaker ensures stable results when events share the
    // same second.  No existing index covers this trailing column — Postgres
    // sorts in memory, which is fine at current scale.  If query performance
    // degrades, add a composite index like `(pubkey, kind, created_at DESC, id ASC)`.
    qb.push(format!(
        " ORDER BY {col_prefix}created_at DESC, {col_prefix}id ASC LIMIT "
    ));
    qb.push_bind(limit_val);
    qb.push(" OFFSET ").push_bind(offset_val);

    let rows = qb.build().fetch_all(pool).await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(ev) = row_to_stored_event(row)? {
            out.push(ev);
        }
    }
    Ok(out)
}

pub(crate) fn row_to_stored_event(row: sqlx::postgres::PgRow) -> Result<Option<StoredEvent>> {
    let id_bytes: Vec<u8> = row.try_get("id")?;
    let pubkey_bytes: Vec<u8> = row.try_get("pubkey")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let kind_i32: i32 = row.try_get("kind")?;
    let tags_json: serde_json::Value = row.try_get("tags")?;
    let content: String = row.try_get("content")?;
    let sig_bytes: Vec<u8> = row.try_get("sig")?;
    let received_at: DateTime<Utc> = row.try_get("received_at")?;

    let channel_id: Option<Uuid> = row.try_get("channel_id")?;

    // kind is stored as i32 (Postgres INT) but Nostr uses u16. Values > 65535 are corrupt.
    let kind_u16 = u16::try_from(kind_i32)
        .map_err(|_| DbError::InvalidData(format!("kind out of u16 range: {kind_i32}")))?;

    let event_json = serde_json::json!({
        "id": hex::encode(&id_bytes),
        "pubkey": hex::encode(&pubkey_bytes),
        "created_at": created_at.timestamp(),
        "kind": kind_u16,
        "tags": tags_json,
        "content": content,
        "sig": hex::encode(&sig_bytes),
    });

    // Avoid the Value → String → parse round-trip: deserialize directly from the Value.
    let event: nostr::Event = match serde_json::from_value(event_json) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("failed to reconstruct event from DB row: {e}");
            return Ok(None);
        }
    };

    Ok(Some(StoredEvent::with_received_at(
        event,
        received_at,
        channel_id,
        true,
    )))
}

/// Count events matching the given query parameters (NIP-45 COUNT support).
///
/// Uses the same filter logic as `query_events` but returns only the count.
pub async fn count_events(pool: &PgPool, q: &EventQuery) -> Result<i64> {
    // Empty list means "match nothing" — return 0 immediately.
    if q.kinds.as_deref().is_some_and(|k| k.is_empty()) {
        return Ok(0);
    }
    if q.authors.as_deref().is_some_and(|a| a.is_empty()) {
        return Ok(0);
    }
    if q.ids.as_deref().is_some_and(|i| i.is_empty()) {
        return Ok(0);
    }
    if q.e_tags.as_deref().is_some_and(|e| e.is_empty()) {
        return Ok(0);
    }

    let mut qb: QueryBuilder<sqlx::Postgres> = if let Some(ref p_hex) = q.p_tag_hex {
        let mut b = QueryBuilder::new(
            "SELECT COUNT(*) as cnt FROM events e \
             INNER JOIN event_mentions m \
                ON e.community_id = m.community_id AND e.id = m.event_id \
             WHERE e.community_id = ",
        );
        b.push_bind(q.community_id.as_uuid());
        b.push(" AND m.community_id = ");
        b.push_bind(q.community_id.as_uuid());
        b.push(" AND e.deleted_at IS NULL AND m.pubkey_hex = ");
        b.push_bind(p_hex.to_ascii_lowercase());
        b
    } else {
        let mut b = QueryBuilder::new("SELECT COUNT(*) as cnt FROM events WHERE community_id = ");
        b.push_bind(q.community_id.as_uuid());
        b.push(" AND deleted_at IS NULL");
        b
    };

    let col_prefix = if q.p_tag_hex.is_some() { "e." } else { "" };

    if let Some(ch) = q.channel_id {
        qb.push(format!(" AND {col_prefix}channel_id = "))
            .push_bind(ch);
    } else if q.global_only {
        qb.push(format!(" AND {col_prefix}channel_id IS NULL"));
    }

    // Multi-channel IN pushdown for COUNT: restrict to accessible channels + global.
    // SECURITY: Some(empty vec) = no channel access → global events only.
    if let Some(ref ch_ids) = q.channel_ids {
        if ch_ids.is_empty() {
            qb.push(format!(" AND {col_prefix}channel_id IS NULL"));
        } else {
            qb.push(format!(
                " AND ({col_prefix}channel_id IS NULL OR {col_prefix}channel_id IN ("
            ));
            let mut sep = qb.separated(", ");
            for ch in ch_ids {
                sep.push_bind(*ch);
            }
            qb.push("))");
        }
    }

    if let Some(ks) = q.kinds.as_deref().filter(|k| !k.is_empty()) {
        qb.push(format!(" AND {col_prefix}kind IN ("));
        let mut sep = qb.separated(", ");
        for k in ks {
            sep.push_bind(*k);
        }
        qb.push(")");
    }

    if let Some(ref pk) = q.pubkey {
        qb.push(format!(" AND {col_prefix}pubkey = "))
            .push_bind(pk.clone());
    }

    if let Some(ref authors) = q.authors {
        if !authors.is_empty() {
            qb.push(format!(" AND {col_prefix}pubkey IN ("));
            let mut sep = qb.separated(", ");
            for a in authors {
                sep.push_bind(a.clone());
            }
            qb.push(")");
        }
    }

    if let Some(ref ids) = q.ids {
        if !ids.is_empty() {
            qb.push(format!(" AND {col_prefix}id IN ("));
            let mut sep = qb.separated(", ");
            for id in ids {
                sep.push_bind(id.clone());
            }
            qb.push(")");
        }
    }

    if let Some(ref e_tags) = q.e_tags {
        if !e_tags.is_empty() {
            qb.push(" AND (");
            for (i, hex_id) in e_tags.iter().enumerate() {
                if i > 0 {
                    qb.push(" OR ");
                }
                let containment = serde_json::json!([["e", hex_id]]);
                qb.push(format!("{col_prefix}tags @> "));
                qb.push_bind(containment);
            }
            qb.push(")");
        }
    }

    if let Some(s) = q.since {
        qb.push(format!(" AND {col_prefix}created_at >= "))
            .push_bind(s);
    }
    if let Some(u) = q.until {
        qb.push(format!(" AND {col_prefix}created_at <= "))
            .push_bind(u);
    }

    if let Some(ref d) = q.d_tag {
        qb.push(format!(" AND {col_prefix}d_tag = "))
            .push_bind(d.clone());
    } else if let Some(ref ds) = q.d_tags {
        if !ds.is_empty() {
            qb.push(format!(" AND {col_prefix}d_tag IN ("));
            let mut sep = qb.separated(", ");
            for d in ds {
                sep.push_bind(d.clone());
            }
            qb.push(")");
        }
    }

    let row = qb.build().fetch_one(pool).await?;
    let cnt: i64 = row.try_get("cnt")?;

    Ok(cnt)
}

/// Soft-delete an event by setting `deleted_at = NOW()`.
///
/// Returns `Ok(true)` if the event was deleted, `Ok(false)` if already deleted
/// or not found. Callers are responsible for decrementing thread reply counts
/// when the deleted event is a thread reply.
pub async fn soft_delete_event(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
            .bind(community_id.as_uuid())
            .bind(event_id)
            .execute(pool)
            .await?;

    Ok(result.rows_affected() > 0)
}

/// Soft-delete the live row for an addressable coordinate
/// `(kind, pubkey, d_tag)` — the NIP-33 replacement key.
///
/// Used by `handle_a_tag_deletion` to honour NIP-09 a-tag deletions for any
/// parameterized-replaceable kind. The WHERE clause mirrors
/// `replace_parameterized_event` so the coordinate semantics stay consistent:
/// `channel_id` is intentionally NOT in the key (NIP-33 replacement is global
/// per the spec — `channel_id` is stored for query scoping, not identity).
///
/// Returns `Ok(true)` if a row was deleted, `Ok(false)` if no live row matched
/// (already deleted, or never existed).
pub async fn soft_delete_by_coordinate(
    pool: &PgPool,
    community_id: CommunityId,
    kind: i32,
    pubkey: &[u8],
    d_tag: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE events SET deleted_at = NOW() \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(kind)
    .bind(pubkey)
    .bind(d_tag)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Atomically soft-delete an event and decrement thread reply counters.
///
/// Wraps the delete + counter update in a single transaction so a crash between
/// them cannot leave counters permanently inflated. Returns `Ok(true)` if the
/// event was deleted this call.
pub async fn soft_delete_event_and_update_thread(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
    parent_event_id: Option<&[u8]>,
    root_event_id: Option<&[u8]>,
) -> Result<bool> {
    let mut tx = pool.begin().await?;

    let result = sqlx::query(
        "UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(event_id)
    .execute(&mut *tx)
    .await?;

    let deleted = result.rows_affected() > 0;

    if deleted {
        if let Some(pid) = parent_event_id {
            sqlx::query(
                "UPDATE thread_metadata \
                 SET reply_count = GREATEST(reply_count - 1, 0) \
                 WHERE community_id = $1 AND event_id = $2",
            )
            .bind(community_id.as_uuid())
            .bind(pid)
            .execute(&mut *tx)
            .await?;

            if let Some(root_id) = root_event_id {
                sqlx::query(
                    "UPDATE thread_metadata \
                     SET descendant_count = GREATEST(descendant_count - 1, 0) \
                     WHERE community_id = $1 AND event_id = $2",
                )
                .bind(community_id.as_uuid())
                .bind(root_id)
                .execute(&mut *tx)
                .await?;
            }
        }
    }

    tx.commit().await?;
    Ok(deleted)
}

/// Returns the `created_at` timestamp of the most recent non-deleted event in a channel.
pub async fn get_last_message_at(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: uuid::Uuid,
) -> Result<Option<DateTime<Utc>>> {
    let row = sqlx::query(
        "SELECT created_at FROM events \
         WHERE community_id = $1 AND channel_id = $2 AND deleted_at IS NULL \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => Ok(Some(r.try_get("created_at")?)),
        None => Ok(None),
    }
}

/// Bulk-fetch the most recent `created_at` for a set of channel IDs.
///
/// Returns a map of `channel_id → last_message_at`. Channels with no events are omitted.
/// Single query regardless of input size.
pub async fn get_last_message_at_bulk(
    pool: &PgPool,
    community_id: CommunityId,
    channel_ids: &[uuid::Uuid],
) -> Result<std::collections::HashMap<uuid::Uuid, DateTime<Utc>>> {
    if channel_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "SELECT channel_id, MAX(created_at) as last_at FROM events \
         WHERE community_id = ",
    );
    qb.push_bind(community_id.as_uuid());
    qb.push(" AND deleted_at IS NULL AND channel_id IN (");
    let mut sep = qb.separated(", ");
    for id in channel_ids {
        sep.push_bind(*id);
    }
    qb.push(") GROUP BY channel_id");

    let rows = qb.build().fetch_all(pool).await?;

    let mut map = std::collections::HashMap::with_capacity(rows.len());
    for row in rows {
        let id: Uuid = row.try_get("channel_id")?;
        let last_at: DateTime<Utc> = row.try_get("last_at")?;
        map.insert(id, last_at);
    }
    Ok(map)
}

/// Fetches a single non-deleted event by its raw 32-byte ID.
///
/// Returns `None` if the event does not exist or has been soft-deleted.
/// Use [`get_event_by_id_including_deleted`] when you need to inspect
/// tombstoned rows (e.g. audit, undelete).
pub async fn get_event_by_id(
    pool: &PgPool,
    community_id: CommunityId,
    id_bytes: &[u8],
) -> Result<Option<StoredEvent>> {
    let row = sqlx::query(
        "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
         FROM events WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(id_bytes)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => row_to_stored_event(r),
        None => Ok(None),
    }
}

/// Fetches the latest global (non-channel, `channel_id IS NULL`) replaceable event
/// for a (kind, pubkey) pair.
///
/// Uses canonical NIP-16 ordering: `created_at DESC, id ASC LIMIT 1`.
/// This matches the write path's tie-breaking logic and handles historical
/// duplicate survivors where multiple live rows share the same timestamp.
pub async fn get_latest_global_replaceable(
    pool: &PgPool,
    community_id: CommunityId,
    kind: i32,
    pubkey_bytes: &[u8],
) -> Result<Option<StoredEvent>> {
    let row = sqlx::query(
        "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
         FROM events \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND channel_id IS NULL AND deleted_at IS NULL \
         ORDER BY created_at DESC, id ASC \
         LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(kind)
    .bind(pubkey_bytes)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => row_to_stored_event(r),
        None => Ok(None),
    }
}

/// Fetches a single event by its raw 32-byte ID, **including soft-deleted rows**.
///
/// Most callers should use [`get_event_by_id`] instead. This variant is needed
/// when the caller must distinguish "never existed" from "was deleted" (e.g.
/// audit trails, compliance queries).
pub async fn get_event_by_id_including_deleted(
    pool: &PgPool,
    community_id: CommunityId,
    id_bytes: &[u8],
) -> Result<Option<StoredEvent>> {
    let row = sqlx::query(
        "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
         FROM events WHERE community_id = $1 AND id = $2 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(id_bytes)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => row_to_stored_event(r),
        None => Ok(None),
    }
}

/// Batch-fetch non-deleted events by their raw 32-byte IDs.
///
/// Returns events in arbitrary order — callers reorder as needed.
/// Uses a single `WHERE id IN (...)` query regardless of input size.
pub async fn get_events_by_ids(
    pool: &PgPool,
    community_id: CommunityId,
    ids: &[&[u8]],
) -> Result<Vec<StoredEvent>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    debug_assert!(ids.len() <= 500, "batch fetch should be bounded by caller");

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
         FROM events WHERE community_id = ",
    );
    qb.push_bind(community_id.as_uuid());
    qb.push(" AND deleted_at IS NULL AND id IN (");
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(id.to_vec());
    }
    qb.push(")");

    let rows = qb.build().fetch_all(pool).await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(ev) = row_to_stored_event(row)? {
            out.push(ev);
        }
    }
    Ok(out)
}

/// Parameters for [`insert_event_with_thread_metadata`].
#[derive(Debug)]
pub struct ThreadMetadataParams<'a> {
    /// The Nostr event ID of this message.
    pub event_id: &'a [u8],
    /// When the event was created.
    pub event_created_at: DateTime<Utc>,
    /// The channel this event belongs to.
    pub channel_id: Uuid,
    /// Event ID of the direct parent, if this is a reply.
    pub parent_event_id: Option<&'a [u8]>,
    /// When the parent event was created.
    pub parent_event_created_at: Option<DateTime<Utc>>,
    /// Event ID of the thread root, if this is a nested reply.
    pub root_event_id: Option<&'a [u8]>,
    /// When the root event was created.
    pub root_event_created_at: Option<DateTime<Utc>>,
    /// Nesting depth (root = 0).
    pub depth: i32,
    /// Whether this reply is broadcast to the channel timeline.
    pub broadcast: bool,
}

async fn insert_event_with_thread_metadata_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &Event,
    channel_id: Option<Uuid>,
    thread_meta: Option<ThreadMetadataParams<'_>>,
) -> Result<(StoredEvent, bool)> {
    let kind_u16 = event.kind.as_u16();
    let kind_u32 = u32::from(kind_u16);

    if kind_u32 == KIND_AUTH {
        return Err(DbError::AuthEventRejected);
    }
    if is_ephemeral(kind_u32) {
        return Err(DbError::EphemeralEventRejected(kind_u16));
    }

    let id_bytes = event.id.as_bytes();
    let pubkey_bytes = event.pubkey.to_bytes();
    let sig_bytes = event.sig.serialize();
    let tags_json = serde_json::to_value(&event.tags)?;
    let kind_i32 = event_kind_i32(event);
    let created_at_secs = event.created_at.as_secs() as i64;
    let created_at = DateTime::from_timestamp(created_at_secs, 0)
        .ok_or(DbError::InvalidTimestamp(created_at_secs))?;
    let received_at = Utc::now();
    let d_tag = extract_d_tag(event);
    let not_before = extract_not_before(event);

    let result = sqlx::query(
        r#"
        INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag, not_before)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id_bytes.as_slice())
    .bind(pubkey_bytes.as_slice())
    .bind(created_at)
    .bind(kind_i32)
    .bind(&tags_json)
    .bind(&event.content)
    .bind(sig_bytes.as_slice())
    .bind(received_at)
    .bind(channel_id)
    .bind(d_tag.as_deref())
    .bind(not_before)
    .execute(&mut **tx)
    .await?;

    let was_inserted = result.rows_affected() > 0;

    if was_inserted {
        if let Some(ref meta) = thread_meta {
            let broadcast_val: bool = meta.broadcast;

            let tm_result = sqlx::query(
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
            .bind(meta.event_created_at)
            .bind(meta.event_id)
            .bind(meta.channel_id)
            .bind(meta.parent_event_id)
            .bind(meta.parent_event_created_at)
            .bind(meta.root_event_id)
            .bind(meta.root_event_created_at)
            .bind(meta.depth)
            .bind(broadcast_val)
            .execute(&mut **tx)
            .await?;

            // Only bump reply counts if the metadata row was actually inserted.
            if tm_result.rows_affected() > 0 {
                if let Some(pid) = meta.parent_event_id {
                    // Ensure the parent has a thread_metadata row so the UPDATE
                    // below has something to hit. Root (depth=0) messages don't
                    // get a row on first insert, so we create a stub here.
                    let parent_ts = meta
                        .parent_event_created_at
                        .unwrap_or(meta.event_created_at);
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
                    .bind(meta.channel_id)
                    .execute(&mut **tx)
                    .await?;

                    // Ensure the root also has a row (may differ from parent for nested replies).
                    if let Some(root_id) = meta.root_event_id {
                        if root_id != pid {
                            let root_ts =
                                meta.root_event_created_at.unwrap_or(meta.event_created_at);
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
                            .bind(root_ts)
                            .bind(root_id)
                            .bind(meta.channel_id)
                            .execute(&mut **tx)
                            .await?;
                        }
                    }

                    sqlx::query(
                        r#"
                        UPDATE thread_metadata
                        SET reply_count = reply_count + 1, last_reply_at = NOW()
                        WHERE community_id = $1 AND event_id = $2
                        "#,
                    )
                    .bind(community_id.as_uuid())
                    .bind(pid)
                    .execute(&mut **tx)
                    .await?;

                    if let Some(root_id) = meta.root_event_id {
                        sqlx::query(
                            r#"
                            UPDATE thread_metadata
                            SET descendant_count = descendant_count + 1
                            WHERE community_id = $1 AND event_id = $2
                            "#,
                        )
                        .bind(community_id.as_uuid())
                        .bind(root_id)
                        .execute(&mut **tx)
                        .await?;
                    }
                }
            }
        }
    }

    Ok((
        StoredEvent::with_received_at(event.clone(), received_at, channel_id, true),
        was_inserted,
    ))
}

/// Atomically insert an event and its optional thread metadata.
///
/// `insert_event` and `insert_thread_metadata` calls could leave reply counters
/// inconsistent if one succeeded and the other failed. Keep this as one
/// transaction so reply metadata and counters commit together with the event.
///
/// Returns `(StoredEvent, was_inserted)`.
pub async fn insert_event_with_thread_metadata(
    pool: &PgPool,
    community_id: CommunityId,
    event: &Event,
    channel_id: Option<Uuid>,
    thread_meta: Option<ThreadMetadataParams<'_>>,
) -> Result<(StoredEvent, bool)> {
    let mut tx = pool.begin().await?;
    let result =
        insert_event_with_thread_metadata_tx(&mut tx, community_id, event, channel_id, thread_meta)
            .await?;
    tx.commit().await?;
    Ok(result)
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
    let mut tx = pool.begin().await?;

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
    let reaction_inserted = crate::reaction::add_reaction_tx(
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

/// A due reminder row returned by [`query_due_reminders`].
#[derive(Debug)]
pub struct DueReminder {
    /// Server-resolved community this reminder row belongs to.
    pub community_id: CommunityId,
    /// Normalized host mapped to that community.
    pub host: String,
    /// The event's raw ID bytes.
    pub id: Vec<u8>,
    /// The event's pubkey bytes.
    pub pubkey: Vec<u8>,
    /// The event's `created_at` timestamp.
    pub created_at: DateTime<Utc>,
    /// The event's kind (always 30300).
    pub kind: i32,
    /// The event's JSONB tags.
    pub tags: serde_json::Value,
    /// The event's encrypted content.
    pub content: String,
    /// The event's signature bytes.
    pub sig: Vec<u8>,
    /// The channel ID (always None for reminders — global events).
    pub channel_id: Option<Uuid>,
}

/// Query due reminders: latest-per-address `kind:30300` rows where
/// `not_before <= now`, `deleted_at IS NULL`, `delivered_at IS NULL`.
///
/// Returns the latest head per `(pubkey, d_tag)` using canonical NIP-16
/// ordering (`created_at DESC, id ASC`).
pub async fn query_due_reminders(
    pool: &PgPool,
    now_secs: i64,
    batch_limit: i64,
) -> Result<Vec<DueReminder>> {
    let kind_i32 = KIND_EVENT_REMINDER as i32;
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT ON (e.community_id, e.pubkey, e.d_tag)
            e.community_id, c.host, e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, e.sig, e.channel_id
        FROM events AS e
        JOIN communities AS c ON c.id = e.community_id
        WHERE e.kind = $1
          AND e.not_before IS NOT NULL
          AND e.not_before <= $2
          AND e.deleted_at IS NULL
          AND e.delivered_at IS NULL
          AND c.archived_at IS NULL
        ORDER BY e.community_id, e.pubkey, e.d_tag, e.created_at DESC, e.id ASC
        LIMIT $3
        "#,
    )
    .bind(kind_i32)
    .bind(now_secs)
    .bind(batch_limit)
    .fetch_all(pool)
    .await?;

    let results = rows
        .into_iter()
        .map(|row| DueReminder {
            community_id: CommunityId::from_uuid(row.get("community_id")),
            host: row.get("host"),
            id: row.get("id"),
            pubkey: row.get("pubkey"),
            created_at: row.get("created_at"),
            kind: row.get("kind"),
            tags: row.get("tags"),
            content: row.get("content"),
            sig: row.get("sig"),
            channel_id: row.get("channel_id"),
        })
        .collect();

    Ok(results)
}

/// Atomically claim a due reminder for delivery. Returns `Some(id)` if this
/// caller won the claim (set `delivered_at`), or `None` if another pod already
/// claimed it. Mirrors the reaper's `archived_at IS NULL` guard for cross-pod
/// idempotency.
pub async fn claim_due_reminder(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
) -> Result<bool> {
    claim_due_reminder_with_stamp(
        pool,
        community_id,
        event_id,
        event_created_at,
        Utc::now().timestamp(),
    )
    .await
}

/// Atomically claim a due reminder using a caller-supplied delivery stamp.
///
/// The same stamp should be passed to [`release_due_reminder`] if the publish
/// side effect fails, so rollback can compare-and-clear only this pod's claim.
///
/// Scoped by `community_id`: `events` is keyed `(community_id, created_at, id)`,
/// and the same Nostr event id (hence the same `id`/`created_at` pair) is
/// allowed across communities. Without the community predicate a claim for
/// `A/X` would also mark `B/X` delivered. The caller already holds the owning
/// community on the `DueReminder` row.
pub async fn claim_due_reminder_with_stamp(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    delivery_stamp: i64,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE events
        SET delivered_at = $1
        WHERE community_id = $2 AND created_at = $3 AND id = $4 AND delivered_at IS NULL
        "#,
    )
    .bind(delivery_stamp)
    .bind(community_id.as_uuid())
    .bind(event_created_at)
    .bind(event_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Release a previously claimed reminder when publish fails.
///
/// The `delivery_stamp` must be the exact value written by the claiming pod;
/// that compare-and-clear prevents one pod from rolling back another pod's
/// later claim after a retry/race.
///
/// Scoped by `community_id` for the same reason as the claim: a release for
/// `A/X` must not clear `B/X` even when their `id`/`created_at`/stamp coincide.
pub async fn release_due_reminder(
    pool: &PgPool,
    community_id: CommunityId,
    event_id: &[u8],
    event_created_at: DateTime<Utc>,
    delivery_stamp: i64,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE events
        SET delivered_at = NULL
        WHERE community_id = $1
          AND created_at = $2
          AND id = $3
          AND delivered_at = $4
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(event_created_at)
    .bind(event_id)
    .bind(delivery_stamp)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

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
        let host = format!("event-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        id
    }

    async fn make_test_channel(
        pool: &PgPool,
        community_id: Uuid,
        ttl_seconds: Option<i32>,
    ) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO channels \
             (id, community_id, name, created_by, ttl_seconds, ttl_deadline) \
             VALUES ($1, $2, $3, $4, $5, \
                     CASE WHEN $5 IS NULL THEN NULL \
                          ELSE clock_timestamp() + make_interval(secs => $5) END)",
        )
        .bind(id)
        .bind(community_id)
        .bind(format!("event-ttl-test-{}", id.simple()))
        .bind(vec![7_u8; 32])
        .bind(ttl_seconds)
        .execute(pool)
        .await
        .expect("insert test channel");
        id
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn event_insert_ttl_trigger_handles_permanent_ephemeral_duplicate_and_activation_race() {
        let pool = setup_pool().await;
        let community_uuid = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_uuid);

        let permanent = make_test_channel(&pool, community_uuid, None).await;
        let permanent_event = make_text_event("permanent channel event");
        assert!(
            insert_event(&pool, community, &permanent_event, Some(permanent))
                .await
                .expect("insert permanent event")
                .1
        );
        let permanent_deadline: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT ttl_deadline FROM channels WHERE community_id = $1 AND id = $2",
        )
        .bind(community_uuid)
        .bind(permanent)
        .fetch_one(&pool)
        .await
        .expect("read permanent deadline");
        assert_eq!(permanent_deadline, None);

        let ephemeral = make_test_channel(&pool, community_uuid, Some(60)).await;
        let initial_deadline: DateTime<Utc> = sqlx::query_scalar(
            "SELECT ttl_deadline FROM channels WHERE community_id = $1 AND id = $2",
        )
        .bind(community_uuid)
        .bind(ephemeral)
        .fetch_one(&pool)
        .await
        .expect("read initial ephemeral deadline");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let ephemeral_event = make_text_event("ephemeral channel event");
        assert!(
            insert_event(&pool, community, &ephemeral_event, Some(ephemeral))
                .await
                .expect("insert ephemeral event")
                .1
        );
        let bumped_deadline: DateTime<Utc> = sqlx::query_scalar(
            "SELECT ttl_deadline FROM channels WHERE community_id = $1 AND id = $2",
        )
        .bind(community_uuid)
        .bind(ephemeral)
        .fetch_one(&pool)
        .await
        .expect("read bumped ephemeral deadline");
        assert!(bumped_deadline > initial_deadline);
        assert!(
            !insert_event(&pool, community, &ephemeral_event, Some(ephemeral))
                .await
                .expect("insert duplicate event")
                .1
        );
        let duplicate_deadline: DateTime<Utc> = sqlx::query_scalar(
            "SELECT ttl_deadline FROM channels WHERE community_id = $1 AND id = $2",
        )
        .bind(community_uuid)
        .bind(ephemeral)
        .fetch_one(&pool)
        .await
        .expect("read deadline after duplicate");
        assert_eq!(duplicate_deadline, bumped_deadline);

        // Reproduce the blocked stale-prefetch ordering: ingest has already
        // observed a permanent channel, then TTL activation locks/updates the
        // row before the event INSERT reaches its trigger. The trigger must
        // wait and refresh from the later event after activation commits.
        let racing = make_test_channel(&pool, community_uuid, None).await;
        let stale_ttl: Option<i32> = sqlx::query_scalar(
            "SELECT ttl_seconds FROM channels WHERE community_id = $1 AND id = $2",
        )
        .bind(community_uuid)
        .bind(racing)
        .fetch_one(&pool)
        .await
        .expect("prefetch permanent channel");
        assert_eq!(stale_ttl, None);

        let mut activation = pool.begin().await.expect("begin TTL activation");
        // Model the repaired update_channel protocol (migration 0024): the
        // TTL transition holds the per-channel advisory key EXCLUSIVE, which
        // is what the event trigger's shared acquisition now waits on.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("buzz_channel_ttl:{community_uuid}:{racing}"))
            .execute(&mut *activation)
            .await
            .expect("acquire exclusive channel TTL key");
        let activation_deadline: DateTime<Utc> = sqlx::query_scalar(
            "UPDATE channels \
             SET ttl_seconds = 60, ttl_deadline = clock_timestamp() + interval '60 seconds' \
             WHERE community_id = $1 AND id = $2 RETURNING ttl_deadline",
        )
        .bind(community_uuid)
        .bind(racing)
        .fetch_one(&mut *activation)
        .await
        .expect("activate TTL while holding channel row lock");

        let race_pool = pool.clone();
        let racing_event = make_text_event("event after stale permanent prefetch");
        let insert = tokio::spawn(async move {
            insert_event(&race_pool, community, &racing_event, Some(racing)).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !insert.is_finished(),
            "event trigger must wait on TTL activation"
        );
        activation.commit().await.expect("commit TTL activation");
        assert!(
            insert
                .await
                .expect("join racing insert")
                .expect("racing insert")
                .1
        );

        let final_deadline: DateTime<Utc> = sqlx::query_scalar(
            "SELECT ttl_deadline FROM channels WHERE community_id = $1 AND id = $2",
        )
        .bind(community_uuid)
        .bind(racing)
        .fetch_one(&pool)
        .await
        .expect("read deadline after racing event");
        assert!(
            final_deadline > activation_deadline + chrono::Duration::milliseconds(50),
            "later event must extend TTL beyond activation deadline: activation={activation_deadline}, final={final_deadline}"
        );
    }

    /// T1a repair regression test (migration 0024): permanent-channel event
    /// commits must not serialize on the channel row. The 0022 trigger took
    /// `FOR UPDATE` on the channel tuple before testing `ttl_seconds`, so
    /// concurrent commits into one hot permanent channel queued at commit
    /// time (deferred trigger) — invisible to any single-connection test.
    /// This holds N insert transactions at a barrier past their INSERTs,
    /// then proves (a) while all N sit pre-commit, no transaction holds a
    /// row-level lock on the channel tuple, and (b) all N commits succeed
    /// with the channel row untouched (permanent ⇒ no deadline write).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn permanent_channel_event_commits_do_not_lock_the_channel_row() {
        const N: usize = 8;
        // setup_pool's default cap (10) covers N held transactions plus the
        // pg_locks inspector connection.
        let pool = setup_pool().await;
        let community_uuid = make_test_community(&pool).await;
        let channel = make_test_channel(&pool, community_uuid, None).await;

        // Open N transactions, run the full event INSERT in each (the deferred
        // trigger fires at COMMIT), and park them at a barrier.
        let mut txs = Vec::new();
        for i in 0..N {
            let mut tx = pool.begin().await.expect("begin insert txn");
            let event = make_text_event(&format!("hot channel event {i}"));
            sqlx::query(
                "INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id) \
                 VALUES ($1,$2,$3,$4,9,$5,$6,$7,now(),$8)",
            )
            .bind(community_uuid)
            .bind(event.id.as_bytes().as_slice())
            .bind(event.pubkey.as_bytes().as_slice())
            .bind(DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).unwrap())
            .bind(serde_json::to_value(&event.tags).unwrap())
            .bind(&event.content)
            .bind(event.sig.serialize().as_slice())
            .bind(channel)
            .execute(&mut *tx)
            .await
            .expect("insert event inside held txn");
            txs.push(tx);
        }

        // With all N transactions holding completed INSERTs, none may hold a
        // row-level lock on the channels tuple. (The 0022 trigger would not
        // have taken it yet either — it locks at COMMIT — so also verify the
        // commit phase below completes without mutual blocking.)
        let tuple_locks: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks l \
             JOIN pg_class c ON c.oid = l.relation \
             WHERE c.relname = 'channels' AND l.locktype = 'tuple'",
        )
        .fetch_one(&pool)
        .await
        .expect("inspect pg_locks");
        assert_eq!(tuple_locks, 0, "no channel tuple locks while txns are held");

        // Release all commits concurrently. Under 0022 these serialized on the
        // channel row (each holding it across its WAL flush); under 0024 the
        // shared advisory key admits them all. Join with a timeout so a
        // regression fails fast instead of hanging the suite.
        let commits = txs
            .into_iter()
            .map(|tx| tokio::spawn(async move { tx.commit().await }))
            .collect::<Vec<_>>();
        for c in commits {
            tokio::time::timeout(std::time::Duration::from_secs(10), c)
                .await
                .expect("concurrent permanent-channel commits must not block")
                .expect("join commit task")
                .expect("commit succeeds");
        }

        let deadline: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT ttl_deadline FROM channels WHERE community_id = $1 AND id = $2",
        )
        .bind(community_uuid)
        .bind(channel)
        .fetch_one(&pool)
        .await
        .expect("read deadline after commits");
        assert_eq!(deadline, None, "permanent channel must remain untouched");
        let stored: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE community_id = $1 AND channel_id = $2",
        )
        .bind(community_uuid)
        .bind(channel)
        .fetch_one(&pool)
        .await
        .expect("count stored events");
        assert_eq!(stored as usize, N);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn get_event_by_id_is_scoped_when_event_id_collides_across_communities() {
        let pool = setup_pool().await;
        let community_a = CommunityId::from_uuid(make_test_community(&pool).await);
        let community_b = CommunityId::from_uuid(make_test_community(&pool).await);
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "same signed event")
            .sign_with_keys(&keys)
            .expect("sign event");

        insert_event(&pool, community_a, &event, None)
            .await
            .expect("insert in community A");
        insert_event(&pool, community_b, &event, None)
            .await
            .expect("insert same event in community B");

        sqlx::query("UPDATE events SET content = $1 WHERE community_id = $2 AND id = $3")
            .bind("community-a-copy")
            .bind(community_a.as_uuid())
            .bind(event.id.as_bytes())
            .execute(&pool)
            .await
            .expect("mark community A row");
        sqlx::query("UPDATE events SET content = $1 WHERE community_id = $2 AND id = $3")
            .bind("community-b-copy")
            .bind(community_b.as_uuid())
            .bind(event.id.as_bytes())
            .execute(&pool)
            .await
            .expect("mark community B row");

        let a = get_event_by_id(&pool, community_a, event.id.as_bytes())
            .await
            .expect("lookup community A")
            .expect("community A row exists");
        let b = get_event_by_id(&pool, community_b, event.id.as_bytes())
            .await
            .expect("lookup community B")
            .expect("community B row exists");

        assert_eq!(a.event.content, "community-a-copy");
        assert_eq!(b.event.content, "community-b-copy");
    }

    fn make_event_with_kind_and_tags(kind: u16, tags: Vec<Tag>) -> nostr::Event {
        let keys = Keys::generate();
        EventBuilder::new(Kind::Custom(kind), "test")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign")
    }

    fn make_text_event(content: &str) -> nostr::Event {
        let keys = Keys::generate();
        EventBuilder::new(Kind::Custom(9), content)
            .sign_with_keys(&keys)
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

    #[test]
    fn extract_d_tag_from_nip33_event() {
        let event = make_event_with_kind_and_tags(
            30023,
            vec![Tag::parse(["d", "my-article-slug"]).unwrap()],
        );
        assert_eq!(extract_d_tag(&event), Some("my-article-slug".to_string()));
    }

    #[test]
    fn extract_d_tag_first_d_wins() {
        let event = make_event_with_kind_and_tags(
            30023,
            vec![
                Tag::parse(["d", "first"]).unwrap(),
                Tag::parse(["d", "second"]).unwrap(),
            ],
        );
        assert_eq!(extract_d_tag(&event), Some("first".to_string()));
    }

    #[test]
    fn extract_d_tag_missing_becomes_empty_string() {
        // NIP-33: "if there is no d tag, the d tag is considered to be ''"
        let event =
            make_event_with_kind_and_tags(30023, vec![Tag::parse(["p", "abc123"]).unwrap()]);
        assert_eq!(extract_d_tag(&event), Some(String::new()));
    }

    #[test]
    fn extract_d_tag_empty_value_preserved() {
        let event = make_event_with_kind_and_tags(30023, vec![Tag::parse(["d", ""]).unwrap()]);
        assert_eq!(extract_d_tag(&event), Some(String::new()));
    }

    #[test]
    fn extract_d_tag_non_nip33_returns_none() {
        // kind:1 (text note) — not parameterized replaceable
        let event =
            make_event_with_kind_and_tags(1, vec![Tag::parse(["d", "should-be-ignored"]).unwrap()]);
        assert_eq!(extract_d_tag(&event), None);
    }

    #[test]
    fn extract_d_tag_nip29_group_metadata() {
        // kind:39000 is in the 30000–39999 range — d_tag should be extracted
        let event =
            make_event_with_kind_and_tags(39000, vec![Tag::parse(["d", "group-id"]).unwrap()]);
        assert_eq!(extract_d_tag(&event), Some("group-id".to_string()));
    }

    #[test]
    fn extract_d_tag_boundary_kinds() {
        // kind:29999 — just below range
        let below = make_event_with_kind_and_tags(29999, vec![Tag::parse(["d", "val"]).unwrap()]);
        assert_eq!(extract_d_tag(&below), None);

        // kind:30000 — lower bound
        let lower = make_event_with_kind_and_tags(30000, vec![Tag::parse(["d", "val"]).unwrap()]);
        assert_eq!(extract_d_tag(&lower), Some("val".to_string()));

        // kind:39999 — upper bound
        let upper = make_event_with_kind_and_tags(39999, vec![Tag::parse(["d", "val"]).unwrap()]);
        assert_eq!(extract_d_tag(&upper), Some("val".to_string()));

        // kind:40000 — just above range
        let above = make_event_with_kind_and_tags(40000, vec![Tag::parse(["d", "val"]).unwrap()]);
        assert_eq!(extract_d_tag(&above), None);
    }

    #[test]
    fn extract_d_tag_single_element_d_tag_ignored() {
        // A d tag with only one element (no value) should not match — parts.len() < 2
        let event = make_event_with_kind_and_tags(30023, vec![Tag::parse(["d"]).unwrap()]);
        // No d tag with a value → empty string per NIP-33
        assert_eq!(extract_d_tag(&event), Some(String::new()));
    }

    #[test]
    fn extract_d_tag_preserves_full_value() {
        // extract_d_tag returns the full value — length enforcement is at the ingest layer.
        let long_val = "x".repeat(2048);
        let event =
            make_event_with_kind_and_tags(30023, vec![Tag::parse(["d", &long_val]).unwrap()]);
        let result = extract_d_tag(&event).unwrap();
        assert_eq!(result.len(), 2048);
        assert_eq!(result, long_val);
    }

    #[test]
    fn extract_not_before_from_reminder() {
        let event = make_event_with_kind_and_tags(
            KIND_EVENT_REMINDER as u16,
            vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
        );
        assert_eq!(extract_not_before(&event), Some(1_717_000_000));
    }

    #[test]
    fn extract_not_before_absent_returns_none() {
        // A bookmark/terminal reminder carries no `not_before` tag.
        let event = make_event_with_kind_and_tags(
            KIND_EVENT_REMINDER as u16,
            vec![Tag::parse(["d", "abc"]).unwrap()],
        );
        assert_eq!(extract_not_before(&event), None);
    }

    #[test]
    fn extract_not_before_non_reminder_returns_none() {
        // Only kind:30300 materializes `not_before`; other kinds stay NULL.
        let event = make_event_with_kind_and_tags(
            30023,
            vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
        );
        assert_eq!(extract_not_before(&event), None);
    }

    #[test]
    fn extract_not_before_non_numeric_returns_none() {
        // Malformed values are rejected by ingest; materialization just skips them.
        let event = make_event_with_kind_and_tags(
            KIND_EVENT_REMINDER as u16,
            vec![Tag::parse(["not_before", "not-a-number"]).unwrap()],
        );
        assert_eq!(extract_not_before(&event), None);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn query_due_reminders_returns_row_community_and_host_per_tenant() {
        let pool = setup_pool().await;
        let community_a_uuid = make_test_community(&pool).await;
        let community_b_uuid = make_test_community(&pool).await;
        let community_a = CommunityId::from_uuid(community_a_uuid);
        let community_b = CommunityId::from_uuid(community_b_uuid);
        let host_a: String = sqlx::query_scalar("SELECT host FROM communities WHERE id = $1")
            .bind(community_a_uuid)
            .fetch_one(&pool)
            .await
            .expect("load host A");
        let host_b: String = sqlx::query_scalar("SELECT host FROM communities WHERE id = $1")
            .bind(community_b_uuid)
            .fetch_one(&pool)
            .await
            .expect("load host B");

        let not_before = Utc::now().timestamp() - 1;
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let event_a = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "a")
            .tags([
                Tag::parse(["d", "due-reminder-scope-a"]).unwrap(),
                Tag::parse(["not_before", &not_before.to_string()]).unwrap(),
            ])
            .sign_with_keys(&keys_a)
            .expect("sign A");
        let event_b = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "b")
            .tags([
                Tag::parse(["d", "due-reminder-scope-b"]).unwrap(),
                Tag::parse(["not_before", &not_before.to_string()]).unwrap(),
            ])
            .sign_with_keys(&keys_b)
            .expect("sign B");

        insert_event(&pool, community_a, &event_a, None)
            .await
            .expect("insert A");
        insert_event(&pool, community_b, &event_b, None)
            .await
            .expect("insert B");

        let due = query_due_reminders(&pool, Utc::now().timestamp(), 100)
            .await
            .expect("query due reminders");

        assert!(due.iter().any(|row| {
            row.id == event_a.id.as_bytes() && row.community_id == community_a && row.host == host_a
        }));
        assert!(due.iter().any(|row| {
            row.id == event_b.id.as_bytes() && row.community_id == community_b && row.host == host_b
        }));
    }

    /// Two pods race to claim the same due reminder: exactly one wins. The
    /// scheduler publishes only on a winning claim (`Ok(true)`) and `continue`s
    /// on the loser (`Ok(false)`), so a single winning claim *is* the proof of
    /// exactly one publish side effect across N pods.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_due_reminder_is_won_by_exactly_one_of_two_racing_pods() {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let not_before = Utc::now().timestamp() - 1;
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "due")
            .tags([
                Tag::parse(["d", "due-reminder-claim-race"]).unwrap(),
                Tag::parse(["not_before", &not_before.to_string()]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign reminder");
        insert_event(&pool, community, &event, None)
            .await
            .expect("insert reminder");

        let id = event.id.as_bytes().to_vec();
        let created_at = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at, 0).expect("created_at");

        // Two pods, two distinct per-attempt stamps, same reminder.
        let stamp_p1: i64 = 0x1111_1111_1111_1111;
        let stamp_p2: i64 = 0x2222_2222_2222_2222;
        let won_p1 = claim_due_reminder_with_stamp(&pool, community, &id, created_at, stamp_p1)
            .await
            .expect("p1 claim");
        let won_p2 = claim_due_reminder_with_stamp(&pool, community, &id, created_at, stamp_p2)
            .await
            .expect("p2 claim");

        assert!(
            won_p1 ^ won_p2,
            "exactly one pod must win the claim (p1={won_p1}, p2={won_p2}) — \
             the loser never reaches the publish side effect"
        );
    }

    /// A failed publish releases the claim so the reminder is redeliverable,
    /// and the compare-and-clear stamp guard prevents one pod from rolling back
    /// another pod's claim.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn release_due_reminder_rolls_back_only_the_matching_stamp() {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let not_before = Utc::now().timestamp() - 1;
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "due")
            .tags([
                Tag::parse(["d", "due-reminder-release"]).unwrap(),
                Tag::parse(["not_before", &not_before.to_string()]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign reminder");
        insert_event(&pool, community, &event, None)
            .await
            .expect("insert reminder");

        let id = event.id.as_bytes().to_vec();
        let created_at = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at, 0).expect("created_at");
        let stamp: i64 = 0x3333_3333_3333_3333;

        assert!(
            claim_due_reminder_with_stamp(&pool, community, &id, created_at, stamp)
                .await
                .expect("claim"),
            "first claim wins"
        );

        // A release with the *wrong* stamp must be a no-op (does not clear
        // another pod's claim).
        assert!(
            !release_due_reminder(&pool, community, &id, created_at, stamp ^ 0xFFFF)
                .await
                .expect("wrong-stamp release"),
            "release with a non-matching stamp must not clear the claim"
        );
        assert!(
            !claim_due_reminder_with_stamp(&pool, community, &id, created_at, stamp)
                .await
                .expect("re-claim after no-op release"),
            "reminder must still be claimed after a no-op release"
        );

        // The matching-stamp release rolls the claim back; the reminder is
        // redeliverable and a subsequent claim wins again.
        assert!(
            release_due_reminder(&pool, community, &id, created_at, stamp)
                .await
                .expect("matching-stamp release"),
            "release with the claiming stamp must clear the claim"
        );
        assert!(
            claim_due_reminder_with_stamp(&pool, community, &id, created_at, stamp)
                .await
                .expect("re-claim after release"),
            "released reminder must be reclaimable for retry"
        );
    }

    /// Cross-community confinement: the same Nostr reminder event (identical
    /// `id` and `created_at`) inserted into communities A and B must claim and
    /// release independently. A claim/release for `A/X` must never touch `B/X`.
    ///
    /// This is the primitive the scheduler's exactly-once-publish proof rests
    /// on: `events` is keyed `(community_id, created_at, id)`, so without the
    /// community predicate a claim for A would mark B delivered (suppressing
    /// B's reminder) and a matching-stamp release for A would clear B.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reminder_claim_and_release_are_confined_to_their_community() {
        let pool = setup_pool().await;
        let community_a = CommunityId::from_uuid(make_test_community(&pool).await);
        let community_b = CommunityId::from_uuid(make_test_community(&pool).await);

        // One signed event, inserted into both communities — same id/created_at.
        let not_before = Utc::now().timestamp() - 1;
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), "due")
            .tags([
                Tag::parse(["d", "due-reminder-cross-community"]).unwrap(),
                Tag::parse(["not_before", &not_before.to_string()]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign reminder");
        insert_event(&pool, community_a, &event, None)
            .await
            .expect("insert A/X");
        insert_event(&pool, community_b, &event, None)
            .await
            .expect("insert B/X");

        let id = event.id.as_bytes().to_vec();
        let created_at = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at, 0).expect("created_at");
        let stamp: i64 = 0x4444_4444_4444_4444;

        // Claim A/X. B/X must remain claimable — A's claim did not mark B.
        assert!(
            claim_due_reminder_with_stamp(&pool, community_a, &id, created_at, stamp)
                .await
                .expect("claim A"),
            "A/X claim wins"
        );
        assert!(
            claim_due_reminder_with_stamp(&pool, community_b, &id, created_at, stamp)
                .await
                .expect("claim B"),
            "B/X must still be claimable after A/X is claimed — \
             a claim for A must not mark B delivered"
        );

        // Both are now claimed under the same stamp. A matching-stamp release
        // for A/X must clear only A/X; B/X must stay claimed.
        assert!(
            release_due_reminder(&pool, community_a, &id, created_at, stamp)
                .await
                .expect("release A"),
            "A/X release with the claiming stamp clears A/X"
        );
        assert!(
            !claim_due_reminder_with_stamp(&pool, community_b, &id, created_at, stamp)
                .await
                .expect("re-claim B after A release"),
            "B/X must remain claimed after A/X is released — \
             a release for A must not clear B"
        );
        // And A/X is genuinely redeliverable (the release was real, not a no-op).
        assert!(
            claim_due_reminder_with_stamp(&pool, community_a, &id, created_at, stamp)
                .await
                .expect("re-claim A after release"),
            "A/X must be reclaimable after its own release"
        );
    }

    #[test]
    fn huddle_started_content_requires_matching_ephemeral_field() {
        let channel_id = Uuid::new_v4();
        let matching = serde_json::json!({
            "ephemeral_channel_id": channel_id.to_string(),
        })
        .to_string();
        assert!(huddle_started_content_links(&matching, channel_id));

        let wrong_field = serde_json::json!({
            "other": channel_id.to_string(),
        })
        .to_string();
        assert!(!huddle_started_content_links(&wrong_field, channel_id));
        assert!(!huddle_started_content_links("not-json", channel_id));
    }
}
