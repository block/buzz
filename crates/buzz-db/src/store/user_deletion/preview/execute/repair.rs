use super::*;

pub(super) async fn batch(conn: &mut PgConnection, community: Uuid, batch: &Value) -> Result<()> {
    let repairs = batch["thread_repairs"]
        .as_array()
        .ok_or_else(|| invalid("missing internal repairs"))?;
    let mut seen = HashSet::new();
    for repair in repairs {
        let mut roots = repair["stored_threads"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // A surviving reply can name a root whose own metadata never existed.
        roots.push(json!({"channel_id":repair["channel_id"], "root_event_id":repair["event_id"]}));
        for root in roots {
            if !seen.insert(root.to_string()) {
                continue;
            }
            let channel: Option<Uuid> = serde_json::from_value(root["channel_id"].clone())?;
            let id = root["root_event_id"]
                .as_str()
                .ok_or_else(|| invalid("missing repair root"))?;
            let id = hex::decode(id).map_err(|_| invalid("invalid repair root"))?;
            repair_thread(conn, community, channel, &id).await?;
        }
    }
    Ok(())
}

async fn repair_thread(
    conn: &mut PgConnection,
    community: Uuid,
    channel: Option<Uuid>,
    root: &[u8],
) -> Result<()> {
    // Preserve only the ancestry needed by retained events. The missing event
    // rows carry no content, author, tags or signature; metadata keeps replies
    // reachable without manufacturing a signed replacement event.
    sqlx::query(
        "WITH RECURSIVE needed AS (
          SELECT tm.* FROM thread_metadata tm
          WHERE tm.community_id=$1 AND tm.channel_id IS NOT DISTINCT FROM $2::uuid AND (tm.event_id=$3 OR tm.root_event_id=$3)
            AND EXISTS (SELECT 1 FROM events e WHERE e.community_id=tm.community_id
              AND e.id=tm.event_id AND e.created_at=tm.event_created_at)
          UNION
          SELECT p.* FROM thread_metadata p JOIN needed n ON p.community_id=n.community_id
            AND p.channel_id IS NOT DISTINCT FROM n.channel_id AND (p.event_id=n.parent_event_id OR p.event_id=n.root_event_id)
        ) DELETE FROM thread_metadata tm WHERE tm.community_id=$1 AND tm.channel_id IS NOT DISTINCT FROM $2::uuid
          AND (tm.event_id=$3 OR tm.root_event_id=$3)
          AND NOT EXISTS (SELECT 1 FROM needed n WHERE n.event_id=tm.event_id AND n.event_created_at=tm.event_created_at)",
    ).bind(community).bind(channel).bind(root).execute(&mut *conn).await?;
    // Recompute, never subtract old counts. Already-soft-deleted messages do
    // not count; received_at matches the server-arrival activity clock.
    // Like live writes, last_reply_at tracks direct replies, not descendants.
    sqlx::query(
        "WITH visible AS MATERIALIZED (
          SELECT tm.parent_event_id,tm.root_event_id,e.received_at FROM thread_metadata tm
          JOIN events e ON e.community_id=tm.community_id AND e.id=tm.event_id AND e.created_at=tm.event_created_at
          WHERE tm.community_id=$1 AND tm.channel_id IS NOT DISTINCT FROM $2::uuid AND tm.root_event_id=$3 AND e.deleted_at IS NULL
        ), direct AS (
          SELECT parent_event_id,count(*)::int AS n,max(received_at) AS last_at FROM visible GROUP BY parent_event_id
        ), descendants AS (
          SELECT root_event_id,count(*)::int AS n FROM visible GROUP BY root_event_id
        ), counts AS (
          SELECT tm.event_id,tm.event_created_at,COALESCE(d.n,0) AS replies,COALESCE(r.n,0) AS descendants,
            d.last_at
          FROM thread_metadata tm LEFT JOIN direct d ON d.parent_event_id=tm.event_id
          LEFT JOIN descendants r ON r.root_event_id=tm.event_id
          WHERE tm.community_id=$1 AND tm.channel_id IS NOT DISTINCT FROM $2::uuid AND (tm.event_id=$3 OR tm.root_event_id=$3)
        ) UPDATE thread_metadata tm SET reply_count=c.replies,descendant_count=c.descendants,last_reply_at=c.last_at
          FROM counts c WHERE tm.community_id=$1 AND tm.event_id=c.event_id AND tm.event_created_at=c.event_created_at",
    ).bind(community).bind(channel).bind(root).execute(&mut *conn).await?;
    Ok(())
}
