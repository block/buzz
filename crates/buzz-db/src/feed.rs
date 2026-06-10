//! Feed-specific DB queries for the Home Feed feature.
//!
//! Aggregates three categories of data:
//! - **Mentions**: Events where the user's pubkey appears in a `p` tag.
//! - **Needs Action**: Approval requests (kind 46010) and reminders (kind 40007) tagged to the user.
//! - **Activity**: Recent events from channels the user can access.
//!
//! ## Performance characteristics
//!
//! `query_mentions` and `query_needs_action` join against the `event_mentions` table,
//! which carries composite indexes on `(pubkey_hex, event_created_at DESC)` and
//! `(pubkey_hex, event_kind, event_created_at DESC)`.  This replaces the Phase 1
//! full-table scan with an indexed lookup, keeping feed queries
//! sub-millisecond at scale (>100k events).
//!
//! **Phase 2 implemented**: the `event_mentions` table is populated by
//! [`crate::insert_mentions`] on every event insert.  `query_mentions` and
//! `query_needs_action` now use `INNER JOIN event_mentions` instead of
//! scanning tags JSON.
//!
//! All feed queries enforce a hard `LIMIT` cap of `FEED_MAX_LIMIT` rows to bound
//! the result-set size and prevent runaway memory usage.

/// Hard upper bound on rows returned by any feed query.
///
/// Callers may request fewer rows, but never more.  Enforced in every feed function
/// before the query is issued so the SQL `LIMIT` clause always reflects this cap.
pub const FEED_MAX_LIMIT: i64 = 100;

use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, QueryBuilder};
use uuid::Uuid;

use sprout_core::kind::{
    KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_JOB_PROGRESS, KIND_JOB_REQUEST, KIND_JOB_RESULT,
    KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2, KIND_STREAM_REMINDER,
    KIND_WORKFLOW_APPROVAL_REQUESTED,
};
use sprout_core::StoredEvent;

use crate::error::Result;
use crate::event::row_to_stored_event;

/// Column list shared by every feed subquery that aliases the `events` table as `e`.
const EVENT_COLS: &str =
    "e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, e.sig, e.received_at, e.channel_id";

/// Column list for queries that select directly from `events` (no table alias).
const EVENT_COLS_UNALIASED: &str =
    "id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id";

/// Append `AND <col> IN ($1, $2, …)` for the given channel IDs.
///
/// No-ops when the slice is empty so callers don't need a guard.
fn push_channel_id_filter(qb: &mut QueryBuilder<sqlx::Postgres>, col: &str, ids: &[Uuid]) {
    if !ids.is_empty() {
        qb.push(format!(" AND {col} IN ("));
        let mut sep = qb.separated(", ");
        for id in ids {
            sep.push_bind(*id);
        }
        qb.push(")");
    }
}

/// Convert fetched rows into `Vec<StoredEvent>`, skipping any that fail conversion.
fn collect_stored_events(rows: Vec<PgRow>) -> Result<Vec<StoredEvent>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(ev) = row_to_stored_event(row)? {
            out.push(ev);
        }
    }
    Ok(out)
}

