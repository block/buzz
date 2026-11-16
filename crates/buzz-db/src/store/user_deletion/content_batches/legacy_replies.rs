//! Bounded compatibility lookup for marked replies without thread metadata.
//! Uses the existing tags GIN index, never a new permanent index. Exceeding
//! either budget aborts the transaction rather than treating an incomplete
//! lookup as evidence that an event has no replies.
use super::*;

const MAX_CANDIDATES: i64 = 10_000;

pub(super) const CANDIDATES_SQL: &str = "SELECT e.id,e.created_at,e.tags FROM events e
         WHERE e.community_id=$1 AND e.tags @> '[[\"e\",\"reply\"]]'::jsonb
           AND NOT EXISTS(SELECT 1 FROM thread_metadata tm
             WHERE tm.community_id=e.community_id AND tm.event_id=e.id
               AND tm.event_created_at=e.created_at)
         LIMIT $2";

pub(super) fn ancestors(tags: &Value) -> Result<Vec<String>> {
    let parts: Vec<Vec<String>> = serde_json::from_value(tags.clone())?;
    Ok(
        buzz_core::nip10::parse_thread_markers_from_parts(parts.iter().map(Vec::as_slice))
            .resolve()
            .map(|(root, parent)| vec![root.to_ascii_lowercase(), parent.to_ascii_lowercase()])
            .unwrap_or_default(),
    )
}

pub(super) async fn needed(
    conn: &mut PgConnection,
    community: Uuid,
    selected: &[(Vec<u8>, DateTime<Utc>)],
) -> Result<HashSet<String>> {
    if selected.is_empty() {
        return Ok(HashSet::new());
    }
    let previous: String = sqlx::query_scalar("SELECT current_setting('statement_timeout')")
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='1s'")
        .execute(&mut *conn)
        .await?;
    // Containment is an indexable candidate filter, not the reply parser.
    // The shared parser below checks positions, valid ids and last-marker wins.
    // Normal replies are found through thread_metadata's parent/root indexes.
    let candidates: Vec<(Vec<u8>, DateTime<Utc>, Value)> = sqlx::query_as(CANDIDATES_SQL)
        .bind(community)
        .bind(MAX_CANDIDATES + 1)
        .fetch_all(&mut *conn)
        .await?;
    sqlx::query("SELECT set_config('statement_timeout',$1,true)")
        .bind(previous)
        .execute(&mut *conn)
        .await?;
    if candidates.len() > MAX_CANDIDATES as usize {
        return Err(invalid("too many replies without thread metadata (10,000); repair thread metadata before content deletion"));
    }
    let removed: HashSet<_> = selected.iter().cloned().collect();
    let ids: HashSet<_> = selected.iter().map(|(id, _)| hex::encode(id)).collect();
    let mut needed = HashSet::new();
    for (id, created, tags) in candidates {
        if !removed.contains(&(id, created)) {
            needed.extend(ancestors(&tags)?.into_iter().filter(|id| ids.contains(id)));
        }
    }
    Ok(needed)
}
