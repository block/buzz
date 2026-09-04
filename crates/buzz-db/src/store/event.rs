//! Event storage and retrieval.
//!
//! AUTH events (kind 22242) are never stored — they carry bearer tokens.
//! Ephemeral events (kinds 20000–29999) are never stored — Redis pub/sub only.
//! Deduplication is application-layer: ON CONFLICT DO NOTHING.

use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder, Row, Transaction};
use uuid::Uuid;

use buzz_core::kind::{
    event_kind_i32, is_ephemeral, is_parameterized_replaceable, KIND_AUTH, KIND_EVENT_REMINDER,
    KIND_HUDDLE_STARTED, SHARED_GATED_KINDS,
};
use buzz_core::{CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;

use crate::error::{DbError, Result};
use crate::Db;

// Compatibility exports preserve the pre-extraction public event-store paths.
pub use crate::reminder::{
    claim_due_reminder, claim_due_reminder_with_stamp, query_due_reminders, release_due_reminder,
    DueReminder,
};

/// Largest page [`query_events`] will return when [`EventQuery::max_limit`] is
/// unset — the effective ceiling on any client-requested `limit`.
///
/// This is the value the relay advertises as NIP-11 `limitation.max_limit`, so
/// the advertised ceiling and the enforced one cannot drift.
pub const DEFAULT_MAX_PAGE_LIMIT: i64 = 1_000;

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
    /// Restrict results to events with an exact custom tag pair.
    /// Uses JSONB containment against `tags` before SQL `LIMIT`.
    pub custom_tag: Option<(String, String)>,
    /// Restrict results to events in any of these channels. By default,
    /// channel-less global events are retained so this can enforce a viewer's
    /// accessible-channel scope without hiding global events. Set
    /// [`EventQuery::channel_ids_include_global`] to `false` for an explicit
    /// multi-channel `#h` filter, which must match only requested channels.
    /// Applied before SQL `LIMIT` so access- and filter-scoped historical pages
    /// have exact exhaustion semantics.
    pub channel_ids: Option<Vec<uuid::Uuid>>,
    /// Whether [`EventQuery::channel_ids`] also retains channel-less global
    /// events. Defaults to `true` for access-scope queries.
    pub channel_ids_include_global: bool,
    /// Override the default page clamp ([`DEFAULT_MAX_PAGE_LIMIT`]). Used by
    /// the COUNT fallback path, which needs to fetch all matching events for
    /// post-filter counting. When None, the default clamp applies.
    pub max_limit: Option<i64>,
    /// Shared-gated visibility reader: when set, append an SQL visibility
    /// clause for every kind in [`SHARED_GATED_KINDS`] before ORDER/LIMIT so
    /// private events are excluded from the candidate page rather than
    /// discarded after it.
    ///
    /// The clause is: `AND (kind NOT IN (...) OR pubkey = $reader OR tags @> ?)`,
    /// where the `IN` list is [`SHARED_GATED_KINDS`] and `?` is the JSONB
    /// literal `[["shared","true"]]`.  The GIN index on `tags` (migration 0004,
    /// jsonb_path_ops) makes the containment check fast.
    ///
    /// NOTE: `tags @> '[["shared","true"]]'` uses JSONB containment, which
    /// matches any tag array that is a superset of `[["shared","true"]]` — it
    /// would match `["shared","true","extra"]` too.  The ingest `parts.len() ==
    /// 2` exact-shape check ensures such malformed tags are never stored, so the
    /// SQL pushdown is sound.  Keeping `event_visible_to_reader` as post-filter
    /// defense-in-depth catches any residual mismatch.
    pub shared_gated_reader: Option<Vec<u8>>,
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
            custom_tag: None,
            channel_ids: None,
            channel_ids_include_global: true,
            max_limit: None,
            shared_gated_reader: None,
        }
    }
}

pub use crate::reaction::{insert_reaction_event_with_thread_metadata, ReactionEventInsertOutcome};

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

/// Resolve creator-authenticated parent links for a bounded set of huddle sessions.
///
/// The creator constraint matters: a member of some unrelated channel can post
/// their own kind:48100 event there, but they cannot sign as the creator of the
/// target ephemeral channel. One set-based query replaces the liveness
/// endpoint's former session × parent lookup loop. Malformed historical start
/// content is ignored rather than aborting the complete liveness snapshot.
pub async fn huddle_started_links(
    pool: &PgPool,
    community_id: CommunityId,
    parent_channel_ids: &[Uuid],
    ephemeral_channel_ids: &[Uuid],
) -> Result<Vec<(Uuid, Uuid, Vec<u8>)>> {
    if parent_channel_ids.is_empty() || ephemeral_channel_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT ON (backing.id)
               backing.id AS session_id,
               start.channel_id AS parent_channel_id,
               backing.created_by
        FROM events start
        JOIN channels backing
          ON backing.community_id = start.community_id
         AND backing.id::text = CASE
             WHEN start.content IS JSON OBJECT
             THEN (start.content::json ->> 'ephemeral_channel_id')
             ELSE NULL
         END
         AND backing.deleted_at IS NULL
        WHERE start.deleted_at IS NULL
          AND start.community_id = $1
          AND start.channel_id = ANY($2)
          AND start.kind = $3
          AND octet_length(start.content) <= $5
          AND backing.id = ANY($4)
          AND start.pubkey = backing.created_by
        ORDER BY backing.id, start.created_at DESC, start.id ASC
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(parent_channel_ids)
    .bind(KIND_HUDDLE_STARTED as i32)
    .bind(ephemeral_channel_ids)
    .bind(HUDDLE_LINK_CONTENT_MAX_BYTES)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok((
                row.try_get("session_id")?,
                row.try_get("parent_channel_id")?,
                row.try_get("created_by")?,
            ))
        })
        .collect()
}

