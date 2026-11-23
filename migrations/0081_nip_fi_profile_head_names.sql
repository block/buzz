-- Profile name search matches verified profiles by name. Index the names in
-- each current head (crates/buzz-search/src/query.rs, verified_names_tsv).
-- Populated databases prebuild this index concurrently before this migration.
-- See docs/profile-search.md.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

DO $$
DECLARE
    expression CONSTANT TEXT := $expr$to_tsvector('simple'::regconfig, ((((COALESCE((jsonb_path_query_first(event, 'strict $."tags"[*]?(@[0] == "display_name")[1]'::jsonpath, '{}'::jsonb, true) #>> '{}'::text[]), ''::text) || ' '::text) || COALESCE((jsonb_path_query_first(event, 'strict $."tags"[*]?(@[0] == "username")[1]'::jsonpath, '{}'::jsonb, true) #>> '{}'::text[]), ''::text)) || ' '::text) || COALESCE((jsonb_path_query_first(event, 'strict $."tags"[*]?(@[0] == "full_name")[1]'::jsonpath, '{}'::jsonb, true) #>> '{}'::text[]), ''::text)))$expr$;
BEGIN
    IF to_regclass('public.idx_nip_fi_profile_heads_names') IS NULL THEN
        IF EXISTS (SELECT 1 FROM public.nip_fi_profile_heads LIMIT 1) THEN
            RAISE EXCEPTION 'prebuild idx_nip_fi_profile_heads_names concurrently; see docs/profile-search.md';
        END IF;
        EXECUTE format('CREATE INDEX idx_nip_fi_profile_heads_names ON public.nip_fi_profile_heads USING gin (%s)', expression);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index WHERE indexrelid = to_regclass('public.idx_nip_fi_profile_heads_names')
            AND indisvalid AND indisready AND indislive
            AND pg_get_indexdef(indexrelid) = format(
                'CREATE INDEX idx_nip_fi_profile_heads_names ON public.nip_fi_profile_heads USING gin (%s)',
                expression)
    ) THEN
        RAISE EXCEPTION 'idx_nip_fi_profile_heads_names is invalid or has a different definition';
    END IF;
END $$;
