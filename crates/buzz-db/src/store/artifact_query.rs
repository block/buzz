//! Permission-scoped NIP-AR queries. Predicates precede pagination and counts.
use crate::{Db, Result};
use buzz_core::{
    artifact::{ArtifactQuery, ArtifactView},
    CommunityId, StoredEvent,
};
use sqlx::{pool::PoolConnection, Postgres, QueryBuilder, Transaction};
use uuid::Uuid;

const EVENT_COLUMNS: &str =
    "SELECT e.id,e.pubkey,e.created_at,e.kind,e.tags,e.content,e.sig,e.received_at,e.channel_id FROM events e ";

/// Append the reader's permission scope and the exact tag predicates. A
/// system channel is readable only when `named_channels` names it (NIP-CL).
fn push_scope(
    qb: &mut QueryBuilder<Postgres>,
    community: CommunityId,
    reader: &[u8],
    tags: &[(String, Vec<String>)],
    named_channels: Vec<Uuid>,
) {
    // The explicit kind lets the community/kind index exclude chat before
    // the join, even for an unfiltered current-state page.
    qb.push("WHERE e.community_id=").push_bind(community.as_uuid()).push(" AND e.kind=45010 AND e.deleted_at IS NULL AND EXISTS (SELECT 1 FROM channels c WHERE c.community_id=e.community_id AND c.id=e.channel_id AND c.deleted_at IS NULL AND (")
        .push(crate::store::channel::legacy_channel_type_sql("c.channel_type"))
        .push(" OR c.id=ANY(")
        .push_bind(named_channels)
        .push(")) AND (c.visibility='open' OR EXISTS (SELECT 1 FROM channel_members m WHERE m.community_id=c.community_id AND m.channel_id=c.id AND m.pubkey=")
        .push_bind(reader.to_vec()).push(" AND m.removed_at IS NULL)))");
    for (name, values) in tags {
        // GIN-usable necessary prefilter; retain the positional recheck
        // because JSON array containment alone also matches swapped values.
        qb.push(" AND (");
        for (index, value) in values.iter().enumerate() {
            if index != 0 {
                qb.push(" OR ");
            }
            qb.push("e.tags @> ")
                .push_bind(serde_json::json!([[name, value]]));
        }
        qb.push(")");
        qb.push(" AND EXISTS (SELECT 1 FROM jsonb_array_elements(e.tags) tag WHERE tag->>0=")
            .push_bind(name)
            .push(" AND tag->>1=ANY(")
            .push_bind(values)
            .push("))");
    }
}

/// Channel UUIDs named by `#h`; malformed values name no channel.
fn named_channels(tags: &[(String, Vec<String>)]) -> Vec<Uuid> {
    tags.iter()
        .filter(|(name, _)| name == "h")
        .flat_map(|(_, values)| values.iter().filter_map(|value| value.parse().ok()))
        .collect()
}

fn rows_to_events(rows: Vec<sqlx::postgres::PgRow>) -> Result<Vec<StoredEvent>> {
    let mut events = vec![];
    for row in rows {
        if let Some(event) = crate::event::row_to_stored_event(row)? {
            events.push(event);
        }
    }
    Ok(events)
}

/// Read-only: a plain writer-pool connection, labeled as the history read it
/// is, so it neither takes community admission nor counts as an event write.
/// The writer is always at least as fresh as every accepted change.
async fn read_connection(db: &Db) -> Result<PoolConnection<Postgres>> {
    crate::observability::acquire_writer(
        &db.pool,
        crate::observability::WriterOperation::SubscriptionHistory,
    )
    .await
    .map_err(Into::into)
}

/// Why a sync snapshot did not open.
#[derive(Debug, PartialEq, Eq)]
pub enum SnapshotDenied {
    /// The reader cannot read a named channel, or it does not exist.
    Restricted,
}

/// One read-only `REPEATABLE READ` snapshot of a sync's current heads, read
/// in keyset pages ordered by artifact identity. Every page sees the same
/// database snapshot, so the pages together are one consistent cut.
pub struct ArtifactSnapshot {
    tx: Transaction<'static, Postgres>,
    community: CommunityId,
    reader: Vec<u8>,
    channels: Vec<Uuid>,
    after: Option<Uuid>,
    done: bool,
}