/// Return whether a creator-signed huddle-start event links a parent channel
/// to the requested ephemeral huddle channel.
pub async fn huddle_started_link_exists(
    pool: &PgPool,
    community_id: CommunityId,
    parent_channel_id: Uuid,
    ephemeral_channel_id: Uuid,
    creator_pubkey: &[u8],
) -> Result<bool> {
    huddle_started_link_exists_with_operation(
        pool,
        community_id,
        parent_channel_id,
        ephemeral_channel_id,
        creator_pubkey,
        crate::observability::WriterOperation::Authorization,
    )
    .await
}

async fn huddle_started_link_exists_with_operation(
    pool: &PgPool,
    community_id: CommunityId,
    parent_channel_id: Uuid,
    ephemeral_channel_id: Uuid,
    creator_pubkey: &[u8],
    operation: crate::observability::WriterOperation,
) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(pool, operation).await?;
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
    .fetch_all(&mut *connection)
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
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    insert_event_on(&mut connection, community_id, event, channel_id).await
}

/// Insert a Nostr event in a caller-owned PostgreSQL transaction.
///
/// This is the transaction-composition seam for callers that must keep the
/// event insert open while performing related work. The caller owns commit or
/// rollback.
pub async fn insert_event_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &Event,
    channel_id: Option<Uuid>,
) -> Result<(StoredEvent, bool)> {
    insert_event_on(tx.as_mut(), community_id, event, channel_id).await
}

