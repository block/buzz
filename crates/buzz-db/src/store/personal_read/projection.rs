//! Bounded evidence projection. The cap limits evidence, not the definition of
//! unread: an unexamined tail yields a positive lower bound or an unknown count.

use super::{model::*, participation, writes};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Acquire, PgConnection, Row};
use std::cmp::Reverse;
use std::collections::{hash_map::Entry, HashMap};
use uuid::Uuid;

use crate::{observability, Db, DbError, Result};

impl Db {
    /// Read a bounded joined roster from the writer. One SQL statement projects
    /// channels, event evidence and read authority at a compatible MVCC cut.
    /// Callers must recheck admission/resource access before releasing this data.
    pub async fn personal_read_sidebar(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        retention_seconds: u32,
        limit: usize,
        after: Option<Uuid>,
    ) -> Result<SidebarPage> {
        if !(1..=MAX_CHANNELS).contains(&limit) {
            return Err(DbError::InvalidData("invalid sidebar limit".into()));
        }
        self.sidebar(community, actor, retention_seconds, limit, after, None)
            .await
    }

    /// Refresh specific joined channels in one snapshot. A requested channel
    /// absent from the result was not a joined, nondeleted channel at that cut.
    pub async fn personal_read_sidebar_channels(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        retention_seconds: u32,
        channels: &[Uuid],
    ) -> Result<SidebarPage> {
        let unique: std::collections::HashSet<_> = channels.iter().collect();
        if !(1..=MAX_CHANNELS).contains(&channels.len()) || unique.len() != channels.len() {
            return Err(DbError::InvalidData("invalid sidebar channels".into()));
        }
        self.sidebar(
            community,
            actor,
            retention_seconds,
            channels.len(),
            None,
            Some(channels),
        )
        .await
    }

