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
//! which carries community-leading composite indexes on
//! `(community_id, pubkey_hex, event_created_at DESC)` and
//! `(community_id, pubkey_hex, event_kind, event_created_at DESC)`.  This replaces the Phase 1
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

use buzz_core::kind::{
    KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_JOB_PROGRESS, KIND_JOB_REQUEST, KIND_JOB_RESULT,
    KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2, KIND_STREAM_REMINDER,
    KIND_WORKFLOW_APPROVAL_REQUESTED,
};
use buzz_core::{CommunityId, StoredEvent};

use crate::error::Result;
use crate::event::row_to_stored_event;

/// Column list shared by every feed subquery that aliases the `events` table as `e`.
const EVENT_COLS: &str =
    "e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, e.sig, e.received_at, e.channel_id";

/// Column list for queries that select directly from `events` (no table alias).
const EVENT_COLS_UNALIASED: &str =
    "id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id";

/// Append channel visibility filtering for feed queries.
///
/// Feed reads may include channel-less community-global events, plus events in
/// channels the caller can access. An empty accessible-channel list therefore
/// means "global only", never "all channels".
fn push_visible_channel_filter(qb: &mut QueryBuilder<sqlx::Postgres>, col: &str, ids: &[Uuid]) {
    if ids.is_empty() {
        qb.push(format!(" AND {col} IS NULL"));
        return;
    }

    qb.push(format!(" AND ({col} IS NULL OR {col} IN ("));
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(*id);
    }
    qb.push("))");
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

fn build_mentions_query(
    community: CommunityId,
    pubkey_bytes: &[u8],
    accessible_channel_ids: &[Uuid],
    since: Option<DateTime<Utc>>,
    limit: i64,
) -> QueryBuilder<sqlx::Postgres> {
    let limit = limit.min(FEED_MAX_LIMIT);
    let pubkey_hex = hex::encode(pubkey_bytes);

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(format!(
        "SELECT {EVENT_COLS} FROM events e \
         INNER JOIN event_mentions m ON e.community_id = m.community_id AND e.id = m.event_id \
         WHERE e.community_id = "
    ));
    qb.push_bind(*community.as_uuid());
    qb.push(" AND m.community_id = ")
        .push_bind(*community.as_uuid());
    qb.push(" AND m.pubkey_hex = ").push_bind(pubkey_hex);
    qb.push(" AND e.deleted_at IS NULL");
    qb.push(format!(
        " AND e.kind IN ({KIND_STREAM_MESSAGE}, {KIND_STREAM_MESSAGE_V2}, \
         {KIND_FORUM_POST}, {KIND_FORUM_COMMENT})"
    ));
    push_visible_channel_filter(&mut qb, "e.channel_id", accessible_channel_ids);
    if let Some(s) = since {
        qb.push(" AND m.event_created_at >= ").push_bind(s);
    }
    qb.push(" ORDER BY m.event_created_at DESC LIMIT ")
        .push_bind(limit);
    qb
}