async fn insert_event_on(
    connection: &mut PgConnection,
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
    .execute(connection)
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
    query_events_with_operation(
        pool,
        q,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await
}

pub(crate) async fn query_events_with_operation(
    pool: &PgPool,
    q: &EventQuery,
    operation: crate::observability::WriterOperation,
) -> Result<Vec<StoredEvent>> {
    let mut conn = crate::observability::acquire_writer(pool, operation).await?;
    query_events_on(&mut conn, q).await
}

/// [`query_events`] on a specific session — the replica-routing path runs
/// follow-up (aux) queries on the exact reader connection whose heartbeat
/// observation proved coverage for the page they annotate.
pub(crate) async fn query_events_on(
    conn: &mut sqlx::PgConnection,
    q: &EventQuery,
) -> Result<Vec<StoredEvent>> {
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

    let clamp = q.max_limit.unwrap_or(DEFAULT_MAX_PAGE_LIMIT);
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

    // Multi-channel IN pushdown. Access-scope queries retain global events;
    // explicit multi-value #h filters do not.
    //
    // SECURITY: Some(empty vec) means "match no channels". Access-scope
    // queries still retain globals; explicit #h queries match nothing.
    if let Some(ref ch_ids) = q.channel_ids {
        if ch_ids.is_empty() {
            if q.channel_ids_include_global {
                qb.push(format!(" AND {col_prefix}channel_id IS NULL"));
            } else {
                qb.push(" AND FALSE");
            }
        } else {
            qb.push(" AND (");
            if q.channel_ids_include_global {
                qb.push(format!("{col_prefix}channel_id IS NULL OR "));
            }
            qb.push(format!("{col_prefix}channel_id IN ("));
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

    if let Some((ref name, ref value)) = q.custom_tag {
        let containment = serde_json::json!([[name, value]]);
        qb.push(format!(" AND {col_prefix}tags @> "))
            .push_bind(containment);
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

    // Shared-gated visibility pushdown: exclude SHARED_GATED_KINDS events that
    // are neither authored by the reader nor explicitly shared.  Applied BEFORE
    // ORDER/LIMIT so that a page of newer private events does not push visible
    // shared ones off the end of the result set (the catalog query pattern).
    //
    // Clause: AND (kind NOT IN (30175, 30178) OR pubkey = $reader
    //              OR tags @> '[["shared","true"]]')
    //
    // The JSONB containment check is served by idx_events_tags_gin (migration
    // 0004, jsonb_path_ops).  `tags @> '[["shared","true"]]'` matches any array
    // that contains exactly the sub-array — a two-element `["shared","true"]`
    // tag passes; a tag-absent event does not.  Because ingest requires exactly
    // two elements for the shared tag (parts.len() == 2), no stored event can
    // carry a three-element superset.
    if let Some(ref reader_bytes) = q.shared_gated_reader {
        let shared_containment = serde_json::json!([["shared", "true"]]);
        qb.push(format!(" AND ({col_prefix}kind NOT IN ("));
        let mut sep = qb.separated(", ");
        for kind in SHARED_GATED_KINDS {
            sep.push_bind(*kind as i32);
        }
        qb.push(format!(") OR {col_prefix}pubkey = "));
        qb.push_bind(reader_bytes.clone());
        qb.push(format!(" OR {col_prefix}tags @> "));
        qb.push_bind(shared_containment);
        qb.push(")");
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

    let rows = qb.build().fetch_all(&mut *conn).await?;

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
    let mut conn = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await?;
    count_events_on(&mut conn, q).await
}

/// [`count_events`] on a specific session — the replica-routing path runs
/// the count on the exact reader connection whose heartbeat observation
/// proved its predicate.
pub(crate) async fn count_events_on(conn: &mut sqlx::PgConnection, q: &EventQuery) -> Result<i64> {
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

    // Multi-channel IN pushdown for COUNT. Access-scope queries retain global
    // events; explicit multi-value #h filters do not.
    if let Some(ref ch_ids) = q.channel_ids {
        if ch_ids.is_empty() {
            if q.channel_ids_include_global {
                qb.push(format!(" AND {col_prefix}channel_id IS NULL"));
            } else {
                qb.push(" AND FALSE");
            }
        } else {
            qb.push(" AND (");
            if q.channel_ids_include_global {
                qb.push(format!("{col_prefix}channel_id IS NULL OR "));
            }
            qb.push(format!("{col_prefix}channel_id IN ("));
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

    let row = qb.build().fetch_one(&mut *conn).await?;
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
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let result = sqlx::query(
        "UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
            .bind(community_id.as_uuid())
            .bind(event_id)
            .execute(&mut *connection)
            .await?;

    Ok(result.rows_affected() > 0)
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
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let result = sqlx::query(
        "UPDATE events SET deleted_at = NOW() \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND d_tag = $4 AND deleted_at IS NULL \
         AND created_at <= $5",
    )
    .bind(community_id.as_uuid())
    .bind(kind)
    .bind(pubkey)
    .bind(d_tag)
    .bind(deletion_created_at)
    .execute(&mut *connection)
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
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;

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
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await?;
    let row = sqlx::query(
        "SELECT created_at FROM events \
         WHERE community_id = $1 AND channel_id = $2 AND deleted_at IS NULL \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(&mut *connection)
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
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await?;

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

    let rows = qb.build().fetch_all(&mut *connection).await?;

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
    get_event_by_id_with_operation(
        pool,
        community_id,
        id_bytes,
        crate::observability::WriterOperation::Authorization,
    )
    .await
}

pub(crate) async fn get_event_by_id_with_operation(
    pool: &PgPool,
    community_id: CommunityId,
    id_bytes: &[u8],
    operation: crate::observability::WriterOperation,
) -> Result<Option<StoredEvent>> {
    let mut connection = crate::observability::acquire_writer(pool, operation).await?;
    let row = sqlx::query(
        "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
         FROM events WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(id_bytes)
    .fetch_optional(&mut *connection)
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
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
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
    .fetch_optional(&mut *connection)
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
    get_event_by_id_including_deleted_with_operation(
        pool,
        community_id,
        id_bytes,
        crate::observability::WriterOperation::Authorization,
    )
    .await
}

pub(crate) async fn get_event_by_id_including_deleted_with_operation(
    pool: &PgPool,
    community_id: CommunityId,
    id_bytes: &[u8],
    operation: crate::observability::WriterOperation,
) -> Result<Option<StoredEvent>> {
    let mut connection = crate::observability::acquire_writer(pool, operation).await?;
    let row = sqlx::query(
        "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
         FROM events WHERE community_id = $1 AND id = $2 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(id_bytes)
    .fetch_optional(&mut *connection)
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
    get_events_by_ids_with_operation(
        pool,
        community_id,
        ids,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await
}

pub(crate) async fn get_events_by_ids_with_operation(
    pool: &PgPool,
    community_id: CommunityId,
    ids: &[&[u8]],
    operation: crate::observability::WriterOperation,
) -> Result<Vec<StoredEvent>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let mut conn = crate::observability::acquire_writer(pool, operation).await?;
    get_events_by_ids_on(&mut conn, community_id, ids).await
}

/// [`get_events_by_ids`] on a specific session — the replica-routing path
/// runs the query on the exact reader connection whose heartbeat
/// observation proved its predicate.
pub(crate) async fn get_events_by_ids_on(
    conn: &mut sqlx::PgConnection,
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

    let rows = qb.build().fetch_all(&mut *conn).await?;

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

pub(crate) async fn insert_event_with_thread_metadata_tx(
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
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;
    let result =
        insert_event_with_thread_metadata_tx(&mut tx, community_id, event, channel_id, thread_meta)
            .await?;
    tx.commit().await?;
    Ok(result)
}

impl Db {
    /// Inserts an event. Returns `(StoredEvent, was_inserted)` — `false` on duplicate.
    #[datastore_span(name = "insert_event", system = "postgresql")]
    pub async fn insert_event(
        &self,
        community_id: CommunityId,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
    ) -> Result<(StoredEvent, bool)> {
        let result =
            crate::event::insert_event(&self.pool, community_id, event, channel_id).await?;
        if result.1 {
            if let Err(e) =
                crate::insert_mentions(&self.pool, community_id, event, channel_id).await
            {
                tracing::warn!(event_id = %event.id, "Failed to insert mentions: {e}");
            }
        }
        Ok(result)
    }

    /// Queries events matching the given filter parameters.
    ///
    /// Always reads from the WRITER pool. If the result influences a write
    /// or a permission decision, this is the method to call. Display-path
    /// callers that tolerate bounded staleness should use
    /// [`Db::query_events_routed`] instead — converting a caller is an
    /// explicit, per-callsite decision, never a change to this method.
    #[datastore_span(name = "query_events", system = "postgresql")]
    pub async fn query_events(&self, q: &EventQuery) -> Result<Vec<StoredEvent>> {
        crate::event::query_events_with_operation(
            &self.pool,
            q,
            crate::observability::WriterOperation::Authorization,
        )
        .await
    }

    /// Query authoritative event state that directly controls a durable event
    /// mutation or its post-commit side effects.
    #[datastore_span(name = "query_events_for_event_write", system = "postgresql")]
    pub async fn query_events_for_event_write(&self, q: &EventQuery) -> Result<Vec<StoredEvent>> {
        crate::event::query_events_with_operation(
            &self.pool,
            q,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
    }

    /// Query authoritative event state for startup reconciliation.
    #[datastore_span(name = "query_events_for_bootstrap", system = "postgresql")]
    pub async fn query_events_for_bootstrap(&self, q: &EventQuery) -> Result<Vec<StoredEvent>> {
        crate::event::query_events_with_operation(
            &self.pool,
            q,
            crate::observability::WriterOperation::Bootstrap,
        )
        .await
    }

    /// Query authoritative event state for background reconciliation or repair.
    #[datastore_span(name = "query_events_for_maintenance", system = "postgresql")]
    pub async fn query_events_for_maintenance(&self, q: &EventQuery) -> Result<Vec<StoredEvent>> {
        crate::event::query_events_with_operation(
            &self.pool,
            q,
            crate::observability::WriterOperation::Maintenance,
        )
        .await
    }

    /// [`Db::query_events`] with replica routing — the opt-in fast path for
    /// display reads.
    ///
    /// Rule of thumb: **if the result influences a write or a permission,
    /// it reads from the writer** — do not convert such a caller to this
    /// method. Every new caller must be added to the caller-classification
    /// table in `PLANS/REPLICA_FULL_READ_ROUTING_DESIGN.md`.
    ///
    /// Routing derives the strongest sound predicate from the query shape
    /// ([`crate::RoutePredicate::for_query`]): a channel-pinned query with an
    /// `until` upper bound may be served covered (provably complete below
    /// the fence wall); anything else is bounded-staleness only. The whole
    /// seam is gated on `BUZZ_REPLICA_READ_MAX_AGE_MS` (default off): when
    /// unset, even covered-eligible queries stay on the writer, so merging
    /// this seam is a true no-op until the budget is configured. Every
    /// failure fails closed to the writer.
    #[datastore_span(name = "query_events_routed", system = "postgresql")]
    pub async fn query_events_routed(
        &self,
        path: &'static str,
        q: &EventQuery,
    ) -> Result<Vec<StoredEvent>> {
        let predicate = crate::RoutePredicate::for_query(q, self.replica_read_max_age.is_some());
        match self
            .route_read(
                path,
                predicate,
                crate::observability::ReaderOperation::SubscriptionHistory,
            )
            .await
        {
            crate::RouteDecision::Replica(mut tx, _entry, reason) => {
                match crate::event::query_events_on(&mut tx, q).await {
                    Ok(events) => {
                        Self::record_route(path, "replica", reason);
                        Ok(events)
                    }
                    Err(e) => {
                        // Mid-query replica failure: fail closed to the
                        // writer rather than surfacing a routed error.
                        tracing::warn!(path, "replica read failed; re-running on writer: {e}");
                        Self::record_route(path, "writer", "replica_error");
                        crate::event::query_events_with_operation(
                            &self.pool,
                            q,
                            crate::observability::WriterOperation::SubscriptionHistory,
                        )
                        .await
                    }
                }
            }
            crate::RouteDecision::Writer => {
                crate::event::query_events_with_operation(
                    &self.pool,
                    q,
                    crate::observability::WriterOperation::SubscriptionHistory,
                )
                .await
            }
        }
    }

    /// [`Db::query_events_routed`] restricted to the BOUNDED arm — for
    /// reads whose result feeds a COUNT rather than a displayed page.
    ///
    /// The covered arm bounds insert-completeness only; stale deletions can
    /// briefly inflate the result set (see [`crate::RoutePredicate::Covered`]). A
    /// display page absorbs that per-row; a number derived from the rows
    /// does not. Same classification-table requirement as
    /// [`Db::query_events_routed`].
    #[datastore_span(name = "query_events_routed_bounded", system = "postgresql")]
    pub async fn query_events_routed_bounded(
        &self,
        path: &'static str,
        q: &EventQuery,
    ) -> Result<Vec<StoredEvent>> {
        match self
            .route_read(
                path,
                crate::RoutePredicate::Bounded,
                crate::observability::ReaderOperation::SubscriptionHistory,
            )
            .await
        {
            crate::RouteDecision::Replica(mut tx, _entry, reason) => {
                match crate::event::query_events_on(&mut tx, q).await {
                    Ok(events) => {
                        Self::record_route(path, "replica", reason);
                        Ok(events)
                    }
                    Err(e) => {
                        tracing::warn!(path, "replica read failed; re-running on writer: {e}");
                        Self::record_route(path, "writer", "replica_error");
                        crate::event::query_events_with_operation(
                            &self.pool,
                            q,
                            crate::observability::WriterOperation::SubscriptionHistory,
                        )
                        .await
                    }
                }
            }
            crate::RouteDecision::Writer => {
                crate::event::query_events_with_operation(
                    &self.pool,
                    q,
                    crate::observability::WriterOperation::SubscriptionHistory,
                )
                .await
            }
        }
    }

    /// Count events matching the given query (NIP-45 COUNT support).
    ///
    /// Always reads from the WRITER pool — see [`Db::query_events`] for the
    /// writer-vs-routed rule.
    #[datastore_span(name = "count_events", system = "postgresql")]
    pub async fn count_events(&self, q: &EventQuery) -> Result<i64> {
        crate::event::count_events(&self.pool, q).await
    }

    /// [`Db::count_events`] with replica routing — same contract, rules,
    /// and classification-table requirement as [`Db::query_events_routed`].
    ///
    /// Counts route on the BOUNDED arm only, never covered: the covered
    /// arm bounds insert-completeness but not deletion visibility (soft
    /// deletes are UPDATEs outside the floor guard), and a count has no
    /// downstream per-row re-filter to absorb extra rows — a silently
    /// inflated number for up to `FENCE_STALENESS` is a different product
    /// statement than a page briefly showing a deleted row. `Bounded` ties
    /// the error to the accepted budget `B`.
    #[datastore_span(name = "count_events_routed", system = "postgresql")]
    pub async fn count_events_routed(&self, path: &'static str, q: &EventQuery) -> Result<i64> {
        match self
            .route_read(
                path,
                crate::RoutePredicate::Bounded,
                crate::observability::ReaderOperation::SubscriptionHistory,
            )
            .await
        {
            crate::RouteDecision::Replica(mut tx, _entry, reason) => {
                match crate::event::count_events_on(&mut tx, q).await {
                    Ok(count) => {
                        Self::record_route(path, "replica", reason);
                        Ok(count)
                    }
                    Err(e) => {
                        tracing::warn!(path, "replica count failed; re-running on writer: {e}");
                        Self::record_route(path, "writer", "replica_error");
                        crate::event::count_events(&self.pool, q).await
                    }
                }
            }
            crate::RouteDecision::Writer => crate::event::count_events(&self.pool, q).await,
        }
    }

    /// Resolve creator-signed huddle-start links for bounded parent/session sets.
    #[datastore_span(name = "huddle_started_links", system = "postgresql")]
    pub async fn huddle_started_links(
        &self,
        community_id: CommunityId,
        parent_channel_ids: &[Uuid],
        ephemeral_channel_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, Uuid, Vec<u8>)>> {
        crate::event::huddle_started_links(
            &self.pool,
            community_id,
            parent_channel_ids,
            ephemeral_channel_ids,
        )
        .await
    }

    /// Return whether a creator-signed huddle-start event links a parent
    /// channel to the requested ephemeral huddle channel.
    #[datastore_span(name = "huddle_started_link_exists", system = "postgresql")]
    pub async fn huddle_started_link_exists(
        &self,
        community_id: CommunityId,
        parent_channel_id: Uuid,
        ephemeral_channel_id: Uuid,
        creator_pubkey: &[u8],
    ) -> Result<bool> {
        crate::event::huddle_started_link_exists(
            &self.pool,
            community_id,
            parent_channel_id,
            ephemeral_channel_id,
            creator_pubkey,
        )
        .await
    }

    /// Validate a huddle link while admitting a huddle event for persistence.
    #[datastore_span(
        name = "huddle_started_link_exists_for_event_write",
        system = "postgresql"
    )]
    pub async fn huddle_started_link_exists_for_event_write(
        &self,
        community_id: CommunityId,
        parent_channel_id: Uuid,
        ephemeral_channel_id: Uuid,
        creator_pubkey: &[u8],
    ) -> Result<bool> {
        crate::event::huddle_started_link_exists_with_operation(
            &self.pool,
            community_id,
            parent_channel_id,
            ephemeral_channel_id,
            creator_pubkey,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
    }

    /// Fetch the latest replaceable event for a (kind, pubkey) pair.
    ///
    /// Uses canonical NIP-16 ordering: `created_at DESC, id ASC`.
    /// This matches the write path in [`replace_addressable_event`] and handles
    /// historical duplicate survivors correctly.
    #[datastore_span(name = "get_latest_global_replaceable", system = "postgresql")]
    pub async fn get_latest_global_replaceable(
        &self,
        community_id: CommunityId,
        kind: i32,
        pubkey_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        crate::event::get_latest_global_replaceable(&self.pool, community_id, kind, pubkey_bytes)
            .await
    }

    /// Fetches a single non-deleted event by its raw ID bytes.
    ///
    /// Returns `None` if the event does not exist or has been soft-deleted.
    #[datastore_span(name = "get_event_by_id", system = "postgresql")]
    pub async fn get_event_by_id(
        &self,
        community_id: CommunityId,
        id_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        crate::event::get_event_by_id(&self.pool, community_id, id_bytes).await
    }

    /// Fetch an event as a prerequisite of an event write or durable
    /// post-write side effect.
    #[datastore_span(name = "get_event_by_id_for_event_write", system = "postgresql")]
    pub async fn get_event_by_id_for_event_write(
        &self,
        community_id: CommunityId,
        id_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        crate::event::get_event_by_id_with_operation(
            &self.pool,
            community_id,
            id_bytes,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
    }

    /// Fetches a single event by its raw ID bytes, **including soft-deleted rows**.
    #[datastore_span(name = "get_event_by_id_including_deleted", system = "postgresql")]
    pub async fn get_event_by_id_including_deleted(
        &self,
        community_id: CommunityId,
        id_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        crate::event::get_event_by_id_including_deleted(&self.pool, community_id, id_bytes).await
    }

    /// Fetch an event including tombstones as a prerequisite of an event
    /// write or durable post-write side effect.
    #[datastore_span(
        name = "get_event_by_id_including_deleted_for_event_write",
        system = "postgresql"
    )]
    pub async fn get_event_by_id_including_deleted_for_event_write(
        &self,
        community_id: CommunityId,
        id_bytes: &[u8],
    ) -> Result<Option<StoredEvent>> {
        crate::event::get_event_by_id_including_deleted_with_operation(
            &self.pool,
            community_id,
            id_bytes,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
    }

    /// Soft-deletes an event. Returns `Ok(true)` if deleted, `Ok(false)` if already deleted.
    #[datastore_span(name = "soft_delete_event", system = "postgresql")]
    pub async fn soft_delete_event(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
    ) -> Result<bool> {
        crate::event::soft_delete_event(&self.pool, community_id, event_id).await
    }

    /// Soft-delete the live row for an addressable coordinate `(kind, pubkey, d_tag)`
    /// when it is not newer than the deletion request.
    /// Used by NIP-09 a-tag deletion for parameterized-replaceable kinds;
    /// `deletion_created_at_secs` is the deletion event's `created_at`.
    #[datastore_span(name = "soft_delete_by_coordinate", system = "postgresql")]
    pub async fn soft_delete_by_coordinate(
        &self,
        community_id: CommunityId,
        kind: i32,
        pubkey: &[u8],
        d_tag: &str,
        deletion_created_at_secs: i64,
    ) -> Result<bool> {
        crate::event::soft_delete_by_coordinate(
            &self.pool,
            community_id,
            kind,
            pubkey,
            d_tag,
            deletion_created_at_secs,
        )
        .await
    }

    /// Atomically soft-delete an event and decrement thread reply counters.
    #[datastore_span(name = "soft_delete_event_and_update_thread", system = "postgresql")]
    pub async fn soft_delete_event_and_update_thread(
        &self,
        community_id: CommunityId,
        event_id: &[u8],
        parent_event_id: Option<&[u8]>,
        root_event_id: Option<&[u8]>,
    ) -> Result<bool> {
        crate::event::soft_delete_event_and_update_thread(
            &self.pool,
            community_id,
            event_id,
            parent_event_id,
            root_event_id,
        )
        .await
    }

    /// Returns the most recent `created_at` for a channel.
    #[datastore_span(name = "get_last_message_at", system = "postgresql")]
    pub async fn get_last_message_at(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Option<DateTime<Utc>>> {
        crate::event::get_last_message_at(&self.pool, community_id, channel_id).await
    }

    /// Bulk-fetch the most recent `created_at` for a set of channel IDs.
    #[datastore_span(name = "get_last_message_at_bulk", system = "postgresql")]
    pub async fn get_last_message_at_bulk(
        &self,
        community_id: CommunityId,
        channel_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, DateTime<Utc>>> {
        crate::event::get_last_message_at_bulk(&self.pool, community_id, channel_ids).await
    }

    /// Batch-fetch non-deleted events by their raw IDs.
    #[datastore_span(name = "get_events_by_ids", system = "postgresql")]
    pub async fn get_events_by_ids(
        &self,
        community_id: CommunityId,
        ids: &[&[u8]],
    ) -> Result<Vec<StoredEvent>> {
        crate::event::get_events_by_ids_with_operation(
            &self.pool,
            community_id,
            ids,
            crate::observability::WriterOperation::Authorization,
        )
        .await
    }

    /// [`Db::get_events_by_ids`] with replica routing — same contract and
    /// classification-table requirement as [`Db::query_events_routed`].
    ///
    /// By-id fetches route on the BOUNDED arm only: an id list carries no
    /// channel pin, so no fence floor can prove insert-completeness — the
    /// covered arm is structurally unavailable. Used for FTS hit hydration,
    /// where a missing row degrades to a skipped search hit downstream.
    #[datastore_span(name = "get_events_by_ids_routed", system = "postgresql")]
    pub async fn get_events_by_ids_routed(
        &self,
        path: &'static str,
        community_id: CommunityId,
        ids: &[&[u8]],
    ) -> Result<Vec<StoredEvent>> {
        match self
            .route_read(
                path,
                crate::RoutePredicate::Bounded,
                crate::observability::ReaderOperation::SubscriptionHistory,
            )
            .await
        {
            crate::RouteDecision::Replica(mut tx, _entry, reason) => {
                match crate::event::get_events_by_ids_on(&mut tx, community_id, ids).await {
                    Ok(events) => {
                        Self::record_route(path, "replica", reason);
                        Ok(events)
                    }
                    Err(e) => {
                        tracing::warn!(path, "replica read failed; re-running on writer: {e}");
                        Self::record_route(path, "writer", "replica_error");
                        crate::event::get_events_by_ids_with_operation(
                            &self.pool,
                            community_id,
                            ids,
                            crate::observability::WriterOperation::SubscriptionHistory,
                        )
                        .await
                    }
                }
            }
            crate::RouteDecision::Writer => {
                crate::event::get_events_by_ids_with_operation(
                    &self.pool,
                    community_id,
                    ids,
                    crate::observability::WriterOperation::SubscriptionHistory,
                )
                .await
            }
        }
    }

    /// Atomically insert an event AND its thread metadata in a single transaction.
    #[datastore_span(name = "insert_event_with_thread_metadata", system = "postgresql")]
    pub async fn insert_event_with_thread_metadata(
        &self,
        community_id: CommunityId,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
        thread_meta: Option<crate::event::ThreadMetadataParams<'_>>,
    ) -> Result<(StoredEvent, bool)> {
        let result = crate::event::insert_event_with_thread_metadata(
            &self.pool,
            community_id,
            event,
            channel_id,
            thread_meta,
        )
        .await?;
        if result.1 {
            if let Err(e) =
                crate::insert_mentions(&self.pool, community_id, event, channel_id).await
            {
                tracing::warn!(event_id = %event.id, "Failed to insert mentions: {e}");
            }
        }
        Ok(result)
    }

    /// Backfill `d_tag` for existing NIP-33 events (kind 30000–39999) that have `d_tag IS NULL`.
    ///
    /// Idempotent — safe to call on every startup. No-ops when all rows are already populated.
    /// Runs a single UPDATE touching only NIP-33 rows with NULL d_tag.
    #[datastore_span(name = "backfill_d_tags", system = "postgresql")]
    pub async fn backfill_d_tags(&self) -> Result<u64> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Bootstrap,
        )
        .await?;
        let result = sqlx::query(
            "UPDATE events \
             SET d_tag = COALESCE( \
                 (SELECT elem->>1 FROM jsonb_array_elements(tags) AS elem \
                  WHERE elem->>0 = 'd' LIMIT 1), \
                 '' \
             ) \
             WHERE kind BETWEEN 30000 AND 39999 AND d_tag IS NULL \
               AND community_write_allowed(community_id)",
        )
        .execute(&mut *connection)
        .await?;
        Ok(result.rows_affected())
    }

    /// Soft-delete NIP-29 discovery events for a channel created by a specific relay pubkey.
    #[datastore_span(name = "soft_delete_discovery_events", system = "postgresql")]
    pub async fn soft_delete_discovery_events(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        relay_pubkey: &[u8],
    ) -> Result<u64> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let result = sqlx::query(
            "UPDATE events SET deleted_at = NOW() \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 AND deleted_at IS NULL AND kind IN (39000, 39001, 39002)",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(relay_pubkey)
        .execute(&mut *connection)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
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
    async fn event_insert_in_existing_transaction_rolls_back_with_caller() {
        let pool = setup_pool().await;
        let community_uuid = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_uuid);
        let event = make_text_event("caller-owned transaction");

        let mut tx = pool.begin().await.expect("begin event insert transaction");
        let (_, was_inserted) = insert_event_in_transaction(&mut tx, community, &event, None)
            .await
            .expect("insert event in caller transaction");
        assert!(was_inserted);
        tx.rollback().await.expect("roll back event insert");

        let persisted: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id = $1 AND id = $2")
                .bind(community_uuid)
                .bind(event.id.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count rolled-back event");
        assert_eq!(persisted, 0);
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

    fn make_event_at(kind: u16, content: &str, created_at: u64) -> nostr::Event {
        EventBuilder::new(Kind::Custom(kind), content)
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(&Keys::generate())
            .expect("sign timestamped event")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn explicit_multi_channel_scope_is_applied_before_historical_page_limit() {
        let pool = setup_pool().await;
        let community_uuid = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_uuid);
        let channel_a = make_test_channel(&pool, community_uuid, None).await;
        let channel_b = make_test_channel(&pool, community_uuid, None).await;
        let unrelated_c = make_test_channel(&pool, community_uuid, None).await;
        let base = 1_800_000_000;

        let older_a = make_event_at(39_000, "older requested A", base + 1);
        insert_event(&pool, community, &older_a, Some(channel_a))
            .await
            .expect("insert requested A candidate");
        let requested_b = make_event_at(39_000, "requested B", base + 2);
        insert_event(&pool, community, &requested_b, Some(channel_b))
            .await
            .expect("insert requested B candidate");
        let newer_c = make_event_at(39_000, "newer unrelated C", base + 3);
        insert_event(&pool, community, &newer_c, Some(unrelated_c))
            .await
            .expect("insert unrelated C candidate");
        let global = make_event_at(39_000, "global candidate", base + 4);
        insert_event(&pool, community, &global, None)
            .await
            .expect("insert global candidate");

        let events = query_events(
            &pool,
            &EventQuery {
                kinds: Some(vec![39_000]),
                channel_ids: Some(vec![channel_a, channel_b]),
                channel_ids_include_global: false,
                limit: Some(1),
                ..EventQuery::for_community(community)
            },
        )
        .await
        .expect("query explicit multi-channel page");

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].event.id, requested_b.id,
            "newer unrelated channel C must not consume the requested A/B limit"
        );

        let partial_authorization_count = count_events(
            &pool,
            &EventQuery {
                kinds: Some(vec![39_000]),
                channel_ids: Some(vec![channel_a]),
                channel_ids_include_global: false,
                ..EventQuery::for_community(community)
            },
        )
        .await
        .expect("count one authorized channel from a multi-channel request");
        assert_eq!(
            partial_authorization_count, 1,
            "partial authorization must exclude requested B, unrelated C, and global rows"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn access_scope_is_applied_before_historical_page_limit() {
        let pool = setup_pool().await;
        let community_uuid = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_uuid);
        let accessible = make_test_channel(&pool, community_uuid, None).await;
        let inaccessible = make_test_channel(&pool, community_uuid, None).await;
        let base = 1_800_000_000;

        // This is the bridge underfetch shape: newer inaccessible candidates
        // outnumber the requested page, while the visible match is older.
        for offset in 10..13 {
            let event = make_event_at(39_000, "newer inaccessible", base + offset);
            insert_event(&pool, community, &event, Some(inaccessible))
                .await
                .expect("insert inaccessible candidate");
        }
        let global = make_event_at(39_000, "newer global", base + 2);
        insert_event(&pool, community, &global, None)
            .await
            .expect("insert global candidate");
        let older_accessible = make_event_at(39_000, "older accessible", base + 1);
        insert_event(&pool, community, &older_accessible, Some(accessible))
            .await
            .expect("insert accessible candidate");

        let events = query_events(
            &pool,
            &EventQuery {
                kinds: Some(vec![39_000]),
                channel_ids: Some(vec![accessible]),
                limit: Some(2),
                ..EventQuery::for_community(community)
            },
        )
        .await
        .expect("query access-scoped page");

        assert_eq!(events.len(), 2, "visible page must be filled before EOF");
        assert_eq!(events[0].event.id, global.id, "global rows remain visible");
        assert_eq!(
            events[1].event.id, older_accessible.id,
            "older accessible row must not be hidden behind newer inaccessible rows"
        );
    }

    fn make_text_event(content: &str) -> nostr::Event {
        let keys = Keys::generate();
        EventBuilder::new(Kind::Custom(9), content)
            .sign_with_keys(&keys)
            .expect("sign text event")
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
    async fn coordinate_delete_spares_head_newer_than_the_deletion() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let db = Db::from_pool(setup_pool().await);
        let community = CommunityId::from_uuid(make_test_community(&db.pool).await);
        let keys = Keys::generate();
        let kind = buzz_core::kind::KIND_PROJECT as i32;
        let d_tag = "stale-tombstone-project";
        let pubkey = keys.public_key().to_bytes().to_vec();
        let base = Timestamp::now().as_secs();

        let version = |content: &str, offset: u64| {
            EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_PROJECT as u16), content)
                .tags(vec![Tag::parse(["d", d_tag]).expect("d tag")])
                .custom_created_at(Timestamp::from(base + offset))
                .sign_with_keys(&keys)
                .expect("sign project version")
        };

        for (content, offset) in [("v1", 0), ("v2", 100)] {
            assert!(
                db.replace_parameterized_event(community, &version(content, offset), d_tag, None)
                    .await
                    .expect("store project version")
                    .1
            );
        }

        // Tombstone timestamped between V1 and V2: it authorizes deleting V1,
        // never the newer head that replaced it.
        let stale_deleted = db
            .soft_delete_by_coordinate(community, kind, &pubkey, d_tag, (base + 50) as i64)
            .await
            .expect("stale coordinate delete");
        assert!(
            !stale_deleted,
            "a tombstone older than the live head must delete nothing"
        );

        let live_content: Option<String> = sqlx::query_scalar(
            "SELECT content FROM events \
             WHERE community_id=$1 AND kind=$2 AND pubkey=$3 AND d_tag=$4 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(kind)
        .bind(&pubkey)
        .bind(d_tag)
        .fetch_optional(&db.pool)
        .await
        .expect("read live head");
        assert_eq!(
            live_content.as_deref(),
            Some("v2"),
            "the newer head must survive a stale tombstone"
        );

        // A tombstone at or after the head's own timestamp still deletes it.
        let current_deleted = db
            .soft_delete_by_coordinate(community, kind, &pubkey, d_tag, (base + 100) as i64)
            .await
            .expect("current coordinate delete");
        assert!(
            current_deleted,
            "a tombstone at the head's timestamp must delete it (NIP-09 is at-or-before)"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn huddle_started_links_batches_valid_creator_links_and_ignores_malformed_content() {
        let pool = setup_pool().await;
        let community_uuid = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_uuid);
        let parent = make_test_channel(&pool, community_uuid, None).await;
        let session = make_test_channel(&pool, community_uuid, Some(60)).await;
        let creator = vec![7_u8; 32];

        for (index, content) in [
            "not-json".to_owned(),
            serde_json::json!({ "ephemeral_channel_id": session }).to_string(),
        ]
        .into_iter()
        .enumerate()
        {
            sqlx::query(
                "INSERT INTO events \
                 (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id) \
                 VALUES ($1, $2, $3, NOW() + make_interval(secs => $4), $5, '[]', $6, $7, $8)",
            )
            .bind(community_uuid)
            .bind(vec![(index + 1) as u8; 32])
            .bind(&creator)
            .bind(index as f64)
            .bind(KIND_HUDDLE_STARTED as i32)
            .bind(content)
            .bind(vec![0_u8; 64])
            .bind(parent)
            .execute(&pool)
            .await
            .expect("insert huddle-start candidate");
        }

        let links = huddle_started_links(&pool, community, &[parent], &[session])
            .await
            .expect("batch huddle links");
        assert_eq!(links, vec![(session, parent, creator)]);
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