    async fn sidebar(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        retention_seconds: u32,
        limit: usize,
        after: Option<Uuid>,
        only: Option<&[Uuid]>,
    ) -> Result<SidebarPage> {
        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut tx = conn.begin().await?;
        // The horizon and frontier evidence share one read-only cut.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        writes::deadlines(&mut tx).await?;
        sqlx::query("SET LOCAL jit = off").execute(&mut *tx).await?;
        let actor_bytes = actor.to_bytes();
        let account = read_account(&mut tx, community, &actor_bytes, retention_seconds).await?;
        // The inner event LIMIT is deliberately before eligibility filtering.
        // This bounds rows/joins even with long deleted or self-authored runs.
        // Aggregate equivalent eligible evidence before transfer. Multiplicity
        // preserves counts; raw scan count (not group count) proves exhaustion.
        // SQL eligibility mirrors classification::eligible (PostgreSQL parity test).
        // Canonical covered rows need no tags; missing ancestry stays unknown.
        // Validate relevant tag parts before compacting directed/ancestry facts.
        // PostgreSQL scalar "p" ->> 0 is "p": the type check must reject it too.
        // Canonical timeline roots share an empty (present) root sentinel.
        // Tags are bounded before transfer; oversized/corrupt evidence stays
        // unknown, never falsely top-level/unmentioned/read.
        // Latest comes from the unread scan, so its ID arrives no earlier than
        // anything counted; the shallow probe answers only when the horizon
        // holds no message. Both are newest-first by author time, so the deeper
        // one finds the same greatest author time whenever the probe finds any.
        let rows = sqlx::query(
            r#"WITH roster AS MATERIALIZED (
                SELECT c.id,c.name,c.channel_type::text AS channel_type,
                    c.archived_at IS NOT NULL AS archived, cm.hidden_at IS NOT NULL AS hidden
                FROM channel_members cm JOIN channels c
                    ON c.community_id=cm.community_id AND c.id=cm.channel_id
                WHERE cm.community_id=$1 AND cm.pubkey=$2 AND cm.removed_at IS NULL
                    AND c.deleted_at IS NULL AND ($3::uuid IS NULL OR c.id>$3)
                    AND ($9::uuid[] IS NULL OR c.id=ANY($9))
                ORDER BY c.id LIMIT $4
             )
             SELECT r.*, COALESCE(e.latest_message_id, latest.latest_message_id) AS latest_message_id,
                COALESCE(e.latest_message_at, latest.latest_message_at) AS latest_message_at,
                (COALESCE(e.latest_message_id, latest.latest_message_id) IS NOT NULL
                    OR latest.candidates <= $5-1) AS latest_message_complete,
                e.scanned,COALESCE(e.evidence,'[]'::jsonb) AS evidence FROM roster r
             LEFT JOIN LATERAL (
                WITH candidates AS MATERIALIZED (
                    SELECT id,created_at,received_at,kind,deleted_at FROM events
                    WHERE community_id=$1 AND channel_id=r.id
                    ORDER BY created_at DESC,id LIMIT $5
                )
                SELECT count(*) AS candidates,
                    (array_agg(encode(id,'hex') ORDER BY received_at DESC,id)
                        FILTER (WHERE kind=ANY($6) AND deleted_at IS NULL))[1] AS latest_message_id,
                    (array_agg(extract(epoch FROM created_at)::bigint ORDER BY created_at DESC,id)
                        FILTER (WHERE kind=ANY($6) AND deleted_at IS NULL))[1] AS latest_message_at
                FROM candidates
             ) latest ON true
             LEFT JOIN personal_read_frontiers cf ON cf.community_id=$1 AND cf.actor=$2
                AND cf.channel_id=r.id AND cf.root_id=''::bytea
             LEFT JOIN LATERAL (
                WITH candidates AS MATERIALIZED (
                    SELECT id,pubkey,created_at,received_at,deleted_at,kind,tags
                    FROM events WHERE community_id=$1 AND channel_id=r.id AND created_at >= $7
                    ORDER BY created_at DESC,id LIMIT $8
                ), classified AS (
                    SELECT e.*, tm.root_event_id AS root, tm.parent_event_id AS parent,
                        COALESCE(tm.root_event_id<>e.id,false) AS is_reply,
                        -- Every frontier is floored at the account's start.
                        COALESCE(e.received_at <= GREATEST(COALESCE($10::timestamptz,'infinity'),
                            CASE WHEN tm.root_event_id IS NOT NULL AND tm.root_event_id<>e.id
                                THEN GREATEST(tf.through_timestamp, cf.threads_through_timestamp)
                                ELSE cf.through_timestamp END),false) AS covered
                    FROM (SELECT * FROM candidates ORDER BY created_at DESC,id LIMIT $8-1) e
                    LEFT JOIN thread_metadata tm ON tm.community_id=$1 AND tm.channel_id=r.id
                        AND tm.event_created_at=e.created_at AND tm.event_id=e.id
                    LEFT JOIN personal_read_frontiers tf ON tf.community_id=$1 AND tf.actor=$2
                        AND tf.channel_id=r.id AND tf.root_id=tm.root_event_id AND tm.root_event_id<>e.id
                    WHERE e.kind=ANY($6) AND e.pubkey<>$2 AND e.deleted_at IS NULL
                ), grouped AS (
                    SELECT CASE WHEN root IS NULL THEN NULL
                            WHEN is_reply THEN encode(root,'hex') ELSE '' END AS root,
                        CASE WHEN is_reply THEN encode(parent,'hex') END AS parent,
                        is_reply, covered,
                        -- ->>0 also selects scalar "p"/"e": reject nonarrays first.
                        -- C collation matches Rust's ASCII case/hex rules. Reply
                        -- markers matter only without canonical ancestry; otherwise
                        -- metadata, not tag spelling, owns the context.
                        CASE WHEN octet_length(tags::text)<=8192
                            AND jsonb_typeof(tags)='array' THEN
                            (SELECT CASE WHEN bool_or(jsonb_typeof(tag)<>'array'
                                    OR jsonb_path_exists(tag,'strict $[*] ? (@.type() != "string")'))
                                THEN NULL ELSE jsonb_build_object(
                                    'directed',COALESCE(bool_or(
                                        (tag->>0='p' AND lower((tag->>1) COLLATE "C")=encode($2,'hex'))
                                        OR (tag->>0='broadcast' AND tag->>1='1')),false),
                                    'reply_marked',root IS NULL AND COALESCE(bool_or(tag->>0='e'
                                        AND tag->>3='reply'
                                        AND (tag->>1) COLLATE "C" ~ '^[0123456789abcdefABCDEF]{64}$'),false)) END
                             FROM jsonb_array_elements(tags) t(tag)
                             WHERE tag->>0 IN ('p','broadcast','e')) ELSE NULL END AS facts,
                        count(*) AS n,
                        (extract(epoch FROM max(received_at))*1000000)::bigint AS newest_arrival,
                        (array_agg(encode(id,'hex') ORDER BY received_at DESC,id))[1] AS newest_id,
                        (array_agg(extract(epoch FROM created_at)::bigint
                            ORDER BY received_at DESC,id))[1] AS newest_at
                    FROM classified
                    WHERE NOT covered OR root IS NULL
                    GROUP BY 1,2,3,4,5
                ), scan AS (
                    SELECT count(*) AS scanned,
                        (array_agg(encode(id,'hex') ORDER BY received_at DESC,id)
                            FILTER (WHERE kind=ANY($6) AND deleted_at IS NULL))[1] AS latest_message_id,
                        max(extract(epoch FROM created_at)::bigint)
                            FILTER (WHERE kind=ANY($6) AND deleted_at IS NULL) AS latest_message_at
                    FROM candidates
                )
                SELECT scan.*, (SELECT jsonb_agg(to_jsonb(grouped)) FROM grouped) AS evidence FROM scan
             ) e ON true ORDER BY r.id"#,
        ).bind(community.as_uuid()).bind(actor_bytes.as_slice()).bind(after)
            .bind((limit+1) as i64).bind((MAX_CHANNEL_SCAN+1) as i64)
            .bind(ELIGIBLE_KINDS.as_slice())
            .bind(DateTime::from_timestamp_millis(account.cutoff_ms)
                .ok_or_else(|| DbError::InvalidData("invalid unread cutoff".into()))?)
            .bind((MAX_UNREAD_SCAN+1) as i64)
            .bind(only)
            .bind(account.started_at)
            .fetch_all(&mut *tx).await?;
        let has_more = rows.len() > limit;
        let mut channels = Vec::new();
        let mut pending = Vec::new();
        for row in rows.into_iter().take(limit) {
            let evidence: Value = row.try_get("evidence")?;
            let evidence = evidence
                .as_array()
                .ok_or_else(|| DbError::InvalidData("invalid sidebar evidence".into()))?;
            let complete = row.try_get::<i64, _>("scanned")? <= MAX_UNREAD_SCAN as i64;
            let channel_type: String = row.try_get("channel_type")?;
            let mut unread = 0;
            let mut attention = 0;
            let mut unread_complete = complete;
            let mut threads: HashMap<Vec<u8>, Replies> = HashMap::new();
            let mut undirected = Vec::new();
            for e in evidence {
                let n = e["n"]
                    .as_u64()
                    .filter(|n| *n <= MAX_UNREAD_SCAN as u64)
                    .ok_or_else(|| DbError::InvalidData("invalid evidence multiplicity".into()))?
                    as u32;
                let Some(facts) = e["facts"].as_object() else {
                    unread_complete = false;
                    continue;
                };
                // SQL's bounded ancestry fact is parity-tested against the shared
                // NIP-10 parser; no raw tag payload crosses the DB boundary.
                if facts.get("reply_marked") == Some(&Value::Bool(true)) && e["root"].is_null() {
                    unread_complete = false;
                    continue;
                }
                if e["covered"] == true {
                    continue;
                }
                let directed =
                    channel_type == "dm" || facts.get("directed") == Some(&Value::Bool(true));
                // Roots are timeline messages; descendants belong exclusively
                // to their canonical thread. Never inherit the channel prefix.
                if e["is_reply"] != true {
                    unread += n;
                    if directed {
                        attention += n;
                    }
                    continue;
                }
                let Some(root) = e["root"].as_str().and_then(writes::event_id) else {
                    unread_complete = false;
                    continue;
                };
                let replies = Replies {
                    n,
                    newest: (
                        e["newest_arrival"].as_i64().ok_or_else(invalid_newest)?,
                        e["newest_id"]
                            .as_str()
                            .ok_or_else(invalid_newest)?
                            .to_owned(),
                    ),
                    newest_at: e["newest_at"].as_i64().ok_or_else(invalid_newest)?,
                };
                // A directed reply counts whatever its conversation. Any other
                // reply counts only in one of the actor's conversations.
                if directed {
                    count(&mut threads, root, replies);
                } else if let Some(parent) = e["parent"].as_str().and_then(writes::event_id) {
                    undirected.push((root, parent, replies));
                } else {
                    unread_complete = false;
                }
            }
            pending.push((threads, undirected, unread, attention, unread_complete));
            channels.push(ChannelReadSummary {
                channel_id: row.try_get("id")?,
                name: row.try_get("name")?,
                channel_type,
                archived: row.try_get("archived")?,
                hidden: row.try_get("hidden")?,
                // Counts and threads wait for conversation membership below.
                unread: ReadCount::Unknown,
                attention: ReadCount::Unknown,
                latest_message_id: row.try_get("latest_message_id")?,
                latest_message_at: row.try_get("latest_message_at")?,
                latest_message_complete: row.try_get("latest_message_complete")?,
                threads: ThreadSummaries {
                    items: Vec::new(),
                    complete: false,
                },
            });
        }
        let targets: Vec<_> = channels
            .iter()
            .zip(&pending)
            .flat_map(|(channel, (_, undirected, ..))| {
                undirected
                    .iter()
                    .map(|(_, parent, _)| (channel.channel_id, parent.clone()))
            })
            .collect();
        let members = participation::resolve(&mut tx, community, &actor_bytes, &targets).await?;
        for (channel, (mut threads, undirected, mut unread, mut attention, mut complete)) in
            channels.iter_mut().zip(pending)
        {
            // Relevance is decided before counts, previews and the thread cap.
            for (root, parent, replies) in undirected {
                match members.get(&(channel.channel_id, parent)) {
                    Some(true) => count(&mut threads, root, replies),
                    Some(false) => {}
                    None => complete = false,
                }
            }
            let mut items = Vec::with_capacity(threads.len());
            for (root, thread) in threads {
                unread += thread.n;
                attention += thread.n;
                items.push(ThreadReadSummary {
                    root_id: hex::encode(root),
                    unread: ReadCount::from_evidence(thread.n, complete),
                    latest_reply_id: thread.newest.1,
                    latest_reply_at: thread.newest_at,
                });
            }
            channel.unread = ReadCount::from_evidence(unread, complete);
            channel.attention = ReadCount::from_evidence(attention, complete);
            channel.threads = summarize(items, complete);
        }
        let next_cursor = if has_more {
            channels.last().map(|c| c.channel_id)
        } else {
            None
        };
        tx.commit().await?;
        Ok(SidebarPage {
            account,
            channels,
            next_cursor,
        })
    }
}

