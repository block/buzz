//! Bounded compatibility lookup for marked replies without thread metadata.
//! Uses the existing tags GIN index, never a new permanent index. Exceeding
//! either budget aborts the transaction rather than treating an incomplete
//! lookup as evidence that an event has no replies.
//! Exact probes cover lowercase and uppercase references. Mixed-case references
//! without thread metadata are not guaranteed; normal metadata uses binary IDs.
use super::*;

const MAX_CANDIDATES: i64 = 10_000;

pub(super) const CANDIDATES_SQL: &str = "SELECT e.id,e.created_at,e.tags
         FROM jsonb_array_elements($2::jsonb) AS probe(tags)
         CROSS JOIN LATERAL (
           SELECT e.id,e.created_at,e.tags FROM events e
           WHERE e.community_id=$1 AND e.tags @> probe.tags
           AND NOT EXISTS(SELECT 1 FROM thread_metadata tm
             WHERE tm.community_id=e.community_id AND tm.event_id=e.id
               AND tm.event_created_at=e.created_at)
           LIMIT $3
         ) e LIMIT $3";

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
    let ids: HashSet<_> = selected.iter().map(|(id, _)| hex::encode(id)).collect();
    let mut probes = Vec::with_capacity(ids.len() * 2);
    for id in &ids {
        probes.push(json!([["e", id]]));
        let upper = id.to_ascii_uppercase();
        if upper != *id {
            probes.push(json!([["e", upper]]));
        }
    }
    let previous: String = sqlx::query_scalar("SELECT current_setting('statement_timeout')")
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='1s'")
        .execute(&mut *conn)
        .await?;
    // Each lateral probe searches for a specific ID through the existing GIN
    // index. Its LIMIT keeps the planner from flattening this into a broad join.
    // One statement bounds all probes together, not one second per selected ID.
    // The outer limit counts matches (a reply can match more than one probe).
    // The shared parser below rejects plain links and checks last-marker wins.
    let candidates: Vec<(Vec<u8>, DateTime<Utc>, Value)> = sqlx::query_as(CANDIDATES_SQL)
        .bind(community)
        .bind(json!(probes))
        .bind(MAX_CANDIDATES + 1)
        .fetch_all(&mut *conn)
        .await?;
    sqlx::query("SELECT set_config('statement_timeout',$1,true)")
        .bind(previous)
        .execute(&mut *conn)
        .await?;
    if candidates.len() > MAX_CANDIDATES as usize {
        return Err(invalid("too many reference matches without thread metadata (10,000); repair thread metadata before content deletion"));
    }
    let removed: HashSet<_> = selected.iter().cloned().collect();
    let mut needed = HashSet::new();
    for (id, created, tags) in candidates {
        if !removed.contains(&(id, created)) {
            needed.extend(ancestors(&tags)?.into_iter().filter(|id| ids.contains(id)));
        }
    }
    Ok(needed)
}
