-- Kind-0 profile search indexes the profile's text fields, not the whole JSON
-- document. `to_tsvector('simple', content)` over a kind-0 event tokenizes an
-- inline base64 avatar into thousands of lexemes, so a shared agent avatar can
-- make a hundred profiles match a name prefix that none of them carry and push
-- the person actually named that off the first page.
--
-- Bound the lock wait like 0049 and 0053: a long-running writer makes this
-- migration fail and the relay retry, instead of queueing every event insert
-- behind the LOCK TABLE below.
SET LOCAL lock_timeout = '5s';

-- This is the first migration to require PostgreSQL 16. `check_function_bodies`
-- validates the SQL body at CREATE FUNCTION, so on an older server the function
-- below would fail with a bare "function pg_input_is_valid(text, unknown) does
-- not exist" and the relay would refuse to start without saying why. Name the
-- real cause first.
DO $$
BEGIN
    IF current_setting('server_version_num')::int < 160000 THEN
        RAISE EXCEPTION 'Buzz requires PostgreSQL 16 or later (migration 0056 uses pg_input_is_valid); this server is %',
            current_setting('server_version');
    END IF;
END $$;

-- Requires PostgreSQL 16 or later: `pg_input_is_valid` guards the
-- `content::jsonb` cast so rows that predate ingest-side JSON validation never
-- raise. It is STABLE, so this IMMUTABLE SQL wrapper is what lets a
-- GENERATED ... STORED column call it; the declaration is truthful because
-- `jsonb_in` is IMMUTABLE and `CASE` never evaluates the cast for invalid
-- input. Invalid or non-object JSON falls back to today's raw-text vector, so
-- no row becomes less discoverable than it is now. Name fields weigh A,
-- contact fields B, `about` D; `picture`, `banner`, `image`, and unknown keys
-- are not indexed. Never DROP this function: once `events.search_tsv` depends
-- on it, a CASCADE drops the column. Never fix it in place either: CREATE OR
-- REPLACE recomputes nothing already stored, so a later change to this body
-- ships with its own maintenance rewrite, the same way this one does.
CREATE FUNCTION profile_search_tsv(content TEXT) RETURNS TSVECTOR
LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT CASE
        WHEN pg_input_is_valid(content, 'jsonb') THEN
            (SELECT CASE WHEN jsonb_typeof(j) = 'object' THEN
                    setweight(to_tsvector('simple'::regconfig, concat_ws(' ',
                        j ->> 'name', j ->> 'display_name', j ->> 'displayName')), 'A')
                 || setweight(to_tsvector('simple'::regconfig, concat_ws(' ',
                        j ->> 'nip05', j ->> 'lud16', j ->> 'website')), 'B')
                 || setweight(to_tsvector('simple'::regconfig,
                        coalesce(j ->> 'about', '')), 'D')
                 ELSE to_tsvector('simple'::regconfig, content) END
             FROM (SELECT content::jsonb AS j) AS parsed)
        ELSE to_tsvector('simple'::regconfig, content)
    END
$$;

-- Give new, empty installations the kind-0 arm without rewriting populated
-- databases during relay startup (same shape as 0008). Replacing a generated
-- column copies every partition of `events` and rebuilds the partitioned GIN
-- index under ACCESS EXCLUSIVE for as long as the copy takes, which a
-- Kubernetes startup probe can kill and repeat. Populated databases keep their
-- current expression until an operator runs the sized out-of-band maintenance
-- script in scripts/maintenance/profile_search_text_fields.sql (procedure in
-- docs/profile-search-deployment.md). Until then the query-side name-match
-- ordering in buzz-search keeps profile lookups usable, and the relay warns at
-- every startup that the rewrite is pending.
--
-- Serialize the emptiness check with event writers. Reads remain available on
-- populated databases; an actually empty table upgrades briefly to ACCESS
-- EXCLUSIVE for the generated-column replacement and index build.
LOCK TABLE events IN SHARE ROW EXCLUSIVE MODE;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM events LIMIT 1) THEN
        ALTER TABLE events DROP COLUMN search_tsv;
        ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (
            CASE WHEN kind = 0 THEN profile_search_tsv(content)
                 WHEN kind IN (9, 40002, 45001, 45003) THEN to_tsvector('simple', content)
                 ELSE NULL::tsvector
            END
        ) STORED;
        CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
    END IF;
END $$;
