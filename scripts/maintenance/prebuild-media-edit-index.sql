-- Run in psql, outside a transaction, before deploying migration 0086.
-- Let PostgreSQL generate child names, matching desired-state partition indexes.
\set ON_ERROR_STOP on
SET lock_timeout = '1s';
SET statement_timeout = '30min';
SELECT $ddl$CREATE INDEX idx_events_media_edit_target ON ONLY public.events
 (community_id,channel_id, (decode(lower(jsonb_path_query_first(tags, 'strict $[*] ? (@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]', '{}'::jsonb, true) #>> '{}'), 'hex')),created_at DESC,id DESC)
 WHERE kind=40003 AND deleted_at IS NULL$ddl$
WHERE to_regclass('public.idx_events_media_edit_target') IS NULL
\gexec
-- Concurrent operations wait for older transactions/snapshots. Keep the overall
-- statement budget, but do not cancel those waits after one second.
SET lock_timeout = 0;
-- A failed generated child keeps its name. Remove only unusable, unattached,
-- definition-matching builds so retries reuse desired-state child names.
SELECT format('DROP INDEX CONCURRENTLY %s', i.indexrelid::regclass)
FROM pg_index i JOIN pg_inherits h ON h.inhrelid=i.indrelid
WHERE h.inhparent='public.events'::regclass AND NOT i.indisvalid AND NOT i.indisunique
 AND NOT EXISTS (SELECT 1 FROM pg_inherits attached WHERE attached.inhrelid=i.indexrelid)
 AND substring(pg_get_indexdef(i.indexrelid) FROM ' USING .*') =
     substring(pg_get_indexdef('public.idx_events_media_edit_target'::regclass) FROM ' USING .*')
ORDER BY i.indexrelid
\gexec
SELECT format($ddl$CREATE INDEX CONCURRENTLY ON %s
 (community_id,channel_id, (decode(lower(jsonb_path_query_first(tags, 'strict $[*] ? (@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]', '{}'::jsonb, true) #>> '{}'), 'hex')),created_at DESC,id DESC)
 WHERE kind=40003 AND deleted_at IS NULL$ddl$,
 c.oid::regclass)
FROM pg_inherits h JOIN pg_class c ON c.oid=h.inhrelid
WHERE h.inhparent='public.events'::regclass
 AND NOT EXISTS (SELECT 1 FROM pg_inherits ix JOIN pg_index i ON i.indexrelid=ix.inhrelid
 WHERE ix.inhparent='public.idx_events_media_edit_target'::regclass AND i.indrelid=c.oid)
 AND NOT EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid=c.oid
   AND i.indisvalid AND i.indisready AND i.indislive
   AND substring(pg_get_indexdef(i.indexrelid) FROM ' USING .*') =
       substring(pg_get_indexdef('public.idx_events_media_edit_target'::regclass) FROM ' USING .*'))
ORDER BY c.oid
\gexec
-- Parent metadata operations must still acquire their locks promptly.
SET lock_timeout = '1s';
SELECT format('ALTER INDEX public.idx_events_media_edit_target ATTACH PARTITION %s', child.indexrelid::regclass)
FROM pg_inherits h JOIN pg_class c ON c.oid=h.inhrelid
CROSS JOIN LATERAL (SELECT i.indexrelid FROM pg_index i WHERE i.indrelid=c.oid
 AND i.indisvalid AND i.indisready AND i.indislive
 AND substring(pg_get_indexdef(i.indexrelid) FROM ' USING .*') =
     substring(pg_get_indexdef('public.idx_events_media_edit_target'::regclass) FROM ' USING .*')
 ORDER BY i.indexrelid LIMIT 1) child
WHERE h.inhparent='public.events'::regclass
 AND NOT EXISTS (SELECT 1 FROM pg_inherits ix JOIN pg_index i ON i.indexrelid=ix.inhrelid
 WHERE ix.inhparent='public.idx_events_media_edit_target'::regclass AND i.indrelid=c.oid)
ORDER BY c.oid
\gexec
-- Validate using the exact same bounded guard as startup (no table build here).
BEGIN;
\ir verify-media-edit-index.sql
COMMIT;
