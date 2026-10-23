//! Permission-scoped NIP-AR queries. Predicates precede pagination and counts.
use crate::{Db, Result};
use buzz_core::{
    artifact::{ArtifactQuery, ArtifactView},
    CommunityId, StoredEvent,
};
use sqlx::QueryBuilder;

impl Db {
    /// Execute a bounded query against current artifacts or revision history.
    /// Permission is resolved in the query snapshot.
    pub async fn query_artifacts(
        &self,
        community: CommunityId,
        reader: &[u8],
        query: &ArtifactQuery,
        count: bool,
    ) -> Result<(Vec<StoredEvent>, i64)> {
        // Read-only: a plain writer-pool snapshot transaction, labeled as the
        // history read it is, so it neither takes community admission nor
        // counts as an event write.
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        let mut qb = QueryBuilder::<sqlx::Postgres>::new(if count {
            "SELECT count(*) AS count FROM events e "
        } else {
            "SELECT e.id,e.pubkey,e.created_at,e.kind,e.tags,e.content,e.sig,e.received_at,e.channel_id FROM events e "
        });
        predicates(&mut qb, community, reader, query);
        if count {
            let n: i64 = qb.build_query_scalar().fetch_one(&mut *tx).await?;
            tx.commit().await?;
            return Ok((vec![], n));
        }
        qb.push(" ORDER BY e.received_at DESC,e.id ASC LIMIT ")
            .push_bind(query.limit)
            .push(" OFFSET ")
            .push_bind(query.offset);
        let rows = qb.build().fetch_all(&mut *tx).await?;
        let mut events = vec![];
        for row in rows {
            if let Some(event) = crate::event::row_to_stored_event(row)? {
                events.push(event);
            }
        }
        tx.commit().await?;
        Ok((events, 0))
    }
}

/// Own the visibility and tag rules for artifact queries and current-ID reads.
pub(super) fn predicates(
    qb: &mut QueryBuilder<sqlx::Postgres>,
    community: CommunityId,
    reader: &[u8],
    query: &ArtifactQuery,
) {
    if query.view == ArtifactView::Current {
        qb.push("JOIN artifact_heads a ON a.community_id=e.community_id AND a.event_id=e.id AND NOT a.deleted ");
    }
    // A system channel is read only by a query whose `#h` names it
    // (NIP-CL); malformed `#h` values name no channel.
    let named_channels: Vec<uuid::Uuid> = query
        .tags
        .iter()
        .filter(|(name, _)| name == "h")
        .flat_map(|(_, values)| values.iter().filter_map(|value| value.parse().ok()))
        .collect();
    // The explicit kind lets the community/kind index exclude chat before
    // the join, even for an unfiltered current-state page.
    qb.push("WHERE e.community_id=").push_bind(community.as_uuid()).push(" AND e.kind=45010 AND e.deleted_at IS NULL AND EXISTS (SELECT 1 FROM channels c WHERE c.community_id=e.community_id AND c.id=e.channel_id AND c.deleted_at IS NULL AND (")
        .push(crate::store::channel::legacy_channel_type_sql("c.channel_type"))
        .push(" OR c.id=ANY(")
        .push_bind(named_channels)
        .push(")) AND (c.visibility='open' OR EXISTS (SELECT 1 FROM channel_members m WHERE m.community_id=c.community_id AND m.channel_id=c.id AND m.pubkey=")
        .push_bind(reader.to_vec()).push(" AND m.removed_at IS NULL)))");
    tag_predicates(qb, query);
    if query.view == ArtifactView::Current {
        for (_, values) in query.tags.iter().filter(|(name, _)| name == "d") {
            qb.push(" AND a.artifact_id=ANY(")
                .push_bind(
                    values
                        .iter()
                        .filter_map(|id| id.parse::<uuid::Uuid>().ok())
                        .collect::<Vec<_>>(),
                )
                .push(")");
        }
    }
}

/// Apply exact artifact tag predicates after access is resolved.
pub(super) fn tag_predicates(qb: &mut QueryBuilder<sqlx::Postgres>, query: &ArtifactQuery) {
    for (name, values) in &query.tags {
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
