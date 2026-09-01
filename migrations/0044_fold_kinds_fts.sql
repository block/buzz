-- Accumulator kinds 30640 (fold spec) and 4640 (artifact version) carry
-- NIP-44 ciphertext encrypted from the author's key to itself and are
-- author-only (AUTHOR_ONLY_KINDS). Exclude them from full-text search so
-- the ciphertext is never tokenized into search_tsv, where a NIP-50 search
-- from any member could otherwise probe it.
--
-- Same shape as 0014 (kind:30350) and 0033 (kind:30179): PostgreSQL cannot
-- alter a generated expression in place, so capture the current expression,
-- drop the column, and re-add it wrapped with the new exclusion. Every other
-- kind keeps whatever policy the database had before.
--
-- Operational cost: DROP COLUMN + ADD ... GENERATED ... STORED rewrites the
-- events heap and rebuilds the GIN index under an ACCESS EXCLUSIVE lock —
-- see the note on 0033. Operators with large brownfield databases should
-- schedule a window.
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
        'ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (CASE WHEN kind IN (4640, 30640) THEN NULL::tsvector ELSE (%s) END) STORED',
        existing_expression
    );
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;
