-- Additive. Large installations can prebuild and attach each partition index;
-- see docs/thread-window-deployment.md.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

DO $$
BEGIN
    IF to_regclass('public.idx_events_window_aux_e_tags') IS NULL THEN
        CREATE INDEX idx_events_window_aux_e_tags ON public.events USING gin (
            jsonb_path_query_array(tags, 'strict $[*] ? (@.type() == "array" && @[0] == "e" && @[1].type() == "string")[1]', '{}'::jsonb, true)
        ) WHERE kind IN (5, 7, 9005, 40003) AND (kind <> 7 OR (deleted_at IS NULL AND NOT reaction_read_projected));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid = to_regclass('public.idx_events_window_aux_e_tags')
          AND i.indisvalid AND i.indisready AND i.indislive
          AND pg_get_indexdef(i.indexrelid) =
              $definition$CREATE INDEX idx_events_window_aux_e_tags ON ONLY public.events USING gin (jsonb_path_query_array(tags, 'strict $[*]?((@.type() == "array" && @[0] == "e") && @[1].type() == "string")[1]'::jsonpath, '{}'::jsonb, true)) WHERE ((kind = ANY (ARRAY[5, 7, 9005, 40003])) AND ((kind <> 7) OR ((deleted_at IS NULL) AND (NOT reaction_read_projected))))$definition$
    ) THEN
        RAISE EXCEPTION 'idx_events_window_aux_e_tags invalid or wrong definition; see docs/thread-window-deployment.md';
    END IF;
END $$;
