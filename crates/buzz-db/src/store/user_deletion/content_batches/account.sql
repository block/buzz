WITH removed AS MATERIALIZED (
    SELECT tm.* FROM content_batch_work w
    JOIN events e ON e.community_id=$1 AND e.id=decode(w.row_key->>'id','hex')
        AND e.created_at=(w.row_key->>'created_at')::timestamptz AND e.deleted_at IS NULL
    JOIN thread_metadata tm ON tm.community_id=$1 AND tm.event_id=e.id AND tm.event_created_at=e.created_at
    WHERE w.table_name='events' AND w.batch=$2
), direct AS (
    SELECT parent_event_id AS id,count(*)::int AS n FROM removed
    WHERE parent_event_id IS NOT NULL GROUP BY parent_event_id
), descendants AS (
    SELECT root_event_id AS id,count(*)::int AS n FROM removed
    WHERE root_event_id IS NOT NULL GROUP BY root_event_id
), changes AS (
    SELECT COALESCE(d.id,r.id) AS id,COALESCE(d.n,0) AS direct,COALESCE(r.n,0) AS descendants
    FROM direct d FULL JOIN descendants r USING(id)
)
UPDATE thread_metadata tm SET reply_count=GREATEST(0,tm.reply_count-c.direct),
    descendant_count=GREATEST(0,tm.descendant_count-c.descendants)
FROM changes c WHERE tm.community_id=$1 AND tm.event_id=c.id
