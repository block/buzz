-- OUT-OF-BAND MAINTENANCE: do not run from relay startup migrations.
--
-- Route kind-0 (profile) rows of a populated database through
-- profile_search_tsv(content) so profile search indexes names, not inline
-- avatar bytes. Migration 0056 creates the function at relay startup and
-- rewrites the column only when `events` is empty; this script finishes the
-- job for every other database. Run it after the relay has started at least
-- once on 0056 (the guard below refuses otherwise):
--
--   psql -v ON_ERROR_STOP=1 -f scripts/maintenance/profile_search_text_fields.sql
--
-- docs/profile-search-deployment.md has the full procedure: who may run it,
-- which timeouts to lift, how to verify, and how to tell whether a database
-- still needs it (the relay also warns at startup until it has run).
--
-- It wraps whatever expression the database has today, so it is correct for
-- both shapes in the field: the fresh-install allowlist from 0008 (kind 0 is
-- one of ARRAY[0, 9, 40002, 45001, 45003]) and the brownfield blocklist from
-- 0001/0005 (kind 0 falls through to to_tsvector). Every other kind keeps the
-- policy the database already had. Re-running is a no-op once applied.
--
-- Cost: PostgreSQL cannot alter a generated expression in place. DROP COLUMN is
-- metadata-only, but ADD COLUMN ... GENERATED ... STORED copies every row of
-- every events partition evaluating the expression, then CREATE INDEX rebuilds
-- the partitioned GIN. All of it runs under ACCESS EXCLUSIVE on `events` in
-- one transaction, so relay reads and writes of events queue behind it.
-- Transient disk is roughly a second copy of the events heap and TOAST plus
-- the new GIN plus WAL. Time is dominated by tokenizing every indexed chat
-- row (about 20 MB of content per second single-threaded). Size it first:
--
--   SELECT kind, count(*), pg_size_pretty(sum(octet_length(content)))
--     FROM events GROUP BY kind ORDER BY 2 DESC;
--   SELECT pg_size_pretty(pg_total_relation_size('events'));
--
-- Rehearse on a staging clone, time it, then schedule production. The
-- lock_timeout makes a busy relay fail the script quickly rather than queue
-- traffic behind the lock wait; rerun when the table is quieter. The index is
-- recreated from the stock definition; non-stock indexes or storage
-- parameters on search_tsv are not captured or replayed.
BEGIN;
SET LOCAL lock_timeout = '5s';

DO $$
DECLARE
    existing_expression TEXT;
BEGIN
    IF to_regprocedure('profile_search_tsv(text)') IS NULL THEN
        RAISE EXCEPTION 'profile_search_tsv(text) is missing; start the relay once so migration 0056 creates it, then rerun';
    END IF;

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

    IF existing_expression LIKE '%profile_search_tsv(content)%' THEN
        RAISE NOTICE 'events.search_tsv already routes kind 0 through profile_search_tsv; nothing to do';
        RETURN;
    END IF;

    ALTER TABLE events DROP COLUMN search_tsv;
    EXECUTE format(
        'ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (CASE WHEN kind = 0 THEN profile_search_tsv(content) ELSE (%s) END) STORED',
        existing_expression
    );
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;

COMMIT;
