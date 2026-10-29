use super::*;

pub(super) async fn remove(
    tx: &mut AdmittedTx,
    community: CommunityId,
    candidates: &[Candidate],
    result: &mut MessageBatch,
) -> Result<()> {
    let thread = candidates
        .first()
        .ok_or_else(|| safety("message batch unexpectedly empty"))?;
    if candidates.iter().any(|row| row.has_media) {
        return Err(safety("message has attachment or link references; shared-file cleanup is required before deletion"));
    }
    let mut ids: Vec<Vec<u8>> = candidates.iter().map(|row| row.id.clone()).collect();
    // Edits and votes use the first valid e tag, reactions use the last. Never treat any
    // arbitrary e tag as authority to delete another person's signed content.
    let mut frontier = ids.clone();
    for round in 0..16 {
        let dependents: Vec<(Vec<u8>, bool)> = sqlx::query_as(
            "SELECT e.id, (e.content ~* 'https?://|/media/' OR EXISTS (
           SELECT 1 FROM jsonb_array_elements(e.tags) t
           WHERE t->>0 IN ('imeta','link-preview','url','thumb','image','x','emoji'))) AS has_media
         FROM events e
         CROSS JOIN LATERAL (
           SELECT decode(t->>1, 'hex') AS id
           FROM jsonb_array_elements(e.tags) WITH ORDINALITY a(t,n)
           WHERE t->>0 = 'e' AND t->>1 ~ '^[0-9a-fA-F]{64}$'
           ORDER BY CASE WHEN e.kind IN (40003,45002) THEN n ELSE -n END LIMIT 1
         ) target
         WHERE e.community_id = $1 AND e.kind IN (7,40003,45002)
           AND target.id = ANY($2) AND NOT (e.id = ANY($5))
           AND e.channel_id IS NOT DISTINCT FROM $3::uuid
         ORDER BY e.created_at,e.id LIMIT $4",
        )
        .bind(community.as_uuid())
        .bind(&frontier)
        .bind(thread.channel_id)
        .bind((MAX_DEPENDENTS + 1) as i64)
        .bind(&ids)
        .fetch_all(tx.conn())
        .await?;
        if ids.len() + dependents.len() > MAX_DEPENDENTS {
            return Err(safety(
                "too many dependent events in one message batch; reduce the batch size",
            ));
        }
        if dependents.iter().any(|(_, media)| *media) {
            return Err(safety(
                "message edit or reaction has file references; shared-file cleanup is required",
            ));
        }
        if dependents.is_empty() {
            break;
        }
        if round == 15 {
            return Err(safety("dependent event chain exceeds 16 levels"));
        }
        frontier = dependents.into_iter().map(|(id, _)| id).collect();
        ids.extend(frontier.iter().cloned());
        ids.sort();
        ids.dedup();
    }

    // A worker may already have fetched the payload even though the user is
    // fenced. Do not delete its recovery record while it can still deliver.
    let delivering: bool = sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM push_match_queue WHERE community_id=$1 AND event_id=ANY($2) AND lease_until>clock_timestamp()
           UNION ALL SELECT 1 FROM push_wake_outbox WHERE community_id=$1 AND event_id=ANY($2) AND lease_until>clock_timestamp()
           UNION ALL SELECT 1 FROM operator_listener_outbox WHERE community_id=$1 AND event_id=ANY($2) AND lease_until>clock_timestamp())",
    ).bind(community.as_uuid()).bind(&ids).fetch_one(tx.conn()).await?;
    if delivering {
        return Err(safety(
            "message delivery workers must drain before deletion",
        ));
    }
    let runs: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, status::text FROM workflow_runs WHERE community_id=$1
         AND (trigger_event_id=ANY($2) OR lower(trigger_context->>'message_id')=ANY($3))
         ORDER BY id LIMIT $4 FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(&ids)
    .bind(ids.iter().map(hex::encode).collect::<Vec<_>>())
    .bind((MAX_DEPENDENTS + 1) as i64)
    .fetch_all(tx.conn())
    .await?;
    if runs.len() > MAX_DEPENDENTS {
        return Err(safety(
            "too many workflow runs in one message batch; reduce the batch size",
        ));
    }
    if runs
        .iter()
        .any(|(_, status)| !matches!(status.as_str(), "completed" | "failed" | "cancelled"))
    {
        return Err(safety(
            "workflow runs triggered by these messages must stop before deletion",
        ));
    }
    let run_ids: Vec<_> = runs.into_iter().map(|(id, _)| id).collect();
    result.workflow_runs_deleted = delete_runs(tx, community, &run_ids).await?;
    result.reactions_deleted = sqlx::query(
        "DELETE FROM reactions WHERE community_id=$1 AND (event_id=ANY($2) OR reaction_event_id=ANY($2))",
    ).bind(community.as_uuid()).bind(&ids).execute(tx.conn()).await?.rows_affected();
    result.mentions_deleted =
        sqlx::query("DELETE FROM event_mentions WHERE community_id=$1 AND event_id=ANY($2)")
            .bind(community.as_uuid())
            .bind(&ids)
            .execute(tx.conn())
            .await?
            .rows_affected();
    for query in [
        "DELETE FROM push_match_queue WHERE community_id=$1 AND event_id=ANY($2)",
        "DELETE FROM push_wake_outbox WHERE community_id=$1 AND event_id=ANY($2)",
        "DELETE FROM operator_listener_outbox WHERE community_id=$1 AND event_id=ANY($2)",
        "DELETE FROM delivery_log WHERE community_id=$1 AND event_id=ANY($2)",
    ] {
        result.deliveries_deleted += sqlx::query(query)
            .bind(community.as_uuid())
            .bind(&ids)
            .execute(tx.conn())
            .await?
            .rows_affected();
    }
    result.events_deleted = sqlx::query("DELETE FROM events WHERE community_id=$1 AND id=ANY($2)")
        .bind(community.as_uuid())
        .bind(&ids)
        .execute(tx.conn())
        .await?
        .rows_affected();
    repair_thread(tx, community, thread).await
}

