-- Attach the universal fence to every existing table carrying community_id,
-- including deployment-private sidecars whose community_id is provenance.
--
-- Desired-state schema application does not replay migration history, so the
-- explicit `SELECT attach_community_write_fence('<table>')` calls in
-- `triggers/<table>.sql` stay as first-class catalog declarations. They also
-- make the fence contract visible to migration linting instead of hiding it
-- only in this dynamic bootstrap loop. This loop runs last, so it fences every
-- desired-state table regardless: a fenced desired state does not prove the
-- migration path attaches the fence. Keep the explicit calls.
DO $$
DECLARE
    target REGCLASS;
BEGIN
    FOR target IN
        SELECT c.oid::REGCLASS
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
          JOIN pg_attribute a ON a.attrelid = c.oid
         WHERE n.nspname = current_schema()
           AND c.relkind IN ('r', 'p')
           AND NOT c.relispartition
           AND a.attname = 'community_id'
           AND NOT a.attisdropped
           AND NOT community_write_fence_excluded_table(c.relname)
         ORDER BY c.oid::REGCLASS::TEXT
    LOOP
        PERFORM attach_community_write_fence(target);
    END LOOP;
END
$$;
