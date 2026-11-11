-- Scope shared by source selection and the excluded-kind inventory.
-- Relay attribution is opt-in: only the supplied trusted signer may use actor/p.
SELECT e.* FROM events e
WHERE e.community_id = $1 AND e.pubkey IN (decode($2, 'hex'), decode($3, 'hex'))
  AND CASE WHEN e.pubkey = decode($3, 'hex') THEN COALESCE(
    (SELECT CASE WHEN t->>1 ~ '^[0-9a-fA-F]{64}$' THEN decode(t->>1, 'hex') END
     FROM jsonb_array_elements(e.tags) t WHERE t->>0 = 'actor' AND t->>1 IS NOT NULL LIMIT 1),
    (SELECT decode(t->>1, 'hex') FROM jsonb_array_elements(e.tags) t
     WHERE t->>0 = 'p' AND t->>1 ~ '^[0-9a-fA-F]{64}$' LIMIT 1), e.pubkey)
    ELSE e.pubkey END = decode($2, 'hex')
  -- Only the original message kinds support trusted relay attribution.
  AND (e.kind IN (7,9,40001,40002,40003,40004,40005,40006,40007,40008,45001,45002,45003) OR e.pubkey = decode($2, 'hex'))