/// Uncovered replies in one canonical thread: one evidence group, or the
/// thread's counted total.
struct Replies {
    n: u32,
    /// Last of them to arrive: (arrival microseconds, lowercase hex ID).
    newest: (i64, String),
    /// Its author seconds.
    newest_at: i64,
}

/// Add replies that count to their thread.
fn count(threads: &mut HashMap<Vec<u8>, Replies>, root: Vec<u8>, replies: Replies) {
    match threads.entry(root) {
        Entry::Vacant(slot) => {
            slot.insert(replies);
        }
        Entry::Occupied(mut slot) => {
            let thread = slot.get_mut();
            thread.n += replies.n;
            // Latest arrival first; equal arrivals break toward the smaller ID.
            if (replies.newest.0, Reverse(&replies.newest.1))
                > (thread.newest.0, Reverse(&thread.newest.1))
            {
                thread.newest = replies.newest;
                thread.newest_at = replies.newest_at;
            }
        }
    }
}

fn invalid_newest() -> DbError {
    DbError::InvalidData("invalid thread evidence".into())
}

/// Order newest unread reply first (root ID breaks ties) and cap the list. The
/// list is complete only when evidence was exhausted and nothing was omitted.
pub(super) fn summarize(
    mut items: Vec<ThreadReadSummary>,
    evidence_complete: bool,
) -> ThreadSummaries {
    items.sort_unstable_by(|a, b| {
        b.latest_reply_at
            .cmp(&a.latest_reply_at)
            .then_with(|| a.root_id.cmp(&b.root_id))
    });
    let complete = evidence_complete && items.len() <= MAX_THREAD_SUMMARIES;
    items.truncate(MAX_THREAD_SUMMARIES);
    ThreadSummaries { items, complete }
}

