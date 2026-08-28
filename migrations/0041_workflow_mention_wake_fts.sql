-- Kind:44620 is a durable, recipient-gated workflow mention wake. Its canonical
-- content is empty, but keep the storage-level full-text-search backstop aligned
-- with every persistent P_GATED_KINDS member, including on brownfield databases
-- that retain the legacy negative skip-set.
--
-- Preserve the database's existing search policy for every other kind. As with
-- 0014 and 0033, replacing this generated column rewrites the events table and
-- rebuilds the GIN index under an ACCESS EXCLUSIVE lock.
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
        'ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (CASE WHEN kind = 44620 THEN NULL::tsvector ELSE (%s) END) STORED',
        existing_expression
    );
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;
