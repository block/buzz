-- Bind $1 to the community UUID. Counts newly edit-governed originals whose
-- selected live edit omits imeta; independent grants can still authorize blobs.
SELECT original.kind, count(*) AS affected_originals
FROM events original
JOIN channels c ON c.community_id = original.community_id
 AND c.id = original.channel_id AND c.deleted_at IS NULL
JOIN LATERAL (
  SELECT e.tags
  FROM events e
  WHERE e.community_id = original.community_id
    AND e.channel_id = original.channel_id
    AND e.kind = 40003 AND e.deleted_at IS NULL
    AND decode(lower(jsonb_path_query_first(tags, 'strict $[*] ? (@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]', '{}'::jsonb, true) #>> '{}'), 'hex') = original.id
  ORDER BY e.created_at DESC, e.id DESC
  LIMIT 1
) latest ON true
WHERE original.community_id = $1
  AND original.kind IN (40008,45001,45003) AND original.deleted_at IS NULL
  AND EXISTS (SELECT 1 FROM jsonb_array_elements(original.tags) t WHERE t->>0 = 'imeta')
  AND NOT EXISTS (SELECT 1 FROM jsonb_array_elements(latest.tags) t WHERE t->>0 = 'imeta')
GROUP BY original.kind
ORDER BY original.kind;
