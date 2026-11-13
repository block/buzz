-- Additive index of valid e references; signed event rows remain unchanged.
-- Large deployments prebuild partitions concurrently before this bounded step.
-- See docs/event-reference-index-deployment.md.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
SET LOCAL search_path = public, pg_catalog;
DO $migration$
BEGIN
    IF to_regclass('public.idx_events_reference_ids') IS NULL THEN
        CREATE INDEX idx_events_reference_ids ON public.events USING GIN (
    (lower(jsonb_path_query_array(tags,
        'strict $[*] ? (@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]',
        '{}'::jsonb, true)::text)::jsonb) jsonb_path_ops
);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid=to_regclass('public.idx_events_reference_ids')
          AND i.indrelid='public.events'::regclass
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid)=$definition$CREATE INDEX idx_events_reference_ids ON ONLY public.events USING gin (((lower((jsonb_path_query_array(tags, 'strict $[*]?(@[0] == "e" && @[1] like_regex "^[0-9A-Fa-f]{64}$")[1]'::jsonpath, '{}'::jsonb, true))::text))::jsonb) jsonb_path_ops)$definition$
    ) THEN
        RAISE EXCEPTION 'idx_events_reference_ids missing, invalid or wrong definition; see docs/event-reference-index-deployment.md';
    END IF;
END $migration$;
