-- OUT-OF-BAND MAINTENANCE: do not run from relay startup migrations.
--
-- This rewrites every events partition and rebuilds the partitioned GIN index.
-- Run only in a maintenance window after confirming enough free space for the
-- replacement heap/TOAST/index files plus WAL. ALTER TABLE takes ACCESS
-- EXCLUSIVE, so event reads and writes block until this transaction commits.
-- Consider combining this with the planned partition repack/reclaim operation.
--
-- The result is the expression migration 0056 gives empty installations:
-- kind 0 routes through profile_search_tsv(content) (created by 0056 at relay
-- startup, so run this after the relay has started on 0056; the guard below
-- refuses otherwise). That keeps this rewrite from undoing
-- scripts/maintenance/profile_search_text_fields.sql.
BEGIN;
SET LOCAL lock_timeout = '5s';

-- Fail with an instruction before ALTER TABLE takes ACCESS EXCLUSIVE, rather
-- than inside the rewrite with a bare "function does not exist".
DO $$
BEGIN
    IF to_regprocedure('profile_search_tsv(text)') IS NULL THEN
        RAISE EXCEPTION 'profile_search_tsv(text) is missing; start the relay once so migration 0056 creates it, then rerun';
    END IF;
END $$;

ALTER TABLE events DROP COLUMN search_tsv;
ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (
    CASE WHEN kind = 0 THEN profile_search_tsv(content)
         WHEN kind IN (9, 40002, 45001, 45003) THEN to_tsvector('simple', content)
         ELSE NULL::tsvector
    END
) STORED;
CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);

COMMIT;
