//! Bounded evidence projection. The cap limits evidence, not the definition of
//! unread: an unexamined tail yields a lower bound, including lower-bound zero.

use super::{model::*, writes};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Acquire, PgConnection, Row};
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
        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut tx = conn.begin().await?;
        // Import status and frontier evidence share a compatible read-only cut.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        writes::deadlines(&mut tx).await?;
        let actor_bytes = actor.to_bytes();
        let account = read_account(&mut tx, community, &actor_bytes, retention_seconds).await?;
        // The inner event LIMIT is deliberately before eligibility filtering.
        // This bounds rows/joins even with long deleted or self-authored runs.
        // Tags are bounded before transfer; oversized/corrupt evidence stays
        // unknown, never falsely top-level/unmentioned/read.
        let rows = sqlx::query(
            "WITH roster AS MATERIALIZED (
                SELECT c.id,c.name,c.channel_type::text AS channel_type,
                    c.archived_at IS NOT NULL AS archived, cm.hidden_at IS NOT NULL AS hidden
                FROM channel_members cm JOIN channels c
                    ON c.community_id=cm.community_id AND c.id=cm.channel_id
                WHERE cm.community_id=$1 AND cm.pubkey=$2 AND cm.removed_at IS NULL
                    AND c.deleted_at IS NULL AND ($3::uuid IS NULL OR c.id>$3)
                ORDER BY c.id LIMIT $4
             )
             SELECT r.*, latest.latest_message_id,
                (latest.latest_message_id IS NOT NULL OR latest.candidates <= $5-1) AS latest_message_complete,
                COALESCE(e.evidence,'[]'::jsonb) AS evidence FROM roster r
             LEFT JOIN LATERAL (
                WITH candidates AS MATERIALIZED (
                    SELECT id,created_at,kind,deleted_at FROM events
                    WHERE community_id=$1 AND channel_id=r.id
                    ORDER BY created_at DESC,id LIMIT $5
                )
                SELECT count(*) AS candidates,
                    (array_agg(encode(id,'hex') ORDER BY created_at DESC,id)
                        FILTER (WHERE kind=ANY($6) AND deleted_at IS NULL))[1] AS latest_message_id
                FROM candidates
             ) latest ON true
             LEFT JOIN LATERAL (
                SELECT jsonb_agg(jsonb_build_object(
                    'id',encode(e.id,'hex'), 'created_at',extract(epoch FROM e.created_at)::bigint,
                    'received_ms',floor(extract(epoch FROM e.received_at)*1000)::bigint,
                    'own',e.pubkey=$2, 'deleted',e.deleted_at IS NOT NULL,
                    'kind',e.kind,
                    'tags',CASE WHEN octet_length(e.tags::text)<=8192
                        AND jsonb_typeof(e.tags)='array' THEN
                        (SELECT COALESCE(jsonb_agg(tag ORDER BY ord),'[]'::jsonb)
                         FROM jsonb_array_elements(e.tags) WITH ORDINALITY t(tag,ord)
                         WHERE tag->>0 IN ('p','broadcast','e')) ELSE NULL END,
                    'root',encode(tm.root_event_id,'hex'),
                    'channel_prefix',cf.through_timestamp,'thread_prefix',tf.through_timestamp
                ) ORDER BY e.created_at DESC,e.id) AS evidence
                FROM (SELECT id,pubkey,created_at,received_at,deleted_at,kind,tags
                    FROM events WHERE community_id=$1 AND channel_id=r.id
                    ORDER BY created_at DESC,id LIMIT $5) e
                LEFT JOIN thread_metadata tm ON tm.community_id=$1 AND tm.channel_id=r.id
                    AND tm.event_created_at=e.created_at AND tm.event_id=e.id
                LEFT JOIN personal_read_frontiers cf ON cf.community_id=$1 AND cf.actor=$2
                    AND cf.channel_id=r.id AND cf.root_id=''::bytea
                LEFT JOIN personal_read_frontiers tf ON tf.community_id=$1 AND tf.actor=$2
                    AND tf.channel_id=r.id AND tf.root_id=tm.root_event_id
             ) e ON true ORDER BY r.id",
        ).bind(community.as_uuid()).bind(actor_bytes.as_slice()).bind(after)
            .bind((limit+1) as i64).bind((MAX_CHANNEL_SCAN+1) as i64)
            .bind(ELIGIBLE_KINDS.as_slice())
            .fetch_all(&mut *tx).await?;
        let has_more = rows.len() > limit;
        let mut channels = Vec::new();
        for row in rows.into_iter().take(limit) {
            let evidence: Value = row.try_get("evidence")?;
            let evidence = evidence
                .as_array()
                .ok_or_else(|| DbError::InvalidData("invalid sidebar evidence".into()))?;
            let complete = evidence.len() <= MAX_CHANNEL_SCAN;
            let channel_type: String = row.try_get("channel_type")?;
            let mut unread = 0;
            let mut attention = 0;
            let mut unread_complete = complete;
            let mut attention_complete = complete;
            for e in evidence.iter().take(MAX_CHANNEL_SCAN) {
                let kind = e["kind"].as_i64().unwrap_or(-1) as i32;
                if !ELIGIBLE_KINDS.contains(&kind)
                    || e["deleted"] == true
                    || e["received_ms"]
                        .as_i64()
                        .is_none_or(|t| t < account.cutoff_ms)
                {
                    continue;
                }
                if e["own"] == true {
                    continue;
                }
                let Some(created) = e["created_at"].as_i64() else {
                    unread_complete = false;
                    attention_complete = false;
                    continue;
                };
                let Some(tags) = e["tags"].as_array() else {
                    unread_complete = false;
                    attention_complete = false;
                    continue;
                };
                // Missing metadata for a marked reply could hide a thread
                // frontier. Until resolved, it cannot establish ordinary unread.
                let parsed: Option<Vec<Vec<String>>> = tags
                    .iter()
                    .map(|tag| {
                        tag.as_array()?
                            .iter()
                            .map(|p| p.as_str().map(str::to_owned))
                            .collect()
                    })
                    .collect();
                let Some(parsed) = parsed else {
                    unread_complete = false;
                    attention_complete = false;
                    continue;
                };
                let markers = buzz_core::nip10::parse_thread_markers_from_parts(
                    parsed.iter().map(Vec::as_slice),
                );
                if markers.resolve().is_some() && e["root"].is_null() {
                    unread_complete = false;
                    attention_complete = false;
                    continue;
                }
                // Roots are timeline messages; descendants belong exclusively
                // to their canonical thread. Never inherit the channel prefix.
                let is_reply = e["root"]
                    .as_str()
                    .is_some_and(|root| Some(root) != e["id"].as_str());
                let prefix = if is_reply {
                    &e["thread_prefix"]
                } else {
                    &e["channel_prefix"]
                };
                if prefix.as_i64().is_some_and(|p| created <= p) {
                    continue;
                }
                unread += 1;
                let directed = channel_type == "dm"
                    || parsed.iter().any(|tag| {
                        tag.len() >= 2
                            && ((tag[0] == "p" && tag[1].eq_ignore_ascii_case(&actor.to_hex()))
                                || (tag[0] == "broadcast" && tag[1] == "1"))
                    });
                if directed {
                    attention += 1;
                } else if is_reply {
                    // Participation is not inferable from the bounded window:
                    // the author's own earlier reply may sit outside it.
                    attention_complete = false;
                }
            }
            let count = |value, complete| {
                if complete {
                    ReadCount::Exact { value }
                } else {
                    ReadCount::AtLeast { value }
                }
            };
            channels.push(ChannelReadSummary {
                channel_id: row.try_get("id")?,
                name: row.try_get("name")?,
                channel_type,
                archived: row.try_get("archived")?,
                hidden: row.try_get("hidden")?,
                unread: count(unread, unread_complete),
                attention: count(attention, attention_complete),
                latest_message_id: row.try_get("latest_message_id")?,
                latest_message_complete: row.try_get("latest_message_complete")?,
            });
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

/// Read-time horizon only: frontier state is not discarded on expiry.
pub(super) async fn read_account(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
    retention_seconds: u32,
) -> Result<ReadAccount> {
    let row = sqlx::query(
        "SELECT date_trunc('milliseconds',transaction_timestamp()-make_interval(secs=>$3::double precision)) AS cutoff,
            a.imported_at FROM (SELECT 1) singleton LEFT JOIN personal_read_accounts a
            ON a.community_id=$1 AND a.actor=$2"
    ).bind(community.as_uuid()).bind(actor).bind(f64::from(retention_seconds)).fetch_one(conn).await?;
    let cutoff: DateTime<Utc> = row.try_get("cutoff")?;
    let imported: Option<DateTime<Utc>> = row.try_get("imported_at")?;
    Ok(ReadAccount {
        retention_seconds,
        cutoff_ms: cutoff.timestamp_millis(),
        imported_at_ms: imported.map(|t| t.timestamp_millis()),
    })
}