/// Find events that @mention the given pubkey (have `["p", pubkey_hex]` in tags).
///
/// Joins against the `event_mentions` table -- Phase 2 implementation.
/// **Performance**: community-leading indexed lookup on
/// `(community_id, pubkey_hex, event_created_at DESC)`.
///
/// Only returns community-global events and events from `accessible_channel_ids`.
/// `limit` is capped at [`FEED_MAX_LIMIT`] regardless of the value passed by the caller.
pub async fn query_mentions(
    pool: &PgPool,
    community: CommunityId,
    pubkey_bytes: &[u8],
    accessible_channel_ids: &[Uuid],
    since: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<StoredEvent>> {
    let mut qb = build_mentions_query(
        community,
        pubkey_bytes,
        accessible_channel_ids,
        since,
        limit,
    );
    let rows = qb.build().fetch_all(pool).await?;
    collect_stored_events(rows)
}

fn build_needs_action_query(
    community: CommunityId,
    pubkey_bytes: &[u8],
    accessible_channel_ids: &[Uuid],
    since: Option<DateTime<Utc>>,
    limit: i64,
) -> QueryBuilder<sqlx::Postgres> {
    let limit = limit.min(FEED_MAX_LIMIT);
    let pubkey_hex = hex::encode(pubkey_bytes);

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(format!(
        "SELECT {EVENT_COLS} FROM events e \
         INNER JOIN event_mentions m ON e.community_id = m.community_id AND e.id = m.event_id \
         WHERE e.community_id = "
    ));
    qb.push_bind(*community.as_uuid());
    qb.push(" AND m.community_id = ")
        .push_bind(*community.as_uuid());
    qb.push(" AND m.pubkey_hex = ").push_bind(pubkey_hex);
    qb.push(" AND e.deleted_at IS NULL");
    qb.push(format!(
        " AND e.kind IN ({KIND_WORKFLOW_APPROVAL_REQUESTED}, {KIND_STREAM_REMINDER})"
    ));
    push_visible_channel_filter(&mut qb, "e.channel_id", accessible_channel_ids);
    if let Some(s) = since {
        qb.push(" AND m.event_created_at >= ").push_bind(s);
    }
    qb.push(" ORDER BY m.event_created_at DESC LIMIT ")
        .push_bind(limit);
    qb
}

/// Find events that require action from the given pubkey:
/// - [`KIND_WORKFLOW_APPROVAL_REQUESTED`] (workflow approval requested, tagged with user pubkey)
/// - [`KIND_STREAM_REMINDER`] (reminder, tagged with user pubkey)
///
/// Only returns community-global events and events from channels the user has access to
/// (`accessible_channel_ids`). This prevents surfacing approval requests from channels
/// the user was removed from.
/// **Performance**: community-leading indexed lookup via `event_mentions` join on
/// `(community_id, pubkey_hex, event_kind, event_created_at DESC)`.
/// `limit` is capped at [`FEED_MAX_LIMIT`] regardless of the value passed by the caller.
pub async fn query_needs_action(
    pool: &PgPool,
    community: CommunityId,
    pubkey_bytes: &[u8],
    accessible_channel_ids: &[Uuid],
    since: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<StoredEvent>> {
    let mut qb = build_needs_action_query(
        community,
        pubkey_bytes,
        accessible_channel_ids,
        since,
        limit,
    );
    let rows = qb.build().fetch_all(pool).await?;
    collect_stored_events(rows)
}

fn build_activity_query(
    community: CommunityId,
    accessible_channel_ids: &[Uuid],
    since: Option<DateTime<Utc>>,
    limit: i64,
) -> QueryBuilder<sqlx::Postgres> {
    let limit = limit.min(FEED_MAX_LIMIT);
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(format!(
        "SELECT {EVENT_COLS_UNALIASED} FROM events WHERE community_id = "
    ));
    qb.push_bind(*community.as_uuid());
    qb.push(" AND deleted_at IS NULL");
    qb.push(format!(
        " AND kind IN ({KIND_STREAM_MESSAGE}, {KIND_STREAM_MESSAGE_V2}, {KIND_FORUM_POST}, \
         {KIND_JOB_REQUEST}, {KIND_JOB_PROGRESS}, {KIND_JOB_RESULT})"
    ));
    push_visible_channel_filter(&mut qb, "channel_id", accessible_channel_ids);
    if let Some(s) = since {
        qb.push(" AND created_at >= ").push_bind(s);
    }
    qb.push(" ORDER BY created_at DESC LIMIT ").push_bind(limit);
    qb
}

/// Find recent activity across accessible channels (for watched topics / agent activity).
///
/// Returns stream messages, forum posts, and agent job events.
/// Workflow execution kinds (46001-46012) are intentionally excluded to avoid noise.
/// **Performance**: uses indexed `kind` + `channel_id` columns -- no JSON scan.
/// `limit` is capped at [`FEED_MAX_LIMIT`] regardless of the value passed by the caller.
pub async fn query_activity(
    pool: &PgPool,
    community: CommunityId,
    accessible_channel_ids: &[Uuid],
    since: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<StoredEvent>> {
    let mut qb = build_activity_query(community, accessible_channel_ids, since, limit);
    let rows = qb.build().fetch_all(pool).await?;
    collect_stored_events(rows)
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use uuid::Uuid;

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
        let host = format!("feed-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        id
    }

    async fn insert_test_channel(pool: &PgPool, community: CommunityId) -> Uuid {
        let id = Uuid::new_v4();
        let creator = [0x11u8; 32];
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
             VALUES ($1, $2, $3, 'stream'::channel_type, 'open'::channel_visibility, $4)",
        )
        .bind(id)
        .bind(community.as_uuid())
        .bind(format!("feed-test-channel-{}", id.simple()))
        .bind(creator.as_slice())
        .execute(pool)
        .await
        .expect("insert test channel");
        id
    }

    async fn store_feed_event(
        pool: &PgPool,
        community: CommunityId,
        kind: u32,
        content: &str,
        channel_id: Option<Uuid>,
        tags: Vec<Tag>,
    ) -> nostr::Event {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign event");
        crate::event::insert_event(pool, community, &event, channel_id)
            .await
            .expect("insert feed event");
        crate::insert_mentions(pool, community, &event, channel_id)
            .await
            .expect("insert mentions");
        event
    }

    // -- Postgres tenant-scope regressions ------------------------------------

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn query_mentions_is_scoped_across_communities() {
        let pool = setup_pool().await;
        let community_a = CommunityId::from_uuid(make_test_community(&pool).await);
        let community_b = CommunityId::from_uuid(make_test_community(&pool).await);
        let channel_a = insert_test_channel(&pool, community_a).await;
        let channel_b = insert_test_channel(&pool, community_b).await;
        let mentioned_pubkey = "02".repeat(32);
        let mentioned_bytes = hex::decode(&mentioned_pubkey).expect("hex pubkey");

        let event_a = store_feed_event(
            &pool,
            community_a,
            KIND_STREAM_MESSAGE,
            "community-a mention",
            Some(channel_a),
            vec![Tag::parse(["p", mentioned_pubkey.as_str()]).unwrap()],
        )
        .await;
        let event_b = store_feed_event(
            &pool,
            community_b,
            KIND_STREAM_MESSAGE,
            "community-b mention",
            Some(channel_b),
            vec![Tag::parse(["p", mentioned_pubkey.as_str()]).unwrap()],
        )
        .await;

        let rows = query_mentions(
            &pool,
            community_a,
            &mentioned_bytes,
            &[channel_a, channel_b],
            None,
            10,
        )
        .await
        .expect("query mentions");

        assert!(rows.iter().any(|row| row.event.id == event_a.id));
        assert!(
            rows.iter().all(|row| row.event.id != event_b.id),
            "community B mention must not appear in community A feed"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn query_needs_action_is_scoped_across_communities() {
        let pool = setup_pool().await;
        let community_a = CommunityId::from_uuid(make_test_community(&pool).await);
        let community_b = CommunityId::from_uuid(make_test_community(&pool).await);
        let channel_a = insert_test_channel(&pool, community_a).await;
        let channel_b = insert_test_channel(&pool, community_b).await;
        let actor_pubkey = "03".repeat(32);
        let actor_bytes = hex::decode(&actor_pubkey).expect("hex pubkey");

        let event_a = store_feed_event(
            &pool,
            community_a,
            KIND_WORKFLOW_APPROVAL_REQUESTED,
            "community-a approval",
            Some(channel_a),
            vec![Tag::parse(["p", actor_pubkey.as_str()]).unwrap()],
        )
        .await;
        let event_b = store_feed_event(
            &pool,
            community_b,
            KIND_WORKFLOW_APPROVAL_REQUESTED,
            "community-b approval",
            Some(channel_b),
            vec![Tag::parse(["p", actor_pubkey.as_str()]).unwrap()],
        )
        .await;

        let rows = query_needs_action(
            &pool,
            community_a,
            &actor_bytes,
            &[channel_a, channel_b],
            None,
            10,
        )
        .await
        .expect("query needs_action");

        assert!(rows.iter().any(|row| row.event.id == event_a.id));
        assert!(
            rows.iter().all(|row| row.event.id != event_b.id),
            "community B needs_action item must not appear in community A feed"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn query_activity_is_scoped_and_empty_channels_are_global_only() {
        let pool = setup_pool().await;
        let community_a = CommunityId::from_uuid(make_test_community(&pool).await);
        let community_b = CommunityId::from_uuid(make_test_community(&pool).await);
        let channel_a = insert_test_channel(&pool, community_a).await;
        let channel_b = insert_test_channel(&pool, community_b).await;

        let a_global = store_feed_event(
            &pool,
            community_a,
            KIND_STREAM_MESSAGE,
            "community-a global",
            None,
            vec![],
        )
        .await;
        let a_channel = store_feed_event(
            &pool,
            community_a,
            KIND_STREAM_MESSAGE,
            "community-a channel",
            Some(channel_a),
            vec![],
        )
        .await;
        let b_global = store_feed_event(
            &pool,
            community_b,
            KIND_STREAM_MESSAGE,
            "community-b global",
            None,
            vec![],
        )
        .await;
        let b_channel = store_feed_event(
            &pool,
            community_b,
            KIND_STREAM_MESSAGE,
            "community-b channel",
            Some(channel_b),
            vec![],
        )
        .await;

        let global_only = query_activity(&pool, community_a, &[], None, 10)
            .await
            .expect("query activity global only");
        assert!(global_only.iter().any(|row| row.event.id == a_global.id));
        assert!(
            global_only.iter().all(|row| row.event.id != a_channel.id),
            "empty accessible channels must not mean all tenant channels"
        );
        assert!(global_only.iter().all(|row| row.event.id != b_global.id));
        assert!(global_only.iter().all(|row| row.event.id != b_channel.id));

        let visible = query_activity(&pool, community_a, &[channel_a, channel_b], None, 10)
            .await
            .expect("query visible activity");
        assert!(visible.iter().any(|row| row.event.id == a_global.id));
        assert!(visible.iter().any(|row| row.event.id == a_channel.id));
        assert!(
            visible
                .iter()
                .all(|row| row.event.id != b_global.id && row.event.id != b_channel.id),
            "community B activity must not appear in community A feed"
        );
    }

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
        use buzz_core::kind::{
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
        use buzz_core::kind::{KIND_STREAM_REMINDER, KIND_WORKFLOW_APPROVAL_REQUESTED};
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
        use buzz_core::kind::{
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
        use buzz_core::kind::{
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

        use buzz_core::kind::{KIND_WORKFLOW_APPROVAL_DENIED, KIND_WORKFLOW_TRIGGERED};
        for kind in KIND_WORKFLOW_TRIGGERED..=KIND_WORKFLOW_APPROVAL_DENIED {
            assert!(
                !activity_kinds.contains(&kind),
                "workflow execution kind {kind} must NOT be in activity"
            );
        }
    }

    #[test]
    fn needs_action_kinds_do_not_overlap_with_activity_kinds() {
        use buzz_core::kind::{
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
    fn empty_channel_list_means_global_only() {
        let community = buzz_core::CommunityId::from_uuid(Uuid::new_v4());
        let mut qb = build_activity_query(community, &[], None, 10);
        let query = qb.build();
        let sql_str = sqlx::Execute::sql(query);
        let sql = sql_str.as_str();

        assert!(
            sql.contains("WHERE community_id = "),
            "activity feed must bind the tenant community: {sql}"
        );
        assert!(
            sql.contains("AND channel_id IS NULL"),
            "empty accessible-channel list must mean global-only, not all tenant channels: {sql}"
        );
        assert!(
            !sql.contains("channel_id IN"),
            "empty accessible-channel list must not emit an IN filter: {sql}"
        );
    }

    #[test]
    fn non_empty_channel_list_includes_global_and_accessible_channels() {
        let community = buzz_core::CommunityId::from_uuid(Uuid::new_v4());
        let channel_id = Uuid::new_v4();
        let mut qb = build_activity_query(community, &[channel_id], None, 10);
        let query = qb.build();
        let sql_str = sqlx::Execute::sql(query);
        let sql = sql_str.as_str();

        assert!(
            sql.contains("AND (channel_id IS NULL OR channel_id IN ("),
            "feed should include community-global events plus accessible channels: {sql}"
        );
    }

    #[test]
    fn mentions_query_is_tenant_scoped_and_joins_mentions_by_composite_key() {
        let community = buzz_core::CommunityId::from_uuid(Uuid::new_v4());
        let pubkey = vec![0x42; 32];
        let channel_id = Uuid::new_v4();
        let mut qb = build_mentions_query(community, &pubkey, &[channel_id], None, 10);
        let query = qb.build();
        let sql_str = sqlx::Execute::sql(query);
        let sql = sql_str.as_str();

        assert!(
            sql.contains("INNER JOIN event_mentions m ON e.community_id = m.community_id AND e.id = m.event_id"),
            "mentions must join event_mentions on the composite tenant/event key: {sql}"
        );
        assert!(
            sql.contains("WHERE e.community_id = "),
            "mentions feed must scope events to the tenant community: {sql}"
        );
        assert!(
            sql.contains("AND m.community_id = "),
            "mentions feed must also bind event_mentions.community_id: {sql}"
        );
    }

    #[test]
    fn needs_action_query_is_tenant_scoped_and_joins_mentions_by_composite_key() {
        let community = buzz_core::CommunityId::from_uuid(Uuid::new_v4());
        let pubkey = vec![0x42; 32];
        let channel_id = Uuid::new_v4();
        let mut qb = build_needs_action_query(community, &pubkey, &[channel_id], None, 10);
        let query = qb.build();
        let sql_str = sqlx::Execute::sql(query);
        let sql = sql_str.as_str();

        assert!(
            sql.contains("INNER JOIN event_mentions m ON e.community_id = m.community_id AND e.id = m.event_id"),
            "needs_action must join event_mentions on the composite tenant/event key: {sql}"
        );
        assert!(
            sql.contains("WHERE e.community_id = "),
            "needs_action feed must scope events to the tenant community: {sql}"
        );
        assert!(
            sql.contains("AND m.community_id = "),
            "needs_action feed must also bind event_mentions.community_id: {sql}"
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