/// Read-time horizon and the account's start, in the caller's snapshot.
/// Frontier state is not discarded on expiry.
pub(super) async fn read_account(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
    retention_seconds: u32,
) -> Result<ReadAccount> {
    let row = sqlx::query(
        "SELECT date_trunc('milliseconds',transaction_timestamp()-make_interval(secs=>$1::double precision)) AS cutoff,
            (SELECT started_at FROM personal_read_accounts WHERE community_id=$2 AND actor=$3) AS started_at",
    )
    .bind(f64::from(retention_seconds))
    .bind(community.as_uuid())
    .bind(actor)
    .fetch_one(conn)
    .await?;
    let cutoff: DateTime<Utc> = row.try_get("cutoff")?;
    Ok(ReadAccount {
        retention_seconds,
        cutoff_ms: cutoff.timestamp_millis(),
        started_at: row.try_get("started_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(root: u8, at: i64) -> ThreadReadSummary {
        ThreadReadSummary {
            root_id: hex::encode([root; 32]),
            unread: ReadCount::Exact { value: 1 },
            latest_reply_id: hex::encode([root; 32]),
            latest_reply_at: at,
        }
    }

    #[test]
    fn thread_summaries_order_newest_first_break_ties_by_root_and_cap_at_five() {
        // Literal contract boundaries deliberately do not derive from the constant.
        for count in [0_u8, 1, 4, 5, 6, 7] {
            // Descending roots with pairwise-equal times exercise the tie-break.
            let items: Vec<_> = (0..count)
                .rev()
                .map(|i| item(i, i64::from(i / 2)))
                .collect();
            let summaries = summarize(items, true);
            let roots: Vec<_> = summaries.items.iter().map(|t| t.root_id.clone()).collect();
            let mut expected: Vec<_> = (0..count).map(|i| (i64::from(i / 2), i)).collect();
            expected.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            let expected: Vec<_> = expected
                .into_iter()
                .take(5)
                .map(|(_, i)| hex::encode([i; 32]))
                .collect();
            assert_eq!(roots, expected, "count {count}");
            assert_eq!(summaries.complete, count <= 5, "count {count}");
        }
    }

    #[test]
    fn thread_summaries_are_incomplete_when_evidence_is() {
        let summaries = summarize(vec![item(1, 1)], false);
        assert_eq!(summaries.items.len(), 1);
        assert!(!summaries.complete);
    }
}
