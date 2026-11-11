-- Upload ownership only. $1 is NULL for discovery, or the exact candidate
-- hashes for revalidation. Copied URLs never create or protect a candidate.
SELECT 'media_uploaders'::text AS source_table, community_id,
       jsonb_build_array(sha256, encode(pubkey,'hex')) AS source_key,
       encode(pubkey,'hex') AS subject, '{}'::jsonb AS payload,
       sha256 AS direct_hash
FROM media_uploaders
WHERE $1::text[] IS NULL OR sha256 = ANY($1)
UNION ALL
SELECT 'audit_log.media_uploaded', community_id, to_jsonb(seq),
       encode(actor_pubkey,'hex'), jsonb_build_object('sha256',detail->'sha256'),
       object_id
FROM audit_log
WHERE action = 'media_uploaded'
  AND ($1::text[] IS NULL OR object_id = ANY($1) OR detail->>'sha256' = ANY($1))
