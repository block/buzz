WITH removed AS MATERIALIZED (
    SELECT decode(row_key->>'id','hex') AS id,(row_key->>'created_at')::timestamptz AS created_at
    FROM content_batch_work WHERE table_name='events' AND batch=$2 AND action='would_delete'
)
SELECT EXISTS(
    SELECT 1 FROM removed r WHERE
    EXISTS(SELECT 1 FROM events e WHERE e.community_id=$1
        AND lower(jsonb_path_query_array(e.tags,
            'strict $[*] ? (@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]',
            '{}'::jsonb, true)::text)::jsonb @> jsonb_build_array(encode(r.id,'hex'))
        AND NOT EXISTS(SELECT 1 FROM removed x WHERE x.id=e.id AND x.created_at=e.created_at))
    OR EXISTS(SELECT 1 FROM thread_metadata tm WHERE tm.community_id=$1
        AND (tm.parent_event_id=r.id OR tm.root_event_id=r.id)
        AND NOT EXISTS(SELECT 1 FROM removed x WHERE x.id=tm.event_id AND x.created_at=tm.event_created_at))
    OR EXISTS(SELECT 1 FROM reactions rx WHERE rx.community_id=$1 AND rx.event_id=r.id)
)
