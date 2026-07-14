-- NIP-PL kind:30350 contains endpoint-bearing NIP-44 ciphertext and is
-- author-only. Exclude it from full-text search without changing the search
-- policy of existing installations. In particular, migration 0008 deliberately
-- gives only empty/fresh databases the positive allowlist; populated databases
-- retain their prior expression until an operator runs the out-of-band rewrite.
--
-- PostgreSQL cannot alter a generated expression in place. Capture the current
-- expression before replacing the column, then wrap it with the new exclusion.
-- This preserves both the fresh-install allowlist and any brownfield/operator-
-- managed expression for every kind other than 30350.
DO $$
DECLARE
    existing_expression TEXT;
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

    ALTER TABLE events DROP COLUMN search_tsv;
    EXECUTE format(
        'ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (CASE WHEN kind = 30350 THEN NULL::tsvector ELSE (%s) END) STORED',
        existing_expression
    );
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;
