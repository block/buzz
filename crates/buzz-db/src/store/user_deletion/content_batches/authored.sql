-- Scope shared by source selection and the excluded-kind inventory.
-- Relay attribution is opt-in: only the supplied trusted signer may use actor/p.
SELECT e.* FROM events e
WHERE e.community_id = $1 AND (e.pubkey = ANY($2::bytea[]) OR e.pubkey = decode($3, 'hex'))
  AND CASE WHEN e.kind = 30622 THEN
    -- Hidden-DM state belongs to its viewer, not the relay signer. Require
    -- the producer's unambiguous subject shape; never infer it from actor,
    -- the first of several p tags, or a client-signed lookalike.
    CASE WHEN e.pubkey = decode($3, 'hex') AND e.channel_id IS NULL
      AND e.d_tag ~ '^[0-9a-f]{64}$' AND jsonb_typeof(e.tags) = 'array'
      AND NOT EXISTS (
        SELECT 1 FROM jsonb_array_elements(
          CASE WHEN jsonb_typeof(e.tags) = 'array' THEN e.tags ELSE '[]'::jsonb END
        ) tag WHERE jsonb_typeof(tag) <> 'array' OR EXISTS (
          SELECT 1 FROM jsonb_array_elements(
            CASE WHEN jsonb_typeof(tag) = 'array' THEN tag ELSE '[]'::jsonb END
          ) value WHERE jsonb_typeof(value) <> 'string'
        )
      )
      AND jsonb_path_query_array(e.tags, 'strict $[*] ? (@[0] == "d")', '{}'::jsonb, true)
        = jsonb_build_array(jsonb_build_array('d', e.d_tag))
      AND jsonb_path_query_array(e.tags, 'strict $[*] ? (@[0] == "p")', '{}'::jsonb, true)
        = jsonb_build_array(jsonb_build_array('p', e.d_tag))
    THEN decode(e.d_tag, 'hex') END
    WHEN e.pubkey = decode($3, 'hex') THEN COALESCE(
    (SELECT CASE WHEN t->>1 ~ '^[0-9a-fA-F]{64}$' THEN decode(t->>1, 'hex') END
     FROM jsonb_array_elements(e.tags) t WHERE t->>0 = 'actor' AND t->>1 IS NOT NULL LIMIT 1),
    (SELECT decode(t->>1, 'hex') FROM jsonb_array_elements(e.tags) t
     WHERE t->>0 = 'p' AND t->>1 ~ '^[0-9a-fA-F]{64}$' LIMIT 1), e.pubkey)
    ELSE e.pubkey END = ANY($2::bytea[])
  -- Only these message kinds and the checked private snapshot support relay attribution.
  AND (e.kind IN (7,9,30622,40001,40002,40003,40004,40005,40006,40007,40008,45001,45002,45003) OR e.pubkey = ANY($2::bytea[]))
