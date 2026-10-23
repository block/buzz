//! Indexed artifact changes. Each page reads current authority on the writer.
use crate::{Db, DbError, Result};
use buzz_core::{artifact::ArtifactQuery, CommunityId, StoredEvent};
use chrono::{DateTime, Utc};
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row};
use uuid::Uuid;

/// Maximum changes or requested identities in one page.
pub const PAGE_ROWS: usize = 100;

/// A stable position within one timestamp window.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Position {
    /// Database timestamp of the change.
    pub at: DateTime<Utc>,
    /// Change source, which separates equal keys from different tables.
    pub source: i32,
    /// Stable identity within that source.
    pub id: String,
}

/// A changed identity that requires a current, permission-scoped read.
pub struct Candidate {
    /// Position used after this candidate is processed.
    pub position: Position,
    /// Artifact to revalidate, absent for a removed membership.
    pub artifact_id: Option<Uuid>,
    /// Removed channel belonging to the authenticated reader's membership.
    pub removed_channel: Option<Uuid>,
}

impl Db {
    /// Read server time and the community's access-policy recovery boundary.
    pub async fn artifact_sync_boundary(
        &self,
        community: CommunityId,
    ) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
        sqlx::query_as("SELECT clock_timestamp(),artifact_policy_modified_at FROM communities WHERE id=$1 AND deletion_state='active' AND deleted_at IS NULL")
            .bind(community.as_uuid()).fetch_optional(&self.pool).await?
            .ok_or_else(|| DbError::InvalidData("artifact sync community unavailable".into()))
    }

    /// Find at most one page plus one look-ahead row, without a channel roster.
    pub async fn artifact_sync_candidates(
        &self,
        community: CommunityId,
        reader: &[u8],
        since: Option<DateTime<Utc>>,
        until: DateTime<Utc>,
        after: Option<&Position>,
        query: &ArtifactQuery,
    ) -> Result<Vec<Candidate>> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        let types = query
            .tags
            .iter()
            .filter(|(n, _)| n == "type")
            .flat_map(|(_, v)| v.iter().cloned())
            .collect::<Vec<_>>();
        let named = query
            .tags
            .iter()
            .filter(|(n, _)| n == "h")
            .flat_map(|(_, v)| v.iter().filter_map(|id| id.parse::<Uuid>().ok()))
            .collect::<Vec<_>>();
        let rows = sqlx::query(CANDIDATES)
            .bind(community.as_uuid())
            .bind(reader)
            .bind(since)
            .bind(until)
            .bind(after.map(|p| p.at))
            .bind(after.map(|p| p.source))
            .bind(after.map(|p| p.id.as_str()))
            .bind(types)
            .bind(named)
            .bind((PAGE_ROWS + 1) as i64)
            .bind(buzz_core::channel::LEGACY_CHANNEL_TYPES.to_vec())
            .fetch_all(&mut *tx)
            .await?;
        let candidates = rows
            .into_iter()
            .map(|r| {
                Ok(Candidate {
                    position: Position {
                        at: r.try_get("at")?,
                        source: r.try_get("source")?,
                        id: r.try_get("id")?,
                    },
                    artifact_id: r.try_get("artifact_id")?,
                    removed_channel: r.try_get("removed_channel")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(candidates)
    }

    /// Return current matching events for bounded known identities.
    /// Omitted identities have the same unavailable meaning for every cause.
    pub async fn artifact_sync_current(
        &self,
        community: CommunityId,
        reader: &[u8],
        query: &ArtifactQuery,
        ids: &[Uuid],
    ) -> Result<Vec<StoredEvent>> {
        if ids.len() > PAGE_ROWS {
            return Err(DbError::ArtifactSyncLimit);
        }
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        let mut qb = QueryBuilder::new("SELECT e.id,e.pubkey,e.created_at,e.kind,e.tags,e.content,e.sig,e.received_at,e.channel_id FROM events e ");
        super::artifact_query::predicates(&mut qb, community, reader, query);
        qb.push(" AND a.artifact_id=ANY(").push_bind(ids).push(")");
        qb.push(" LIMIT ").push_bind(PAGE_ROWS as i64);
        let mut events = Vec::new();
        let mut bytes = 0;
        {
            let mut rows = qb.build().fetch(&mut *tx);
            while let Some(row) = rows.try_next().await? {
                if let Some(event) = crate::event::row_to_stored_event(row)? {
                    bytes += serde_json::to_vec(&event.event)
                        .map_err(|e| DbError::InvalidData(e.to_string()))?
                        .len()
                        + 512;
                    if bytes > 8_388_608 {
                        return Err(DbError::ArtifactSyncLimit);
                    }
                    events.push(event);
                }
            }
        }
        let homes = events
            .iter()
            .filter_map(|e| e.channel_id)
            .collect::<Vec<_>>();
        let ttl:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE community_id=$1 AND id=ANY($2) AND ttl_seconds IS NOT NULL)")
            .bind(community.as_uuid()).bind(homes).fetch_one(&mut *tx).await?;
        if ttl {
            return Err(DbError::ArtifactSyncUnsupportedScope);
        }
        tx.commit().await?;
        Ok(events)
    }
}

// Keep candidate access rules aligned with artifact_query::predicates, which owns
// current-state visibility. This query also reads retained move and membership rows.
const CANDIDATES: &str = r#"
WITH readable AS NOT MATERIALIZED (
    SELECT c.id FROM channels c
    WHERE c.community_id=$1 AND c.deleted_at IS NULL
      AND (c.channel_type::text=ANY($11) OR c.id=ANY($9))
      AND (c.visibility='open' OR EXISTS (
        SELECT 1 FROM channel_members m WHERE m.community_id=c.community_id
        AND m.channel_id=c.id AND m.pubkey=$2 AND m.removed_at IS NULL))
), changes AS (
    SELECT a.modified_at AS at,1 AS source,a.artifact_id::text AS id,
           a.artifact_id,NULL::uuid AS removed_channel
    FROM artifact_heads a
    WHERE a.community_id=$1 AND a.modified_at >= COALESCE($3::timestamptz,'epoch'::timestamptz)
      AND a.modified_at <= $4 AND a.channel_id IN (SELECT id FROM readable)
      AND (cardinality($8::text[])=0 OR a.artifact_type=ANY($8))
    UNION ALL
    SELECT m.sync_modified_at,0,m.channel_id::text,NULL::uuid,m.channel_id
    FROM channel_members m WHERE m.community_id=$1 AND m.pubkey=$2
      AND $3::timestamptz IS NOT NULL AND m.sync_modified_at >= $3
      AND m.sync_modified_at <= $4 AND m.removed_at IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM readable r WHERE r.id=m.channel_id)
    UNION ALL
    SELECT m.sync_modified_at,2,a.artifact_id::text,a.artifact_id,NULL::uuid
    FROM channel_members m JOIN artifact_heads a
      ON a.community_id=m.community_id AND a.channel_id=m.channel_id
    WHERE m.community_id=$1 AND m.pubkey=$2 AND m.removed_at IS NULL
      AND $3::timestamptz IS NOT NULL AND m.sync_modified_at >= $3
      AND m.sync_modified_at <= $4 AND a.channel_id IN (SELECT id FROM readable)
      AND (cardinality($8::text[])=0 OR a.artifact_type=ANY($8))
    UNION ALL
    SELECT DISTINCT r.modified_at,3,r.artifact_id::text,r.artifact_id,NULL::uuid
    FROM artifact_revisions r WHERE r.community_id=$1
      AND r.source_channel_id IS NOT NULL AND r.source_channel_id IN (SELECT id FROM readable)
      AND $3::timestamptz IS NOT NULL AND r.modified_at >= $3 AND r.modified_at <= $4
)
SELECT * FROM changes WHERE $5::timestamptz IS NULL OR (at,source,id)>($5,$6,$7)
ORDER BY at,source,id LIMIT $10
"#;
