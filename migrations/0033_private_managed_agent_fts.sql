-- NIP-PMA kind:30179 carries the owner's private managed-agent payload
-- (agent nsec, env vars, prompt) as NIP-44 ciphertext and is author-only.
-- Exclude it from full-text search without changing the search policy of
-- existing installations. Fresh installs are already safe via the positive
-- allowlist from migration 0008; this closes the brownfield gap where a
-- populated database still runs the legacy negative skip-set (0001/0005)
-- and would tokenize the ciphertext into search_tsv.
--
-- Same shape as 0014 (kind:30350): PostgreSQL cannot alter a generated
-- expression in place, so capture the current expression, drop the column,
-- and re-add it wrapped with the new exclusion. Every other kind keeps
-- whatever policy the database had before.
--
-- Operational cost: this is not free on large databases. DROP COLUMN +
-- ADD ... GENERATED ... STORED rewrites the entire events heap and then
-- rebuilds the GIN index, all under an ACCESS EXCLUSIVE lock inside the
-- migration transaction (CREATE INDEX CONCURRENTLY is not possible
-- here), with no lock_timeout. Expect relay downtime proportional to
-- the size of events. The index is recreated from the stock definition
-- below; any non-stock indexes or storage parameters on search_tsv are
-- not captured or replayed. 0014 set this precedent on smaller tables;
-- operators with large brownfield databases should schedule a window.
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
        'ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (CASE WHEN kind = 30179 THEN NULL::tsvector ELSE (%s) END) STORED',
        existing_expression
    );
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;