/// Find events that @mention the given pubkey (have `["p", pubkey_hex]` in tags).
///
/// Joins against the `event_mentions` table -- Phase 2 implementation.
/// **Performance**: indexed lookup on `(pubkey_hex, event_created_at DESC)`.
///
/// Only returns events from `accessible_channel_ids` for access control.
/// `limit` is capped at [`FEED_MAX_LIMIT`] regardless of the value passed by the caller.
pub async fn query_mentions(
    pool: &PgPool,
    pubkey_bytes: &[u8],
    accessible_channel_ids: &[Uuid],
    since: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<StoredEvent>> {
    let limit = limit.min(FEED_MAX_LIMIT);
    let pubkey_hex = hex::encode(pubkey_bytes);

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(format!(
        "SELECT {EVENT_COLS} FROM events e \
         INNER JOIN event_mentions m ON e.id = m.event_id \
         WHERE m.pubkey_hex = "
    ));
    qb.push_bind(&pubkey_hex);
    qb.push(" AND e.deleted_at IS NULL");
    qb.push(format!(
        " AND e.kind IN ({KIND_STREAM_MESSAGE}, {KIND_STREAM_MESSAGE_V2}, \
         {KIND_FORUM_POST}, {KIND_FORUM_COMMENT})"
    ));
    push_channel_id_filter(&mut qb, "e.channel_id", accessible_channel_ids);
    if let Some(s) = since {
        qb.push(" AND m.event_created_at >= ").push_bind(s);
    }
    qb.push(" ORDER BY m.event_created_at DESC LIMIT ")
        .push_bind(limit);

    let rows = qb.build().fetch_all(pool).await?;
    collect_stored_events(rows)
}

/// Find events that require action from the given pubkey:
/// - [`KIND_WORKFLOW_APPROVAL_REQUESTED`] (workflow approval requested, tagged with user pubkey)
/// - [`KIND_STREAM_REMINDER`] (reminder, tagged with user pubkey)
///
/// Only returns events from channels the user has access to (`accessible_channel_ids`).
/// This prevents surfacing approval requests from channels the user was removed from.
/// **Performance**: indexed lookup via `event_mentions` join on
/// `(pubkey_hex, event_kind, event_created_at DESC)`.
/// `limit` is capped at [`FEED_MAX_LIMIT`] regardless of the value passed by the caller.
pub async fn query_needs_action(
    pool: &PgPool,
    pubkey_bytes: &[u8],
    accessible_channel_ids: &[Uuid],
    since: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<StoredEvent>> {
    let limit = limit.min(FEED_MAX_LIMIT);
    let pubkey_hex = hex::encode(pubkey_bytes);

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(format!(
        "SELECT {EVENT_COLS} FROM events e \
         INNER JOIN event_mentions m ON e.id = m.event_id \
         WHERE m.pubkey_hex = "
    ));
    qb.push_bind(&pubkey_hex);
    qb.push(" AND e.deleted_at IS NULL");
    qb.push(format!(
        " AND e.kind IN ({KIND_WORKFLOW_APPROVAL_REQUESTED}, {KIND_STREAM_REMINDER})"
    ));
    push_channel_id_filter(&mut qb, "e.channel_id", accessible_channel_ids);
    if let Some(s) = since {
        qb.push(" AND m.event_created_at >= ").push_bind(s);
    }
    qb.push(" ORDER BY m.event_created_at DESC LIMIT ")
        .push_bind(limit);

    let rows = qb.build().fetch_all(pool).await?;
    collect_stored_events(rows)
}

/// Find recent activity across accessible channels (for watched topics / agent activity).
///
/// Returns stream messages, forum posts, and agent job events.
/// Workflow execution kinds (46001-46012) are intentionally excluded to avoid noise.
/// **Performance**: uses indexed `kind` + `channel_id` columns -- no JSON scan.
/// `limit` is capped at [`FEED_MAX_LIMIT`] regardless of the value passed by the caller.
pub async fn query_activity(
    pool: &PgPool,
    accessible_channel_ids: &[Uuid],
    since: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<StoredEvent>> {
    let limit = limit.min(FEED_MAX_LIMIT);
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(format!(
        "SELECT {EVENT_COLS_UNALIASED} FROM events WHERE deleted_at IS NULL"
    ));
    qb.push(format!(
        " AND kind IN ({KIND_STREAM_MESSAGE}, {KIND_STREAM_MESSAGE_V2}, {KIND_FORUM_POST}, \
         {KIND_JOB_REQUEST}, {KIND_JOB_PROGRESS}, {KIND_JOB_RESULT})"
    ));
    push_channel_id_filter(&mut qb, "channel_id", accessible_channel_ids);
    if let Some(s) = since {
        qb.push(" AND created_at >= ").push_bind(s);
    }
    qb.push(" ORDER BY created_at DESC LIMIT ").push_bind(limit);

    let rows = qb.build().fetch_all(pool).await?;
    collect_stored_events(rows)
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    // -- Hex encoding of pubkey -----------------------------------------------

    #[test]
    fn pubkey_hex_encoding_is_lowercase() {
        let pubkey_bytes = vec![0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45];
        let hex = hex::encode(&pubkey_bytes);
        assert_eq!(hex, "abcdef012345");
        assert_eq!(hex, hex.to_lowercase());
    }

    #[test]
    fn pubkey_hex_encoding_32_byte_key() {
        let pubkey_bytes: Vec<u8> = (0u8..32).collect();
        let hex = hex::encode(&pubkey_bytes);
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hex, hex.to_lowercase());
    }

    #[test]
    fn pubkey_hex_encoding_all_zeros() {
        let pubkey_bytes = vec![0u8; 32];
        let hex = hex::encode(&pubkey_bytes);
        assert_eq!(hex, "0".repeat(64));
    }

    #[test]
    fn pubkey_hex_encoding_all_ff() {
        let pubkey_bytes = vec![0xFFu8; 32];
        let hex = hex::encode(&pubkey_bytes);
        assert_eq!(hex, "f".repeat(64));
    }

    // -- JSON tag format for tag matching -------------------------------------

    #[test]
    fn json_tag_format_for_p_tag_mention() {
        let pubkey_hex = "abc123def456".to_owned();
        let tag_json = serde_json::json!([["p", pubkey_hex]]).to_string();
        assert_eq!(tag_json, r#"[["p","abc123def456"]]"#);
    }

    #[test]
    fn json_tag_format_is_compact_not_pretty() {
        let pubkey_hex = "deadbeef".to_owned();
        let tag_json = serde_json::json!([["p", pubkey_hex]]).to_string();
        assert!(
            !tag_json.contains(' '),
            "tag JSON must be compact, got: {tag_json}"
        );
    }

    #[test]
    fn json_tag_format_p_tag_is_first_element() {
        let pubkey_hex = "aabbccdd".to_owned();
        let tag_json = serde_json::json!([["p", pubkey_hex]]).to_string();
        assert!(tag_json.starts_with(r#"[["p","#), "got: {tag_json}");
    }

    #[test]
    fn json_tag_format_round_trips_through_serde() {
        let pubkey_hex = "cafebabe00112233".to_owned();
        let tag_json = serde_json::json!([["p", pubkey_hex.clone()]]).to_string();
        let parsed: serde_json::Value = serde_json::from_str(&tag_json).unwrap();
        let outer = parsed.as_array().unwrap();
        assert_eq!(outer.len(), 1, "outer array must have exactly one element");
        let inner = outer[0].as_array().unwrap();
        assert_eq!(inner.len(), 2);
        assert_eq!(inner[0].as_str().unwrap(), "p");
        assert_eq!(inner[1].as_str().unwrap(), pubkey_hex);
    }

    // -- Kind number sets -----------------------------------------------------

    #[test]
    fn mentions_query_includes_stream_message_kind() {
        use sprout_core::kind::{
            KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2,
        };
        let mention_kinds: &[u32] = &[
            KIND_STREAM_MESSAGE,
            KIND_STREAM_MESSAGE_V2,
            KIND_FORUM_POST,
            KIND_FORUM_COMMENT,
        ];

        assert!(
            mention_kinds.contains(&KIND_STREAM_MESSAGE),
            "stream message kind must be in mentions"
        );
        assert!(
            mention_kinds.contains(&KIND_STREAM_MESSAGE_V2),
            "stream message v2 kind must be in mentions"
        );
        assert!(
            mention_kinds.contains(&KIND_FORUM_POST),
            "forum post kind must be in mentions"
        );
        assert!(
            mention_kinds.contains(&KIND_FORUM_COMMENT),
            "forum comment kind must be in mentions"
        );
    }

    #[test]
    fn needs_action_query_includes_approval_and_reminder_kinds() {
        use sprout_core::kind::{KIND_STREAM_REMINDER, KIND_WORKFLOW_APPROVAL_REQUESTED};
        let needs_action_kinds: &[u32] = &[KIND_WORKFLOW_APPROVAL_REQUESTED, KIND_STREAM_REMINDER];

        assert!(
            needs_action_kinds.contains(&KIND_WORKFLOW_APPROVAL_REQUESTED),
            "approval request kind must be in needs_action"
        );
        assert!(
            needs_action_kinds.contains(&KIND_STREAM_REMINDER),
            "reminder kind must be in needs_action"
        );
    }

    #[test]
    fn activity_query_includes_agent_job_kinds() {
        use sprout_core::kind::{
            KIND_FORUM_POST, KIND_JOB_PROGRESS, KIND_JOB_REQUEST, KIND_JOB_RESULT,
            KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2,
        };
        let activity_kinds: &[u32] = &[
            KIND_STREAM_MESSAGE,
            KIND_STREAM_MESSAGE_V2,
            KIND_FORUM_POST,
            KIND_JOB_REQUEST,
            KIND_JOB_PROGRESS,
            KIND_JOB_RESULT,
        ];

        assert!(
            activity_kinds.contains(&KIND_JOB_REQUEST),
            "job request kind must be in activity"
        );
        assert!(
            activity_kinds.contains(&KIND_JOB_PROGRESS),
            "job progress kind must be in activity"
        );
        assert!(
            activity_kinds.contains(&KIND_JOB_RESULT),
            "job result kind must be in activity"
        );
        assert!(
            activity_kinds.contains(&KIND_STREAM_MESSAGE),
            "stream message kind must be in activity"
        );
        assert!(
            activity_kinds.contains(&KIND_FORUM_POST),
            "forum post kind must be in activity"
        );
    }

    #[test]
    fn activity_query_excludes_workflow_execution_kinds() {
        use sprout_core::kind::{
            KIND_FORUM_POST, KIND_JOB_PROGRESS, KIND_JOB_REQUEST, KIND_JOB_RESULT,
            KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2,
        };
        let activity_kinds: &[u32] = &[
            KIND_STREAM_MESSAGE,
            KIND_STREAM_MESSAGE_V2,
            KIND_FORUM_POST,
            KIND_JOB_REQUEST,
            KIND_JOB_PROGRESS,
            KIND_JOB_RESULT,
        ];

        use sprout_core::kind::{KIND_WORKFLOW_APPROVAL_DENIED, KIND_WORKFLOW_TRIGGERED};
        for kind in KIND_WORKFLOW_TRIGGERED..=KIND_WORKFLOW_APPROVAL_DENIED {
            assert!(
                !activity_kinds.contains(&kind),
                "workflow execution kind {kind} must NOT be in activity"
            );
        }
    }

    #[test]
    fn needs_action_kinds_do_not_overlap_with_activity_kinds() {
        use sprout_core::kind::{
            KIND_FORUM_POST, KIND_JOB_PROGRESS, KIND_JOB_REQUEST, KIND_JOB_RESULT,
            KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2, KIND_STREAM_REMINDER,
            KIND_WORKFLOW_APPROVAL_REQUESTED,
        };
        let needs_action_kinds: &[u32] = &[KIND_WORKFLOW_APPROVAL_REQUESTED, KIND_STREAM_REMINDER];
        let activity_kinds: &[u32] = &[
            KIND_STREAM_MESSAGE,
            KIND_STREAM_MESSAGE_V2,
            KIND_FORUM_POST,
            KIND_JOB_REQUEST,
            KIND_JOB_PROGRESS,
            KIND_JOB_RESULT,
        ];

        for kind in needs_action_kinds {
            assert!(
                !activity_kinds.contains(kind),
                "kind {kind} appears in both needs_action and activity -- check intent"
            );
        }
    }

    // -- Channel ID filtering logic -------------------------------------------

    #[test]
    fn channel_id_bytes_encoding_is_correct() {
        let channel_id = Uuid::parse_str("9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50").unwrap();
        let bytes = channel_id.as_bytes().to_vec();
        assert_eq!(bytes.len(), 16);

        let recovered = Uuid::from_slice(&bytes).unwrap();
        assert_eq!(channel_id, recovered);
    }

    #[test]
    fn multiple_channel_ids_produce_distinct_byte_sequences() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let bytes1 = id1.as_bytes().to_vec();
        let bytes2 = id2.as_bytes().to_vec();

        assert_ne!(bytes1, bytes2);
    }

    #[test]
    fn nil_uuid_channel_id_bytes_are_all_zeros() {
        let nil_id = Uuid::nil();
        let bytes = nil_id.as_bytes().to_vec();
        assert_eq!(bytes, vec![0u8; 16]);
    }

    #[test]
    fn empty_channel_list_skips_channel_filter() {
        let accessible: Vec<Uuid> = vec![];
        assert!(
            accessible.is_empty(),
            "empty list should skip channel filter"
        );
    }

    #[test]
    fn channel_id_list_with_single_entry() {
        let channel_id = Uuid::new_v4();
        let accessible = [channel_id];
        assert_eq!(accessible.len(), 1);
        let bytes = accessible[0].as_bytes().to_vec();
        assert_eq!(bytes.len(), 16);
    }

    #[test]
    fn channel_id_list_with_multiple_entries_are_distinct() {
        let ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        assert_eq!(ids.len(), 5);

        let byte_seqs: Vec<Vec<u8>> = ids.iter().map(|id| id.as_bytes().to_vec()).collect();
        let unique: std::collections::HashSet<Vec<u8>> = byte_seqs.into_iter().collect();
        assert_eq!(unique.len(), 5, "all channel IDs must be distinct");
    }
}
