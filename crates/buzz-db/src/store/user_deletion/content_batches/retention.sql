WITH RECURSIVE targets AS MATERIALIZED (
    SELECT decode(k->>'id','hex') AS id,(k->>'created_at')::timestamptz AS created_at
    FROM jsonb_array_elements($2::jsonb) k
), edges AS MATERIALIZED (
    SELECT t.id,t.created_at,decode(tag->>1,'hex') AS parent
    FROM targets t JOIN events e ON e.community_id=$1 AND e.id=t.id AND e.created_at=t.created_at
    CROSS JOIN LATERAL jsonb_array_elements(e.tags) tag
    WHERE tag->>0='e' AND tag->>1 ~ '^[0-9a-fA-F]{64}$'
    UNION
    SELECT t.id,t.created_at,p.parent FROM targets t
    JOIN thread_metadata tm ON tm.community_id=$1 AND tm.event_id=t.id AND tm.event_created_at=t.created_at
    CROSS JOIN LATERAL (VALUES (tm.parent_event_id),(tm.root_event_id)) p(parent)
    WHERE p.parent IS NOT NULL
), needed AS (
    SELECT t.id,t.created_at FROM targets t WHERE
      EXISTS (SELECT 1 FROM events e WHERE e.community_id=$1
        AND lower(jsonb_path_query_array(e.tags,
            'strict $[*] ? (@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]',
            '{}'::jsonb, true)::text)::jsonb @> jsonb_build_array(encode(t.id,'hex'))
        AND NOT EXISTS (SELECT 1 FROM targets x WHERE x.id=e.id AND x.created_at=e.created_at))
      OR EXISTS (SELECT 1 FROM thread_metadata tm JOIN events e
        ON e.community_id=tm.community_id AND e.id=tm.event_id AND e.created_at=tm.event_created_at
        WHERE tm.community_id=$1
          AND (tm.parent_event_id=t.id OR tm.root_event_id=t.id)
          AND NOT EXISTS (SELECT 1 FROM targets x WHERE x.id=e.id AND x.created_at=e.created_at))
      OR EXISTS (SELECT 1 FROM reactions r WHERE r.community_id=$1 AND r.event_id=t.id
        AND NOT(r.pubkey=ANY($3::bytea[])))
    UNION
    SELECT p.id,p.created_at FROM needed n
    JOIN edges e ON e.id=n.id AND e.created_at=n.created_at
    JOIN targets p ON p.id=e.parent
)
SELECT encode(id,'hex') FROM needed
