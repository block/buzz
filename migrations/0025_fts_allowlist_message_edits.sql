-- Expand the fresh-install FTS allowlist to include kind:40003 (message edits)
-- so edited bodies are discoverable via NIP-50 search.
--
-- Migration 0008 (checksum-frozen) allowlists only (0, 9, 40002, 45001, 45003).
-- Edits are separate events; without indexing them, searching for post-edit text
-- misses. Brownfield deny-list installs already index 40003 and are left alone.
--
-- PostgreSQL cannot alter a generated expression in place. When the allowlist
-- form is present (IN-list or ARRAY-normalized), rewrite the column while
-- preserving any wrappers added by later migrations (e.g. 0014's 30350 guard).
DO $$
DECLARE
    existing_expression TEXT;
    new_expression TEXT;
BEGIN
    SELECT pg_get_expr(d.adbin, d.adrelid)
      INTO existing_expression
      FROM pg_attrdef d
      JOIN pg_attribute a
        ON a.attrelid = d.adrelid
       AND a.attnum = d.adnum
     WHERE d.adrelid = 'events'::regclass
       AND a.attname = 'search_tsv';

    IF existing_expression IS NULL THEN
        RAISE EXCEPTION 'events.search_tsv generated expression not found';
    END IF;

    new_expression := replace(
        existing_expression,
        'kind IN (0, 9, 40002, 45001, 45003)',
        'kind IN (0, 9, 40002, 40003, 45001, 45003)'
    );
    new_expression := replace(
        new_expression,
        'ARRAY[0, 9, 40002, 45001, 45003]',
        'ARRAY[0, 9, 40002, 40003, 45001, 45003]'
    );

    IF new_expression = existing_expression THEN
        -- Already includes 40003, or this install uses the deny-list form.
        RETURN;
    END IF;

    ALTER TABLE events DROP COLUMN search_tsv;
    EXECUTE format(
        'ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (%s) STORED',
        new_expression
    );
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;
