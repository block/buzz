-- Run in psql, outside a transaction, before deploying migration 0089.
-- Let PostgreSQL generate child names, matching desired-state partition indexes.
\set ON_ERROR_STOP on
SET lock_timeout = '1s';
SET statement_timeout = '30min';
SELECT $ddl$CREATE INDEX idx_events_reference_ids ON ONLY public.events
 USING GIN ((lower(jsonb_path_query_array(tags, 'strict $[*] ? (@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]', '{}'::jsonb, true)::text)::jsonb) jsonb_path_ops)$ddl$
WHERE to_regclass('public.idx_events_reference_ids') IS NULL
\gexec
-- A same-named index of another shape must not drive cleanup or attachment.
DO $check$
BEGIN
    IF pg_get_indexdef('public.idx_events_reference_ids'::regclass) <> $definition$CREATE INDEX idx_events_reference_ids ON ONLY public.events USING gin (((lower((jsonb_path_query_array(tags, 'strict $[*]?(@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]'::jsonpath, '{}'::jsonb, true))::text))::jsonb) jsonb_path_ops)$definition$ THEN
        RAISE EXCEPTION 'idx_events_reference_ids has a different definition; inspect it before retrying';
    END IF;
END $check$;
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
     substring(pg_get_indexdef('public.idx_events_reference_ids'::regclass) FROM ' USING .*')
ORDER BY i.indexrelid
\gexec
SELECT format($ddl$CREATE INDEX CONCURRENTLY ON %s
 USING GIN ((lower(jsonb_path_query_array(tags, 'strict $[*] ? (@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]', '{}'::jsonb, true)::text)::jsonb) jsonb_path_ops)$ddl$,
 c.oid::regclass)
FROM pg_inherits h JOIN pg_class c ON c.oid=h.inhrelid
WHERE h.inhparent='public.events'::regclass
 AND NOT EXISTS (SELECT 1 FROM pg_inherits ix JOIN pg_index i ON i.indexrelid=ix.inhrelid
 WHERE ix.inhparent='public.idx_events_reference_ids'::regclass AND i.indrelid=c.oid)
 AND NOT EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid=c.oid
   AND i.indisvalid AND i.indisready AND i.indislive
   AND substring(pg_get_indexdef(i.indexrelid) FROM ' USING .*') =
       substring(pg_get_indexdef('public.idx_events_reference_ids'::regclass) FROM ' USING .*'))
ORDER BY c.oid
\gexec
-- Parent metadata operations must still acquire their locks promptly.
SET lock_timeout = '1s';
SELECT format('ALTER INDEX public.idx_events_reference_ids ATTACH PARTITION %s', child.indexrelid::regclass)
FROM pg_inherits h JOIN pg_class c ON c.oid=h.inhrelid
CROSS JOIN LATERAL (SELECT i.indexrelid FROM pg_index i WHERE i.indrelid=c.oid
 AND i.indisvalid AND i.indisready AND i.indislive
 AND substring(pg_get_indexdef(i.indexrelid) FROM ' USING .*') =
     substring(pg_get_indexdef('public.idx_events_reference_ids'::regclass) FROM ' USING .*')
 ORDER BY i.indexrelid LIMIT 1) child
WHERE h.inhparent='public.events'::regclass
 AND NOT EXISTS (SELECT 1 FROM pg_inherits ix JOIN pg_index i ON i.indexrelid=ix.inhrelid
 WHERE ix.inhparent='public.idx_events_reference_ids'::regclass AND i.indrelid=c.oid)
ORDER BY c.oid
\gexec
-- Validate using the exact same bounded guard as startup (no table build here).
BEGIN;
\ir ../../migrations/0089_event_reference_index.sql
COMMIT;
