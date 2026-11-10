use super::*;

// Use the existing due/recovery indexes. Filtering before LIMIT prevents
// invalid jobs from starving live work.
const DUE_COMMUNITIES_SQL: &str = r#"
    SELECT c.id
    FROM communities c
    WHERE ($1::uuid IS NULL OR c.id > $1)
      AND c.archived_at IS NULL
      AND c.deleted_at IS NULL
      AND c.deletion_state = 'active'
      AND EXISTS (
          SELECT 1
          FROM push_wake_outbox o
          JOIN push_leases l
            ON l.community_id = o.community_id
           AND l.author = o.author
           AND l.installation_id = o.installation_id
           AND l.generation = o.lease_generation
           AND l.endpoint_hash = o.endpoint_hash
          JOIN events e
            ON e.community_id = o.community_id
           AND e.id = o.event_id
           AND e.deleted_at IS NULL
          WHERE o.community_id = c.id
            AND o.expires_at > EXTRACT(EPOCH FROM now())::bigint
            AND o.next_attempt_at <= now()
            AND (o.state = 'pending' OR (o.state = 'sending' AND o.lease_until < now()))
            AND l.active
            AND l.endpoint_enabled
            AND l.expires_at > EXTRACT(EPOCH FROM now())::bigint
      )
    ORDER BY c.id
    LIMIT 64
"#;

/// Read up to 64 communities with eligible due wakes, ordered after `after`.
///
/// This read takes no community admission locks and releases its connection
/// before the caller claims work. A full sweep advances after each page's last
/// ID, so a busy community cannot hide quieter communities beyond the first page.
/// An empty page ends the sweep; newly due work behind the cursor is found on
/// the next sweep. Callers must still claim and revalidate before sending.
pub async fn due_wake_communities(
    pool: &PgPool,
    after: Option<CommunityId>,
) -> Result<Vec<CommunityId>> {
    let mut connection =
        acquire_operation_connection(pool, crate::observability::WriterOperation::Maintenance)
            .await?;
    let ids: Vec<Uuid> = sqlx::query_scalar(DUE_COMMUNITIES_SQL)
        .bind(after.map(|community| *community.as_uuid()))
        .fetch_all(&mut *connection)
        .await?;
    Ok(ids.into_iter().map(CommunityId::from_uuid).collect())
}
