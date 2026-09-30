-- NIP-AO kind:24200 (Agent Observer Frame) is durable despite the 20xxx kind
-- number. Frames carry NIP-44 ciphertext (ACP transcripts / thinking) and are
-- #p-gated to the owner. Exclude them from full-text search without changing
-- the search policy of existing installations.
--
-- Same shape as 0033 (kind:30179): PostgreSQL cannot alter a generated
-- expression in place, so capture the current expression, drop the column,
-- and re-add it wrapped with the new exclusion. Every other kind keeps
-- whatever policy the database had before.
--
-- Fresh installs pick up 24200 via schema/schema.sql. This migration closes
-- the brownfield gap.
--
-- Operational cost: DROP COLUMN + ADD ... GENERATED ... STORED rewrites the
-- events heap and rebuilds the GIN index under ACCESS EXCLUSIVE lock. Schedule
-- a window on large brownfield databases.

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
        'ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (CASE WHEN kind = 24200 THEN NULL::tsvector ELSE (%s) END) STORED',
        existing_expression
    );
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;