async fn repair_thread(
    tx: &mut AdmittedTx,
    community: CommunityId,
    thread: &Candidate,
) -> Result<()> {
    // Preserve only the ancestry needed by retained events. The missing event
    // rows carry no content, author, tags or signature; metadata keeps replies
    // reachable without manufacturing a signed replacement event.
    sqlx::query(
        "WITH RECURSIVE needed AS (
          SELECT tm.* FROM thread_metadata tm
          WHERE tm.community_id=$1 AND tm.channel_id=$2 AND (tm.event_id=$3 OR tm.root_event_id=$3)
            AND EXISTS (SELECT 1 FROM events e WHERE e.community_id=tm.community_id
              AND e.id=tm.event_id AND e.created_at=tm.event_created_at)
          UNION
          SELECT p.* FROM thread_metadata p JOIN needed n ON p.community_id=n.community_id
            AND p.channel_id=n.channel_id AND (p.event_id=n.parent_event_id OR p.event_id=n.root_event_id)
        ) DELETE FROM thread_metadata tm WHERE tm.community_id=$1 AND tm.channel_id=$2
          AND (tm.event_id=$3 OR tm.root_event_id=$3)
          AND NOT EXISTS (SELECT 1 FROM needed n WHERE n.event_id=tm.event_id AND n.event_created_at=tm.event_created_at)",
    ).bind(community.as_uuid()).bind(thread.channel_id).bind(&thread.root_id).execute(tx.conn()).await?;
    // Recompute, never subtract old counts. Already-soft-deleted messages do
    // not count; received_at matches the server-arrival activity clock.
    sqlx::query(
        "WITH visible AS MATERIALIZED (
          SELECT tm.parent_event_id,tm.root_event_id,e.received_at FROM thread_metadata tm
          JOIN events e ON e.community_id=tm.community_id AND e.id=tm.event_id AND e.created_at=tm.event_created_at
          WHERE tm.community_id=$1 AND tm.channel_id=$2 AND tm.root_event_id=$3 AND e.deleted_at IS NULL
        ), direct AS (
          SELECT parent_event_id,count(*)::int AS n,max(received_at) AS last_at FROM visible GROUP BY parent_event_id
        ), descendants AS (
          SELECT root_event_id,count(*)::int AS n,max(received_at) AS last_at FROM visible GROUP BY root_event_id
        ), counts AS (
          SELECT tm.event_id,tm.event_created_at,COALESCE(d.n,0) AS replies,COALESCE(r.n,0) AS descendants,
            GREATEST(d.last_at,r.last_at) AS last_at
          FROM thread_metadata tm LEFT JOIN direct d ON d.parent_event_id=tm.event_id
          LEFT JOIN descendants r ON r.root_event_id=tm.event_id
          WHERE tm.community_id=$1 AND tm.channel_id=$2 AND (tm.event_id=$3 OR tm.root_event_id=$3)
        ) UPDATE thread_metadata tm SET reply_count=c.replies,descendant_count=c.descendants,last_reply_at=c.last_at
          FROM counts c WHERE tm.community_id=$1 AND tm.event_id=c.event_id AND tm.event_created_at=c.event_created_at",
    ).bind(community.as_uuid()).bind(thread.channel_id).bind(&thread.root_id).execute(tx.conn()).await?;
    Ok(())
}