impl ArtifactSnapshot {
    /// Read the next page of at most `page` heads that match `tags`, ordered
    /// by `d`. Returns an empty page once the set is exhausted.
    pub async fn next_page(
        &mut self,
        tags: &[(String, Vec<String>)],
        page: i64,
    ) -> Result<Vec<StoredEvent>> {
        if self.done {
            return Ok(vec![]);
        }
        let mut qb = QueryBuilder::<Postgres>::new(
            "SELECT e.id,e.pubkey,e.created_at,e.kind,e.tags,e.content,e.sig,e.received_at,e.channel_id,a.artifact_id FROM events e ",
        );
        qb.push("JOIN artifact_heads a ON a.community_id=e.community_id AND a.event_id=e.id AND NOT a.deleted ");
        push_scope(
            &mut qb,
            self.community,
            &self.reader,
            tags,
            self.channels.clone(),
        );
        if let Some(after) = self.after {
            qb.push(" AND a.artifact_id>").push_bind(after);
        }
        qb.push(" ORDER BY a.artifact_id LIMIT ").push_bind(page);
        let rows = qb.build().fetch_all(&mut *self.tx).await?;
        self.done = (rows.len() as i64) < page;
        if let Some(last) = rows.last() {
            self.after = Some(sqlx::Row::try_get(last, "artifact_id")?);
        }
        rows_to_events(rows)
    }

    /// End the snapshot. Dropping it rolls the read-only transaction back.
    pub async fn finish(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }
}

impl Db {
    /// Execute a bounded query against current artifacts, current heads
    /// including deletions, or revision history. Permission is resolved in
    /// the query snapshot.
    pub async fn query_artifacts(
        &self,
        community: CommunityId,
        reader: &[u8],
        query: &ArtifactQuery,
        count: bool,
    ) -> Result<(Vec<StoredEvent>, i64)> {
        let connection = read_connection(self).await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        let mut qb = QueryBuilder::<Postgres>::new(if count {
            "SELECT count(*) AS count FROM events e "
        } else {
            EVENT_COLUMNS
        });
        match query.view {
            ArtifactView::Current => {
                qb.push("JOIN artifact_heads a ON a.community_id=e.community_id AND a.event_id=e.id AND NOT a.deleted ");
            }
            ArtifactView::Head => {
                qb.push(
                    "JOIN artifact_heads a ON a.community_id=e.community_id AND a.event_id=e.id ",
                );
            }
            ArtifactView::History => {}
        }
        push_scope(
            &mut qb,
            community,
            reader,
            &query.tags,
            named_channels(&query.tags),
        );
        if count {
            let n: i64 = qb.build_query_scalar().fetch_one(&mut *tx).await?;
            tx.commit().await?;
            return Ok((vec![], n));
        }
        qb.push(" ORDER BY e.received_at DESC,e.id ASC LIMIT ")
            .push_bind(query.limit)
            .push(" OFFSET ")
            .push_bind(query.offset);
        let events = rows_to_events(qb.build().fetch_all(&mut *tx).await?)?;
        tx.commit().await?;
        Ok((events, 0))
    }

    /// The current head of one artifact, including a deletion, when the
    /// reader can read the head's channel, with whether it is a deletion.
    /// `named_channels` are the system channels the caller's request names.
    pub async fn artifact_head(
        &self,
        community: CommunityId,
        reader: &[u8],
        artifact: Uuid,
        named_channels: &[Uuid],
    ) -> Result<Option<(StoredEvent, bool)>> {
        let mut connection = read_connection(self).await?;
        let mut qb = QueryBuilder::<Postgres>::new(
            "SELECT e.id,e.pubkey,e.created_at,e.kind,e.tags,e.content,e.sig,e.received_at,e.channel_id,a.deleted FROM events e ",
        );
        qb.push("JOIN artifact_heads a ON a.community_id=e.community_id AND a.event_id=e.id AND a.artifact_id=")
            .push_bind(artifact)
            .push(" ");
        push_scope(&mut qb, community, reader, &[], named_channels.to_vec());
        let Some(row) = qb.build().fetch_optional(&mut *connection).await? else {
            return Ok(None);
        };
        let deleted: bool = sqlx::Row::try_get(&row, "deleted")?;
        Ok(rows_to_events(vec![row])?
            .into_iter()
            .next()
            .map(|head| (head, deleted)))
    }

    /// Open a sync snapshot over `channels`, after confirming in the same
    /// snapshot that the reader can read every one of them.
    /// `statement_timeout` bounds each page read.
    pub async fn open_artifact_snapshot(
        &self,
        community: CommunityId,
        reader: &[u8],
        channels: &[Uuid],
    ) -> Result<std::result::Result<ArtifactSnapshot, SnapshotDenied>> {
        let connection = read_connection(self).await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        let readable: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM channels c WHERE c.community_id=$1 AND c.id=ANY($2) AND c.deleted_at IS NULL AND (c.visibility='open' OR EXISTS (SELECT 1 FROM channel_members m WHERE m.community_id=c.community_id AND m.channel_id=c.id AND m.pubkey=$3 AND m.removed_at IS NULL))",
        )
        .bind(community.as_uuid())
        .bind(channels)
        .bind(reader)
        .fetch_one(&mut *tx)
        .await?;
        if readable != channels.len() as i64 {
            return Ok(Err(SnapshotDenied::Restricted));
        }
        Ok(Ok(ArtifactSnapshot {
            tx,
            community,
            reader: reader.to_vec(),
            channels: channels.to_vec(),
            after: None,
            done: false,
        }))
    }
}
