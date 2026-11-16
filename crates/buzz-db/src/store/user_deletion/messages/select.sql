-- $1 community, $2 subject, $3 trusted relay signer, $4 optional root,
-- $5 limit, $6 optional channel (used only with $4).
WITH authored AS (
    SELECT e.*
    FROM events e
    WHERE e.community_id = $1 AND e.pubkey IN ($2, $3)
      AND e.kind IN (9, 40001, 40002, 45001, 45003, 7, 45002, 40003, 40004, 40005, 40006, 40007, 40008)
      AND CASE WHEN e.pubkey = $3 THEN COALESCE(
        -- Match effective_message_author: first actor with a value, then first valid p.
        (SELECT CASE WHEN t->>1 ~ '^[0-9a-fA-F]{64}$' THEN decode(t->>1, 'hex') END
         FROM jsonb_array_elements(e.tags) t WHERE t->>0 = 'actor' AND t->>1 IS NOT NULL LIMIT 1),
        (SELECT decode(t->>1, 'hex') FROM jsonb_array_elements(e.tags) t
         WHERE t->>0 = 'p' AND t->>1 ~ '^[0-9a-fA-F]{64}$' LIMIT 1), e.pubkey)
        ELSE e.pubkey END = $2
), located AS (
    SELECT e.id, e.created_at, e.channel_id,
      CASE WHEN e.kind IN (9, 40001, 40002, 45001, 45003, 40006, 40007) THEN
        COALESCE(tm.root_event_id,
          CASE WHEN markers.reply IS NOT NULL THEN COALESCE(markers.root, parent.root_event_id, markers.reply) END, e.id)
      ELSE COALESCE(target.root_event_id,
        CASE WHEN target_markers.reply IS NOT NULL THEN COALESCE(target_markers.root, target_markers.reply) END,
        refs.target_id, e.id) END AS root_id,
      -- Deliberately conservative until shared-file cleanup is available. Never
      -- discard the only source of a blob/preview reference and claim success.
      (e.content ~* 'https?://|/media/' OR EXISTS (
        SELECT 1 FROM jsonb_array_elements(e.tags) t
        WHERE t->>0 IN ('imeta', 'link-preview', 'url', 'thumb', 'image', 'x', 'emoji')
      )) AS has_media
    FROM authored e
    LEFT JOIN thread_metadata tm ON tm.community_id = e.community_id
      AND tm.event_id = e.id AND tm.event_created_at = e.created_at AND tm.channel_id = e.channel_id
    LEFT JOIN LATERAL (
      SELECT
        (array_agg(decode(t->>1, 'hex') ORDER BY n DESC) FILTER (WHERE t->>3 = 'root'))[1] AS root,
        (array_agg(decode(t->>1, 'hex') ORDER BY n DESC) FILTER (WHERE t->>3 = 'reply'))[1] AS reply
      FROM jsonb_array_elements(e.tags) WITH ORDINALITY a(t,n)
      WHERE t->>0 = 'e' AND t->>1 ~ '^[0-9a-fA-F]{64}$'
    ) markers ON true
    LEFT JOIN LATERAL (
      SELECT root_event_id FROM thread_metadata p WHERE p.community_id = e.community_id
        AND p.channel_id = e.channel_id AND p.event_id = markers.reply LIMIT 1
    ) parent ON true
    LEFT JOIN LATERAL (
      SELECT decode(t->>1, 'hex') AS target_id
      FROM jsonb_array_elements(e.tags) WITH ORDINALITY a(t,n)
      WHERE t->>0 = 'e' AND t->>1 ~ '^[0-9a-fA-F]{64}$'
      ORDER BY CASE WHEN e.kind IN (40003,45002) THEN n ELSE -n END LIMIT 1
    ) refs ON true
    LEFT JOIN LATERAL (
      SELECT root_event_id FROM thread_metadata p WHERE p.community_id = e.community_id
        AND p.channel_id = e.channel_id AND p.event_id = refs.target_id LIMIT 1
    ) target ON true
    LEFT JOIN LATERAL (
      SELECT p.tags FROM events p WHERE p.community_id = e.community_id
        AND p.channel_id IS NOT DISTINCT FROM e.channel_id AND p.id = refs.target_id LIMIT 1
    ) target_event ON true
    LEFT JOIN LATERAL (
      SELECT
        (array_agg(decode(t->>1, 'hex') ORDER BY n DESC) FILTER (WHERE t->>3 = 'root'))[1] AS root,
        (array_agg(decode(t->>1, 'hex') ORDER BY n DESC) FILTER (WHERE t->>3 = 'reply'))[1] AS reply
      FROM jsonb_array_elements(target_event.tags) WITH ORDINALITY a(t,n)
      WHERE t->>0 = 'e' AND t->>1 ~ '^[0-9a-fA-F]{64}$'
    ) target_markers ON true
)
SELECT id, created_at, channel_id, root_id, has_media FROM located
WHERE ($4::bytea IS NULL OR (root_id = $4 AND channel_id IS NOT DISTINCT FROM $6::uuid))
ORDER BY root_id, channel_id NULLS FIRST, created_at, id LIMIT $5