// The source event may have been removed before this handler existed. A saved
// trigger author still identifies the copy without guessing from mention tags.
pub(super) async fn remove_orphan_runs(
    tx: &mut AdmittedTx,
    community: CommunityId,
    subject: &str,
    limit: u16,
) -> Result<u64> {
    let runs: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id,status::text FROM workflow_runs WHERE community_id=$1
         AND lower(trigger_context->>'author')=$2 ORDER BY id LIMIT $3 FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(subject)
    .bind(i64::from(limit))
    .fetch_all(tx.conn())
    .await?;
    if runs
        .iter()
        .any(|(_, status)| !matches!(status.as_str(), "completed" | "failed" | "cancelled"))
    {
        return Err(safety(
            "workflow runs triggered by this user must stop before deletion",
        ));
    }
    delete_runs(
        tx,
        community,
        &runs.into_iter().map(|(id, _)| id).collect::<Vec<_>>(),
    )
    .await
}

async fn delete_runs(tx: &mut AdmittedTx, community: CommunityId, run_ids: &[Uuid]) -> Result<u64> {
    // Retain the scheduled-fire claim: removing it would allow cron to fire
    // again. Only clear its optional reference to the run being removed.
    sqlx::query("UPDATE scheduled_workflow_fires SET workflow_run_id=NULL WHERE community_id=$1 AND workflow_run_id=ANY($2)")
        .bind(community.as_uuid()).bind(run_ids).execute(tx.conn()).await?;
    let deleted = sqlx::query("DELETE FROM workflow_runs WHERE community_id=$1 AND id=ANY($2)")
        .bind(community.as_uuid())
        .bind(run_ids)
        .execute(tx.conn())
        .await?
        .rows_affected();
    // workflow_approvals cascade from each removed run; the workflow definition
    // (possibly owned by a different user) is left intact.
    Ok(deleted)
}

// A reaction can outlive its signed source event in historical data.
pub(super) async fn remove_orphan_reactions(
    tx: &mut AdmittedTx,
    community: CommunityId,
    subject: &[u8],
    limit: u16,
) -> Result<u64> {
    Ok(sqlx::query(
        "DELETE FROM reactions WHERE (community_id,event_created_at,event_id,pubkey,emoji) IN (
         SELECT community_id,event_created_at,event_id,pubkey,emoji FROM reactions
         WHERE community_id = $1 AND pubkey = $2 LIMIT $3)",
    )
    .bind(community.as_uuid())
    .bind(subject)
    .bind(i64::from(limit))
    .execute(tx.conn())
    .await?
    .rows_affected())
}
